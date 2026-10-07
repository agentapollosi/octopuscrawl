//! Sponsored hatchlings. A holder sends 150,000 $OCTO to the treasury from their
//! own wallet; the server — reading the chain only, never signing anything —
//! finds that transfer and spawns a hatchling owned by the sending wallet. Each
//! transfer signature is claimed once; amounts above a multiple of the price are
//! kept as credit toward the next hatchling.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use octopuscrawl_core::{Crawler, CrawlerKind, CrawlerStatus, LiveMsg, SpawnResult, SponsorView};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::Mutex;

use crate::treasury::{mint, rpc, wallet as treasury_wallet};
use crate::AppState;

/// $OCTO per hatchling.
pub const PRICE: f64 = 150_000.0;
const FILE: &str = "data/sponsors.json";
/// Recent transfers into the treasury that are checked per request.
const SCAN: usize = 100;
/// At most this many hatchlings per wallet (extra $OCTO stays as credit).
const MAX_PER_WALLET: usize = 10;
/// Sponsorship opened at this moment (unix seconds). Earlier transfers into the
/// treasury — e.g. the project's own allocation — are not sponsorships.
const OPENS_AT: i64 = 1791400800;

const NAMES: &[&str] = &[
    "reef-pup", "ink-sprout", "tide-hatch", "coral-kit", "brine-tot", "kelp-cub", "shoal-imp", "drift-pip",
];

#[derive(Clone, Serialize, Deserialize)]
pub struct Sponsor {
    pub wallet: String,
    pub crawler_id: String,
    pub name: String,
    pub sigs: Vec<String>,
    pub since: DateTime<Utc>,
    pub pages: u64,
    pub tokens: u64,
}

#[derive(Default, Serialize, Deserialize)]
pub struct Book {
    pub sponsors: Vec<Sponsor>,
    /// Transfer signatures already turned into hatchlings / credit.
    pub claimed: HashSet<String>,
    /// $OCTO credit per wallet, below the price of a hatchling.
    pub credit: HashMap<String, f64>,
}

impl Book {
    pub fn load() -> Self {
        std::fs::read_to_string(FILE)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        if let Ok(body) = serde_json::to_string(self) {
            let tmp = format!("{FILE}.tmp");
            let _ = std::fs::create_dir_all("data");
            if std::fs::write(&tmp, body).is_ok() {
                let _ = std::fs::rename(&tmp, FILE);
            }
        }
    }

    pub fn views(&self) -> Vec<SponsorView> {
        self.sponsors
            .iter()
            .map(|s| SponsorView {
                wallet: s.wallet.clone(),
                crawler_id: s.crawler_id.clone(),
                name: s.name.clone(),
                pages: s.pages,
                tokens: s.tokens,
                since: s.since,
            })
            .collect()
    }

    /// Credit a new page read by a sponsored hatchling to its owner's tally.
    pub fn credit_read(&mut self, crawler_id: &str, tokens: u32) -> bool {
        match self.sponsors.iter_mut().find(|s| s.crawler_id == crawler_id) {
            Some(s) => {
                s.pages += 1;
                s.tokens += tokens as u64;
                true
            }
            None => false,
        }
    }
}

/// The hatchling a sponsor record stands for (as it appears on the board).
pub fn crawler_for(s: &Sponsor) -> Crawler {
    let now = crate::clock::now();
    Crawler {
        id: s.crawler_id.clone(),
        name: s.name.clone(),
        kind: CrawlerKind::Owned,
        owner: Some(s.wallet.clone()),
        status: CrawlerStatus::Idle,
        url: String::new(),
        title: String::new(),
        target: None,
        pages_read: s.pages,
        thought: "hatched — waiting for its first page".to_string(),
        relevance: 0.0,
        started_at: s.since,
        updated_at: now,
        links_seq: 0,
        links: Vec::new(),
    }
}

/// A plausible Solana address: base58, 32–44 characters.
fn valid_wallet(w: &str) -> bool {
    (32..=44).contains(&w.len())
        && w.chars().all(|c| c.is_ascii_alphanumeric() && !matches!(c, '0' | 'O' | 'I' | 'l'))
}

/// What a transaction did with the token: per-owner deltas, plus who signed it.
/// Cached — a finalized transaction never changes.
#[derive(Clone, Default)]
struct TxFacts {
    deltas: Vec<(String, f64)>,
    signers: Vec<String>,
}

fn tx_cache() -> &'static Mutex<HashMap<String, TxFacts>> {
    static C: std::sync::OnceLock<Mutex<HashMap<String, TxFacts>>> = std::sync::OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

