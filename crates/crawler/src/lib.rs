//! The real crawl engine: drives a headless Chrome through public security
//! pages and brings back, for each page, a screenshot plus the on-screen boxes
//! of every link and heading — the targets the octopus reaches for.
//!
//! Guardrails live here: robots.txt is honoured, only http(s) is followed, and
//! the engine never submits anything. It reads; it does not act.

use std::time::Duration;

use anyhow::{anyhow, Result};
use chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat;
use chromiumoxide::handler::viewport::Viewport;
use chromiumoxide::page::ScreenshotParams;
use chromiumoxide::{Browser, BrowserConfig};
use futures_util::StreamExt;
use octopuscrawl_core::LinkBox;

pub const VIEWPORT_W: u32 = 1280;
pub const VIEWPORT_H: u32 = 800;

/// Resolve the Chrome binary: $OCTOPUSCRAWL_CHROME, else a local
/// chrome-headless-shell, else the standard macOS install.
pub fn chrome_path() -> String {
    if let Ok(p) = std::env::var("OCTOPUSCRAWL_CHROME") {
        return p;
    }
    // chrome-headless-shell is a bare binary, not an .app bundle, so on macOS it
    // never becomes the foreground app — full Chrome, even headless, steals
    // keyboard focus every time it opens a page.
    if let Ok(home) = std::env::var("HOME") {
        let shell =
            format!("{home}/.local/share/chrome-headless-shell/current/chrome-headless-shell");
        if std::path::Path::new(&shell).exists() {
            return shell;
        }
    }
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome".into()
}

pub struct Engine {
    pub browser: Browser,
    _handle: tokio::task::JoinHandle<()>,
}

impl Engine {
    /// Launch headless Chrome with a throwaway profile (never the user's own).
    pub async fn launch() -> Result<Engine> {
        // A fresh, unique profile per process so a stale SingletonLock from a
        // previously killed run can never block startup. Never the user's own
        // Chrome profile.
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let profile =
            std::env::temp_dir().join(format!("octopuscrawl-chrome-{}-{}", std::process::id(), nanos));
        let chrome = chrome_path();
        let mut cfg = BrowserConfig::builder()
            .chrome_executable(&chrome)
            .user_data_dir(&profile)
            .viewport(Viewport {
                width: VIEWPORT_W,
                height: VIEWPORT_H,
                ..Viewport::default()
            });
        // full Chrome needs the new headless mode; chrome-headless-shell is
        // headless by construction
        if !chrome.contains("chrome-headless-shell") {
            cfg = cfg.arg("--headless=new");
        }
        let cfg = cfg
            .arg(format!("--window-size={VIEWPORT_W},{VIEWPORT_H}"))
            .arg("--hide-scrollbars")
            .arg("--mute-audio")
            .arg("--disable-background-networking")
            .arg("--disable-extensions")
            // required on headless servers (and when running as root)
            .arg("--no-sandbox")
            .arg("--disable-dev-shm-usage")
            .build()
            .map_err(|e| anyhow!("browser config: {e}"))?;
        let (mut browser, mut handler) = Browser::launch(cfg).await?;
        if let Some(pid) = browser.get_mut_child().and_then(|c| c.as_mut_inner().id()) {
            reap_chrome_after_us(pid, &profile);
        }
        let handle = tokio::spawn(async move { while handler.next().await.is_some() {} });
        Ok(Engine {
            browser,
            _handle: handle,
        })
    }

    pub async fn read_page(&self, url: &str) -> Result<PageRead> {
        read_page(&self.browser, url).await
    }
}

/// When this process dies by a signal (pkill, Ctrl+C, crash) no destructors run,
/// so chromiumoxide's kill-on-drop never fires and Chrome lives on as an orphan.
/// A detached watchdog waits for us to exit, then kills Chrome — only if that
/// pid is still our Chrome, matched by its profile path — and drops the profile.
fn reap_chrome_after_us(chrome_pid: u32, profile: &std::path::Path) {
    const WATCHDOG: &str = r#"trap "" INT TERM HUP
while kill -0 "$0" 2>/dev/null; do sleep 1; done
ps -ww -p "$1" -o command= 2>/dev/null | grep -qF -- "$2" && kill "$1"
sleep 2; rm -rf -- "$2""#;
    let spawned = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(WATCHDOG)
        .arg(std::process::id().to_string())
        .arg(chrome_pid.to_string())
        .arg(profile)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    if let Err(e) = spawned {
        eprintln!("[crawl] chrome watchdog failed to start: {e}");
    }
}

