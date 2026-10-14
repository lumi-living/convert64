//! The opt-in Refine step, ported from png2amiga's `--cell-refine` for the C64 bitmap
//! modes (`c64::refine_cells` and `cell_graph_refine.hpp`).
//!
//! Coordinate descent on a global objective: the squared difference between the
//! 3 × 3 binomial blur of the rendered picture and of the source (each cell's one-pixel
//! halo included, so cells are no longer solved in isolation). Variables are the colour
//! registers: the multicolor background (all 16 values), each cell's screen-RAM pair and
//! colour-RAM colour; selectors are the bitmap bits. Four passes (alternating
//! direction) over every register, then every pixel's bit unless the dither is ordered
//! (then a register change re-runs the method's picker over every pixel using it, so the
//! pattern's phase is kept). The descent runs twice, once in linear RGB and once in
//! OKLab, and after each pass the full-image SSIMULACRA2 is taken; the best legal
//! encoding is kept only if it scores higher than the solve it started from.
//!
//! png2amiga's `cell_refine.hpp` (pattern-table refinement) is used only for its AFLI
//! mode with error diffusion, which Convert64 does not have; it is not ported.

use super::colour::{Lab, linear_to_oklab};
use super::dither::{self, PxSettings};
use super::{Image, Mode, Rgb, Screen};
use crate::quality::ssimulacra2;

/// png2amiga's `cell_graph_refine::Model` for a bitmap screen.
#[derive(Clone)]
struct Model {
    palette: Vec<Rgb>,
    values: Vec<u8>,
    limits: Vec<usize>,
    slots: Vec<[usize; 4]>,
    labels: Vec<u8>,
    groups: Vec<usize>,
    choices: usize,
}

impl Model {
    fn add(&mut self, value: u8, limit: usize) -> usize {
        self.values.push(value);
        self.limits.push(limit);
        self.values.len() - 1
    }
}

#[inline]
fn dot(a: Rgb, b: Rgb) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

