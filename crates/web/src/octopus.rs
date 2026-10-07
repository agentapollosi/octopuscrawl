//! The octopus hatchling — the signature live animation, drawn to a 2D canvas.
//!
//! Two modes:
//! - **document mode** (`set_doc`): the canvas is transparent and overlays a real
//!   page screenshot; the tentacle targets are the real link/heading boxes on
//!   that page, and a tip that reaches one *inks* the text — the way a reader
//!   highlights a line. This is the main live view.
//! - **reef mode** (no doc): a self-contained scene over a dark ground, used as a
//!   fallback when there is no real frame yet (e.g. the demo feed).

use web_sys::CanvasRenderingContext2d;

const ARMS: usize = 8;
const SEGS: usize = 9; // segments per arm; points = SEGS + 1 (incl. anchor)

#[derive(Clone, Copy, Default)]
pub struct V {
    pub x: f64,
    pub y: f64,
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
    fn len(self) -> f64 {
        (self.x * self.x + self.y * self.y).sqrt()
    }
    fn norm(self) -> V {
        let l = self.len();
        if l > 1e-6 {
            self.mul(1.0 / l)
        } else {
            V::new(0.0, 0.0)
        }
    }
}

fn dir(a: f64) -> V {
    V::new(a.cos(), a.sin())
}

/// Tiny LCG — the whole sim is seeded so a hatchling moves the same each load.
struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed | 1)
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
    base_angle: f64,
    target: Option<usize>,
}

#[derive(Clone)]
struct Target {
    pos: V,
    rect: [f64; 4], // x,y,w,h in display space (w=h=0 in reef mode)
    inked: bool,
    kind: u8, // 0 link, 1 heading, 2 body text
}

struct Ink {
    pos: V,
    rect: [f64; 4],
    age: f64,
    max_r: f64,
    seed: u64,
    kind: u8,
}

pub struct Octopus {
    w: f64,
    h: f64,
    scale: f64,
    pos: V,
    vel: V,
    heading: f64,
    t: f64,
    spawn_age: f64,
    arms: Vec<Arm>,
    targets: Vec<Target>,
    inks: Vec<Ink>,
    rng: Rng,
    caption: String,
    has_doc: bool,
    doc_seq: u64,
    read_cursor: usize,
}

impl Octopus {
    pub fn new(w: f64, h: f64) -> Self {
        let mut o = Octopus {
            w,
            h,
            scale: 1.0,
            pos: V::new(w * 0.5, h * 0.5),
            vel: V::default(),
            heading: -std::f64::consts::FRAC_PI_2,
            t: 0.0,
            spawn_age: 0.0,
            arms: Vec::new(),
            targets: Vec::new(),
            inks: Vec::new(),
            rng: Rng::new(0xA17E_B0BA),
            caption: String::new(),
            has_doc: false,
            doc_seq: 0,
            read_cursor: 0,
        };
        o.rebuild(w, h);
        o.spawn_targets();
        o
    }

    fn rebuild(&mut self, w: f64, h: f64) {
        self.w = w;
        self.h = h;
        self.scale = w.min(h).max(1.0);
        let r = self.body_r();
        let rest = self.rest();
        let pos = self.pos;
        let arms: Vec<Arm> = (0..ARMS)
            .map(|i| {
                let base_angle = std::f64::consts::PI
                    + (i as f64 + 0.5) / ARMS as f64 * std::f64::consts::TAU;
                let anchor = pos.add(dir(base_angle).mul(r * 0.8));
                let pts: Vec<V> = (0..=SEGS)
                    .map(|s| anchor.add(dir(base_angle).mul(rest * s as f64)))
                    .collect();
                Arm { prev: pts.clone(), pts, base_angle, target: None }
            })
            .collect();
        self.arms = arms;
    }

    pub fn resize(&mut self, w: f64, h: f64) {
        if (w - self.w).abs() > 1.0 || (h - self.h).abs() > 1.0 {
            self.w = w;
            self.h = h;
            self.scale = w.min(h).max(1.0);
            self.pos.x = self.pos.x.clamp(self.margin(), w - self.margin());
            self.pos.y = self.pos.y.clamp(self.margin(), h - self.margin());
        }
    }

    pub fn set_caption(&mut self, c: String) {
        self.caption = c;
    }

