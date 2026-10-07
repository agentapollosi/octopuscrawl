//! The real crawl driver (feature `real`). It reads a curated set of public
//! security pages, then follows links WITHIN the same allowlisted hosts to grow
//! coverage — politely (robots-checked, spaced out, one host per fetch) and
//! read-only. Captures are cached so the live view stays busy without re-hitting
//! anyone's servers.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use octopuscrawl_core::*;
use octopuscrawl_crawler::{robots_allows, Engine, PageRead};
use tokio::sync::{mpsc, Mutex};
use octopuscrawl_ingest::Ingest;

use crate::discover;
use crate::frontier::{Frontier, Item};
use crate::AppState;

/// Curated public security pages the crawl seeds from.
const SEEDS: &[&str] = &[
    // standards — OWASP, MITRE (weaknesses, attack patterns, techniques)
    "https://owasp.org/www-project-top-ten/",
    "https://owasp.org/Top10/A03_2021-Injection/",
    "https://owasp.org/Top10/A01_2021-Broken_Access_Control/",
    "https://owasp.org/www-community/attacks/xss/",
    "https://cwe.mitre.org/data/definitions/79.html",
    "https://cwe.mitre.org/data/definitions/89.html",
    "https://cwe.mitre.org/data/definitions/20.html",
    "https://cwe.mitre.org/data/definitions/787.html",
    "https://attack.mitre.org/techniques/T1059/",
    "https://attack.mitre.org/tactics/TA0001/",
    "https://capec.mitre.org/data/definitions/66.html",
    // advisories — NVD CVE records, CISA
    "https://nvd.nist.gov/vuln/detail/CVE-2021-44228",
    "https://nvd.nist.gov/vuln/detail/CVE-2014-0160",
    "https://nvd.nist.gov/vuln/detail/CVE-2017-0144",
    "https://nvd.nist.gov/vuln/detail/CVE-2019-0708",
    "https://www.cisa.gov/known-exploited-vulnerabilities-catalog",
    // patches — distro security notices
    "https://ubuntu.com/security/notices",
    "https://www.debian.org/security/",
    // writeups — PortSwigger Web Security Academy
    "https://portswigger.net/web-security/sql-injection",
    "https://portswigger.net/web-security/cross-site-scripting",
    "https://portswigger.net/web-security/authentication",
    "https://portswigger.net/web-security/access-control",
    "https://portswigger.net/web-security/ssrf",
    // tooling — tool documentation (read-only knowledge, never operation)
    "https://nmap.org/book/man.html",
    "https://nmap.org/book/toc.html",
    "https://www.zaproxy.org/docs/",
    // rfcs — IETF standards
    "https://www.rfc-editor.org/rfc/rfc8446",
    "https://www.rfc-editor.org/rfc/rfc6749",
    "https://www.rfc-editor.org/rfc/rfc9110",
    "https://www.rfc-editor.org/rfc/rfc6265",
    "https://www.rfc-editor.org/rfc/rfc7519",
    "https://www.rfc-editor.org/rfc/rfc5280",
    // advisories — more CVE records + the CISA advisory index
    "https://nvd.nist.gov/vuln/detail/CVE-2024-3094",
    "https://nvd.nist.gov/vuln/detail/CVE-2023-44487",
    "https://nvd.nist.gov/vuln/detail/CVE-2023-4863",
    "https://nvd.nist.gov/vuln/detail/CVE-2022-22965",
    "https://nvd.nist.gov/vuln/detail/CVE-2021-34527",
    "https://www.cisa.gov/news-events/cybersecurity-advisories",
    // patches — distro security trackers
    "https://ubuntu.com/security/cves",
    // writeups — more of the web-security academy
    "https://portswigger.net/web-security/csrf",
    "https://portswigger.net/web-security/xxe",
    "https://portswigger.net/web-security/request-smuggling",
    "https://portswigger.net/web-security/deserialization",
    "https://portswigger.net/web-security/file-path-traversal",
    // tooling — tool manuals
    "https://nmap.org/book/man-port-scanning-techniques.html",
    "https://nmap.org/book/nse.html",
    "https://www.wireshark.org/docs/wsug_html_chunked/",
    // hubs — index pages whose links reach whole catalogs
    "https://cwe.mitre.org/data/slices/2000.html",
    "https://capec.mitre.org/data/slices/2000.html",
    "https://attack.mitre.org/techniques/enterprise/",
    "https://attack.mitre.org/mitigations/enterprise/",
    "https://attack.mitre.org/groups/",
    "https://attack.mitre.org/software/",
    "https://owasp.org/www-community/attacks/",
    "https://owasp.org/www-community/vulnerabilities/",
    "https://cheatsheetseries.owasp.org/IndexAlphabetical.html",
    "https://portswigger.net/web-security/all-topics",
    "https://isc.sans.edu/diaryarchive.html",
    "https://www.debian.org/security/dsa",
    "https://lists.debian.org/debian-security-announce/",
];

