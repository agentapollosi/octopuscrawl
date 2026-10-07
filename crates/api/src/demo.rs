//! Demo data source. Until the real chromiumoxide crawler lands, this fabricates
//! a believable live feed so the frontend has something to render. Everything it
//! produces is read-only, public-host flavoured, and phrased defensively.

use chrono::Utc;
use octopuscrawl_core::*;
use std::sync::atomic::{AtomicU64, Ordering};

/// House crawler names: octopus + security, never operational-attack names.
pub const HOUSE_NAMES: &[&str] = &[
    "ink-sniffer", "reef-runner", "kraken-pup", "patch-coral", "cve-diver",
    "advisory-angler", "hash-tentacle", "cipher-polyp", "packet-nautilus", "siphon-squid",
    "abyss-reader", "coral-crawler", "mantle-mapper", "sucker-scout", "deep-diver",
    "brine-browser", "tide-tracer", "lagoon-lurker", "pearl-parser", "squid-scribe",
    "nautilus-note", "trench-tracker", "anemone-archivist", "octo-observer",
];

/// Public, legal-to-read security pages — every one a real, specific document
/// (no placeholder or fabricated IDs). These are only the *initial* labels for
/// the hatchlings; in real-crawl mode each is replaced by a page actually read.
const SEEDS: &[(&str, &str, &str)] = &[
    ("nvd.nist.gov", "/vuln/detail/CVE-2021-44228", "CVE-2021-44228 (Log4Shell) — NVD"),
    ("github.com", "/advisories/GHSA-jfh8-c2jp-5v3q", "Log4Shell advisory — GitHub"),
    ("owasp.org", "/www-project-top-ten/", "OWASP Top Ten"),
    ("owasp.org", "/Top10/A03_2021-Injection/", "A03:2021 Injection — OWASP"),
    ("attack.mitre.org", "/techniques/T1059/", "Command and Scripting Interpreter — MITRE ATT&CK"),
    ("attack.mitre.org", "/tactics/TA0001/", "Initial Access — MITRE ATT&CK"),
    ("cwe.mitre.org", "/data/definitions/79.html", "CWE-79: Cross-site Scripting"),
    ("cwe.mitre.org", "/data/definitions/89.html", "CWE-89: SQL Injection"),
    ("cisa.gov", "/known-exploited-vulnerabilities-catalog", "Known Exploited Vulnerabilities — CISA"),
    ("portswigger.net", "/web-security/sql-injection", "SQL injection — PortSwigger"),
    ("ubuntu.com", "/security/notices", "Ubuntu Security Notices"),
    ("www.rfc-editor.org", "/rfc/rfc8446", "RFC 8446: TLS 1.3"),
];

const THOUGHTS: &[&str] = &[
    "diving to {host}",
    "reading an advisory on {host}",
    "waiting for a free browser — {n} ahead",
    "{host} took too long — moving on",
    "carrying \u{201c}{title}\u{201d} back to the queen",
    "surfacing with a fresh patch note",
    "mapping links on {host}",
];

/// Tiny deterministic LCG so the demo needs no rand dependency.
pub struct Lcg(u64);
impl Lcg {
    pub fn new(seed: u64) -> Self {
        Lcg(seed | 1)
    }
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0
    }
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() >> 33) as usize % n.max(1)
    }
    pub fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}

static LINKS_SEQ: AtomicU64 = AtomicU64::new(1);
static PAGE_SEQ: AtomicU64 = AtomicU64::new(1);

pub fn seed_crawlers(n: usize) -> Vec<Crawler> {
    let now = Utc::now();
    (0..n)
        .map(|i| {
            let mut r = Lcg::new(0x1234_5678 ^ (i as u64).wrapping_mul(2654435761));
            let (host, path, title) = SEEDS[r.below(SEEDS.len())];
            Crawler {
                id: format!("octo-{:02}", i + 1),
                name: HOUSE_NAMES[i % HOUSE_NAMES.len()].to_string(),
                kind: CrawlerKind::Queen,
                owner: None,
                status: if r.unit() < 0.82 { CrawlerStatus::Crawling } else { CrawlerStatus::Idle },
                url: format!("https://{host}{path}"),
                title: title.to_string(),
                target: None,
                pages_read: 0,
                thought: format!("reading an advisory on {host}"),
                relevance: (0.5 + r.unit() * 0.5 * 100.0).round() / 100.0,
                started_at: now,
                updated_at: now,
                links_seq: LINKS_SEQ.fetch_add(1, Ordering::Relaxed),
                links: Vec::new(),
            }
        })
        .collect()
}

