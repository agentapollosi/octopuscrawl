//! The queen scene (/queen): a large crowned queen octopus at the centre of a
//! slowly turning reef of domain nodes. Pages swim in from the domains and are
//! absorbed; her mantle fills with light as the dataset grows toward the next
//! version. She receives — she does not ink.

use leptos::html::Canvas;
use leptos::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement};

use crate::raf::{run_raf, window};
use crate::ws::Live;

const DOMAINS: &[&str] = &[
    "nvd.nist.gov", "owasp.org", "attack.mitre.org", "cwe.mitre.org",
    "cisa.gov", "portswigger.net", "rfc-editor.org", "ubuntu.com",
    "msrc.microsoft.com", "isc.sans.edu", "capec.mitre.org", "access.redhat.com",
];

#[derive(Clone, Copy, Default)]
struct V {
    x: f64,
    y: f64,
}
impl V {
    fn new(x: f64, y: f64) -> Self {
        V { x, y }
    }
    fn add(self, o: V) -> V {
        V::new(self.x + o.x, self.y + o.y)
    }
    fn sub(self, o: V) -> V {
        V::new(self.x - o.x, self.y - o.y)
    }
    fn mul(self, s: f64) -> V {
        V::new(self.x * s, self.y * s)
    }
    fn lerp(self, o: V, t: f64) -> V {
        self.add(o.sub(self).mul(t))
    }
    fn len(self) -> f64 {
        (self.x * self.x + self.y * self.y).sqrt()
    }
}
fn dir(a: f64) -> V {
    V::new(a.cos(), a.sin())
}

struct Rng(u64);
impl Rng {
    fn new(s: u64) -> Self {
        Rng(s | 1)
    }
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn range(&mut self, a: f64, b: f64) -> f64 {
        a + (b - a) * self.unit()
    }
}

struct Arm {
    pts: Vec<V>,
    prev: Vec<V>,
    base: f64,
}
struct Node {
    angle: f64,
    radius: f64,
    label: &'static str,
}
struct Page {
    from: V,
    ctrl: V,
    t: f64,
    dur: f64,
}
struct Ripple {
    at: V,
    age: f64,
}

struct QueenViz {
    w: f64,
    h: f64,
    t: f64,
    fill_cur: f64,
    fill_target: f64,
    arms: Vec<Arm>,
    nodes: Vec<Node>,
    pages: Vec<Page>,
    ripples: Vec<Ripple>,
    rng: Rng,
    spawn: f64,
}

impl QueenViz {
    fn new(w: f64, h: f64) -> Self {
        let mut rng = Rng::new(0xC0FFEE);
        let nodes = DOMAINS
            .iter()
            .enumerate()
            .map(|(i, l)| Node {
                angle: i as f64 / DOMAINS.len() as f64 * std::f64::consts::TAU,
                radius: if i % 2 == 0 { 0.82 } else { 0.62 },
                label: l,
            })
            .collect();
        let arms = (0..8)
            .map(|i| {
                let base = std::f64::consts::PI
                    + (i as f64 + 0.5) / 8.0 * std::f64::consts::TAU;
                let pts: Vec<V> = (0..=10).map(|_| V::new(w * 0.5, h * 0.5)).collect();
                Arm {
                    prev: pts.clone(),
                    pts,
                    base,
                }
            })
            .collect();
        QueenViz {
            w,
            h,
            t: 0.0,
            fill_cur: 0.0,
            fill_target: 0.0,
            arms,
            nodes,
            pages: Vec::new(),
            ripples: Vec::new(),
            rng,
            spawn: 0.0,
        }
    }

    fn center(&self) -> V {
        V::new(self.w * 0.5, self.h * 0.52)
    }
    fn ring(&self) -> f64 {
        self.w.min(self.h) * 0.42
    }
    fn body_r(&self) -> f64 {
        self.w.min(self.h) * 0.12
    }

    fn node_pos(&self, n: &Node) -> V {
        let a = n.angle + self.t * 0.06;
        self.center().add(dir(a).mul(self.ring() * n.radius))
    }

    fn set_fill(&mut self, f: f64) {
        self.fill_target = f.clamp(0.0, 1.0);
    }