const MAX_CAPTURES: usize = 140;
/// Headless tabs reading in parallel (sponsored hatchlings add more).
const READERS: usize = 2;
const MAX_READERS: usize = 5;

struct Capture {
    host: String,
    url: String,
    title: String,
    links: Vec<LinkBox>,
    jpeg: Vec<u8>,
    tokens: u32,
    chapter: String,
    text: String,
}

fn cap_from(p: octopuscrawl_crawler::PageRead) -> Capture {
    let host = host_of(&p.url);
    let chapter = chapter_for(&host);
    let tokens = 400 + p.links.len() as u32 * 50;
    Capture {
        host,
        url: p.url,
        title: p.title,
        links: p.links,
        tokens,
        chapter,
        jpeg: p.jpeg,
        text: p.text,
    }
}

fn remember(state: &AppState, cap: &Capture) {
    state.corpus.add(crate::corpus::Doc {
        url: cap.url.clone(),
        title: cap.title.clone(),
        host: cap.host.clone(),
        chapter: cap.chapter.clone(),
        text: cap.text.clone(),
    });
}

pub async fn run(state: Arc<AppState>) {
    let engine = match Engine::launch().await {
        Ok(e) => Arc::new(e),
        Err(e) => {
            eprintln!("[crawl] chrome launch failed: {e:#}; falling back to demo");
            crate::run_demo(state).await;
            return;
        }
    };
    // a hatchling shows nothing until the crawl has actually read a page for it —
    // no placeholder URLs or made-up statuses on the live board
    {
        let mut snap = state.snapshot.write().await;
        for c in snap.crawlers.iter_mut() {
            c.status = CrawlerStatus::Idle;
            c.url.clear();
            c.title.clear();
            c.links.clear();
            c.thought = "waiting for its first page".to_string();
        }
        snap.stats.crawlers_running = 0;
    }

    // dedup + real BPE token counts + the on-disk dataset (also de-duplicates
    // the corpus on load), and the counters restored from it
    let mut ingest = Ingest::new("data");
    let already = init_stats_from_dataset(&state).await;

    // the frontier survives restarts; seeds only matter on a fresh start (a seed
    // already in the dataset is never queued again)
    let allowed: HashSet<String> = SEEDS.iter().map(|s| host_of(s)).collect();
    let frontier = Arc::new(Mutex::new(Frontier::load(allowed.clone(), &already)));
    {
        let mut f = frontier.lock().await;
        let fresh = SEEDS.iter().filter(|s| f.push(s, None)).count();
        println!(
            "[crawl] headless chrome up · {} pages waiting across {} hosts ({fresh} new seeds)",
            f.len(),
            f.hosts()
        );
    }
    let mut origins: Vec<String> = SEEDS
        .iter()
        .filter_map(|s| s.splitn(4, '/').nth(2).map(|h| format!("https://{h}")))
        .collect();
    origins.sort();
    origins.dedup();
    discover::spawn(frontier.clone(), origins);

    // readers: headless tabs pull from the frontier in parallel (the frontier
    // rotates hosts so no site is hit more than once every few seconds).
    // Sponsored hatchlings add real capacity: one more tab for every three.
    let (tx, mut rx) = mpsc::channel::<(Item, anyhow::Result<PageRead>)>(8);
    {
        let (engine, frontier, state) = (engine.clone(), frontier.clone(), state.clone());
        tokio::spawn(async move {
            let mut running = 0usize;
            loop {
                let sponsored = state.sponsors.read().await.sponsors.len();
                let want = (READERS + sponsored / 3).min(MAX_READERS);
                while running < want {
                    spawn_reader(running, engine.clone(), frontier.clone(), state.clone(), tx.clone());
                    running += 1;
                    if running > READERS {
                        println!("[crawl] sponsors added a reader — {running} tabs reading now");
                    }
                }
                tokio::time::sleep(Duration::from_secs(20)).await;
            }
        });
    }

    let seq = AtomicU64::new(1);
    let mut captures: Vec<Capture> = Vec::new();
    let mut slot: usize = 0;
    let mut t: usize = 0;
    let mut tick = tokio::time::interval(Duration::from_millis(1400));
    loop {
        tokio::select! {
            _ = tick.tick() => {
                t = t.wrapping_add(1);
                let num = state.snapshot.read().await.crawlers.len().max(1);
                // keep the whole board moving: a handful of hatchlings jump to a
                // different recent read every tick, so none sits on one page
                if !captures.is_empty() {
                    for k in 0..4usize {
                        let idx = (t.wrapping_mul(4).wrapping_add(k)) % num;
                        let ci = (t.wrapping_mul(7).wrapping_add(k.wrapping_mul(29))) % captures.len();
                        let s = seq.fetch_add(1, Ordering::Relaxed);
                        apply(&state, idx, &captures[ci], s, false, None).await;
                    }
                }
                if t % 43 == 0 {
                    frontier.lock().await.save();
                    state.sponsors.read().await.save();
                }
            }
            msg = rx.recv() => {
                let Some((item, res)) = msg else { break };
                let p = match res {
                    Ok(p) => p,
                    Err(e) => {
                        println!("[crawl] {} failed: {e:#} — will retry later", item.url);
                        frontier.lock().await.retry(item, Duration::from_secs(60));
                        continue;
                    }
                };
                if looks_transient(&p.title, &p.text) {
                    println!("[crawl] {} is temporarily down — pausing that site, will retry", item.url);
                    frontier.lock().await.retry(item, Duration::from_secs(600));
                    continue;
                }
                // discovery: every link on the page, not just what is on screen
                let found = {
                    let mut f = frontier.lock().await;
                    p.links_all.iter().filter(|l| f.push(l, Some(&item.url))).count()
                };
                let mut cap = cap_from(p);
                if looks_dead(&cap) {
                    println!("[crawl] dead page, skipped {}", cap.url);
                    continue;
                }
                let thin = cap.text.split_whitespace().count() < 80;
                let ing = if thin {
                    octopuscrawl_ingest::Ingested { accepted: false, tokens: 0 }
                } else {
                    ingest.process(&cap.url, &cap.host, &cap.chapter, &cap.title, &cap.text)
                };
                let num = state.snapshot.read().await.crawlers.len().max(1);
                let idx = slot % num;
                slot = slot.wrapping_add(1);
                let s = seq.fetch_add(1, Ordering::Relaxed);
                if ing.accepted {
                    cap.tokens = ing.tokens;
                    remember(&state, &cap);
                    update_vocab(&state, &ingest).await;
                    apply(&state, idx, &cap, s, true, item.parent.as_deref()).await;
                    // a sponsored hatchling's new page counts toward its owner
                    let cid = state.snapshot.read().await.crawlers.get(idx).map(|c| c.id.clone());
                    if let Some(cid) = cid.filter(|c| c.starts_with("own-")) {
                        state.sponsors.write().await.credit_read(&cid, cap.tokens);
                    }
                    println!("[crawl] read {} ({} tok, +{found} links)", cap.url, cap.tokens);
                } else {
                    // shown live, but it adds nothing new to the dataset
                    apply(&state, idx, &cap, s, false, None).await;
                    println!(
                        "[crawl] {} not counted {}",
                        if thin { "thin page," } else { "near-dup," },
                        cap.url
                    );
                }
                if !cap.links.is_empty() {
                    captures.push(cap);
                    if captures.len() > MAX_CAPTURES {
                        captures.remove(0);
                    }
                }
            }
        }
    }
}

