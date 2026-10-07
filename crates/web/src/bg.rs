//! Ambient deep-sea backdrop: slow-drifting bioluminescent plankton behind the
//! whole app. A single fixed, pointer-transparent canvas.

use leptos::html::Canvas;
use leptos::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement};

use crate::raf::{run_raf, window};

struct P {
    x: f64,
    y: f64,
    r: f64,
    spd: f64,
    ph: f64,
    teal: bool,
}

fn frand(s: &mut u64) -> f64 {
    *s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    (*s >> 33) as f64 / (1u64 << 31) as f64
}

#[component]
pub fn AmbientBg() -> impl IntoView {
    let cref = NodeRef::<Canvas>::new();
    Effect::new(move |_| {
        if let Some(c) = cref.get() {
            let c: HtmlCanvasElement = c.unchecked_into();
            start(c);
        }
    });
    view! { <canvas node_ref=cref class="ambient"></canvas> }
}

fn start(canvas: HtmlCanvasElement) {
    let ctx: CanvasRenderingContext2d = match canvas.get_context("2d").ok().flatten() {
        Some(o) => match o.dyn_into() {
            Ok(c) => c,
            Err(_) => return,
        },
        None => return,
    };
    let mut parts: Vec<P> = Vec::new();
    let mut seed = 0x0c70_9115_a1b2_c3d4u64;
    let mut t = 0.0f64;
    let mut last = 0.0f64;

    run_raf(move |now| {
        let w = window().inner_width().ok().and_then(|v| v.as_f64()).unwrap_or(1200.0);
        let h = window().inner_height().ok().and_then(|v| v.as_f64()).unwrap_or(800.0);
        if w < 2.0 || h < 2.0 {
            return;
        }
        let dpr = window().device_pixel_ratio().max(1.0);
        let (bw, bh) = ((w * dpr) as u32, (h * dpr) as u32);
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

        let want = (((w * h) / 24000.0) as usize).min(110);
        while parts.len() < want {
            parts.push(P {
                x: frand(&mut seed) * w,
                y: frand(&mut seed) * h,
                r: 0.6 + frand(&mut seed) * 1.9,
                spd: 4.0 + frand(&mut seed) * 16.0,
                ph: frand(&mut seed) * 6.2832,
                teal: frand(&mut seed) > 0.5,
            });
        }

        ctx.set_global_alpha(1.0);
        ctx.clear_rect(0.0, 0.0, w, h);
        for p in parts.iter_mut() {
            p.y -= p.spd * dt;
            p.x += (t * 0.3 + p.ph).sin() * 0.25;
            if p.y < -4.0 {
                p.y = h + 4.0;
                p.x = frand(&mut seed) * w;
            }
            let pulse = 0.5 + 0.5 * (t * 1.2 + p.ph).sin();
            ctx.set_global_alpha(0.06 + 0.20 * pulse);
            ctx.set_shadow_blur(6.0);
            ctx.set_shadow_color(if p.teal { "#4cd7c0" } else { "#2ad4ff" });
            ctx.set_fill_style(&wasm_bindgen::JsValue::from_str(if p.teal {
                "#4cd7c0"
            } else {
                "#2ad4ff"
            }));
            ctx.begin_path();
            let _ = ctx.arc(p.x, p.y, p.r, 0.0, 6.2832);
            ctx.fill();
        }
        ctx.set_shadow_blur(0.0);
        ctx.set_global_alpha(1.0);
    });
}
