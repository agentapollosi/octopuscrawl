//! The live knowledge graph: a force-directed map of every page the crawl has
//! read (nodes, coloured by chapter, sized by tokens) and the real links it
//! followed between them (edges). All data is live from `/v1/graph` + the feed.

use std::collections::HashMap;

use leptos::html::Canvas;
use leptos::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement};

use crate::raf::{run_raf, window};
use crate::ws::Live;

/// Fixed chapter → hue, assigned in a stable order (identity is always paired
/// with the legend label, never colour alone).
pub fn chapter_color(ch: &str) -> &'static str {
    match ch {
        "advisories" => "#ff6b6b",
        "patches" => "#4cd7c0",
        "standards" => "#2ad4ff",
        "writeups" => "#ffd166",
        "tooling" => "#b794f6",
        "forums" => "#7bed9f",
        "rfcs" => "#ff9f6b",
        _ => "#8aa0c6",
    }
}

const CHAPTERS: &[&str] = &[
    "advisories", "patches", "standards", "writeups", "tooling", "forums", "rfcs",
];

struct GN {
    x: f64,
    y: f64,
    vx: f64,
    vy: f64,
    r: f64,
    color: &'static str,
    host: String,
    title: String,
    age: f64,
}

fn frand(s: &mut u64) -> f64 {
    *s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    (*s >> 33) as f64 / (1u64 << 31) as f64
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
pub fn KnowledgeGraph(live: Live) -> impl IntoView {
    let cref = NodeRef::<Canvas>::new();
    let hover = RwSignal::new((-1.0f64, -1.0f64));
    Effect::new(move |_| {
        if let Some(c) = cref.get() {
            let c: HtmlCanvasElement = c.unchecked_into();
            start(c, live, hover);
        }
    });

    let counts = move || {
        live.nodes.with(|ns| {
            let mut m: HashMap<&str, u32> = HashMap::new();
            for n in ns {
                *m.entry(n.chapter.as_deref().unwrap_or("")).or_insert(0) += 1;
            }
            CHAPTERS
                .iter()
                .map(|c| (*c, m.get(c).copied().unwrap_or(0)))
                .collect::<Vec<_>>()
        })
    };
    let total_nodes = move || live.nodes.with(|n| n.len());
    let total_edges = move || live.edges.with(|e| e.len());

    view! {
        <div class="graph__wrap">
            <canvas
                node_ref=cref
                class="graph__cv"
                on:mousemove=move |e| hover.set((e.offset_x() as f64, e.offset_y() as f64))
                on:mouseleave=move |_| hover.set((-1.0, -1.0))
            ></canvas>
            <div class="graph__hud">
                <span class="num">{move || total_nodes().to_string()}</span>" pages · "
                <span class="num">{move || total_edges().to_string()}</span>" links"
            </div>
            <div class="graph__legend">
                <For
                    each=counts
                    key=|(c, n)| format!("{c}:{n}")
                    let:row
                >
                    <span class="glg">
                        <i class="glg__dot" style=format!("background:{}", chapter_color(row.0))></i>
                        {row.0.to_string()}
                        <b class="num dim">{row.1.to_string()}</b>
                    </span>
                </For>
            </div>
        </div>
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
    let mut nodes: Vec<GN> = Vec::new();
    let mut idx: HashMap<String, usize> = HashMap::new();
    let mut seed = 0x51ED_270B_1CA7_E5A3u64;
    let mut last = 0.0f64;

    run_raf(move |t| {
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
        let dt = if last == 0.0 { 0.016 } else { ((t - last) / 1000.0).min(0.05) };
        last = t;

        // sync new nodes in from the live graph (keep existing positions)
        live.nodes.with_untracked(|lns| {
            for n in lns {
                if !idx.contains_key(&n.id) {
                    let ang = frand(&mut seed) * std::f64::consts::TAU;
                    let rad = 30.0 + frand(&mut seed) * cw.min(ch) * 0.28;
                    nodes.push(GN {
                        x: cw * 0.5 + ang.cos() * rad,
                        y: ch * 0.5 + ang.sin() * rad,
                        vx: 0.0,
                        vy: 0.0,
                        r: (2.2 + (n.tokens as f64).sqrt() * 0.22).min(13.0),
                        color: chapter_color(n.chapter.as_deref().unwrap_or("")),
                        host: host_of(&n.url),
                        title: n.title.clone(),
                        age: 0.0,
                    });
                    idx.insert(n.id.clone(), nodes.len() - 1);
                }
            }
        });

        let edges: Vec<(usize, usize)> = live.edges.with_untracked(|es| {
            es.iter()
                .filter_map(|e| Some((*idx.get(&e.from)?, *idx.get(&e.to)?)))
                .collect()
        });

        let n = nodes.len();
        let cx = cw * 0.5;
        let cy = ch * 0.5;

        // --- forces ---
        // repulsion (O(n^2), fine for a few hundred nodes)
        for i in 0..n {
            for j in (i + 1)..n {
                let dx = nodes[i].x - nodes[j].x;
                let dy = nodes[i].y - nodes[j].y;
                let d2 = (dx * dx + dy * dy).max(16.0);
                let f = 900.0 / d2;
                let d = d2.sqrt();
                let (ux, uy) = (dx / d, dy / d);
                nodes[i].vx += ux * f * dt;
                nodes[i].vy += uy * f * dt;
                nodes[j].vx -= ux * f * dt;
                nodes[j].vy -= uy * f * dt;
            }
        }
        // springs along real edges
        for (a, b) in &edges {
            let (a, b) = (*a, *b);
            if a == b {
                continue;
            }
            let dx = nodes[b].x - nodes[a].x;
            let dy = nodes[b].y - nodes[a].y;
            let d = (dx * dx + dy * dy).sqrt().max(1.0);
            let rest = 70.0;
            let f = (d - rest) * 0.9 * dt;
            let (ux, uy) = (dx / d, dy / d);
            nodes[a].vx += ux * f;
            nodes[a].vy += uy * f;
            nodes[b].vx -= ux * f;
            nodes[b].vy -= uy * f;
        }
        // centering + damping + integrate
        for nd in nodes.iter_mut() {
            nd.vx += (cx - nd.x) * 0.45 * dt;
            nd.vy += (cy - nd.y) * 0.45 * dt;
            nd.vx *= 0.86;
            nd.vy *= 0.86;
            nd.x += nd.vx;
            nd.y += nd.vy;
            let m = 10.0;
            nd.x = nd.x.clamp(m, cw - m);
            nd.y = nd.y.clamp(m, ch - m);
            nd.age += dt;
        }

        // --- draw ---
        ctx.set_global_alpha(1.0);
        ctx.clear_rect(0.0, 0.0, cw, ch);

        // edges
        set_stroke(&ctx, "#1a2b52");
        ctx.set_line_width(1.0);
        ctx.set_global_alpha(0.5);
        for (a, b) in &edges {
            let (a, b) = (*a, *b);
            ctx.begin_path();
            ctx.move_to(nodes[a].x, nodes[a].y);
            ctx.line_to(nodes[b].x, nodes[b].y);
            ctx.stroke();
        }
        ctx.set_global_alpha(1.0);

        // nodes
        for nd in nodes.iter() {
            let pop = (nd.age / 0.6).min(1.0);
            let r = nd.r * (0.3 + 0.7 * pop);
            ctx.save();
            ctx.set_shadow_blur(8.0);
            ctx.set_shadow_color(nd.color);
            set_fill(&ctx, nd.color);
            ctx.begin_path();
            let _ = ctx.arc(nd.x, nd.y, r, 0.0, std::f64::consts::TAU);
            ctx.fill();
            ctx.restore();
        }

        // hover: nearest node within a small radius → label it
        let (hx, hy) = hover.get_untracked();
        if hx >= 0.0 {
            let mut best: Option<usize> = None;
            let mut bd = 18.0 * 18.0;
            for (i, nd) in nodes.iter().enumerate() {
                let dx = nd.x - hx;
                let dy = nd.y - hy;
                let d2 = dx * dx + dy * dy;
                if d2 < bd {
                    bd = d2;
                    best = Some(i);
                }
            }
            if let Some(i) = best {
                let nd = &nodes[i];
                // ring
                set_stroke(&ctx, "#eaf2ff");
                ctx.set_line_width(1.5);
                ctx.begin_path();
                let _ = ctx.arc(nd.x, nd.y, nd.r + 4.0, 0.0, std::f64::consts::TAU);
                ctx.stroke();
                // tooltip
                let label = if nd.title.is_empty() {
                    nd.host.clone()
                } else {
                    format!("{} — {}", nd.host, trunc(&nd.title, 48))
                };
                ctx.set_font("12px 'IBM Plex Mono', monospace");
                // monospace ≈ 7.2px per glyph at 12px; avoids the TextMetrics feature
                let tw = label.chars().count() as f64 * 7.2 + 14.0;
                let bx = (nd.x + 10.0).min(cw - tw - 6.0).max(6.0);
                let by = (nd.y - 26.0).max(6.0);
                ctx.set_global_alpha(0.92);
                set_fill(&ctx, "#060a16");
                ctx.fill_rect(bx, by, tw, 20.0);
                set_stroke(&ctx, nd.color);
                ctx.set_line_width(1.0);
                ctx.stroke_rect(bx, by, tw, 20.0);
                ctx.set_global_alpha(1.0);
                set_fill(&ctx, "#eaf2ff");
                let _ = ctx.fill_text(&label, bx + 7.0, by + 14.0);
            }
        }
    });
}

fn trunc(s: &str, n: usize) -> String {
    if s.chars().count() > n {
        let mut out: String = s.chars().take(n).collect();
        out.push('…');
        out
    } else {
        s.to_string()
    }
}

fn set_fill(c: &CanvasRenderingContext2d, s: &str) {
    c.set_fill_style(&wasm_bindgen::JsValue::from_str(s));
}
fn set_stroke(c: &CanvasRenderingContext2d, s: &str) {
    c.set_stroke_style(&wasm_bindgen::JsValue::from_str(s));
}
