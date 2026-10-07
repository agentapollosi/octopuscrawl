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
                    <div class="pane__bar">"dataset growth — tokens over the last 24h"<span class="dim">" · hover for any minute"</span></div>
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
    let hover = RwSignal::new(-1.0f64);
    Effect::new(move |_| {
        if let Some(c) = cref.get() {
            let c: HtmlCanvasElement = c.unchecked_into();
            start_growth(c, live, hover);
        }
    });
    view! {
        <div class="growth">
            <canvas
                node_ref=cref
                class="growth__cv"
                on:mousemove=move |e| hover.set(e.offset_x() as f64)
                on:mouseleave=move |_| hover.set(-1.0)
            ></canvas>
        </div>
    }
}

fn hhmm(t: i64) -> String {
    let d = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(t as f64 * 1000.0));
    format!("{:02}:{:02}", d.get_utc_hours(), d.get_utc_minutes())
}

fn short_num(v: f64) -> String {
    if v >= 1_000_000.0 {
        format!("{:.2}M", v / 1_000_000.0)
    } else if v >= 1000.0 {
        format!("{:.0}k", v / 1000.0)
    } else {
        format!("{v:.0}")
    }
}

fn start_growth(canvas: HtmlCanvasElement, live: Live, hover: RwSignal<f64>) {
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

        // server history (24h, one point a minute) + the live value as the tip
        let mut pts: Vec<(f64, f64, f64)> = live
            .growth
            .with_untracked(|g| g.iter().map(|p| (p.t as f64, p.tokens as f64, p.pages as f64)).collect());
        // the live counters as the newest point, stamped with the server's clock so
        // it lines up with the server-sampled history
        if let Some(s) = live.stats.get_untracked() {
            let now = s.updated_at.timestamp() as f64;
            pts.push((now, s.dataset_tokens as f64, s.pages_read as f64));
        }

        let (pl, pr, pt, pb) = (46.0, 14.0, 12.0, 24.0);
        let (x0, x1, y0, y1) = (pl, cw - pr, pt, ch - pb);

        ctx.set_font("10px 'IBM Plex Mono', monospace");
        set_stroke(&ctx, "#16203f");
        ctx.set_line_width(1.0);
        for k in 0..=3 {
            let y = y0 + (y1 - y0) * k as f64 / 3.0;
            ctx.begin_path();
            ctx.move_to(x0, y);
            ctx.line_to(x1, y);
            ctx.stroke();
        }
        if pts.len() < 2 {
            set_fill(&ctx, "#5f7199");
            let _ = ctx.fill_text("collecting the first samples…", x0 + 8.0, (y0 + y1) * 0.5);
            return;
        }
        let tmin = pts[0].0;
        let tmax = pts[pts.len() - 1].0.max(tmin + 60.0);
        let vmin = pts.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
        let vmax = pts.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
        // pad the range so a flat stretch sits mid-chart, not on the floor
        let span = (vmax - vmin).max(vmax * 0.02).max(1.0);
        let lo = (vmin - span * 0.08).max(0.0);
        let hi = vmax + span * 0.08;
        let xy = |t: f64, v: f64| -> (f64, f64) {
            (x0 + (x1 - x0) * (t - tmin) / (tmax - tmin), y1 - (y1 - y0) * (v - lo) / (hi - lo))
        };

        // axis labels: value range + time range (UTC)
        set_fill(&ctx, "#5f7199");
        ctx.set_text_align("right");
        let _ = ctx.fill_text(&short_num(hi), x0 - 6.0, y0 + 4.0);
        let _ = ctx.fill_text(&short_num(lo), x0 - 6.0, y1 + 3.0);
        ctx.set_text_align("left");
        let _ = ctx.fill_text(&format!("{} UTC", hhmm(tmin as i64)), x0, ch - 7.0);
        ctx.set_text_align("right");
        let _ = ctx.fill_text("now", x1, ch - 7.0);
        ctx.set_text_align("left");

        // area
        ctx.begin_path();
        let (sx, _) = xy(pts[0].0, pts[0].1);
        ctx.move_to(sx, y1);
        for p in &pts {
            let (x, y) = xy(p.0, p.1);
            ctx.line_to(x, y);
        }
        let (ex, _) = xy(pts[pts.len() - 1].0, 0.0);
        ctx.line_to(ex, y1);
        ctx.close_path();
        ctx.set_global_alpha(0.14);
        set_fill(&ctx, "#2ad4ff");
        ctx.fill();
        ctx.set_global_alpha(1.0);

        // line (stepped: tokens only change when a page lands)
        ctx.save();
        ctx.set_shadow_blur(8.0);
        ctx.set_shadow_color("#2ad4ff");
        set_stroke(&ctx, "#35e0ff");
        ctx.set_line_width(2.0);
        ctx.begin_path();
        let mut prev: Option<(f64, f64)> = None;
        for p in &pts {
            let (x, y) = xy(p.0, p.1);
            match prev {
                None => ctx.move_to(x, y),
                Some((_, py)) => {
                    ctx.line_to(x, py);
                    ctx.line_to(x, y);
                }
            }
            prev = Some((x, y));
        }
        ctx.stroke();
        ctx.restore();

        let last = pts[pts.len() - 1];
        let (lx, ly) = xy(last.0, last.1);
        set_fill(&ctx, "#eaf2ff");
        ctx.begin_path();
        let _ = ctx.arc(lx, ly, 3.0, 0.0, 6.2832);
        ctx.fill();

        // hover: crosshair + the reading at that minute
        let hx = hover.get_untracked();
        if hx >= x0 && hx <= x1 {
            let t = tmin + (hx - x0) / (x1 - x0) * (tmax - tmin);
            let mut best = &pts[0];
            for p in &pts {
                if p.0 <= t {
                    best = p;
                }
            }
            let (bx, by) = xy(t, best.1);
            set_stroke(&ctx, "#3b496b");
            ctx.set_line_width(1.0);
            ctx.begin_path();
            ctx.move_to(bx, y0);
            ctx.line_to(bx, y1);
            ctx.stroke();
            set_fill(&ctx, "#eaf2ff");
            ctx.begin_path();
            let _ = ctx.arc(bx, by, 3.5, 0.0, 6.2832);
            ctx.fill();
            let label = format!(
                "{} UTC · {} tokens · {} pages",
                hhmm(t as i64),
                grouped(best.1 as u64),
                grouped(best.2 as u64)
            );
            let tw = label.chars().count() as f64 * 6.1 + 14.0;
            let tx = (bx + 10.0).min(x1 - tw).max(x0);
            let ty = (by - 30.0).max(y0);
            ctx.set_global_alpha(0.94);
            set_fill(&ctx, "#060a16");
            ctx.fill_rect(tx, ty, tw, 20.0);
            set_stroke(&ctx, "#1b2a4a");
            ctx.stroke_rect(tx, ty, tw, 20.0);
            ctx.set_global_alpha(1.0);
            set_fill(&ctx, "#eaf2ff");
            let _ = ctx.fill_text(&label, tx + 7.0, ty + 14.0);
        }
    });
}

fn set_fill(c: &CanvasRenderingContext2d, s: &str) {
    c.set_fill_style(&wasm_bindgen::JsValue::from_str(s));
}
fn set_stroke(c: &CanvasRenderingContext2d, s: &str) {
    c.set_stroke_style(&wasm_bindgen::JsValue::from_str(s));
}