pub struct PageRead {
    pub url: String,
    pub title: String,
    pub jpeg: Vec<u8>,
    pub links: Vec<LinkBox>,
    /// Cleaned visible text of the page (truncated) — the corpus the queen reads.
    pub text: String,
    /// Every http(s) link on the page (fragment stripped) — for discovery, not
    /// display. The on-screen `links` boxes are only what fits in the viewport.
    pub links_all: Vec<String>,
}

// Pull out the real *content* the queen is reading — headings and body text in
// the main article, in reading order — not the nav bar, footer or cookie banner.
// Each box is tagged (head / text / link) and clamped to the 1280×800 viewport
/// Hide (never accept) common cookie / consent overlays before the capture.
const HIDE_OVERLAYS_JS: &str = r#"
(() => {
  const css = `
    #onetrust-banner-sdk, #onetrust-consent-sdk, .onetrust-pc-dark-filter,
    #CybotCookiebotDialog, #cookiebanner, #cookie-banner, #cookieConsent,
    .cc-window, .cc-banner, .cookie-banner, .cookie-consent, .cookie-notice,
    .truste_box_overlay, #truste-consent-track, #usercentrics-root,
    [id*="cookie" i][class*="banner" i], [class*="cookie" i][class*="banner" i],
    [id*="consent" i][class*="banner" i], [class*="consent" i][class*="banner" i],
    [aria-label*="cookie" i][role="dialog"], [aria-label*="consent" i][role="dialog"]
    { display: none !important; visibility: hidden !important; }
  `;
  const st = document.createElement('style');
  st.setAttribute('data-octopuscrawl', 'hide-overlays');
  st.textContent = css;
  document.documentElement.appendChild(st);
  // fixed bottom/top bars that mention cookies but carry no known id/class
  for (const el of document.querySelectorAll('body *')) {
    const cs = getComputedStyle(el);
    if (cs.position !== 'fixed' && cs.position !== 'sticky') continue;
    const t = (el.innerText || '').slice(0, 600).toLowerCase();
    if (t.includes('cookie') && (t.includes('accept') || t.includes('consent') || t.includes('privacy'))) {
      el.style.setProperty('display', 'none', 'important');
    }
  }
  return true;
})()
"#;

// so it lines up exactly with the screenshot the tentacles ink.
const EXTRACT_JS: &str = r#"
(() => {
  const vw = 1280, vh = 800;
  const root = document.querySelector('main, article, [role="main"], #content, .content') || document.body;
  // Exclude page chrome by SEMANTIC role (robust across sites) plus the few
  // class patterns that rarely use semantic tags (cookie/consent/breadcrumb).
  // Deliberately NOT matching broad class names like "header"/"menu"/"sidebar",
  // which on many sites wrap the real article and would hide all its content.
  const CHROME = 'nav, header, footer, aside,'
    + '[role="navigation"], [role="banner"], [role="contentinfo"],'
    + '[class*="cookie" i], [id*="cookie" i], [class*="consent" i], [class*="breadcrumb" i]';
  const bad = (e) => !!e.closest(CHROME);
  const vis = (e) => { const s = getComputedStyle(e);
    return s && s.visibility !== 'hidden' && s.display !== 'none' && parseFloat(s.opacity || '1') > 0.05; };
  const grab = (sel, kind, minLen) => Array.from(root.querySelectorAll(sel)).map(e => {
    if (bad(e) || !vis(e)) return null;
    const text = (e.innerText || e.textContent || "").replace(/\s+/g, " ").trim();
    if (text.length < minLen) return null;
    const r = e.getBoundingClientRect();
    const w = Math.min(r.width, vw - r.x), h = r.height;
    if (!(w > 24 && h > 8 && h <= vh * 0.6 && r.x >= 0 && r.y >= 0 && r.x <= vw - 4 && r.y <= vh - 4)) return null;
    return { href: "", text: text.slice(0, 90), x: r.x, y: r.y, w, h, kind };
  }).filter(Boolean);

  // In-content links (carry href): they feed both the reading animation and the
  // crawl frontier / knowledge graph. Chrome (nav/footer) is already excluded.
  const grabLinks = () => Array.from(root.querySelectorAll('a[href]')).map(e => {
    if (bad(e) || !vis(e)) return null;
    const text = (e.innerText || e.textContent || "").replace(/\s+/g, " ").trim();
    if (text.length < 2) return null;
    const r = e.getBoundingClientRect();
    if (!(r.width > 8 && r.height > 6 && r.x >= 0 && r.y >= 0 && r.x <= vw - 4 && r.y <= vh - 4)) return null;
    return { href: e.href, text: text.slice(0, 80), x: r.x, y: r.y, w: r.width, h: r.height, kind: 'link' };
  }).filter(Boolean);

  let out = [...grab('h1,h2,h3,h4', 'head', 3),
             ...grab('p,li,td,pre,blockquote,dd', 'text', 24),
             ...grabLinks()];

  // Reading order, de-duping boxes that start at the same spot (content wins over
  // a link that sits on the same line because it was pushed first).
  out.sort((a, b) => (a.y - b.y) || (a.x - b.x));
  const seen = [];
  for (const b of out) {
    if (!seen.some(d => Math.abs(d.y - b.y) < 6 && Math.abs(d.x - b.x) < 6)) seen.push(b);
  }
  return seen.slice(0, 48);
})()
"#;

