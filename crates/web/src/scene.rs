//! The hero live scene: the real page a hatchling just captured, painted into the
//! canvas, with the octopus reading it top-to-bottom. Each line she touches gets a
//! highlighter stroke; lines that carry something that matters (a CVE id, a CVSS
//! score, an RCE, a mitigation, an RFC "MUST"…) get a bold amber stroke and a tag.
//! One page is read through to the end, then the scene moves to another hatchling.

use leptos::html::{Canvas, Img};
use leptos::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement, HtmlImageElement};

use crate::octopus::{DocBox, Octopus};
use crate::raf::{run_raf, window};
use crate::ws::{http_base, Live};

/// Never sit on one page longer than this, even if it has many lines.
const MAX_HOLD_MS: f64 = 18_000.0;
/// After the last line is read, linger so the finished page can be seen.
const LINGER_MS: f64 = 2_600.0;
/// Cap on tagged lines per page so the tags stay meaningful.
const MAX_KEYS: usize = 7;

#[component]
pub fn OctopusScene(live: Live) -> impl IntoView {
    let canvas_ref = NodeRef::<Canvas>::new();
    let img_ref = NodeRef::<Img>::new();
    let title = RwSignal::new(String::from("waking the reef…"));
    let url = RwSignal::new(String::from("octopuscrawl.net"));
    let progress = RwSignal::new((0usize, 0usize, 0usize));

    Effect::new(move |_| {
        if let (Some(canvas), Some(img)) = (canvas_ref.get(), img_ref.get()) {
            let canvas: HtmlCanvasElement = canvas.unchecked_into();
            let img: HtmlImageElement = img.unchecked_into();
            start_loop(canvas, img, live, title, url, progress);
        }
    });

    let prog_on = move || progress.get().1 > 0;
    let pct = move || {
        let (d, n, _) = progress.get();
        if n == 0 { 0.0 } else { d as f64 / n as f64 * 100.0 }
    };
    view! {
        <div class="stage">
            <div class="stage__chrome">
                <span class="stage__dots" aria-hidden="true"><i></i><i></i><i></i></span>
                <span class="stage__url">{move || url.get()}</span>
                <span class="stage__live"><span class="dot on"></span>"live"</span>
            </div>
            <div class="scene__wrap">
                <img class="scene__doc" node_ref=img_ref hidden=true alt="" />
                <canvas node_ref=canvas_ref class="scene"></canvas>
            </div>
            <div class="stage__foot" class:stage__foot--on=prog_on>
                <span class="stage__who">{move || title.get()}</span>
                <span class="stage__bar"><span class="stage__fill" style=move || format!("width:{:.1}%", pct())></span></span>
                <span class="num stage__n">{move || { let (d, n, _) = progress.get(); format!("{d}/{n} lines") }}</span>
                <span class="num stage__k">
                    {move || {
                        let k = progress.get().2;
                        if k == 1 { "1 key fact".to_string() } else { format!("{k} key facts") }
                    }}
                </span>
                <span class="stage__legend" aria-label="highlight legend">
                    <span class="lg lg--key">"key fact"</span>
                    <span class="lg lg--head">"heading"</span>
                    <span class="lg lg--body">"read"</span>
                </span>
            </div>
        </div>
    }
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

/// What makes a line worth a tag — the facts a security reader looks for first.
fn key_tag(text: &str) -> Option<&'static str> {
    let low = text.to_ascii_lowercase();
    let has = |s: &str| low.contains(s);
    let word = |w: &str| {
        low.split(|c: char| !c.is_ascii_alphanumeric()).any(|t| t == w)
    };
    if has("cve-") {
        return Some("CVE");
    }
    if has("[dsa") || has("dsa-") || has("usn-") || has("security update") || has("security advisory") || has("ghsa-") {
        return Some("advisory");
    }
    if has("cvss") || has("base score") {
        return Some("CVSS");
    }
    if has("remote code execution") || has("arbitrary code") || word("rce") {
        return Some("RCE");
    }
    if has("critical") || has("severity") {
        return Some("severity");
    }
    if has("exploit") {
        return Some("exploit");
    }
    if has("cwe-") {
        return Some("CWE");
    }
    // MITRE ATT&CK technique ids: T1059, T1059.001 …
    if low
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|t| t.len() == 5 && t.starts_with("t1") && t[1..].chars().all(|c| c.is_ascii_digit()))
    {
        return Some("ATT&CK");
    }
    if has("sql injection") || has("command injection") || word("injection") {
        return Some("injection");
    }
    if has("cross-site scripting") || word("xss") {
        return Some("XSS");
    }
    if has("overflow") || has("use-after-free") || has("out-of-bounds") {
        return Some("memory");
    }
    if has("privilege") {
        return Some("privesc");
    }
    if has("bypass") {
        return Some("bypass");
    }
    if has("mitigat") || has("remediat") || has("workaround") {
        return Some("mitigation");
    }
    if has("fixed in") || has("upgrade to") || has("update to") || word("patch") || word("patched") {
        return Some("fix");
    }
    // RFC 2119 keywords are written in capitals on purpose
    if text.contains("MUST NOT") || text.contains(" MUST ") || text.contains("SHALL") {
        return Some("MUST");
    }
    None
}

