//! The hero live scene: octopus over the real page screenshot. Reads the focus
//! crawler's live link boxes as tentacle targets and swaps document every ~12s.

use leptos::html::{Canvas, Img};
use leptos::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement, HtmlImageElement};

use crate::octopus::Octopus;
use crate::raf::{run_raf, window};
use crate::ws::{http_base, Live};

const HOLD_MS: f64 = 12_000.0;

#[component]
pub fn OctopusScene(live: Live) -> impl IntoView {
    let canvas_ref = NodeRef::<Canvas>::new();
    let img_ref = NodeRef::<Img>::new();
    let title = RwSignal::new(String::from("waking the reef…"));

    Effect::new(move |_| {
        if let (Some(canvas), Some(img)) = (canvas_ref.get(), img_ref.get()) {
            let canvas: HtmlCanvasElement = canvas.unchecked_into();
            let img: HtmlImageElement = img.unchecked_into();
            start_loop(canvas, img, live, title);
        }
    });

    view! {
        <div class="scene__wrap">
            <img class="scene__doc" node_ref=img_ref hidden=true alt="" />
            <canvas node_ref=canvas_ref class="scene"></canvas>
            <div class="scene__title">{move || title.get()}</div>
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
        .to_string()
}

fn start_loop(canvas: HtmlCanvasElement, img: HtmlImageElement, live: Live, title: RwSignal<String>) {
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
    let mut hold_until: f64 = 0.0;
    let mut img_seq: u64 = u64::MAX;

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
            if !cs.iter().any(|c| !c.links.is_empty()) {
                return None;
            }
            if t > hold_until {
                let n = cs.len();
                for step in 1..=n {
                    let idx = (focus_idx + step) % n;
                    if !cs[idx].links.is_empty() {
                        focus_idx = idx;
                        break;
                    }
                }
                hold_until = t + HOLD_MS;
            }
            if focus_idx >= cs.len() || cs[focus_idx].links.is_empty() {
                focus_idx = cs.iter().position(|c| !c.links.is_empty()).unwrap_or(0);
            }
            let fc = &cs[focus_idx];
            let sx = cw / 1280.0;
            let sy = ch / 800.0;
            let mut boxes: Vec<([f64; 4], u8)> = fc
                .links
                .iter()
                .map(|b| {
                    let kind = match b.kind {
                        octopuscrawl_core::BoxKind::Head => 1u8,
                        octopuscrawl_core::BoxKind::Text => 2u8,
                        octopuscrawl_core::BoxKind::Link => 0u8,
                    };
                    ([b.x as f64 * sx, b.y as f64 * sy, b.w as f64 * sx, b.h as f64 * sy], kind)
                })
                .collect();
            // reading order: top-to-bottom, then left-to-right
            boxes.sort_by(|a, b| {
                a.0[1].partial_cmp(&b.0[1]).unwrap_or(std::cmp::Ordering::Equal)
                    .then(a.0[0].partial_cmp(&b.0[0]).unwrap_or(std::cmp::Ordering::Equal))
            });
            Some((fc.id.clone(), fc.name.clone(), host_of(&fc.url), fc.links_seq, boxes))
        });

        sim.resize(cw, ch);
        match doc {
            Some((id, name, host, seq, boxes)) => {
                sim.set_doc(boxes, seq);
                if seq != img_seq {
                    img_seq = seq;
                    img.set_src(&format!("{}/v1/crawlers/{id}/frame.jpg?seq={seq}", http_base()));
                    img.set_hidden(false);
                    title.set(format!("live · {name} reading {host}"));
                }
            }
            None => {
                sim.clear_doc();
                if img_seq != u64::MAX {
                    img.set_hidden(true);
                    img_seq = u64::MAX;
                    title.set("waking the reef…".to_string());
                }
            }
        }
        sim.step(dt);
        sim.draw(&ctx);
    });
}