fn signers(tx: &Value) -> Vec<String> {
    tx.pointer("/transaction/message/accountKeys")
        .and_then(|a| a.as_array())
        .map(|keys| {
            keys.iter()
                .filter(|k| k.get("signer").and_then(|s| s.as_bool()).unwrap_or(false))
                .filter_map(|k| k.get("pubkey").and_then(|p| p.as_str()).map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn deltas(tx: &Value, mint: &str) -> Vec<(String, f64)> {
    let mut by_owner: HashMap<String, f64> = HashMap::new();
    for (key, sign) in [("preTokenBalances", -1.0), ("postTokenBalances", 1.0)] {
        let Some(arr) = tx.pointer(&format!("/meta/{key}")).and_then(|a| a.as_array()) else { continue };
        for b in arr {
            if b.get("mint").and_then(|m| m.as_str()) != Some(mint) {
                continue;
            }
            let owner = b.get("owner").and_then(|o| o.as_str()).unwrap_or("").to_string();
            let amt = b.pointer("/uiTokenAmount/uiAmountString").and_then(|a| a.as_str()).and_then(|a| a.parse::<f64>().ok()).unwrap_or(0.0);
            *by_owner.entry(owner).or_insert(0.0) += sign * amt;
        }
    }
    by_owner.into_iter().collect()
}

/// New (unclaimed) transfers of the token from `from` into the treasury:
/// (signature, amount received by the treasury).
async fn find_transfers(from: &str, claimed: &HashSet<String>) -> Result<Vec<(String, f64)>, String> {
    let client = reqwest::Client::new();
    let (treasury, mint) = (treasury_wallet(), mint());
    let accounts = rpc(&client, "getTokenAccountsByOwner", json!([treasury, { "mint": mint }, { "encoding": "jsonParsed" }]))
        .await
        .ok_or("could not reach Solana — try again in a minute")?;
    let accounts: Vec<String> = accounts
        .get("value")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|x| x.get("pubkey").and_then(|p| p.as_str()).map(str::to_string)).collect())
        .unwrap_or_default();
    let mut out = Vec::new();
    for acct in accounts {
        let sigs = rpc(&client, "getSignaturesForAddress", json!([acct, { "limit": SCAN }]))
            .await
            .ok_or("could not reach Solana — try again in a minute")?;
        let sigs: Vec<String> = sigs
            .as_array()
            .map(|a| {
                a.iter()
                    .filter(|s| s.get("err").map_or(true, |e| e.is_null()))
                    .filter(|s| s.get("blockTime").and_then(|t| t.as_i64()).map_or(false, |t| t >= OPENS_AT))
                    .filter_map(|s| s.get("signature").and_then(|x| x.as_str()).map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        for sig in sigs {
            if claimed.contains(&sig) {
                continue;
            }
            let cached = tx_cache().lock().await.get(&sig).cloned();
            let facts = match cached {
                Some(f) => f,
                None => {
                    let Some(tx) = rpc(
                        &client,
                        "getTransaction",
                        json!([sig, { "encoding": "jsonParsed", "maxSupportedTransactionVersion": 0, "commitment": "finalized" }]),
                    )
                    .await
                    else {
                        continue; // not finalized yet, or rate-limited — next request will see it
                    };
                    let f = TxFacts { deltas: deltas(&tx, &mint), signers: signers(&tx) };
                    tx_cache().lock().await.insert(sig.clone(), f.clone());
                    tokio::time::sleep(Duration::from_millis(120)).await;
                    f
                }
            };
            // a sponsorship is a transfer the sender signed themselves — a pool or
            // curve that merely sold tokens to the treasury is not a sponsor
            if !facts.signers.iter().any(|s| s == from) {
                continue;
            }
            let d = &facts.deltas;
            let sent = d.iter().find(|(o, _)| o == from).map_or(0.0, |(_, v)| -v);
            let got = d.iter().find(|(o, _)| *o == treasury).map_or(0.0, |(_, v)| *v);
            if sent > 0.0 && got > 0.0 {
                out.push((sig, sent.min(got)));
            }
        }
    }
    Ok(out)
}

/// Verify a wallet's transfers to the treasury and spawn its hatchlings.
pub async fn claim(state: &Arc<AppState>, wallet: &str) -> SpawnResult {
    // one verification at a time, and not more than one every few seconds —
    // the public RPC is shared
    static GATE: std::sync::OnceLock<Mutex<Option<Instant>>> = std::sync::OnceLock::new();
    let gate = GATE.get_or_init(|| Mutex::new(None));
    let mut last = gate.lock().await;
    if last.map_or(false, |t| t.elapsed() < Duration::from_secs(3)) {
        return SpawnResult { message: "busy verifying another wallet — try again in a few seconds".into(), ..Default::default() };
    }
    *last = Some(Instant::now());

    let wallet = wallet.trim();
    if !valid_wallet(wallet) {
        return SpawnResult { message: "that does not look like a Solana wallet address".into(), ..Default::default() };
    }
    if wallet == treasury_wallet() {
        return SpawnResult { message: "that is the treasury itself — paste the wallet you sent from".into(), ..Default::default() };
    }
    let claimed = state.sponsors.read().await.claimed.clone();
    let found = match find_transfers(wallet, &claimed).await {
        Ok(f) => f,
        Err(e) => return SpawnResult { message: e, ..Default::default() },
    };
    let found_total: f64 = found.iter().map(|(_, a)| a).sum();

    let mut book = state.sponsors.write().await;
    // re-check under the lock: a signature claimed meanwhile is skipped
    let fresh: Vec<(String, f64)> = found.into_iter().filter(|(s, _)| !book.claimed.contains(s)).collect();
    let mut credit = book.credit.get(wallet).copied().unwrap_or(0.0) + fresh.iter().map(|(_, a)| a).sum::<f64>();
    let sigs: Vec<String> = fresh.iter().map(|(s, _)| s.clone()).collect();
    for s in &sigs {
        book.claimed.insert(s.clone());
    }
    let mut spawned = Vec::new();
    let mut new_crawlers = Vec::new();
    let owned = book.sponsors.iter().filter(|s| s.wallet == wallet).count();
    while credit + 1e-6 >= PRICE && owned + spawned.len() < MAX_PER_WALLET {
        credit -= PRICE;
        let n = book.sponsors.len() + 1;
        let tail: String = wallet.chars().take(4).collect::<String>().to_ascii_lowercase();
        let s = Sponsor {
            wallet: wallet.to_string(),
            crawler_id: format!("own-{n:03}"),
            name: format!("{}-{tail}", NAMES[(n - 1) % NAMES.len()]),
            sigs: sigs.clone(),
            since: crate::clock::now(),
            pages: 0,
            tokens: 0,
        };
        new_crawlers.push(crawler_for(&s));
        spawned.push(s.name.clone());
        book.sponsors.push(s);
    }
    book.credit.insert(wallet.to_string(), credit.max(0.0));
    book.save();
    drop(book);

    if !new_crawlers.is_empty() {
        let mut snap = state.snapshot.write().await;
        for c in &new_crawlers {
            snap.crawlers.push(c.clone());
        }
        snap.stats.crawlers_total = snap.crawlers.len() as u32;
        drop(snap);
        for c in new_crawlers {
            let _ = state.tx.send(LiveMsg::Crawler { crawler: c });
        }
        println!("[sponsor] {wallet} spawned {}", spawned.join(", "));
    }

    let at_cap = owned + spawned.len() >= MAX_PER_WALLET;
    let message = if !spawned.is_empty() {
        format!(
            "spawned {} — live on the board now{}",
            spawned.join(", "),
            if credit > 0.5 { format!(" · {} $OCTO credit kept for the next one", fmt(credit)) } else { String::new() }
        )
    } else if at_cap {
        format!("this wallet already runs {MAX_PER_WALLET} hatchlings, the current limit — {} $OCTO kept as credit", fmt(credit))
    } else if fresh.is_empty() && credit < 0.5 {
        format!(
            "no new $OCTO transfer from this wallet to the treasury yet (checked the last {SCAN}). If you just sent it, wait ~30 seconds for it to finalize."
        )
    } else {
        format!("{} $OCTO credited — {} more spawns a hatchling", fmt(credit), fmt(PRICE - credit))
    };
    SpawnResult { ok: !spawned.is_empty(), message, spawned, found: found_total, credit }
}

fn fmt(v: f64) -> String {
    let n = v.round() as u64;
    let s = n.to_string();
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wallet_check() {
        assert!(valid_wallet("APoLLoJvoZSDerPhbtyBvoAcZNgkVpG4aDeH12gFv7rH"));
        assert!(!valid_wallet("0xabc"));
        assert!(!valid_wallet("APoLLoJvoZSDerPhbtyBvoAcZNgkVpG4aDeH12gFv7r0"));
    }

    #[test]
    fn token_deltas_by_owner() {
        let tx = json!({"meta": {
            "preTokenBalances": [
                {"mint": "M", "owner": "A", "uiTokenAmount": {"uiAmountString": "200000"}},
                {"mint": "M", "owner": "T", "uiTokenAmount": {"uiAmountString": "10"}}
            ],
            "postTokenBalances": [
                {"mint": "M", "owner": "A", "uiTokenAmount": {"uiAmountString": "50000"}},
                {"mint": "M", "owner": "T", "uiTokenAmount": {"uiAmountString": "150010"}},
                {"mint": "X", "owner": "T", "uiTokenAmount": {"uiAmountString": "999"}}
            ]
        }});
        let d: HashMap<String, f64> = deltas(&tx, "M").into_iter().collect();
        assert_eq!(d["A"], -150000.0);
        assert_eq!(d["T"], 150000.0);
    }

    #[test]
    fn signer_list() {
        let tx = json!({"transaction": {"message": {"accountKeys": [
            {"pubkey": "A", "signer": true, "writable": true},
            {"pubkey": "T", "signer": false, "writable": true}
        ]}}});
        assert_eq!(signers(&tx), vec!["A".to_string()]);
    }
}