pub async fn read_page(browser: &Browser, url: &str) -> Result<PageRead> {
    let page = browser.new_page("about:blank").await?;

    tokio::time::timeout(Duration::from_secs(20), async {
        page.goto(url).await?;
        page.wait_for_navigation().await?;
        anyhow::Ok(())
    })
    .await
    .map_err(|_| anyhow!("navigation timed out"))??;

    // settle for late layout and JS-rendered content (NVD, GitHub advisories,
    // etc. fetch their body after load), then make sure we're at the top so the
    // extracted boxes line up with the screenshot.
    tokio::time::sleep(Duration::from_millis(1800)).await;
    let _ = page.evaluate("window.scrollTo(0,0)").await;
    // Cookie/consent overlays cover the content in a capture. Hide them with CSS
    // only — nothing is clicked, so no consent is ever given on anyone's behalf.
    let _ = page.evaluate(HIDE_OVERLAYS_JS).await;

    let title = page.get_title().await?.unwrap_or_default();

    let links: Vec<LinkBox> = page
        .evaluate(EXTRACT_JS)
        .await?
        .into_value()
        .unwrap_or_default();

    let text: String = page
        .evaluate(TEXT_JS)
        .await
        .ok()
        .and_then(|v| v.into_value().ok())
        .unwrap_or_default();

    let links_all: Vec<String> = page
        .evaluate(LINKS_JS)
        .await
        .ok()
        .and_then(|v| v.into_value().ok())
        .unwrap_or_default();

    let jpeg = page
        .screenshot(
            ScreenshotParams::builder()
                .format(CaptureScreenshotFormat::Jpeg)
                .full_page(false)
                .build(),
        )
        .await?;

    let _ = page.close().await;

    Ok(PageRead {
        url: url.to_string(),
        title,
        jpeg,
        links,
        text,
        links_all,
    })
}

/// Every link on the page, absolute, fragment stripped, de-duplicated.
const LINKS_JS: &str = r#"
(() => {
  const out = new Set();
  for (const a of document.querySelectorAll('a[href]')) {
    try {
      const u = new URL(a.getAttribute('href'), location.href);
      if (u.protocol !== 'http:' && u.protocol !== 'https:') continue;
      u.hash = '';
      out.add(u.href);
    } catch (e) {}
    if (out.size >= 1500) break;
  }
  return Array.from(out);
})()
"#;

/// Cleaned, truncated visible text of the page.
const TEXT_JS: &str = r#"
(() => {
  // hide site chrome for a moment so innerText is the page's own content — the
  // same menu and footer repeated on every page would drown what is unique
  const st = document.createElement('style');
  st.textContent = 'nav,header,footer,aside,form,[role="navigation"],[role="banner"],' +
    '[role="contentinfo"],[role="search"],[aria-label*="breadcrumb" i],.breadcrumb,' +
    '[class*="cookie" i],[id*="cookie" i],[class*="consent" i]{display:none!important}';
  document.documentElement.appendChild(st);
  const clean = (el) => ((el && el.innerText) || "").replace(/\s+/g, " ").trim();
  const main = document.querySelector('main, article, [role="main"], #main-content, #content, #main');
  let t = clean(main);
  if (t.length < 200) t = clean(document.body);
  st.remove();
  return t.slice(0, 8000);
})()
"#;

/// robots.txt rules for `User-agent: *`, plus the sitemaps the site publishes.
#[derive(Debug, Default, Clone)]
pub struct RobotsRules {
    /// (allow?, pattern) — the longest matching pattern wins, Allow on a tie.
    rules: Vec<(bool, String)>,
    pub sitemaps: Vec<String>,
}