#[derive(Clone)]
struct Doc {
    id: String,
    name: String,
    host: String,
    url: String,
    seq: u64,
    boxes: Vec<DocBox>,
}

fn start_loop(
    canvas: HtmlCanvasElement,
    img: HtmlImageElement,
    live: Live,
    title: RwSignal<String>,
    url: RwSignal<String>,
    progress: RwSignal<(usize, usize, usize)>,
) {
    let ctx: CanvasRenderingContext2d = canvas
        .get_context("2d")
        .ok()
        .flatten()
        .expect("2d ctx")
        .dyn_into()
        .expect("ctx cast");

    let mut sim = Octopus::new(800.0, 500.0);
    let mut last = 0.0f64;
    let mut focus_idx: usize = 0;
    let mut cur: Option<Doc> = None;
    let mut started = 0.0f64;
    let mut done_at: Option<f64> = None;
    let mut img_loaded_at: Option<f64> = None;
    let mut last_host = String::new();
    let mut size = (0.0f64, 0.0f64);

    run_raf(move |t| {
        let dpr = window().device_pixel_ratio().max(1.0);
        let cw = canvas.client_width() as f64;
        let ch = canvas.client_height() as f64;
        if cw <= 1.0 || ch <= 1.0 {
            return;
        }
        let bw = (cw * dpr) as u32;
        let bh = (ch * dpr) as u32;
        if canvas.width() != bw {
            canvas.set_width(bw);
        }
        if canvas.height() != bh {
            canvas.set_height(bh);
        }
        let _ = ctx.set_transform(dpr, 0.0, 0.0, dpr, 0.0, 0.0);

        let dt = if last == 0.0 { 0.016 } else { ((t - last) / 1000.0).min(0.05) };
        last = t;

        // a resize re-lays the page, so re-read it at the new scale
        let resized = (size.0 - cw).abs() > 1.0 || (size.1 - ch).abs() > 1.0;
        size = (cw, ch);

        // move on once the page is read through (plus a short linger) or the
        // hold runs out — never mid-line
        let finished = done_at.map_or(false, |d| t - d > LINGER_MS);
        let need_next = cur.is_none() || finished || t - started > MAX_HOLD_MS || resized;

        if need_next {
            let next = live.crawlers.with_untracked(|cs| {
                let n = cs.len();
                if n == 0 {
                    return None;
                }
                // prefer a hatchling on a different site than the one just read
                let mut pick: Option<usize> = None;
                for step in 1..=n {
                    let idx = (focus_idx + step) % n;
                    let c = &cs[idx];
                    if c.links.is_empty() {
                        continue;
                    }
                    if pick.is_none() {
                        pick = Some(idx);
                    }
                    if host_of(&c.url) != last_host {
                        pick = Some(idx);
                        break;
                    }
                }
                let idx = pick?;
                focus_idx = idx;
                let fc = &cs[idx];
                let sx = cw / 1280.0;
                let sy = ch / 800.0;
                let mut keys = 0usize;
                let mut boxes: Vec<DocBox> = fc
                    .links
                    .iter()
                    // only what is actually on the captured screen
                    .filter(|b| b.y >= 0.0 && b.y + b.h <= 800.0 && b.w > 4.0 && b.h > 3.0)
                    .map(|b| {
                        let kind = match b.kind {
                            octopuscrawl_core::BoxKind::Head => 1u8,
                            octopuscrawl_core::BoxKind::Text => 2u8,
                            octopuscrawl_core::BoxKind::Link => 0u8,
                        };
                        DocBox {
                            rect: [b.x as f64 * sx, b.y as f64 * sy, b.w as f64 * sx, b.h as f64 * sy],
                            kind,
                            tag: key_tag(&b.text),
                        }
                    })
                    .collect();
                boxes.sort_by(|a, b| {
                    a.rect[1]
                        .partial_cmp(&b.rect[1])
                        .unwrap_or(std::cmp::Ordering::Equal)
                        .then(a.rect[0].partial_cmp(&b.rect[0]).unwrap_or(std::cmp::Ordering::Equal))
                });
                for b in boxes.iter_mut() {
                    if b.tag.is_some() {
                        keys += 1;
                        if keys > MAX_KEYS {
                            b.tag = None;
                        }
                    }
                }
                Some(Doc {
                    id: fc.id.clone(),
                    name: fc.name.clone(),
                    host: host_of(&fc.url),
                    url: fc
                        .url
                        .trim_start_matches("https://")
                        .trim_start_matches("http://")
                        .trim_start_matches("www.")
                        .to_string(),
                    seq: fc.links_seq,
                    boxes,
                })
            });
            if let Some(d) = next {
                let new_page = cur.as_ref().map_or(true, |c| c.seq != d.seq || c.id != d.id);
                if new_page {
                    img.set_src(&format!("{}/v1/crawlers/{}/frame.jpg?seq={}", http_base(), d.id, d.seq));
                    img_loaded_at = None;
                }
                title.set(format!("{} · {}", d.name, d.host));
                url.set(d.url.clone());
                last_host = d.host.clone();
                sim.resize(cw, ch);
                sim.set_doc(d.boxes.clone());
                started = t;
                done_at = None;
                cur = Some(d);
            }
        }

        sim.resize(cw, ch);
        if cur.is_none() {
            sim.clear_doc();
        }
        sim.step(dt);

        let (d, n, k) = sim.progress();
        if progress.get_untracked() != (d, n, k) {
            progress.set((d, n, k));
        }
        if n > 0 && d >= n && done_at.is_none() {
            done_at = Some(t);
        }

        // paint the captured page first, fading in when a new one arrives, so the
        // highlighter strokes can blend into it like ink on paper
        ctx.clear_rect(0.0, 0.0, cw, ch);
        let ready = cur.is_some() && img.complete() && img.natural_width() > 0;
        if ready {
            let at = *img_loaded_at.get_or_insert(t);
            let a = ((t - at) / 320.0).clamp(0.0, 1.0);
            set_fill(&ctx, "#05070f");
            ctx.fill_rect(0.0, 0.0, cw, ch);
            ctx.set_global_alpha(a);
            let _ = ctx.draw_image_with_html_image_element_and_dw_and_dh(&img, 0.0, 0.0, cw, ch);
            ctx.set_global_alpha(1.0);
        } else if cur.is_some() {
            set_fill(&ctx, "#05070f");
            ctx.fill_rect(0.0, 0.0, cw, ch);
        }
        sim.draw(&ctx);
    });
}

fn set_fill(c: &CanvasRenderingContext2d, s: &str) {
    c.set_fill_style(&wasm_bindgen::JsValue::from_str(s));
}
