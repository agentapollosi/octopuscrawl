//! Treasury: reads the project's Solana wallet on-chain (balance + recent
//! transactions) via public JSON-RPC. Read-only — it never signs or sends
//! anything. The wallet and RPC are configurable via env.

use std::sync::Arc;
use std::time::Duration;

use octopuscrawl_core::{LiveMsg, TokenInfo, TreasurySnapshot, TreasuryTx};
use serde_json::{json, Value};

use crate::AppState;

const RPC_DEFAULT: &str = "https://api.mainnet-beta.solana.com";
const WALLET_DEFAULT: &str = "APoLLoJvoZSDerPhbtyBvoAcZNgkVpG4aDeH12gFv7rH";
const WINDOW: usize = 12; // recent signatures to detail
const MINT_DEFAULT: &str = "EdXrLt2PMZF4wwAsytQRAryG3eNy3dtLo6rBGKMMpump";
const SYMBOL: &str = "OCTO";

fn rpc_url() -> String {
    std::env::var("OCTOPUSCRAWL_SOLANA_RPC").unwrap_or_else(|_| RPC_DEFAULT.to_string())
}
pub fn mint() -> String {
    std::env::var("OCTOPUSCRAWL_TOKEN_MINT").unwrap_or_else(|_| MINT_DEFAULT.to_string())
}

/// The token's mint account (supply + authorities) and the treasury's holding.
async fn fetch_token(client: &reqwest::Client, owner: &str) -> TokenInfo {
    let mint = mint();
    let mut t = TokenInfo { mint: mint.clone(), symbol: SYMBOL.to_string(), ..Default::default() };
    if let Some(r) = rpc(client, "getAccountInfo", json!([mint, { "encoding": "jsonParsed" }])).await {
        if let Some(info) = r.pointer("/value/data/parsed/info") {
            let dec = info.get("decimals").and_then(|d| d.as_u64()).unwrap_or(0) as u8;
            let raw = info.get("supply").and_then(|x| x.as_str()).and_then(|x| x.parse::<f64>().ok()).unwrap_or(0.0);
            t.decimals = dec;
            t.supply = raw / 10f64.powi(dec as i32);
            t.mint_authority = info.get("mintAuthority").and_then(|a| a.as_str()).map(str::to_string);
            t.freeze_authority = info.get("freezeAuthority").and_then(|a| a.as_str()).map(str::to_string);
            t.ok = true;
        }
    }
    if let Some(r) = rpc(
        client,
        "getTokenAccountsByOwner",
        json!([owner, { "mint": mint }, { "encoding": "jsonParsed" }]),
    )
    .await
    {
        if let Some(arr) = r.get("value").and_then(|v| v.as_array()) {
            t.treasury_holding = arr
                .iter()
                .filter_map(|a| a.pointer("/account/data/parsed/info/tokenAmount/uiAmount").and_then(|u| u.as_f64()))
                .sum();
        }
    }
    t
}

pub fn wallet() -> String {
    std::env::var("OCTOPUSCRAWL_TREASURY_WALLET").unwrap_or_else(|_| WALLET_DEFAULT.to_string())
}

pub(crate) async fn rpc(client: &reqwest::Client, method: &str, params: Value) -> Option<Value> {
    let body = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
    let resp = client
        .post(rpc_url())
        .json(&body)
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .ok()?;
    let v: Value = resp.json().await.ok()?;
    v.get("result").cloned()
}

/// One read of the treasury's on-chain state.
pub async fn fetch() -> TreasurySnapshot {
    let client = reqwest::Client::new();
    let addr = wallet();
    let mut snap = TreasurySnapshot {
        address: addr.clone(),
        ..Default::default()
    };

    snap.token = fetch_token(&client, &addr).await;

    if let Some(r) = rpc(&client, "getBalance", json!([addr])).await {
        if let Some(v) = r.get("value").and_then(|x| x.as_u64()) {
            snap.balance_sol = v as f64 / 1e9;
            snap.ok = true;
        }
    }

    if let Some(sigs) = rpc(&client, "getSignaturesForAddress", json!([addr, { "limit": WINDOW }])).await {
        if let Some(arr) = sigs.as_array() {
            for s in arr {
                let sig = s.get("signature").and_then(|x| x.as_str()).unwrap_or("").to_string();
                if sig.is_empty() {
                    continue;
                }
                let time = s.get("blockTime").and_then(|x| x.as_i64());
                let ok = s.get("err").map(|e| e.is_null()).unwrap_or(true);
                let delta = tx_delta(&client, &sig, &addr).await;
                snap.txs.push(TreasuryTx { sig, time, delta_sol: delta, ok });
            }
        }
    }

    snap.updated_at = Some(crate::clock::now());
    snap
}

/// The SOL delta for `addr` in transaction `sig` (+ inflow, − outflow).
async fn tx_delta(client: &reqwest::Client, sig: &str, addr: &str) -> f64 {
    let Some(tx) = rpc(
        client,
        "getTransaction",
        json!([sig, { "maxSupportedTransactionVersion": 0, "encoding": "json" }]),
    )
    .await
    else {
        return 0.0;
    };
    let keys = tx.pointer("/transaction/message/accountKeys").and_then(|k| k.as_array());
    let pre = tx.pointer("/meta/preBalances").and_then(|k| k.as_array());
    let post = tx.pointer("/meta/postBalances").and_then(|k| k.as_array());
    if let (Some(keys), Some(pre), Some(post)) = (keys, pre, post) {
        if let Some(idx) = keys.iter().position(|k| k.as_str() == Some(addr)) {
            let p0 = pre.get(idx).and_then(|x| x.as_i64()).unwrap_or(0);
            let p1 = post.get(idx).and_then(|x| x.as_i64()).unwrap_or(0);
            return (p1 - p0) as f64 / 1e9;
        }
    }
    0.0
}

/// Poll the chain on a steady cadence and publish into shared state.
pub fn spawn_poller(state: Arc<AppState>) {
    tokio::spawn(async move {
        loop {
            let snap = fetch().await;
            {
                let mut t = state.treasury.write().await;
                *t = snap;
            }
            let t = state.treasury.read().await.clone();
            let _ = state.tx.send(LiveMsg::Treasury { treasury: t });
            tokio::time::sleep(Duration::from_secs(120)).await;
        }
    });
}