    /// Enter/refresh document mode. `boxes` are link/heading rects in DISPLAY
    /// space (already scaled to the canvas). Resets targets only when the
    /// document (identified by `seq`) actually changes.
    pub fn set_doc(&mut self, boxes: Vec<([f64; 4], u8)>, seq: u64) {
        self.has_doc = true;
        if seq == self.doc_seq {
            return;
        }
        self.doc_seq = seq;
        self.inks.clear();
        self.targets = boxes
            .into_iter()
            .map(|(r, kind)| Target {
                // read a line from its left edge inward, not dead-centre of a
                // wide paragraph — keeps the tentacle tip on the actual text.
                pos: V::new(r[0] + (r[2] * 0.5).min(70.0), r[1] + r[3] * 0.5),
                rect: r,
                inked: false,
                kind,
            })
            .collect();
        // defensively keep reading order (top-to-bottom), so the cursor advances
        // down the page like a reader.
        self.targets
            .sort_by(|a, b| a.rect[1].partial_cmp(&b.rect[1]).unwrap_or(std::cmp::Ordering::Equal));
        self.read_cursor = 0;
    }

    /// Leave document mode and fall back to the reef scene.
    pub fn clear_doc(&mut self) {
        if self.has_doc {
            self.has_doc = false;
            self.doc_seq = 0;
            self.spawn_targets();
        }
    }

    fn body_r(&self) -> f64 {
        let base = if self.has_doc { 0.06 } else { 0.085 };
        base * self.scale
    }
    fn arm_len(&self) -> f64 {
        self.body_r() * if self.has_doc { 3.0 } else { 2.7 }
    }
    fn rest(&self) -> f64 {
        self.arm_len() / SEGS as f64
    }
    fn margin(&self) -> f64 {
        self.arm_len() * 1.1
    }

    fn spawn_targets(&mut self) {
        self.targets.clear();
        self.inks.clear();
        let n = 14;
        let m = self.margin();
        let (w, h) = (self.w, self.h);
        for _ in 0..n {
            let x = self.rng.range(m, w - m);
            let y = self.rng.range(m * 0.7, h - m * 0.7);
            self.targets.push(Target {
                pos: V::new(x, y),
                rect: [x, y, 0.0, 0.0],
                inked: false,
                kind: 0,
            });
        }
        self.read_cursor = 0;
        self.spawn_age = 0.0;
    }

    // -- simulation ---------------------------------------------------------

    pub fn step(&mut self, dt: f64) {
        self.t += dt;
        self.spawn_age += dt;

        self.move_body(dt);
        self.step_arms(dt);

        for ink in self.inks.iter_mut() {
            ink.age += dt;
        }

        // reef mode respawns its own field; document mode waits for a new doc
        if !self.has_doc {
            let uninked = self.targets.iter().filter(|t| !t.inked).count();
            if uninked == 0 || self.spawn_age > 16.0 {
                self.spawn_targets();
            }
        }
    }

    fn move_body(&mut self, dt: f64) {
        let br = self.body_r();
        let mgn = self.margin();

        // advance the reading cursor past any lines already inked
        while self.read_cursor < self.targets.len() && self.targets[self.read_cursor].inked {
            self.read_cursor += 1;
        }

        let goal = if self.has_doc {
            if self.read_cursor < self.targets.len() {
                // hover just above the current line so the tentacles drape down
                // onto the exact text being read — top-to-bottom, like a reader.
                let t = &self.targets[self.read_cursor];
                let bob = (self.t * 1.1).sin() * br * 0.22;
                V::new(t.pos.x + br * 0.15, t.rect[1] - br * 0.55 + bob)
            } else {
                V::new(self.w * 0.5, self.h * 0.5)
            }
        } else {
            let mut c = V::default();
            let mut n = 0.0;
            for t in &self.targets {
                if !t.inked {
                    c = c.add(t.pos);
                    n += 1.0;
                }
            }
            if n > 0.0 {
                c.mul(1.0 / n).add(dir(self.t * 0.6).mul(br * 1.2))
            } else {
                V::new(self.w * 0.5, self.h * 0.5)
            }
        };

        let to = goal.sub(self.pos);
        self.vel = self.vel.mul(0.90).add(to.mul(1.6 * dt));
        self.vel.y += (self.t * 1.7).sin() * br * 0.03;
        self.pos = self.pos.add(self.vel.mul(dt));
        let m = mgn;
        self.pos.x = self.pos.x.clamp(m, self.w - m);
        self.pos.y = self.pos.y.clamp(m * 0.7, self.h - m * 0.7);

        let speed = self.vel.len();
        if speed > 4.0 {
            let want = self.vel.y.atan2(self.vel.x);
            self.heading = lerp_angle(self.heading, want, (dt * 2.0).min(1.0));
        }
    }