/// One headless tab: take the next URL from the frontier, read it, hand it back.
fn spawn_reader(
    w: usize,
    engine: Arc<Engine>,
    frontier: Arc<Mutex<Frontier>>,
    state: Arc<AppState>,
    tx: mpsc::Sender<(Item, anyhow::Result<PageRead>)>,
) {
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(900 * w as u64)).await;
        loop {
            let ratio = chapter_ratio(&state).await;
            let item = frontier.lock().await.pick(&ratio, chapter_for);
            let Some(item) = item else {
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            };
            if !robots_allows(&item.url).await {
                continue;
            }
            let res = engine.read_page(&item.url).await;
            if tx.send((item, res)).await.is_err() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(800)).await;
        }
    });
}

/// How full each chapter is (covered / target) — the frontier reads toward the
/// emptiest one first.
async fn chapter_ratio(state: &AppState) -> HashMap<String, f64> {
    let snap = state.snapshot.read().await;
    snap.stats
        .chapters
        .iter()
        .map(|c| {
            let r = if c.target == 0 { 1.0 } else { c.covered as f64 / c.target as f64 };
            (c.id.clone(), r)
        })
        .collect()
}

/// The site answered with a temporary error page (its backend is down, a
/// gateway timed out…) — worth another try later, not a real page.
fn looks_transient(title: &str, text: &str) -> bool {
    let t = title.to_ascii_lowercase();
    let x: String = text.chars().take(600).collect::<String>().to_ascii_lowercase();
    ["502", "503", "504", "bad gateway", "gateway time", "service unavailable"].iter().any(|k| t.contains(k))
        || ["api is down", "try reloading the page", "temporarily unavailable", "service unavailable", "error occurred while fetching"]
            .iter()
            .any(|k| x.contains(k))
}

