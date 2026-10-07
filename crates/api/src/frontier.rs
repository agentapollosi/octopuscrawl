//! The crawl frontier: every URL the swarm has found but not read yet, kept per
//! host so reads rotate politely across sites (never hammering one), with
//! content pages ahead of navigation pages and the least-covered chapter first.
//! Saved to disk, so a restart picks up where the swarm left off.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

const FILE: &str = "data/frontier.json";
/// Never hit the same host more often than this.
const HOST_GAP: Duration = Duration::from_secs(6);
const PER_HOST_CAP: usize = 6_000;

#[derive(Clone, Serialize, Deserialize)]
pub struct Item {
    pub url: String,
    /// The page whose link led here (None for seeds / catalogs / sitemaps).
    pub parent: Option<String>,
    /// Reads that hit a temporary error (site down, timeout) — retried later.
    #[serde(default)]
    pub tries: u8,
}

#[derive(Default, Serialize, Deserialize)]
struct Saved {
    queues: HashMap<String, Vec<Item>>,
    seen: Vec<String>,
}

pub struct Frontier {
    queues: HashMap<String, VecDeque<Item>>,
    /// Queued or already read — a URL enters the frontier once, ever.
    seen: HashSet<String>,
    last_hit: HashMap<String, Instant>,
    allowed: HashSet<String>,
}

pub fn host_of(url: &str) -> String {
    url.strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url)
        .split('/')
        .next()
        .unwrap_or("")
        .trim_start_matches("www.")
        .to_ascii_lowercase()
}

fn path_of(url: &str) -> &str {
    let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://")).unwrap_or(url);
    rest.find('/').map(|i| &rest[i..]).unwrap_or("/")
}

/// The security part of each site — the swarm never wanders outside it (no
/// "about us", codes of conduct or event pages). Hosts not listed are all in scope.
pub fn in_scope(host: &str, path: &str) -> bool {
    let any = |xs: &[&str]| xs.iter().any(|x| path.starts_with(x));
    match host {
        "nvd.nist.gov" => any(&["/vuln"]),
        "cwe.mitre.org" => any(&["/data/", "/top25", "/scoring/"]),
        "capec.mitre.org" => any(&["/data/"]),
        "attack.mitre.org" => any(&["/techniques", "/tactics", "/mitigations", "/groups", "/software", "/campaigns", "/datasources", "/matrices"]),
        "owasp.org" => any(&["/www-community/", "/Top10/", "/www-project-", "/API-Security/"]),
        "cheatsheetseries.owasp.org" => any(&["/cheatsheets/", "/Index"]),
        "portswigger.net" => {
            (path.starts_with("/web-security") && !path.starts_with("/web-security/certification"))
                || path.starts_with("/research")
        }
        "ubuntu.com" => any(&["/security"]),
        "debian.org" => any(&["/security/"]),
        // the advisory archive, from 2015 on (older years are kernel-2.0 history)
        "lists.debian.org" => any(&["/debian-security-announce/"]) && archive_year(path).map_or(true, |y| y >= 2015),
        "cisa.gov" => any(&[
            "/news-events/cybersecurity-advisories",
            "/news-events/ics-advisories",
            "/news-events/alerts",
            "/news-events/directives",
            "/binding-operational-directive",
            "/known-exploited",
            "/kev",
            "/topics/cyber",
            "/topics/industrial-control",
            "/topics/information-communications-technology-supply-chain",
            "/resources-tools/",
        ]),
        "isc.sans.edu" => any(&["/diary"]),
        "rfc-editor.org" => any(&["/rfc/"]),
        "nmap.org" => any(&["/book/", "/nsedoc/", "/npcap/"]),
        "wireshark.org" => any(&["/docs/"]),
        "zaproxy.org" => any(&["/docs/"]),
        _ => true,
    }
}