    fn step_arms(&mut self, dt: f64) {
        let r = self.body_r();
        let rest = self.rest();
        let al = self.arm_len();
        let reach = al * if self.has_doc { 1.3 } else { 1.15 };
        let ink_dist = 0.02 * self.scale;
        let iters = if self.w < 360.0 { 3 } else { 6 };

        let pos = self.pos;
        let heading = self.heading;
        let t_now = self.t;

        let mut new_inks: Vec<(V, [f64; 4], u8)> = Vec::new();

        // In document mode only the lines around the reading cursor are in play,
        // so the hatchling works down the page in order instead of darting to a
        // random far line.
        let (lo, hi) = if self.has_doc {
            (self.read_cursor, (self.read_cursor + 8).min(self.targets.len()))
        } else {
            (0, self.targets.len())
        };

        for (ai, arm) in self.arms.iter_mut().enumerate() {
            let a = heading + arm.base_angle;
            let anchor = pos.add(dir(a).mul(r * 0.8));

            let mut best: Option<usize> = None;
            let mut best_d = reach;
            for ti in lo..hi {
                let t = &self.targets[ti];
                if t.inked {
                    continue;
                }
                let d = t.pos.sub(anchor).len();
                if d < best_d {
                    best_d = d;
                    best = Some(ti);
                }
            }
            arm.target = best;
            let reaching = best.is_some();
            let goal = match best {
                Some(ti) => self.targets[ti].pos,
                None => {
                    let sway = (t_now * 1.3 + ai as f64 * 0.8).sin() * 0.5;
                    let base = anchor.add(dir(a + sway * 0.4).mul(al * 0.72));
                    base.add(dir(a + 1.9).mul(al * 0.18))
                }
            };

            let wave = (t_now * 2.4 + ai as f64).sin();
            let perp = V::new(-dir(a).y, dir(a).x);
            for i in 1..arm.pts.len() {
                let p = arm.pts[i];
                let pv = arm.prev[i];
                let mut np = p.add(p.sub(pv).mul(0.86));
                let tfac = i as f64 / SEGS as f64;
                np = np.add(perp.mul(wave * 0.6 * tfac));
                np = np.add(V::new(0.0, 9.0 * dt * dt * self.scale * 0.02));
                arm.prev[i] = p;
                arm.pts[i] = np;
            }
            let last = arm.pts.len() - 1;
            let pull = if reaching { 0.30 } else { 0.12 };
            arm.pts[last] = arm.pts[last].add(goal.sub(arm.pts[last]).mul(pull));

            for _ in 0..iters {
                arm.pts[0] = anchor;
                for i in 1..arm.pts.len() {
                    let diff = arm.pts[i].sub(arm.pts[i - 1]);
                    let d = diff.len();
                    if d > 1e-6 {
                        let corr = diff.mul((d - rest) / d * 0.5);
                        if i > 1 {
                            arm.pts[i - 1] = arm.pts[i - 1].add(corr);
                        }
                        arm.pts[i] = arm.pts[i].sub(corr);
                    }
                }
                arm.pts[0] = anchor;
            }

            if let Some(ti) = best {
                if arm.pts[last].sub(self.targets[ti].pos).len() < ink_dist.max(7.0)
                    && !self.targets[ti].inked
                {
                    self.targets[ti].inked = true;
                    new_inks.push((self.targets[ti].pos, self.targets[ti].rect, self.targets[ti].kind));
                }
            }
        }

        let br = self.body_r();
        for (p, rect, kind) in new_inks {
            let seed = self.rng.next();
            let max_r = br * self.rng.range(0.5, 0.9);
            self.inks.push(Ink {
                pos: p,
                rect,
                age: 0.0,
                max_r,
                seed,
                kind,
            });
        }
        if self.inks.len() > 60 {
            let drop = self.inks.len() - 60;
            self.inks.drain(0..drop);
        }
    }

    // -- rendering ----------------------------------------------------------