pub fn seed_stats(crawlers: &[Crawler]) -> Stats {
    let now = Utc::now();
    let running = crawlers.iter().filter(|c| c.status == CrawlerStatus::Crawling).count() as u32;
    let pages: u64 = crawlers.iter().map(|c| c.pages_read).sum();
    // no model is trained yet — the crawl is still collecting the corpus
    let queen = QueenVersion {
        version: 1,
        status: QueenStatus::Funding,
        dataset_tokens: 0,
        dataset_pages: 0,
        cost_sol: 0.06,
        funded_sol: 0.0,
        params: None,
        trained_at: None,
        weights_url: None,
    };
    let next_queen = QueenVersion {
        version: 1,
        status: QueenStatus::Funding,
        dataset_tokens: 0,
        dataset_pages: 0,
        cost_sol: 0.06,
        funded_sol: 0.0,
        params: None,
        trained_at: None,
        weights_url: None,
    };
    let chapters = vec![
        chapter("advisories", 1, "advisories", 180, 0, ChapterStatus::Active),
        chapter("patches", 2, "patches", 160, 0, ChapterStatus::Active),
        chapter("standards", 3, "standards", 90, 0, ChapterStatus::Active),
        chapter("writeups", 4, "writeups", 140, 0, ChapterStatus::Active),
        chapter("tooling", 5, "tooling", 110, 0, ChapterStatus::Active),
        chapter("forums", 6, "forums", 0, 0, ChapterStatus::Locked),
        chapter("rfcs", 7, "rfcs", 0, 0, ChapterStatus::Locked),
    ];
    Stats {
        crawlers_total: crawlers.len() as u32,
        crawlers_running: running,
        pages_read: pages,
        dataset_tokens: 0,
        domains: 0,
        projects_mapped: 0,
        projects_target: 275,
        queen,
        next_queen,
        chapters,
        updated_at: now,
    }
}

fn chapter(id: &str, index: u32, title: &str, target: u32, covered: u32, status: ChapterStatus) -> Chapter {
    Chapter { id: id.into(), index, title: title.into(), target, covered, status }
}

/// Advance one crawler in place and, when it accepts a page, return the WebNode
/// plus a ledger line for the compute cost.
pub fn tick_crawler(c: &mut Crawler, r: &mut Lcg) -> Option<(WebNode, LedgerEntry)> {
    let (host, path, title) = SEEDS[r.below(SEEDS.len())];
    c.url = format!("https://{host}{path}");
    c.title = title.to_string();
    c.updated_at = Utc::now();
    let tmpl = THOUGHTS[r.below(THOUGHTS.len())];
    c.thought = tmpl
        .replace("{host}", host)
        .replace("{title}", title)
        .replace("{n}", &(1 + r.below(4)).to_string());
    c.relevance = ((0.5 + r.unit() * 0.5) * 100.0).round() / 100.0;

    if r.unit() < 0.6 {
        c.status = CrawlerStatus::Crawling;
        c.pages_read += 1;
        let tokens = 400 + r.below(3200) as u32;
        let pid = PAGE_SEQ.fetch_add(1, Ordering::Relaxed);
        let node = WebNode {
            id: format!("{pid:016x}"),
            url: c.url.clone(),
            domain: host.to_string(),
            title: title.to_string(),
            chapter: Some("advisories".to_string()),
            project_id: None,
            tokens,
            crawler_id: c.id.clone(),
            read_at: Utc::now(),
        };
        let entry = LedgerEntry {
            id: format!("led-{pid:016x}"),
            at: Utc::now(),
            kind: LedgerKind::Crawl,
            sol: -(tokens as f64) * 1.2e-9,
            usd: -(tokens as f64) * 1.5e-7,
            memo: format!("{} \u{00b7} 1 page \u{00b7} {}k tokens", c.name, tokens / 1000),
            tx: None,
        };
        Some((node, entry))
    } else {
        c.status = if r.unit() < 0.85 { CrawlerStatus::Returning } else { CrawlerStatus::Idle };
        None
    }
}
