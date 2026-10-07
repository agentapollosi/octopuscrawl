//! The real crawl driver (feature `real`). It reads a curated set of public
//! security pages, then follows links WITHIN the same allowlisted hosts to grow
//! coverage — politely (robots-checked, spaced out, one host per fetch) and
//! read-only. Captures are cached so the live view stays busy without re-hitting
//! anyone's servers.

use std::collections::{HashSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use octopuscrawl_core::*;
use octopuscrawl_crawler::{robots_allows, Engine};
use octopuscrawl_ingest::Ingest;

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
];

const MAX_CAPTURES: usize = 140;
const MAX_FRONTIER: usize = 600;

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
        Ok(e) => e,
        Err(e) => {
            eprintln!("[crawl] chrome launch failed: {e:#}; falling back to demo");
            crate::run_demo(state).await;
            return;
        }
    };
    let num = { state.snapshot.read().await.crawlers.len() };
    println!("[crawl] headless chrome up; reading {} seeds", SEEDS.len());

    let allowed: HashSet<String> = SEEDS.iter().map(|s| host_of(s)).collect();
    let mut visited: HashSet<String> = SEEDS.iter().map(|s| s.to_string()).collect();
    // (parent_url, child_url) — the parent lets us draw the real link edge.
    let mut frontier: VecDeque<(String, String)> = VecDeque::new();

    let seq = AtomicU64::new(1);
    let mut captures: Vec<Capture> = Vec::new();
    // dedup + real BPE token counts + the on-disk dataset
    let mut ingest = Ingest::new("data");

    // phase 1 — read each seed once, assign to two crawlers for a fast fill
    for (i, url) in SEEDS.iter().enumerate() {
        if !robots_allows(url).await {
            println!("[crawl] robots.txt disallows {url}");
            continue;
        }
        match engine.read_page(url).await {
            Ok(p) => {
                let mut cap = cap_from(p);
                let ing = ingest.process(&cap.url, &cap.host, &cap.chapter, &cap.title, &cap.text);
                if !ing.accepted {
                    println!("[crawl] near-dup, skipped {}", cap.url);
                } else {
                    cap.tokens = ing.tokens;
                    remember(&state, &cap);
                    update_vocab(&state, &ingest).await;
                    enqueue_links(&cap, &allowed, &mut visited, &mut frontier);
                    let s1 = seq.fetch_add(1, Ordering::Relaxed);
                    apply(&state, i % num, &cap, s1, true, None).await;
                    let j = (i + SEEDS.len()) % num;
                    if j != i % num {
                        let s2 = seq.fetch_add(1, Ordering::Relaxed);
                        apply(&state, j, &cap, s2, false, None).await;
                    }
                    println!("[crawl] read {} ({} boxes, {} tok)", cap.url, cap.links.len(), cap.tokens);
                    captures.push(cap);
                }
            }
            Err(e) => println!("[crawl] {url} failed: {e:#}"),
        }
        tokio::time::sleep(Duration::from_millis(1500)).await;
    }

    if captures.is_empty() {
        eprintln!("[crawl] no captures; falling back to demo");
        crate::run_demo(state).await;
        return;
    }
    println!(
        "[crawl] {} pages cached, {} urls queued; following links within {} hosts",
        captures.len(),
        frontier.len(),
        allowed.len()
    );

    // phase 2 — keep the view busy from cache, and every ~10s fetch one new page
    // discovered by following links inside the allowlist
    let mut t: usize = 0;
    loop {
        tokio::time::sleep(Duration::from_millis(1700)).await;
        t = t.wrapping_add(1);

        let ci = t.wrapping_mul(5) % captures.len();
        let idx = t % num;
        let s = seq.fetch_add(1, Ordering::Relaxed);
        apply(&state, idx, &captures[ci], s, false, None).await;

        if t % 6 == 0 {
            if let Some((parent, url)) = frontier.pop_front() {
                if robots_allows(&url).await {
                    if let Ok(p) = engine.read_page(&url).await {
                        let mut cap = cap_from(p);
                        if allowed.contains(&cap.host) && !cap.links.is_empty() {
                            let ing = ingest.process(&cap.url, &cap.host, &cap.chapter, &cap.title, &cap.text);
                            if ing.accepted {
                                cap.tokens = ing.tokens;
                                remember(&state, &cap);
                                update_vocab(&state, &ingest).await;
                                enqueue_links(&cap, &allowed, &mut visited, &mut frontier);
                                let s = seq.fetch_add(1, Ordering::Relaxed);
                                apply(&state, t % num, &cap, s, true, Some(&parent)).await;
                                println!("[crawl] followed {} ({} boxes, {} tok)", cap.url, cap.links.len(), cap.tokens);
                                captures.push(cap);
                                if captures.len() > MAX_CAPTURES {
                                    captures.remove(0);
                                }
                            } else {
                                println!("[crawl] near-dup, skipped {}", cap.url);
                            }
                        }
                    }
                }
            }
        }
    }
}

fn enqueue_links(
    cap: &Capture,
    allowed: &HashSet<String>,
    visited: &mut HashSet<String>,
    frontier: &mut VecDeque<(String, String)>,
) {
    for l in &cap.links {
        let href = &l.href;
        if frontier.len() >= MAX_FRONTIER {
            break;
        }
        if href.is_empty() || !href.starts_with("http") || visited.contains(href) {
            continue;
        }
        if allowed.contains(&host_of(href)) {
            visited.insert(href.clone());
            frontier.push_back((cap.url.clone(), href.clone()));
        }
    }
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
        c.pages_read += 1;
        c.relevance = score_for(&cap.host);
        c.thought = format!("inking {} — {}", cap.host, short(&cap.title, 40));
        c.updated_at = Utc::now();
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
        snap.stats.updated_at = Utc::now();
    }

    state.frames.put(&id, seq, cap.jpeg.clone());

    let frame = Frame {
        crawler_id: id.clone(),
        seq,
        width: octopuscrawl_crawler::VIEWPORT_W,
        height: octopuscrawl_crawler::VIEWPORT_H,
        at: Utc::now(),
        url: format!("/v1/crawlers/{id}/frame.jpg?seq={seq}"),
    };

    let _ = state.tx.send(LiveMsg::Crawler { crawler });
    let _ = state.tx.send(LiveMsg::Frames { frames: vec![frame] });

    // every read costs compute → a live ledger line (even on cache re-reads)
    let sol = -(cap.tokens as f64) * 1.0e-6;
    let _ = state.tx.send(LiveMsg::Ledger {
        entry: LedgerEntry {
            id: format!("led-{seq:016x}"),
            at: Utc::now(),
            kind: LedgerKind::Crawl,
            sol,
            usd: sol * 150.0,
            memo: format!("{} · {}k tokens · {}", id, cap.tokens / 1000, cap.host),
            tx: None,
        },
    });

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
            read_at: Utc::now(),
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
    } else if host.contains("portswigger") {
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