/// A page that is really an error screen (bad id, removed advisory, …).
fn looks_dead(cap: &Capture) -> bool {
    let t = cap.title.to_ascii_lowercase();
    t.contains("404") || t.contains("page not found") || t.contains("not found") && cap.text.split_whitespace().count() < 200
}

async fn apply(state: &AppState, idx: usize, cap: &Capture, seq: u64, first: bool, parent: Option<&str>) {
    let id;
    let crawler;
    // distinct hosts read so far — computed before taking the snapshot lock so we
    // never hold two locks at once.
    let domains = if first {
        Some(state.corpus.host_count() as u32)
    } else {
        None
    };
    {
        let mut snap = state.snapshot.write().await;
        if idx >= snap.crawlers.len() {
            return;
        }
        let c = &mut snap.crawlers[idx];
        id = c.id.clone();
        c.url = cap.url.clone();
        c.title = cap.title.clone();
        c.links = cap.links.clone();
        c.status = CrawlerStatus::Crawling;
        if first {
            c.pages_read += 1;
        }
        c.relevance = score_for(&cap.host);
        c.thought = format!("inking {} — {}", cap.host, short(&cap.title, 40));
        c.updated_at = crate::clock::now();
        c.links_seq = seq;
        crawler = c.clone();

        snap.stats.crawlers_running = snap
            .crawlers
            .iter()
            .filter(|c| c.status == CrawlerStatus::Crawling)
            .count() as u32;
        if first {
            snap.stats.pages_read += 1;
            snap.stats.dataset_tokens += cap.tokens as u64;
            if let Some(d) = domains {
                snap.stats.domains = d;
            }
            if let Some(ch) = snap.stats.chapters.iter_mut().find(|c| c.id == cap.chapter) {
                if ch.covered < ch.target {
                    ch.covered += 1;
                }
            }
        }
        snap.stats.updated_at = crate::clock::now();
    }

    state.frames.put(&id, seq, cap.jpeg.clone());

    let frame = Frame {
        crawler_id: id.clone(),
        seq,
        width: octopuscrawl_crawler::VIEWPORT_W,
        height: octopuscrawl_crawler::VIEWPORT_H,
        at: crate::clock::now(),
        url: format!("/v1/crawlers/{id}/frame.jpg?seq={seq}"),
    };

    let _ = state.tx.send(LiveMsg::Crawler { crawler });
    let _ = state.tx.send(LiveMsg::Frames { frames: vec![frame] });

    if first {
        // stable id per URL so the knowledge graph has one node per page
        let node = WebNode {
            id: node_id(&cap.url),
            url: cap.url.clone(),
            domain: cap.host.clone(),
            title: cap.title.clone(),
            chapter: Some(cap.chapter.clone()),
            project_id: None,
            tokens: cap.tokens,
            crawler_id: id.clone(),
            read_at: crate::clock::now(),
        };
        // the real link that led here (seeds have no parent)
        let edge = parent.map(|p| Edge {
            from: node_id(p),
            to: node.id.clone(),
        });

        // grow the persistent graph snapshot (bounded, deduped)
        {
            let mut g = state.graph.write().await;
            if !g.nodes.iter().any(|n| n.id == node.id) {
                g.nodes.push(node.clone());
                if g.nodes.len() > 240 {
                    let d = g.nodes.len() - 240;
                    g.nodes.drain(0..d);
                }
            }
            if let Some(e) = &edge {
                if e.from != e.to && !g.edges.iter().any(|x| x.from == e.from && x.to == e.to) {
                    g.edges.push(e.clone());
                    if g.edges.len() > 480 {
                        let d = g.edges.len() - 480;
                        g.edges.drain(0..d);
                    }
                }
            }
        }

        let _ = state.tx.send(LiveMsg::Page { node, edge });
    }
}

