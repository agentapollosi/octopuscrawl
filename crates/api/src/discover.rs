//! Smart discovery: beyond following links, the swarm finds pages from sources
//! built for exactly this — CISA's catalog of exploited vulnerabilities (each CVE
//! becomes its NVD record) and the sitemaps sites publish for crawlers in their
//! robots.txt. Everything is a plain read, and robots.txt is checked first.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;

use crate::frontier::{host_of, is_content, Frontier};

const KEV_FEED: &str = "https://www.cisa.gov/sites/default/files/feeds/known_exploited_vulnerabilities.json";
const PER_SITEMAP_HOST: usize = 2_500;

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(25))
        .user_agent("octopuscrawl/0.1 (+https://octopuscrawl.net; read-only security crawler)")
        .build()
        .unwrap_or_default()
}

async fn get_text(c: &reqwest::Client, url: &str) -> Option<String> {
    if !octopuscrawl_crawler::robots_allows(url).await {
        println!("[discover] robots.txt disallows {url}");
        return None;
    }
    let r = c.get(url).send().await.ok()?;
    if !r.status().is_success() {
        return None;
    }
    r.text().await.ok()
}

/// Every CVE in CISA's Known Exploited Vulnerabilities catalog → its NVD record.
async fn kev(c: &reqwest::Client, frontier: &Arc<Mutex<Frontier>>) -> usize {
    let Some(body) = get_text(c, KEV_FEED).await else { return 0 };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&body) else { return 0 };
    let ids: Vec<String> = v
        .get("vulnerabilities")
        .and_then(|a| a.as_array())
        .map(|a| a.iter().filter_map(|x| x.get("cveID").and_then(|i| i.as_str()).map(str::to_string)).collect())
        .unwrap_or_default();
    let mut f = frontier.lock().await;
    // newest first: the feed is appended over time
    ids.iter()
        .rev()
        .filter(|id| f.push(&format!("https://nvd.nist.gov/vuln/detail/{id}"), None))
        .count()
}

/// `<loc>…</loc>` values of a sitemap or sitemap index.
fn locs(xml: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(i) = rest.find("<loc>") {
        rest = &rest[i + 5..];
        let Some(j) = rest.find("</loc>") else { break };
        let v = rest[..j].trim().replace("&amp;", "&");
        if !v.is_empty() {
            out.push(v);
        }
        rest = &rest[j + 6..];
    }
    out
}

/// Content pages listed in a host's sitemaps (following one level of sitemap
/// indexes, preferring child sitemaps that look security-related).
async fn sitemaps_for(c: &reqwest::Client, origin: &str, frontier: &Arc<Mutex<Frontier>>) -> usize {
    let Some(rules) = octopuscrawl_crawler::robots_for(origin).await else { return 0 };
    let host = host_of(origin);
    let mut added = 0usize;
    let mut queue: Vec<String> = rules.sitemaps.clone();
    let mut fetched = 0usize;
    while let Some(sm) = queue.pop() {
        if fetched >= 12 || added >= PER_SITEMAP_HOST {
            break;
        }
        if sm.ends_with(".gz") {
            continue;
        }
        fetched += 1;
        let Some(xml) = get_text(c, &sm).await else { continue };
        let list = locs(&xml);
        if xml.contains("<sitemapindex") {
            // security-looking children first
            let mut kids: Vec<String> = list;
            kids.sort_by_key(|k| {
                let k = k.to_ascii_lowercase();
                !(k.contains("secur") || k.contains("notice") || k.contains("advis") || k.contains("diary") || k.contains("web-security"))
            });
            for k in kids.into_iter().take(10).rev() {
                queue.push(k);
            }
            continue;
        }
        let mut f = frontier.lock().await;
        for u in list {
            if added >= PER_SITEMAP_HOST {
                break;
            }
            let h = host_of(&u);
            if h != host {
                continue;
            }
            let path = u.splitn(4, '/').nth(3).map(|p| format!("/{p}")).unwrap_or_default();
            if is_content(&h, &path) && f.push(&u, None) {
                added += 1;
            }
        }
        drop(f);
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    added
}

/// Run every discovery source once, then again every 12 hours (catalogs grow).
pub fn spawn(frontier: Arc<Mutex<Frontier>>, origins: Vec<String>) {
    tokio::spawn(async move {
        let c = client();
        loop {
            let n = kev(&c, &frontier).await;
            println!("[discover] CISA KEV catalog → {n} new CVE records queued");
            for o in &origins {
                let n = sitemaps_for(&c, o, &frontier).await;
                if n > 0 {
                    println!("[discover] {o} sitemaps → {n} new pages queued");
                }
            }
            let (len, hosts) = {
                let f = frontier.lock().await;
                (f.len(), f.hosts())
            };
            println!("[discover] frontier: {len} pages waiting across {hosts} hosts");
            tokio::time::sleep(Duration::from_secs(12 * 3600)).await;
        }
    });
}