    fn step(&mut self, dt: f64) {
        self.t += dt;
        self.fill_cur += (self.fill_target - self.fill_cur) * (dt * 1.5).min(1.0);

        // spawn swimming pages from random domains
        self.spawn -= dt;
        if self.spawn <= 0.0 {
            self.spawn = self.rng.range(0.25, 0.7);
            if self.pages.len() < 36 {
                let ni = self.rng.next() as usize % self.nodes.len();
                let from = self.node_pos(&self.nodes[ni]);
                let mid = from.lerp(self.center(), 0.5);
                let off = dir(self.rng.range(0.0, std::f64::consts::TAU))
                    .mul(self.ring() * 0.25);
                self.pages.push(Page {
                    from,
                    ctrl: mid.add(off),
                    t: 0.0,
                    dur: self.rng.range(1.4, 2.6),
                });
            }
        }

        let c = self.center();
        let mut arrived = 0;
        self.pages.retain_mut(|p| {
            p.t += dt / p.dur;
            if p.t >= 1.0 {
                arrived += 1;
                false
            } else {
                true
            }
        });
        for _ in 0..arrived {
            self.ripples.push(Ripple { at: c, age: 0.0 });
        }
        for r in self.ripples.iter_mut() {
            r.age += dt;
        }
        self.ripples.retain(|r| r.age < 0.9);

        // idle arm sway (verlet, anchored at the mantle base)
        let r = self.body_r();
        let rest = r * 2.6 / 10.0;
        for (ai, arm) in self.arms.iter_mut().enumerate() {
            let anchor = c.add(dir(arm.base).mul(r * 0.8));
            let wave = (self.t * 1.1 + ai as f64).sin();
            let perp = V::new(-dir(arm.base).y, dir(arm.base).x);
            for i in 1..arm.pts.len() {
                let p = arm.pts[i];
                let pv = arm.prev[i];
                let tf = i as f64 / 10.0;
                let mut np = p.add(p.sub(pv).mul(0.9));
                np = np.add(perp.mul(wave * 0.9 * tf));
                np = np.add(V::new(0.0, 7.0 * dt * dt * r));
                arm.prev[i] = p;
                arm.pts[i] = np;
            }
            // reach outward toward a resting splay
            let goal = anchor.add(dir(arm.base + wave * 0.1).mul(r * 2.4));
            let last = arm.pts.len() - 1;
            arm.pts[last] = arm.pts[last].lerp(goal, 0.08);
            for _ in 0..4 {
                arm.pts[0] = anchor;
                for i in 1..arm.pts.len() {
                    let d = arm.pts[i].sub(arm.pts[i - 1]);
                    let dl = d.len();
                    if dl > 1e-6 {
                        let corr = d.mul((dl - rest) / dl * 0.5);
                        if i > 1 {
                            arm.pts[i - 1] = arm.pts[i - 1].add(corr);
                        }
                        arm.pts[i] = arm.pts[i].sub(corr);
                    }
                }
                arm.pts[0] = anchor;
            }
        }
    }

    fn draw(&self, c: &CanvasRenderingContext2d) {
        c.clear_rect(0.0, 0.0, self.w, self.h);
        fill(c, "#04060e");
        c.fill_rect(0.0, 0.0, self.w, self.h);

        let ctr = self.center();

        // faint links from each domain to the queen
        stroke(c, "#122a52");
        c.set_line_width(1.0);
        c.set_global_alpha(0.5);
        for n in &self.nodes {
            let p = self.node_pos(n);
            c.begin_path();
            c.move_to(p.x, p.y);
            c.line_to(ctr.x, ctr.y);
            c.stroke();
        }
        c.set_global_alpha(1.0);

        // domain nodes + labels
        c.set_font("11px 'IBM Plex Mono', monospace");
        for n in &self.nodes {
            let p = self.node_pos(n);
            fill(c, "#2ad4ff");
            c.set_global_alpha(0.9);
            dot(c, p.x, p.y, 3.0);
            c.set_global_alpha(0.65);
            fill(c, "#8aa0c8");
            let _ = c.fill_text(n.label, p.x + 6.0, p.y + 3.0);
        }
        c.set_global_alpha(1.0);

        // swimming pages (quadratic bezier from domain → queen)
        for p in &self.pages {
            let pos = bezier(p.from, p.ctrl, ctr, p.t);
            let a = (1.0 - p.t).min(1.0);
            c.set_global_alpha(0.25 + 0.6 * a);
            fill(c, "#35e0ff");
            dot(c, pos.x, pos.y, 2.2);
        }
        c.set_global_alpha(1.0);

        // ripples on arrival
        for r in &self.ripples {
            let rr = self.body_r() * (0.8 + r.age * 2.2);
            c.set_global_alpha((1.0 - r.age / 0.9) * 0.5);
            stroke(c, "#35e0ff");
            c.set_line_width(1.5);
            c.begin_path();
            let _ = c.arc(ctr.x, ctr.y, rr, 0.0, std::f64::consts::TAU);
            c.stroke();
        }
        c.set_global_alpha(1.0);

        self.draw_queen(c, ctr);
    }