/// The first 4-digit year in a mailing-list archive path, if any.
fn archive_year(path: &str) -> Option<u32> {
    let b = path.as_bytes();
    (0..b.len().saturating_sub(3)).find_map(|i| {
        let w = &path[i..i + 4];
        let edge = i == 0 || !b[i - 1].is_ascii_digit();
        (edge && w.chars().all(|c| c.is_ascii_digit()) && (w.starts_with("19") || w.starts_with("20")))
            .then(|| w.parse().ok())
            .flatten()
    })
}

/// Pages a security reader wants, per site — these jump the queue.
pub fn is_content(host: &str, path: &str) -> bool {
    let p = path;
    match host {
        "nvd.nist.gov" => p.starts_with("/vuln/detail/CVE-"),
        "cwe.mitre.org" | "capec.mitre.org" => p.starts_with("/data/definitions/"),
        "attack.mitre.org" => ["/techniques/T", "/mitigations/M", "/groups/G", "/software/S", "/tactics/TA", "/campaigns/C"]
            .iter()
            .any(|x| p.starts_with(x)),
        "owasp.org" => p.starts_with("/www-community/") || p.starts_with("/Top10/") || p.starts_with("/www-project-"),
        "cheatsheetseries.owasp.org" => p.starts_with("/cheatsheets/"),
        "portswigger.net" => p.starts_with("/web-security/"),
        "ubuntu.com" => p.starts_with("/security/notices/USN-") || p.starts_with("/security/CVE-"),
        "debian.org" => p.starts_with("/security/20") && p.contains("dsa-"),
        "lists.debian.org" => p.starts_with("/debian-security-announce/20") && p.contains("/msg"),
        "cisa.gov" => {
            p.starts_with("/news-events/cybersecurity-advisories/")
                || p.starts_with("/news-events/ics-advisories/")
                || p.starts_with("/known-exploited")
        }
        "isc.sans.edu" => p.starts_with("/diary/"),
        "rfc-editor.org" => p.starts_with("/rfc/rfc"),
        "nmap.org" => p.starts_with("/book/"),
        "wireshark.org" => p.starts_with("/docs/wsug_html_chunked/"),
        "zaproxy.org" => p.starts_with("/docs/"),
        _ => false,
    }
}

/// Clean a discovered URL, or None if it is not worth a read: other schemes,
/// query strings (filters, logins, calculators — the same page again and
/// again), files, account/search pages and non-English copies.
pub fn normalize(raw: &str) -> Option<String> {
    let url = raw.split('#').next()?.trim();
    if !(url.starts_with("https://") || url.starts_with("http://")) || url.contains('?') {
        return None;
    }
    let mut url = url.replacen("http://", "https://", 1);
    // an RFC's info page → the RFC itself
    if let Some(rest) = url.strip_prefix("https://www.rfc-editor.org/info/rfc") {
        let n: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if !n.is_empty() {
            url = format!("https://www.rfc-editor.org/rfc/rfc{n}");
        }
    }
    let path = path_of(&url).to_ascii_lowercase();
    const FILES: &[&str] = &[
        ".pdf", ".zip", ".gz", ".tgz", ".tar", ".png", ".jpg", ".jpeg", ".gif", ".svg", ".webp", ".ico",
        ".mp4", ".mp3", ".xml", ".json", ".csv", ".txt", ".exe", ".msi", ".dmg", ".iso", ".deb", ".rpm",
        ".apk", ".doc", ".docx", ".xls", ".xlsx", ".ppt", ".pptx", ".css", ".js", ".atom", ".rss",
    ];
    if FILES.iter().any(|e| path.ends_with(e)) {
        return None;
    }
    const SKIP: &[&str] = &[
        "/login", "/signin", "/sign-in", "/logout", "/register", "/signup", "/sign-up", "/users", "/account",
        "/search", "/cart", "/tag/", "/tags/", "/author/", "/feed", "/print/", "/share", "/subscribe",
        "/donate", "/jobs", "/careers", "/contact", "/privacy", "/cookie", "/terms", "/legal",
    ];
    if SKIP.iter().any(|x| path.contains(x)) {
        return None;
    }
    // translated copies: /es/…, /fr/…, index.fr.html, …
    let first = path.trim_start_matches('/').split('/').next().unwrap_or("");
    const LANGS: &[&str] = &["es", "fr", "de", "ja", "zh", "zh-cn", "zh-tw", "pt", "pt-br", "ru", "it", "ko", "nl", "pl", "tr", "uk", "ar", "fa", "id", "vi"];
    if LANGS.contains(&first) {
        return None;
    }
    if let Some(stem) = path.strip_suffix(".html") {
        if let Some((_, lang)) = stem.rsplit_once('.') {
            if lang.len() <= 5 && lang != "en" && lang.chars().all(|c| c.is_ascii_alphabetic() || c == '-') {
                return None;
            }
        }
    }
    Some(url)
}