    pub fn draw(&self, c: &CanvasRenderingContext2d) {
        c.clear_rect(0.0, 0.0, self.w, self.h);
        if !self.has_doc {
            set_fill(c, "#05070f");
            c.fill_rect(0.0, 0.0, self.w, self.h);
            c.set_global_alpha(0.10);
            set_fill(c, "#0c2a4a");
            circle(c, self.pos.x, self.pos.y, self.body_r() * 4.5);
            c.set_global_alpha(1.0);
        }

        self.draw_targets(c);
        self.draw_inks(c);
        if self.has_doc {
            self.draw_focus(c);
        }
        self.draw_arms(c);
        self.draw_body(c);
        if !self.has_doc {
            self.draw_caption(c);
        }
    }

    fn draw_targets(&self, c: &CanvasRenderingContext2d) {
        for t in &self.targets {
            if t.inked {
                continue;
            }
            if self.has_doc {
                // faint cyan underline on the real link/heading being eyed
                let [x, y, w, h] = t.rect;
                c.set_global_alpha(0.28);
                set_stroke(c, "#2ad4ff");
                c.set_line_width(1.5);
                c.begin_path();
                c.move_to(x, y + h + 1.5);
                c.line_to(x + w, y + h + 1.5);
                c.stroke();
            } else {
                set_stroke(c, "#ff5a7a");
                c.set_global_alpha(0.75);
                c.set_line_width(1.5);
                c.begin_path();
                let _ = c.arc(t.pos.x, t.pos.y, 4.5, 0.0, std::f64::consts::TAU);
                c.stroke();
            }
        }
        c.set_global_alpha(1.0);
    }

    fn draw_inks(&self, c: &CanvasRenderingContext2d) {
        for ink in &self.inks {
            let grow = (ink.age / 0.45).min(1.0);

            if self.has_doc && ink.rect[2] > 2.0 {
                // ink a real content box PRECISELY: a cyan "read" wash hugging the
                // exact text (still legible), an underline sweeping in left-to-
                // right like a finger tracing the line, and a tiny droplet where a
                // tentacle touched. Headings read bolder than body prose.
                let [x, y, w, h] = ink.rect;
                let g = ease_out(grow);
                let (wash, uw, uline) = match ink.kind {
                    1 => (0.20, 2.0, "#7febff"), // heading — stands out
                    2 => (0.11, 1.4, "#35e0ff"), // body text
                    _ => (0.10, 1.4, "#35e0ff"), // link
                };
                c.set_global_alpha(wash * g);
                set_fill(c, "#2ad4ff");
                c.fill_rect(x, y, w, h);
                c.set_global_alpha(0.9 * g);
                set_stroke(c, uline);
                c.set_line_width(uw);
                c.begin_path();
                c.move_to(x, y + h);
                c.line_to(x + w * (0.2 + 0.8 * g), y + h);
                c.stroke();
                c.set_global_alpha(0.55 * g);
                set_fill(c, "#0a1c3e");
                circle(c, ink.pos.x, ink.pos.y, 2.0 + 1.3 * g);
            } else {
                let r = ink.max_r * ease_out(grow);
                let settle = if ink.age > 0.45 { 0.34 } else { 0.6 };
                c.set_global_alpha(settle);
                set_fill(c, "#03040a");
                circle(c, ink.pos.x, ink.pos.y, r);
                let mut rng = Rng::new(ink.seed);
                for _ in 0..6 {
                    let a = rng.range(0.0, std::f64::consts::TAU);
                    let dd = rng.range(r * 0.6, r * 1.3);
                    let rr = rng.range(r * 0.12, r * 0.3);
                    circle(c, ink.pos.x + a.cos() * dd, ink.pos.y + a.sin() * dd, rr * grow);
                }
            }
        }
        c.set_global_alpha(1.0);
    }

    /// Thin cyan corner brackets around the text each tentacle is lining up on —
    /// the hatchling is reading precisely, not smearing ink at random.
    fn draw_focus(&self, c: &CanvasRenderingContext2d) {
        set_stroke(c, "#35e0ff");
        c.set_line_width(1.3);
        c.set_global_alpha(0.7);
        for arm in &self.arms {
            let Some(i) = arm.target else { continue };
            let Some(t) = self.targets.get(i) else { continue };
            if t.inked {
                continue;
            }
            let [x, y, w, h] = t.rect;
            let s = 7.0_f64.min(w * 0.35).min(h * 0.7).max(2.0);
            // four corner brackets
            c.begin_path();
            c.move_to(x, y + s);
            c.line_to(x, y);
            c.line_to(x + s, y);
            c.move_to(x + w - s, y);
            c.line_to(x + w, y);
            c.line_to(x + w, y + s);
            c.move_to(x, y + h - s);
            c.line_to(x, y + h);
            c.line_to(x + s, y + h);
            c.move_to(x + w - s, y + h);
            c.line_to(x + w, y + h);
            c.line_to(x + w, y + h - s);
            c.stroke();
        }
        c.set_global_alpha(1.0);
    }

