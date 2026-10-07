//! One crawler tile for the /crawlers board: a small octopus over the exact
//! document that hatchling is reading. Drives its own cleanup-safe rAF loop.

use leptos::html::{Canvas, Img};
use leptos::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement, HtmlImageElement};

use crate::octopus::{DocBox, Octopus};
use crate::raf::{run_raf, window};
use crate::ws::{http_base, Live};

#[component]
pub fn Tile(id: String) -> impl IntoView {
    let live = use_context::<Live>().expect("live");
    let canvas_ref = NodeRef::<Canvas>::new();
    let img_ref = NodeRef::<Img>::new();

    let fid = id.clone();
    let snap = move || live.crawlers.with(|cs| cs.iter().find(|c| c.id == fid).cloned());

    {
        let id = id.clone();
        Effect::new(move |_| {
            if let (Some(cv), Some(im)) = (canvas_ref.get(), img_ref.get()) {
                start_tile(cv.unchecked_into(), im.unchecked_into(), live, id.clone());
            }
        });
    }

    let s_name = snap.clone();
    let s_owner = snap.clone();
    let s_status = snap.clone();
    let s_letter = snap.clone();
    let s_host = snap.clone();
    let s_thought = snap.clone();

    view! {
        <div class="tile">
            <div class="tile__screen">
                <img class="tile__doc" node_ref=img_ref hidden=true alt="" />
                <canvas class="tile__cv" node_ref=canvas_ref></canvas>
            </div>
            <div class="tile__ft">
                <div class="tile__row">
                    <span class="tile__name">
                        {move || s_name().map(|c| c.name).unwrap_or_default()}
                        {move || s_owner().and_then(|c| c.owner).map(|o| {
                            let n = o.chars().count();
                            let short = if n > 8 {
                                format!("{}…{}", o.chars().take(4).collect::<String>(), o.chars().skip(n - 4).collect::<String>())
                            } else {
                                o.clone()
                            };
                            view! { <span class="owned" title=o>"◆ "{short}</span> }
                        })}
                    </span>
                    <span class=move || s_status().map(|c| status_cls(&c)).unwrap_or("s")>
                        {move || s_letter().map(|c| c.status.letter().to_string()).unwrap_or_default()}
                    </span>
                </div>
                <div class="tile__host dim">{move || s_host().map(|c| host_of(&c.url)).unwrap_or_default()}</div>
                <div class="tile__thought dim">{move || s_thought().map(|c| c.thought).unwrap_or_default()}</div>
            </div>
        </div>
    }
}

fn start_tile(canvas: HtmlCanvasElement, img: HtmlImageElement, live: Live, id: String) {
    let ctx: CanvasRenderingContext2d = match canvas.get_context("2d").ok().flatten() {
        Some(o) => match o.dyn_into() {
            Ok(c) => c,
            Err(_) => return,
        },
        None => return,
    };

    let mut sim = Octopus::new(240.0, 150.0);
    let mut last = 0.0f64;
    let mut img_seq = u64::MAX;

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

        let doc = live.crawlers.with_untracked(|cs| {
            let fc = cs.iter().find(|c| c.id == id)?;
            if fc.links.is_empty() {
                return None;
            }
            let sx = cw / 1280.0;
            let sy = ch / 800.0;
            let mut boxes: Vec<DocBox> = fc
                .links
                .iter()
                .filter(|b| b.y >= 0.0 && b.y + b.h <= 800.0)
                .map(|b| {
                    let kind = match b.kind {
                        octopuscrawl_core::BoxKind::Head => 1u8,
                        octopuscrawl_core::BoxKind::Text => 2u8,
                        octopuscrawl_core::BoxKind::Link => 0u8,
                    };
                    DocBox {
                        rect: [b.x as f64 * sx, b.y as f64 * sy, b.w as f64 * sx, b.h as f64 * sy],
                        kind,
                        tag: None,
                    }
                })
                .collect();
            boxes.sort_by(|a, b| {
                a.rect[1].partial_cmp(&b.rect[1]).unwrap_or(std::cmp::Ordering::Equal)
                    .then(a.rect[0].partial_cmp(&b.rect[0]).unwrap_or(std::cmp::Ordering::Equal))
            });
            Some((fc.links_seq, boxes))
        });

        sim.resize(cw, ch);
        match doc {
            Some((seq, boxes)) => {
                if seq != img_seq {
                    sim.set_doc(boxes);
                    img_seq = seq;
                    img.set_src(&format!("{}/v1/crawlers/{id}/frame.jpg?seq={seq}", http_base()));
                    img.set_hidden(false);
                }
            }
            None => {
                sim.clear_doc();
                if img_seq != u64::MAX {
                    img.set_hidden(true);
                    img_seq = u64::MAX;
                }
            }
        }
        sim.step(dt);
        // the page itself is the <img> under this canvas
        ctx.clear_rect(0.0, 0.0, cw, ch);
        sim.draw(&ctx);
    });
}

fn status_cls(c: &octopuscrawl_core::Crawler) -> &'static str {
    match c.status {
        octopuscrawl_core::CrawlerStatus::Crawling => "s s-r",
        octopuscrawl_core::CrawlerStatus::Returning => "s s-w",
        octopuscrawl_core::CrawlerStatus::Idle => "s s-s",
        octopuscrawl_core::CrawlerStatus::Error => "s s-e",
    }
}

fn host_of(url: &str) -> String {
    url.strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url)
        .split('/')
        .next()
        .unwrap_or("")
        .to_string()
}