#[inline]
fn sub(a: Rgb, b: Rgb) -> Rgb {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

#[inline]
fn mul(a: Rgb, s: f32) -> Rgb {
    [a[0] * s, a[1] * s, a[2] * s]
}

#[inline]
fn add_to(a: &mut Rgb, b: Rgb) {
    *a = [a[0] + b[0], a[1] + b[1], a[2] + b[2]];
}

/// `cell_graph_refine::Objective`.
struct Objective {
    w: usize,
    h: usize,
    residual: Vec<Rgb>,
    delta: Vec<Rgb>,
    touched: Vec<usize>,
    marked: Vec<bool>,
    pixels: Vec<Rgb>,
}

impl Objective {
    fn halo(w: usize, h: usize, p: usize, mut f: impl FnMut(usize, f32)) {
        let (x, y) = ((p % w) as i64, (p / w) as i64);
        for dy in -1i64..=1 {
            for dx in -1i64..=1 {
                let (xx, yy) = (x + dx, y + dy);
                if xx < 0 || yy < 0 || xx >= w as i64 || yy >= h as i64 {
                    continue;
                }
                let wt =
                    ((if dx == 0 { 2 } else { 1 }) * (if dy == 0 { 2 } else { 1 })) as f32 / 16.0;
                f(yy as usize * w + xx as usize, wt);
            }
        }
    }

    fn new(source: &[Rgb], w: usize, h: usize, m: &Model) -> Self {
        let n = w * h;
        let pixels: Vec<Rgb> = (0..n)
            .map(|p| m.palette[usize::from(m.values[m.slots[p][usize::from(m.labels[p])]])])
            .collect();
        let mut residual = vec![[0.0f32; 3]; n];
        for p in 0..n {
            let d = sub(pixels[p], source[p]);
            Self::halo(w, h, p, |q, a| add_to(&mut residual[q], mul(d, a)));
        }
        Self {
            w,
            h,
            residual,
            delta: vec![[0.0; 3]; n],
            touched: Vec::new(),
            marked: vec![false; n],
            pixels,
        }
    }

    fn clear(&mut self) {
        for &p in &self.touched {
            self.delta[p] = [0.0; 3];
            self.marked[p] = false;
        }
        self.touched.clear();
    }

    fn add(&mut self, p: usize, d: Rgb) {
        let (w, h) = (self.w, self.h);
        let (marked, touched, delta) = (&mut self.marked, &mut self.touched, &mut self.delta);
        Self::halo(w, h, p, |q, a| {
            if !marked[q] {
                marked[q] = true;
                touched.push(q);
            }
            add_to(&mut delta[q], mul(d, a));
        });
    }

    fn score(&mut self, positions: &[usize], colors: &[Rgb], commit: bool) -> f32 {
        self.clear();
        for (&p, &c) in positions.iter().zip(colors) {
            let d = sub(c, self.pixels[p]);
            self.add(p, d);
        }
        let mut e = 0.0f32;
        for &p in &self.touched {
            e += 2.0 * dot(self.residual[p], self.delta[p]) + dot(self.delta[p], self.delta[p]);
        }
        if commit {
            for &p in &self.touched {
                let d = self.delta[p];
                add_to(&mut self.residual[p], d);
            }
            for (&p, &c) in positions.iter().zip(colors) {
                self.pixels[p] = c;
            }
        }
        e
    }

    /// `recolor_ordered`.
    fn recolor_ordered(
        &mut self,
        m: &mut Model,
        v: usize,
        users: &[usize],
        targets: &[Lab],
        palette_lab: &[Lab],
        s: &PxSettings,
    ) {
        if users.is_empty() || m.limits[v] <= 1 {
            return;
        }
        let mut best = m.values[v];
        let mut cost = 0.0f32;
        let mut colors = vec![[0.0f32; 3]; users.len()];
        let mut labels = vec![0u8; users.len()];
        let (mut best_colors, mut best_labels) = (Vec::new(), Vec::new());
        for c in 0..m.limits[v] {
            m.values[v] = c as u8;
            for (i, &p) in users.iter().enumerate() {
                let local: Vec<Lab> = (0..m.choices)
                    .map(|q| palette_lab[usize::from(m.values[m.slots[p][q]])])
                    .collect();
                let q = dither::pick(s.method, targets[p], &local, p % self.w, p / self.w, s.strength);
                labels[i] = q;
                colors[i] = m.palette[usize::from(m.values[m.slots[p][usize::from(q)]])];
            }
            let e = self.score(users, &colors, false);
            if e < cost - 1e-7 {
                cost = e;
                best = c as u8;
                best_colors.clone_from(&colors);
                best_labels.clone_from(&labels);
            }
        }
        m.values[v] = best;
        if !best_labels.is_empty() {
            for (&p, &l) in users.iter().zip(&best_labels) {
                m.labels[p] = l;
            }
            self.score(users, &best_colors, true);
        }
    }

    /// `recolor`.
    fn recolor(&mut self, m: &mut Model, v: usize, users: &[usize]) {
        let mut active = Vec::new();
        self.clear();
        for &p in users {
            if m.slots[p][usize::from(m.labels[p])] == v {
                active.push(p);
                self.add(p, [1.0, 0.0, 0.0]);
            }
        }
        if active.is_empty() || m.limits[v] <= 1 {
            return;
        }
        let mut aa = 0.0f32;
        let mut b = [0.0f32; 3];
        for &p in &self.touched {
            let a = self.delta[p][0];
            aa += a * a;
            add_to(&mut b, mul(self.residual[p], a));
        }
        let old = m.palette[usize::from(m.values[v])];
        let mut best = m.values[v];
        let mut cost = 0.0f32;
        for c in 0..m.limits[v] {
            let d = sub(m.palette[c], old);
            let e = 2.0 * dot(b, d) + aa * dot(d, d);
            if e < cost - 1e-7 {
                cost = e;
                best = c as u8;
            }
        }
        if best != m.values[v] {
            m.values[v] = best;
            let colors = vec![m.palette[usize::from(best)]; active.len()];
            self.score(&active, &colors, true);
        }
    }
}

/// `refine_passes`: four passes in linear RGB or (`lab`) OKLab; returns whether any
/// pass's S2 beat the start, leaving the best values and labels in `m` and its render in
/// `rendered`.
fn refine_passes(
    source: &Image,
    m: &mut Model,
    rendered: &mut [Rgb],
    lab: bool,
    ordered: Option<&PxSettings>,
    serpentine: bool,
) -> bool {
    let (w, h) = (source.w, source.h);
    let linear_palette = m.palette.clone();
    let before = rendered.to_vec();
    let (mut targets, mut palette_lab) = (Vec::new(), Vec::new());
    if let Some(s) = ordered {
        palette_lab = linear_palette.iter().map(|&c| linear_to_oklab(c)).collect();
        targets = vec![[0.0f32; 3]; w * h];
        // Reuse the encoder's threshold offsets, clipping and phase exactly.
        let px = vec![*s; w * h];
        dither::diffuse(source, &px, serpentine, |t, x, y, _| {
            targets[y * w + x] = t;
            t
        });
    }
    let mut metric_source: Vec<Rgb> = source.px.clone();
    if lab {
        for c in &mut m.palette {
            *c = linear_to_oklab(*c);
        }
        for c in &mut metric_source {
            *c = linear_to_oklab(*c);
        }
    }
    let mut obj = Objective::new(&metric_source, w, h, m);
    let mut users: Vec<Vec<usize>> = vec![Vec::new(); m.values.len()];
    for p in 0..m.slots.len() {
        for q in 0..m.choices {
            let v = m.slots[p][q];
            if !(0..q).any(|k| m.slots[p][k] == v) {
                users[v].push(p);
            }
        }
    }
    const PASSES: usize = 4;
    let s2 = |px: &[Rgb]| ssimulacra2(&source.px, px, w, h);
    let mut best_score = s2(&before);
    let mut best_values = m.values.clone();
    let mut best_labels = m.labels.clone();
    let mut best_pixels = before;
    let mut improved = false;
    for pass in 0..PASSES {
        let n = m.values.len();
        for k in 0..n {
            let v = if pass > 0 { n - 1 - k } else { k };
            match ordered {
                Some(s) => obj.recolor_ordered(m, v, &users[v], &targets, &palette_lab, s),
                None => obj.recolor(m, v, &users[v]),
            }
        }
        if ordered.is_none() {
            let g = m.groups.len();
            for k in 0..g {
                let p = m.groups[if pass > 0 { g - 1 - k } else { k }];
                let mut best = m.labels[p];
                let mut cost = 0.0f32;
                for q in 0..m.choices {
                    let c = [m.palette[usize::from(m.values[m.slots[p][q]])]];
                    let e = obj.score(&[p], &c, false);
                    if e < cost - 1e-7 {
                        cost = e;
                        best = q as u8;
                    }
                }
                if best != m.labels[p] {
                    m.labels[p] = best;
                    let c = [m.palette[usize::from(m.values[m.slots[p][usize::from(best)]])]];
                    obj.score(&[p], &c, true);
                }
            }
        }
        let candidate: Vec<Rgb> = (0..m.slots.len())
            .map(|p| linear_palette[usize::from(m.values[m.slots[p][usize::from(m.labels[p])]])])
            .collect();
        let score = s2(&candidate);
        if score.is_finite() && score > best_score {
            best_score = score;
            best_values.clone_from(&m.values);
            best_labels.clone_from(&m.labels);
            best_pixels = candidate;
            improved = true;
        }
    }
    m.palette = linear_palette;
    m.values = best_values;
    m.labels = best_labels;
    if improved {
        rendered.copy_from_slice(&best_pixels);
    }
    improved
}

/// png2amiga's `--cell-refine` on a multicolor or hi-res screen solved from `source`
/// with `palette` (linear). `ordered` is the method's settings when it does not
/// diffuse error (png2amiga: `!uses_error_diffusion`), else `None`. Returns whether the
/// screen changed; it changes only if its SSIMULACRA2 against `source` went up.
pub fn refine(
    source: &Image,
    screen: &mut Screen,
    palette: &[Rgb; 16],
    settings: &PxSettings,
    serpentine: bool,
) -> bool {
    let mode = screen.mode;
    let mc = mode == Mode::Multicolor;
    let (w, h) = mode.screen();
    let (cw, ch) = mode.cell();
    let cols = mode.cells().0;
    let n = cols * mode.cells().1;
    let mut m = Model {
        palette: palette.to_vec(),
        values: Vec::new(),
        limits: Vec::new(),
        slots: vec![[0; 4]; w * h],
        labels: vec![0; w * h],
        groups: Vec::new(),
        choices: if mc { 4 } else { 2 },
    };
    // png2amiga's registers, in its order: the FLI edge colour, the background, mc1 and
    // mc2 (fixed outside charsets), then per cell its colour RAM (multicolor) and its
    // screen-RAM high and low nibbles.
    m.add(15, 1);
    let bg = m.add(screen.bg, 16);
    m.add(0, 1);
    m.add(0, 1);
    let (mut hi, mut lo, mut fg) = (vec![0; n], vec![0; n], vec![0; n]);
    for c in 0..n {
        let colors = &screen.cells[c].colors;
        if mc {
            fg[c] = m.add(colors[3], 16);
            hi[c] = m.add(colors[1], 16);
            lo[c] = m.add(colors[2], 16);
        } else {
            hi[c] = m.add(colors[1], 16);
            lo[c] = m.add(colors[0], 16);
        }
    }
    for c in 0..n {
        let slots = if mc {
            [bg, hi[c], lo[c], fg[c]]
        } else {
            [lo[c], hi[c], bg, bg]
        };
        for y in 0..ch {
            for x in 0..cw {
                let p = ((c / cols) * ch + y) * w + (c % cols) * cw + x;
                m.slots[p] = slots;
                m.labels[p] = screen.cells[c].px[y * cw + x];
                m.groups.push(p);
            }
        }
    }
    let mut rendered: Vec<Rgb> = (0..w * h)
        .map(|p| palette[usize::from(m.values[m.slots[p][usize::from(m.labels[p])]])])
        .collect();
    let ordered = (!settings.method.diffuses()).then_some(settings);
    let mut alternative = m.clone();
    let mut alt_render = rendered.clone();
    let improved = refine_passes(source, &mut m, &mut rendered, false, ordered, serpentine);
    let other = refine_passes(source, &mut alternative, &mut alt_render, true, ordered, serpentine);
    let s2 = |px: &[Rgb]| ssimulacra2(&source.px, px, w, h);
    let chosen = if other && (!improved || s2(&alt_render) > s2(&rendered)) {
        alternative
    } else if improved {
        m
    } else {
        return false;
    };
    let new_bg = chosen.values[bg];
    for c in 0..n {
        let cell = &mut screen.cells[c];
        cell.colors = if mc {
            vec![new_bg, chosen.values[hi[c]], chosen.values[lo[c]], chosen.values[fg[c]]]
        } else {
            vec![chosen.values[lo[c]], chosen.values[hi[c]]]
        };
        for y in 0..ch {
            for x in 0..cw {
                let p = ((c / cols) * ch + y) * w + (c % cols) * cw + x;
                cell.px[y * cw + x] = chosen.labels[p];
            }
        }
    }
    if mc {
        screen.bg = new_bg;
    }
    true
}