    fn draw_arms(&self, c: &CanvasRenderingContext2d) {
        let base_w = self.body_r() * 0.42;
        let show_suckers = self.w >= 360.0;
        for arm in &self.arms {
            c.set_line_cap("round");
            for i in 0..arm.pts.len() - 1 {
                let tfac = 1.0 - i as f64 / SEGS as f64;
                c.set_line_width((base_w * tfac).max(1.2));
                set_stroke(c, "#122455");
                c.begin_path();
                c.move_to(arm.pts[i].x, arm.pts[i].y);
                c.line_to(arm.pts[i + 1].x, arm.pts[i + 1].y);
                c.stroke();
            }
            if show_suckers {
                set_fill(c, "#2ad4ff");
                c.set_global_alpha(0.5);
                for i in 1..arm.pts.len() {
                    if i % 2 == 0 {
                        continue;
                    }
                    let seg = arm.pts[i].sub(arm.pts[i - 1]);
                    let perp = V::new(-seg.y, seg.x).norm();
                    let tfac = 1.0 - i as f64 / SEGS as f64;
                    let p = arm.pts[i].add(perp.mul(base_w * 0.18 * tfac));
                    circle(c, p.x, p.y, (base_w * 0.12 * tfac).max(0.6));
                }
                c.set_global_alpha(1.0);
            }
        }
    }

    fn draw_body(&self, c: &CanvasRenderingContext2d) {
        let r = self.body_r();
        let pulse = 0.5 + 0.5 * (self.t * 1.9).sin();

        c.save();
        c.set_shadow_blur(18.0 + 10.0 * pulse);
        c.set_shadow_color("#2ad4ff");

        let rx = r * 0.92;
        let ry = r * 1.18;
        set_fill(c, "#0a1836");
        c.begin_path();
        let _ = c.ellipse(
            self.pos.x,
            self.pos.y,
            rx,
            ry,
            self.heading + std::f64::consts::FRAC_PI_2,
            0.0,
            std::f64::consts::TAU,
        );
        c.fill();
        set_stroke(c, "#2ad4ff");
        c.set_global_alpha(0.55 + 0.35 * pulse);
        c.set_line_width(1.6);
        c.stroke();
        c.restore();
        c.set_global_alpha(1.0);

        c.set_global_alpha(0.22);
        set_fill(c, "#1b3a78");
        circle(c, self.pos.x, self.pos.y - ry * 0.15, r * 0.55);
        c.set_global_alpha(1.0);

        let fwd = dir(self.heading);
        let side = V::new(-fwd.y, fwd.x);
        let eye_c = self.pos.add(fwd.mul(r * 0.35));
        for s in [-1.0, 1.0] {
            let e = eye_c.add(side.mul(r * 0.42 * s));
            set_fill(c, "#d8f6ff");
            circle(c, e.x, e.y, r * 0.2);
            set_fill(c, "#05070f");
            let p = e.add(fwd.mul(r * 0.06));
            circle(c, p.x, p.y, r * 0.1);
        }
    }

    fn draw_caption(&self, c: &CanvasRenderingContext2d) {
        if self.caption.is_empty() {
            return;
        }
        set_fill(c, "#5b6b8c");
        c.set_font("12px 'IBM Plex Mono', monospace");
        let _ = c.fill_text(&self.caption, 14.0, self.h - 14.0);
    }
}

fn lerp_angle(a: f64, b: f64, t: f64) -> f64 {
    let mut d = (b - a) % std::f64::consts::TAU;
    if d > std::f64::consts::PI {
        d -= std::f64::consts::TAU;
    }
    if d < -std::f64::consts::PI {
        d += std::f64::consts::TAU;
    }
    a + d * t
}

fn ease_out(t: f64) -> f64 {
    1.0 - (1.0 - t) * (1.0 - t)
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