/// Stable 64-bit FNV-1a of a URL → the node's id in the knowledge graph.
fn node_id(url: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in url.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// Seed the live counters from the on-disk (de-duplicated) dataset so pages,
/// tokens, domains and chapter coverage carry over across restarts.
async fn init_stats_from_dataset(state: &AppState) -> HashSet<String> {
    let mut urls: HashSet<String> = HashSet::new();
    let Ok(body) = std::fs::read_to_string("data/corpus.jsonl") else {
        return urls;
    };
    let mut pages = 0u64;
    let mut tokens = 0u64;
    let mut hosts: HashSet<String> = HashSet::new();
    let mut chapters: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
            pages += 1;
            tokens += v.get("tokens").and_then(|t| t.as_u64()).unwrap_or(0);
            let host = v.get("host").and_then(|h| h.as_str()).unwrap_or("").to_string();
            let chapter = v.get("chapter").and_then(|c| c.as_str()).unwrap_or("").to_string();
            if !host.is_empty() {
                hosts.insert(host.clone());
            }
            if !chapter.is_empty() {
                *chapters.entry(chapter.clone()).or_insert(0) += 1;
            }
            // also load into the in-memory corpus so host counts stay correct and
            // the queen's retrieval ("ask") works over the whole dataset, not just
            // this session's reads
            let url = v.get("url").and_then(|u| u.as_str()).unwrap_or("").to_string();
            if !url.is_empty() {
                urls.insert(url.clone());
            }
            state.corpus.add(crate::corpus::Doc {
                url,
                title: v.get("title").and_then(|t| t.as_str()).unwrap_or("").to_string(),
                host,
                chapter,
                text: v.get("text").and_then(|t| t.as_str()).unwrap_or("").to_string(),
            });
        }
    }
    let mut snap = state.snapshot.write().await;
    snap.stats.pages_read = pages;
    snap.stats.dataset_tokens = tokens;
    snap.stats.domains = hosts.len() as u32;
    for ch in snap.stats.chapters.iter_mut() {
        if let Some(&c) = chapters.get(&ch.id) {
            ch.covered = c.min(ch.target);
        }
    }
    println!("[crawl] restored stats from dataset: {pages} pages, {tokens} tokens, {} domains", hosts.len());
    urls
}

/// Publish the latest tokenizer/vocabulary snapshot for `/v1/vocab`.
async fn update_vocab(state: &AppState, ingest: &Ingest) {
    let vi = ingest.vocab_info();
    let mut v = state.vocab.write().await;
    *v = VocabStats {
        trained: vi.trained,
        size: vi.size,
        accepted: vi.accepted,
        next_train_at: vi.next_train_at,
        terms: vi.terms,
    };
}

fn host_of(url: &str) -> String {
    url.strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url)
        .split('/')
        .next()
        .unwrap_or("")
        .trim_start_matches("www.")
        .to_string()
}

fn chapter_for(host: &str) -> String {
    let c = if host.contains("nvd.nist") || host.contains("cisa") || host.contains("cve") {
        "advisories"
    } else if host.contains("ubuntu")
        || host.contains("redhat")
        || host.contains("debian")
        || host.contains("suse")
        || host.contains("msrc")
    {
        "patches"
    } else if host.contains("nmap")
        || host.contains("zaproxy")
        || host.contains("wireshark")
        || host.contains("metasploit")
        || host.contains("kali")
    {
        "tooling"
    } else if host.contains("portswigger") || host.contains("sans") {
        "writeups"
    } else if host.contains("rfc-editor") || host.contains("ietf") {
        "rfcs"
    } else if host.contains("owasp") || host.contains("mitre") {
        "standards"
    } else {
        "writeups"
    };
    c.to_string()
}

fn score_for(host: &str) -> f64 {
    if host.contains("nvd.nist") || host.contains("cisa") || host.contains("cve") {
        0.95
    } else if host.contains("mitre") || host.contains("owasp") {
        0.88
    } else if host.contains("portswigger") {
        0.82
    } else {
        0.7
    }
}

fn short(s: &str, n: usize) -> String {
    let t: String = s.chars().take(n).collect();
    if s.chars().count() > n {
        format!("{t}…")
    } else {
        t
    }
}