impl Frontier {
    pub fn load(allowed: HashSet<String>, already_read: &HashSet<String>) -> Self {
        let saved: Saved = std::fs::read_to_string(FILE)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let mut seen: HashSet<String> = saved.seen.into_iter().collect();
        seen.extend(already_read.iter().cloned());
        let queues = saved
            .queues
            .into_iter()
            .map(|(h, v)| {
                let q = v
                    .into_iter()
                    .filter(|i| !already_read.contains(&i.url) && in_scope(&h, path_of(&i.url)))
                    .collect::<VecDeque<_>>();
                (h, q)
            })
            .collect();
        Frontier { queues, seen, last_hit: HashMap::new(), allowed }
    }

    pub fn save(&self) {
        let saved = Saved {
            queues: self.queues.iter().map(|(h, q)| (h.clone(), q.iter().cloned().collect())).collect(),
            seen: self.seen.iter().cloned().collect(),
        };
        if let Ok(body) = serde_json::to_string(&saved) {
            let tmp = format!("{FILE}.tmp");
            let _ = std::fs::create_dir_all("data");
            if std::fs::write(&tmp, body).is_ok() {
                let _ = std::fs::rename(&tmp, FILE);
            }
        }
    }

    pub fn allow_host(&mut self, host: &str) {
        self.allowed.insert(host.to_string());
    }

    pub fn is_allowed(&self, url: &str) -> bool {
        self.allowed.contains(&host_of(url))
    }

    /// Queue a URL once. Content pages go to the front of their host's queue.
    pub fn push(&mut self, raw: &str, parent: Option<&str>) -> bool {
        let Some(url) = normalize(raw) else { return false };
        let host = host_of(&url);
        if !self.allowed.contains(&host) || self.seen.contains(&url) || !in_scope(&host, path_of(&url)) {
            return false;
        }
        let q = self.queues.entry(host.clone()).or_default();
        if q.len() >= PER_HOST_CAP {
            return false;
        }
        self.seen.insert(url.clone());
        let item = Item { url: url.clone(), parent: parent.map(str::to_string), tries: 0 };
        if is_content(&host, path_of(&url)) {
            q.push_front(item);
        } else {
            q.push_back(item);
        }
        true
    }

    /// Next URL to read: among hosts that have rested long enough, the one whose
    /// chapter is least covered (then the one rested longest).
    pub fn pick(&mut self, chapter_ratio: &HashMap<String, f64>, chapter_for: impl Fn(&str) -> String) -> Option<Item> {
        let now = Instant::now();
        let mut best: Option<(f64, Duration, String)> = None;
        for (host, q) in &self.queues {
            if q.is_empty() {
                continue;
            }
            let rested = self.last_hit.get(host).map_or(Duration::from_secs(3600), |t| now.saturating_duration_since(*t));
            if rested < HOST_GAP {
                continue;
            }
            let r = chapter_ratio.get(&chapter_for(host)).copied().unwrap_or(1.0);
            let better = match &best {
                None => true,
                Some((br, bt, _)) => r < *br - 1e-9 || ((r - *br).abs() < 1e-9 && rested > *bt),
            };
            if better {
                best = Some((r, rested, host.clone()));
            }
        }
        let (_, _, host) = best?;
        self.last_hit.insert(host.clone(), now);
        self.queues.get_mut(&host)?.pop_front()
    }