impl RobotsRules {
    pub fn parse(body: &str) -> Self {
        let mut out = RobotsRules::default();
        let mut star = false;
        let mut reading_agents = false;
        for raw in body.lines() {
            let line = raw.split('#').next().unwrap_or("").trim();
            let Some((k, v)) = line.split_once(':') else { continue };
            let (k, v) = (k.trim().to_ascii_lowercase(), v.trim().to_string());
            match k.as_str() {
                "user-agent" => {
                    if !reading_agents {
                        star = false;
                        reading_agents = true;
                    }
                    if v == "*" {
                        star = true;
                    }
                }
                "allow" | "disallow" => {
                    reading_agents = false;
                    if star && !v.is_empty() {
                        out.rules.push((k == "allow", v));
                    }
                }
                "sitemap" => {
                    // "Sitemap: https://…" — split_once cut at the scheme's colon
                    let full = line[line.find(':').map(|i| i + 1).unwrap_or(0)..].trim().to_string();
                    if full.starts_with("http") {
                        out.sitemaps.push(full);
                    }
                }
                _ => {}
            }
        }
        out
    }

    /// Is `path` (path + query) allowed for `User-agent: *`?
    pub fn allows(&self, path: &str) -> bool {
        let mut best: Option<(usize, bool)> = None;
        for (allow, pat) in &self.rules {
            if pattern_matches(pat, path) {
                let len = pat.len();
                best = match best {
                    Some((l, a)) if l > len || (l == len && a) => Some((l, a)),
                    _ => Some((len, *allow)),
                };
            }
        }
        best.map_or(true, |(_, allow)| allow)
    }
}

/// robots.txt pattern match: `*` is any run of characters, a trailing `$`
/// anchors the end; otherwise it is a prefix match.
fn pattern_matches(pattern: &str, path: &str) -> bool {
    let (pat, anchored) = match pattern.strip_suffix('$') {
        Some(p) => (p, true),
        None => (pattern, false),
    };
    let parts: Vec<&str> = pat.split('*').collect();
    if !path.starts_with(parts[0]) {
        return false;
    }
    let mut pos = parts[0].len();
    for part in parts.iter().skip(1) {
        if part.is_empty() {
            continue;
        }
        match path[pos..].find(part) {
            Some(off) => pos += off + part.len(),
            None => return false,
        }
    }
    if anchored {
        let last = parts.last().copied().unwrap_or("");
        return if parts.len() == 1 { path == pat } else { path.ends_with(last) };
    }
    true
}

type RobotsCache = tokio::sync::Mutex<std::collections::HashMap<String, (std::time::Instant, std::sync::Arc<RobotsRules>)>>;

fn robots_cache() -> &'static RobotsCache {
    static CACHE: std::sync::OnceLock<RobotsCache> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| tokio::sync::Mutex::new(std::collections::HashMap::new()))
}

/// The robots rules for a URL's origin — fetched once and kept for 6 hours, so a
/// site is asked for robots.txt once, not once per page.
pub async fn robots_for(target: &str) -> Option<std::sync::Arc<RobotsRules>> {
    let parsed = url::Url::parse(target).ok()?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return None;
    }
    let origin = parsed.origin().ascii_serialization();
    {
        let cache = robots_cache().lock().await;
        if let Some((at, rules)) = cache.get(&origin) {
            if at.elapsed() < Duration::from_secs(6 * 3600) {
                return Some(rules.clone());
            }
        }
    }
    let body = match reqwest::Client::new()
        .get(format!("{origin}/robots.txt"))
        .timeout(Duration::from_secs(8))
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => r.text().await.unwrap_or_default(),
        _ => String::new(), // no robots.txt → everything allowed
    };
    let rules = std::sync::Arc::new(RobotsRules::parse(&body));
    robots_cache().lock().await.insert(origin, (std::time::Instant::now(), rules.clone()));
    Some(rules)
}

/// Is this URL allowed by its site's robots.txt (`User-agent: *`)?
pub async fn robots_allows(target: &str) -> bool {
    let Ok(parsed) = url::Url::parse(target) else { return false };
    let Some(rules) = robots_for(target).await else { return false };
    let mut path = parsed.path().to_string();
    if let Some(q) = parsed.query() {
        path.push('?');
        path.push_str(q);
    }
    rules.allows(&path)
}

#[cfg(test)]
mod robots_tests {
    use super::RobotsRules;

    #[test]
    fn longest_match_and_wildcards() {
        let r = RobotsRules::parse(
            "User-agent: Googlebot\nDisallow: /\n\nUser-agent: *\nDisallow: /private/\nAllow: /private/ok\nDisallow: /*.pdf$\nDisallow: /*?\nSitemap: https://x.org/sitemap.xml\n",
        );
        assert!(r.allows("/docs/a.html"));
        assert!(!r.allows("/private/x"));
        assert!(r.allows("/private/ok/1"));
        assert!(!r.allows("/files/a.pdf"));
        assert!(r.allows("/files/a.pdf.html"));
        assert!(!r.allows("/search?q=1"));
        assert_eq!(r.sitemaps, vec!["https://x.org/sitemap.xml".to_string()]);
    }
}
