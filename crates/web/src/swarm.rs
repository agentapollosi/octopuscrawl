//! Fleet radar: the whole swarm of hatchlings at a glance — one pulsing dot per
//! crawler, arranged in a ring around the queen, coloured by status, with a live
//! thread drawn from the queen to each one that is currently reading.

use leptos::html::Canvas;
use leptos::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement};

use octopuscrawl_core::CrawlerStatus;

use crate::raf::{run_raf, window};
use crate::ws::Live;

fn status_color(s: CrawlerStatus) -> &'static str {
    match s {
        CrawlerStatus::Crawling => "#35e0ff",
        CrawlerStatus::Returning => "#ffd166",
        CrawlerStatus::Idle => "#5f7199",
        CrawlerStatus::Error => "#ff5a7a",
    }
}

#[derive(Clone)]
struct Dot {
    color: &'static str,
    host: String,
    name: String,
    active: bool,
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

#[component]
pub fn SwarmRadar(live: Live) -> impl IntoView {
    let cref = NodeRef::<Canvas>::new();
    let hover = RwSignal::new((-1.0f64, -1.0f64));
    Effect::new(move |_| {
        if let Some(c) = cref.get() {
            let c: HtmlCanvasElement = c.unchecked_into();
            start(c, live, hover);
        }
    });
    let running = move || live.crawlers.with(|cs| cs.iter().filter(|c| c.status == CrawlerStatus::Crawling).count());
    let total = move || live.crawlers.with(|c| c.len());
    view! {
        <section class="pane swarm">
            <div class="pane__bar">
                "the swarm"
                <span class="dim">" — every hatchling, live · "</span>
                <span class="num">{move || running().to_string()}</span>
                <span class="dim">" / "</span>
                <span class="num">{move || total().to_string()}</span>
                <span class="dim">" reading"</span>
            </div>
            <div class="swarm__wrap">
                <canvas
                    node_ref=cref
                    class="swarm__cv"
                    on:mousemove=move |e| hover.set((e.offset_x() as f64, e.offset_y() as f64))
                    on:mouseleave=move |_| hover.set((-1.0, -1.0))
                ></canvas>
            </div>
        </section>
    }
}

fn start(canvas: HtmlCanvasElement, live: Live, hover: RwSignal<(f64, f64)>) {
    let ctx: CanvasRenderingContext2d = match canvas.get_context("2d").ok().flatten() {
        Some(o) => match o.dyn_into() {
            Ok(c) => c,
            Err(_) => return,
        },
        None => return,
    };
    let mut t = 0.0f64;
    let mut last = 0.0f64;

    run_raf(move |now| {
        let dpr = window().device_pixel_ratio().max(1.0);
        let cw = canvas.client_width() as f64;
        let ch = canvas.client_height() as f64;
        if cw < 2.0 || ch < 2.0 {
            return;
        }
        let (bw, bh) = ((cw * dpr) as u32, (ch * dpr) as u32);
        if canvas.width() != bw {
            canvas.set_width(bw);
        }
        if canvas.height() != bh {
            canvas.set_height(bh);
        }
        let _ = ctx.set_transform(dpr, 0.0, 0.0, dpr, 0.0, 0.0);
        let dt = if last == 0.0 { 0.016 } else { ((now - last) / 1000.0).min(0.05) };
        last = now;
        t += dt;

        let dots: Vec<Dot> = live.crawlers.with_untracked(|cs| {
            cs.iter()
                .map(|c| Dot {
                    color: status_color(c.status),
                    host: host_of(&c.url),
                    name: c.name.clone(),
                    active: c.status == CrawlerStatus::Crawling,
                })
                .collect()
        });

        ctx.clear_rect(0.0, 0.0, cw, ch);
        let n = dots.len();
        if n == 0 {
            return;
        }
        let cx = cw * 0.5;
        let cy = ch * 0.5;
        let radius = cw.min(ch) * 0.40;

        // queen core
        let pulse = 0.5 + 0.5 * (t * 1.6).sin();
        ctx.save();
        ctx.set_shadow_blur(16.0 + 8.0 * pulse);
        ctx.set_shadow_color("#2ad4ff");
        set_fill(&ctx, "#0a1836");
        circle(&ctx, cx, cy, 11.0);
        set_stroke(&ctx, "#2ad4ff");
        ctx.set_line_width(1.5);
        ctx.set_global_alpha(0.6 + 0.3 * pulse);
        ctx.begin_path();
        let _ = ctx.arc(cx, cy, 11.0, 0.0, std::f64::consts::TAU);
        ctx.stroke();
        ctx.restore();
        ctx.set_global_alpha(1.0);

        let mut positions: Vec<(f64, f64)> = Vec::with_capacity(n);
        for (i, d) in dots.iter().enumerate() {
            let base = i as f64 / n as f64 * std::f64::consts::TAU - std::f64::consts::FRAC_PI_2;
            let wob = (t * 0.5 + i as f64 * 0.7).sin() * 6.0;
            let r = radius + wob;
            let x = cx + base.cos() * r;
            let y = cy + base.sin() * r;
            positions.push((x, y));

            // thread from the queen to active readers
            if d.active {
                set_stroke(&ctx, "#16406a");
                ctx.set_line_width(1.0);
                ctx.set_global_alpha(0.5 + 0.3 * (t * 2.0 + i as f64).sin().abs());
                ctx.begin_path();
                ctx.move_to(cx, cy);
                ctx.line_to(x, y);
                ctx.stroke();
                ctx.set_global_alpha(1.0);
            }
        }

        for (i, d) in dots.iter().enumerate() {
            let (x, y) = positions[i];
            let p = 0.5 + 0.5 * (t * 2.2 + i as f64 * 0.5).sin();
            let r = if d.active { 4.0 + 1.6 * p } else { 3.0 };
            ctx.save();
            if d.active {
                ctx.set_shadow_blur(8.0);
                ctx.set_shadow_color(d.color);
            }
            set_fill(&ctx, d.color);
            ctx.set_global_alpha(if d.active { 1.0 } else { 0.55 });
            circle(&ctx, x, y, r);
            ctx.restore();
            ctx.set_global_alpha(1.0);
        }

        // hover label
        let (hx, hy) = hover.get_untracked();
        if hx >= 0.0 {
            let mut best = None;
            let mut bd = 16.0 * 16.0;
            for (i, (x, y)) in positions.iter().enumerate() {
                let d2 = (x - hx) * (x - hx) + (y - hy) * (y - hy);
                if d2 < bd {
                    bd = d2;
                    best = Some(i);
                }
            }
            if let Some(i) = best {
                let (x, y) = positions[i];
                let d = &dots[i];
                set_stroke(&ctx, "#eaf2ff");
                ctx.set_line_width(1.2);
                ctx.begin_path();
                let _ = ctx.arc(x, y, 7.0, 0.0, std::f64::consts::TAU);
                ctx.stroke();
                let label = format!("{} · {}", d.name, if d.host.is_empty() { "idle" } else { &d.host });
                ctx.set_font("11px 'IBM Plex Mono', monospace");
                let tw = label.chars().count() as f64 * 6.6 + 12.0;
                let bx = (x + 10.0).min(cw - tw - 4.0).max(4.0);
                let by = (y - 22.0).max(4.0);
                ctx.set_global_alpha(0.92);
                set_fill(&ctx, "#060a16");
                ctx.fill_rect(bx, by, tw, 18.0);
                set_stroke(&ctx, d.color);
                ctx.set_line_width(1.0);
                ctx.stroke_rect(bx, by, tw, 18.0);
                ctx.set_global_alpha(1.0);
                set_fill(&ctx, "#eaf2ff");
                let _ = ctx.fill_text(&label, bx + 6.0, by + 12.5);
            }
        }
    });
}

fn set_fill(c: &CanvasRenderingContext2d, s: &str) {
    c.set_fill_style(&wasm_bindgen::JsValue::from_str(s));
}
fn set_stroke(c: &CanvasRenderingContext2d, s: &str) {
    c.set_stroke_style(&wasm_bindgen::JsValue::from_str(s));
}
fn circle(c: &CanvasRenderingContext2d, x: f64, y: f64, r: f64) {
    c.begin_path();
    let _ = c.arc(x, y, r.max(0.1), 0.0, std::f64::consts::TAU);
    c.fill();
}