    /// Put a URL back for a later attempt after a temporary failure, and rest
    /// its host for `pause` (a site that is down is not hammered).
    pub fn retry(&mut self, mut item: Item, pause: Duration) {
        let host = host_of(&item.url);
        self.last_hit.insert(host.clone(), Instant::now() + pause.saturating_sub(HOST_GAP));
        if item.tries >= 3 {
            return;
        }
        item.tries += 1;
        self.queues.entry(host).or_default().push_back(item);
    }

    pub fn len(&self) -> usize {
        self.queues.values().map(|q| q.len()).sum()
    }

    pub fn hosts(&self) -> usize {
        self.queues.values().filter(|q| !q.is_empty()).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_and_filters() {
        assert_eq!(normalize("https://nvd.nist.gov/vuln/detail/CVE-2024-3094#x").as_deref(), Some("https://nvd.nist.gov/vuln/detail/CVE-2024-3094"));
        assert!(normalize("https://ubuntu.com/security/cves?version=noble").is_none());
        assert!(normalize("https://portswigger.net/users").is_none());
        assert!(normalize("https://www.debian.org/security/index.fr.html").is_none());
        assert!(normalize("https://www.debian.org/security/2024/dsa-5600").is_some());
        assert!(normalize("https://owasp.org/es/foo").is_none());
        assert!(normalize("https://x.org/a.pdf").is_none());
        assert!(!in_scope("debian.org", "/social_contract"));
        assert!(in_scope("debian.org", "/security/dsa"));
        assert!(in_scope("lists.debian.org", "/debian-security-announce/2026/msg00117.html"));
        assert!(!in_scope("lists.debian.org", "/debian-security-announce/debian-security-announce-1999/threads.html"));
        assert!(in_scope("lists.debian.org", "/debian-security-announce/"));
        assert!(!in_scope("portswigger.net", "/web-security/certification/faq"));
        assert!(in_scope("portswigger.net", "/research/some-paper"));
        assert!(!in_scope("portswigger.net", "/burp/pro"));
        assert_eq!(normalize("https://www.rfc-editor.org/info/rfc5246/").as_deref(), Some("https://www.rfc-editor.org/rfc/rfc5246"));
    }

    #[test]
    fn content_first_and_host_rotation() {
        let allowed: HashSet<String> = ["nvd.nist.gov", "owasp.org"].iter().map(|s| s.to_string()).collect();
        let mut f = Frontier { queues: HashMap::new(), seen: HashSet::new(), last_hit: HashMap::new(), allowed };
        assert!(f.push("https://nvd.nist.gov/vuln/categories", None));
        assert!(f.push("https://nvd.nist.gov/vuln/detail/CVE-2021-44228", None));
        assert!(!f.push("https://nvd.nist.gov/vuln/detail/CVE-2021-44228", None));
        assert!(f.push("https://owasp.org/www-community/attacks/csrf", None));
        let ratio = HashMap::new();
        let a = f.pick(&ratio, |_| "x".into()).unwrap();
        let b = f.pick(&ratio, |_| "x".into()).unwrap();
        // two different hosts back to back, and the CVE page before the index
        assert_ne!(host_of(&a.url), host_of(&b.url));
        let nvd = if host_of(&a.url) == "nvd.nist.gov" { a } else { b };
        assert!(nvd.url.contains("CVE-2021-44228"));
        // nvd is resting now
        assert!(f.pick(&ratio, |_| "x".into()).is_none());
    }
}
