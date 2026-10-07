//! The live stats dashboard on the home page: count-up stat tiles with
//! sparklines, a dataset-growth chart, chapter coverage bars and a crawler
//! status strip. All live.

use leptos::html::Canvas;
use leptos::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement};

use crate::raf::{run_raf, window};
use crate::ws::Live;

fn ease(sig: RwSignal<f64>, target: f64) {
    let cur = sig.get_untracked();
    let next = cur + (target - cur) * 0.14;
    let next = if (target - next).abs() < 0.5 { target } else { next };
    if (next - cur).abs() > 1e-6 {
        sig.set(next);
    }
}

fn grouped(n: u64) -> String {
    let s = n.to_string();
    let len = s.len();
    let mut out = String::with_capacity(len + len / 3);
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (len - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

fn spark_points(vals: &[f64]) -> String {
    if vals.len() < 2 {
        return String::new();
    }
    let n = vals.len();
    let mn = vals.iter().cloned().fold(f64::INFINITY, f64::min);
    let mx = vals.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let range = (mx - mn).max(1e-9);
    let mut s = String::new();
    for (i, v) in vals.iter().enumerate() {
        let x = i as f64 / (n - 1) as f64 * 100.0;
        let y = 22.0 - ((v - mn) / range) * 20.0;
        s.push_str(&format!("{x:.1},{y:.1} "));
    }
    s
}

#[component]
pub fn StatsDashboard(live: Live) -> impl IntoView {
    let running = RwSignal::new(0.0f64);
    let total = RwSignal::new(0.0f64);
    let pages = RwSignal::new(0.0f64);
    let tokens = RwSignal::new(0.0f64);
    let domains = RwSignal::new(0.0f64);
    let chaps = RwSignal::new(0.0f64);
    let chaps_target = RwSignal::new(0.0f64);

    Effect::new(move |_| {
        run_raf(move |_| {
            if let Some(s) = live.stats.get_untracked() {
                let cd: u32 = s.chapters.iter().map(|c| c.covered).sum();
                let ct: u32 = s.chapters.iter().map(|c| c.target).sum();
                ease(running, s.crawlers_running as f64);
                ease(total, s.crawlers_total as f64);
                ease(pages, s.pages_read as f64);
                ease(tokens, s.dataset_tokens as f64);
                ease(domains, s.domains as f64);
                ease(chaps, cd as f64);
                ease(chaps_target, ct as f64);
            }
        });
    });

    let hist_running = move || live.history.get().iter().map(|s| s.running as f64).collect::<Vec<_>>();
    let hist_pages = move || live.history.get().iter().map(|s| s.pages as f64).collect::<Vec<_>>();
    let hist_tokens = move || live.history.get().iter().map(|s| s.tokens as f64).collect::<Vec<_>>();

    view! {
        <section class="dash">
            <div class="dash__tiles">
                <div class="t2">
                    <div class="t2__label">"hatchlings awake"</div>
                    <div class="t2__val num">
                        {move || format!("{} / {}", running.get().round() as u64, total.get().round() as u64)}
                    </div>
                    <svg class="spark" viewBox="0 0 100 24" preserveAspectRatio="none">
                        <polyline points=move || spark_points(&hist_running())></polyline>
                    </svg>
                </div>
                <div class="t2">
                    <div class="t2__label">"pages read"</div>
                    <div class="t2__val num">{move || grouped(pages.get().round() as u64)}</div>
                    <svg class="spark" viewBox="0 0 100 24" preserveAspectRatio="none">
                        <polyline points=move || spark_points(&hist_pages())></polyline>
                    </svg>
                </div>
                <div class="t2 t2--wide">
                    <div class="t2__label">"dataset tokens"</div>
                    <div class="t2__val num">{move || grouped(tokens.get().round() as u64)}</div>
                    <svg class="spark" viewBox="0 0 100 24" preserveAspectRatio="none">
                        <polyline points=move || spark_points(&hist_tokens())></polyline>
                    </svg>
                </div>
                <div class="t2">
                    <div class="t2__label">"domains"</div>
                    <div class="t2__val num">{move || grouped(domains.get().round() as u64)}</div>
                </div>
                <div class="t2">
                    <div class="t2__label">"chapters covered"</div>
                    <div class="t2__val num">
                        {move || format!("{} / {}", chaps.get().round() as u64, chaps_target.get().round() as u64)}
                    </div>
                </div>
            </div>

            <div class="dash__grid">
                <div class="pane dash__chart">
                    <div class="pane__bar">"dataset growth — tokens read over time"</div>
                    <GrowthChart live=live/>
                </div>
                <div class="pane dash__chaps">
                    <div class="pane__bar">"chapter coverage"</div>
                    <div class="chaps2">
                        <For
                            each=move || live.stats.get().map(|s| s.chapters).unwrap_or_default()
                            key=|c| format!("{}:{}:{}", c.id, c.covered, c.target)
                            let:ch
                        >
                            <div class="chap2">
                                <span class="chap2__n">{ch.title.clone()}</span>
                                <span class="bar2">
                                    <span
                                        class="bar2__f"
                                        style=format!(
                                            "width:{:.0}%",
                                            if ch.target == 0 { 0.0 } else { (ch.covered as f64 / ch.target as f64 * 100.0).min(100.0) },
                                        )
                                    ></span>
                                </span>
                                <span class="chap2__v num dim">
                                    {if matches!(ch.status, octopuscrawl_core::ChapterStatus::Locked) {
                                        "locked".to_string()
                                    } else {
                                        format!("{}/{}", ch.covered, ch.target)
                                    }}
                                </span>
                            </div>
                        </For>
                    </div>
                </div>
            </div>
        </section>
    }
}

#[component]
fn GrowthChart(live: Live) -> impl IntoView {
    let cref = NodeRef::<Canvas>::new();
    Effect::new(move |_| {
        if let Some(c) = cref.get() {
            let c: HtmlCanvasElement = c.unchecked_into();
            start_growth(c, live);
        }
    });
    view! { <div class="growth"><canvas node_ref=cref class="growth__cv"></canvas></div> }
}

fn start_growth(canvas: HtmlCanvasElement, live: Live) {
    let ctx: CanvasRenderingContext2d = match canvas.get_context("2d").ok().flatten() {
        Some(o) => match o.dyn_into() {
            Ok(c) => c,
            Err(_) => return,
        },
        None => return,
    };
    run_raf(move |_| {
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
        ctx.clear_rect(0.0, 0.0, cw, ch);

        let vals: Vec<f64> = live.history.with_untracked(|h| h.iter().map(|s| s.tokens as f64).collect());
        let pad = 10.0;
        // grid
        set_stroke(&ctx, "#16203f");
        ctx.set_line_width(1.0);
        for k in 0..=3 {
            let y = pad + (ch - 2.0 * pad) * k as f64 / 3.0;
            ctx.begin_path();
            ctx.move_to(pad, y);
            ctx.line_to(cw - pad, y);
            ctx.stroke();
        }
        if vals.len() < 2 {
            return;
        }
        let n = vals.len();
        let mn = vals.iter().cloned().fold(f64::INFINITY, f64::min);
        let mx = vals.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let range = (mx - mn).max(1.0);
        let xy = |i: usize, v: f64| -> (f64, f64) {
            let x = pad + (cw - 2.0 * pad) * i as f64 / (n - 1) as f64;
            let y = pad + (ch - 2.0 * pad) * (1.0 - (v - mn) / range);
            (x, y)
        };

        // area fill
        ctx.begin_path();
        let (x0, _) = xy(0, vals[0]);
        ctx.move_to(x0, ch - pad);
        for (i, v) in vals.iter().enumerate() {
            let (x, y) = xy(i, *v);
            ctx.line_to(x, y);
        }
        let (xl, _) = xy(n - 1, vals[n - 1]);
        ctx.line_to(xl, ch - pad);
        ctx.close_path();
        ctx.set_global_alpha(0.14);
        set_fill(&ctx, "#2ad4ff");
        ctx.fill();
        ctx.set_global_alpha(1.0);

        // line
        ctx.save();
        ctx.set_shadow_blur(8.0);
        ctx.set_shadow_color("#2ad4ff");
        set_stroke(&ctx, "#35e0ff");
        ctx.set_line_width(2.0);
        ctx.begin_path();
        for (i, v) in vals.iter().enumerate() {
            let (x, y) = xy(i, *v);
            if i == 0 {
                ctx.move_to(x, y);
            } else {
                ctx.line_to(x, y);
            }
        }
        ctx.stroke();
        ctx.restore();

        // endpoint dot
        let (ex, ey) = xy(n - 1, vals[n - 1]);
        set_fill(&ctx, "#eaf2ff");
        ctx.begin_path();
        let _ = ctx.arc(ex, ey, 3.0, 0.0, 6.2832);
        ctx.fill();
    });
}

fn set_fill(c: &CanvasRenderingContext2d, s: &str) {
    c.set_fill_style(&wasm_bindgen::JsValue::from_str(s));
}
fn set_stroke(c: &CanvasRenderingContext2d, s: &str) {
    c.set_stroke_style(&wasm_bindgen::JsValue::from_str(s));
}