    fn draw_queen(&self, c: &CanvasRenderingContext2d, ctr: V) {
        let r = self.body_r();
        let pulse = 0.5 + 0.5 * (self.t * 1.6).sin();

        // tentacles behind the mantle
        c.set_line_cap("round");
        for arm in &self.arms {
            for i in 0..arm.pts.len() - 1 {
                let tf = 1.0 - i as f64 / 10.0;
                c.set_line_width((r * 0.3 * tf).max(1.5));
                stroke(c, "#102250");
                c.begin_path();
                c.move_to(arm.pts[i].x, arm.pts[i].y);
                c.line_to(arm.pts[i + 1].x, arm.pts[i + 1].y);
                c.stroke();
            }
        }

        // mantle with a luminous fill rising to fill_cur
        let rx = r * 1.0;
        let ry = r * 1.3;
        c.save();
        c.set_shadow_blur(26.0 + 16.0 * pulse);
        c.set_shadow_color("#2ad4ff");
        fill(c, "#0a1836");
        c.begin_path();
        let _ = c.ellipse(ctr.x, ctr.y, rx, ry, 0.0, 0.0, std::f64::consts::TAU);
        c.fill();
        c.restore();

        // clip to mantle, draw the dataset "light" filling from the bottom
        c.save();
        c.begin_path();
        let _ = c.ellipse(ctr.x, ctr.y, rx, ry, 0.0, 0.0, std::f64::consts::TAU);
        c.clip();
        let top = ctr.y + ry - (2.0 * ry) * self.fill_cur;
        c.set_global_alpha(0.85);
        fill(c, "#163e86");
        c.fill_rect(ctr.x - rx, top, rx * 2.0, ctr.y + ry - top);
        // wavy luminous surface
        stroke(c, "#35e0ff");
        c.set_line_width(2.0);
        c.begin_path();
        let mut first = true;
        let mut x = ctr.x - rx;
        while x <= ctr.x + rx {
            let yy = top + (x * 0.05 + self.t * 2.0).sin() * 3.0;
            if first {
                c.move_to(x, yy);
                first = false;
            } else {
                c.line_to(x, yy);
            }
            x += 6.0;
        }
        c.stroke();
        c.restore();
        c.set_global_alpha(1.0);

        // mantle rim
        stroke(c, "#2ad4ff");
        c.set_global_alpha(0.5 + 0.35 * pulse);
        c.set_line_width(1.8);
        c.begin_path();
        let _ = c.ellipse(ctr.x, ctr.y, rx, ry, 0.0, 0.0, std::f64::consts::TAU);
        c.stroke();
        c.set_global_alpha(1.0);

        // crown — three spikes
        fill(c, "#35e0ff");
        for k in -1..=1 {
            let bx = ctr.x + k as f64 * r * 0.42;
            let by = ctr.y - ry * 0.92;
            c.begin_path();
            c.move_to(bx - r * 0.12, by);
            c.line_to(bx, by - r * 0.4);
            c.line_to(bx + r * 0.12, by);
            c.close_path();
            c.fill();
        }

        // eyes
        for s in [-1.0, 1.0] {
            let e = V::new(ctr.x + s * r * 0.4, ctr.y - ry * 0.1);
            fill(c, "#d8f6ff");
            dot(c, e.x, e.y, r * 0.17);
            fill(c, "#04060e");
            dot(c, e.x, e.y + r * 0.02, r * 0.08);
        }
    }
}

fn bezier(a: V, b: V, c: V, t: f64) -> V {
    let u = 1.0 - t;
    a.mul(u * u).add(b.mul(2.0 * u * t)).add(c.mul(t * t))
}

#[component]
pub fn QueenScene(live: Live) -> impl IntoView {
    let canvas_ref = NodeRef::<Canvas>::new();

    Effect::new(move |_| {
        if let Some(canvas) = canvas_ref.get() {
            let canvas: HtmlCanvasElement = canvas.unchecked_into();
            start(canvas, live);
        }
    });

    view! {
        <div class="queen__canvaswrap">
            <canvas node_ref=canvas_ref class="queen__cv"></canvas>
        </div>
    }
}

fn start(canvas: HtmlCanvasElement, live: Live) {
    let ctx: CanvasRenderingContext2d = match canvas.get_context("2d").ok().flatten() {
        Some(o) => match o.dyn_into() {
            Ok(c) => c,
            Err(_) => return,
        },
        None => return,
    };
    let mut viz = QueenViz::new(800.0, 460.0);
    let mut last = 0.0f64;

    run_raf(move |t| {
        let dpr = window().device_pixel_ratio().max(1.0);
        let cw = canvas.client_width() as f64;
        let ch = canvas.client_height() as f64;
        if cw <= 1.0 || ch <= 1.0 {
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

        if (cw - viz.w).abs() > 1.0 || (ch - viz.h).abs() > 1.0 {
            viz.w = cw;
            viz.h = ch;
        }

        // fill = dataset progress toward the next version's target tokens
        let fill = live.stats.with_untracked(|s| {
            s.as_ref()
                .map(|s| {
                    let target = s.next_queen.dataset_tokens.max(1) as f64;
                    (s.dataset_tokens as f64 / target).min(1.0)
                })
                .unwrap_or(0.0)
        });
        viz.set_fill(fill);

        viz.step(dt);
        viz.draw(&ctx);
    });
}

fn fill(c: &CanvasRenderingContext2d, s: &str) {
    c.set_fill_style(&wasm_bindgen::JsValue::from_str(s));
}
fn stroke(c: &CanvasRenderingContext2d, s: &str) {
    c.set_stroke_style(&wasm_bindgen::JsValue::from_str(s));
}
fn dot(c: &CanvasRenderingContext2d, x: f64, y: f64, r: f64) {
    c.begin_path();
    let _ = c.arc(x, y, r.max(0.1), 0.0, std::f64::consts::TAU);
    c.fill();
}
