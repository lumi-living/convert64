//! The C64 bitmap encoders, ported from png2amiga's `c64.cpp` (`encode_multicolor`,
//! `encode_hires`).
//!
//! What differs from png2c64's (`png2c64_ref::quantize`):
//!
//! - **The multicolor background is always colour 0** (black): png2amiga writes it as the
//!   one shared register and never searches it; png2c64 tries all 16.
//! - **Cell colours for any dither but `none`** are the most used colours of a global
//!   Floyd–Steinberg pass (png2c64's "global-FS coherence", now for ordered methods
//!   too): multicolor the top three besides the background (padded with colour 0, not
//!   the background), hi-res the pair whose segment mixes best over the 3 × 3-blurred
//!   cell (`mix_segment_score`, λ = 0.09375, a colour may pair with itself).
//! - **`none`** brute-forces each cell with the cell metric: `mse` (each pixel's nearest
//!   colour) or `blur` (png2amiga's default: the 3 × 3 binomial blur of the rendered cell
//!   against the globally blurred source, both in OKLab).
//! - **Then every method dithers** within the cell's colours ([`super::dither::diffuse`]).
//!
//! **Hooks** (Convert64's, each inactive by default, so the result stays png2amiga's): a
//! fixed background or png2c64's background search ([`search_background`]), per-cell
//! constraints ([`CellRule`]: the colour histogram and the
//! brute force keep to a cell's candidates and start from its required colours), and
//! fixed cells copied through untouched (locks, the interactive fast path).

use super::super::png2c64_ref::quantize::{Cell, CellRule, Mode, Screen};
use super::colour::{Lab, dist2, fma_dist_sq, linear_to_oklab};
use super::dither::{self, Method, PxSettings};
use super::{Image, Rgb};

/// The per-cell metric of the `none` brute force (`c64::Metric`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Metric {
    /// Each pixel's squared OKLab distance to its nearest cell colour.
    Mse,
    /// Pappas–Neuhoff: the 3 × 3 binomial blur of the rendered cell against the globally
    /// blurred source (png2amiga's default).
    #[default]
    Blur,
}

/// What the encoders may be told beyond png2amiga's own inputs. The default changes
/// nothing.
#[derive(Clone, Debug, Default)]
pub struct Hooks {
    /// The multicolor background; `None`: colour 0, as png2amiga, unless
    /// `search_background`.
    pub background: Option<u8>,
    /// With no `background`, search the multicolor background as png2c64 does
    /// ([`search_background`]) instead of png2amiga's colour 0 (`BackgroundRule::Search`).
    pub search_background: bool,
    /// Per cell, its constraints (empty: none).
    pub rules: Vec<CellRule>,
    /// Per cell, a result copied through untouched (empty: none). Its pixels are neither
    /// picked nor take or pass on error.
    pub fixed: Vec<Option<Cell>>,
    /// Every cell's colours by slot (multicolor `[bg, c1, c2, c3]` with `background` set
    /// to the same `bg`; hi-res `[a, b]`), given instead of png2amiga's cell-colour stage;
    /// the dither pass then runs within them as usual. The reference solver gives
    /// png2c64's cells to run png2amiga's methods on png2c64's pipeline (#275,
    /// `png2c64_ref::amiga`); the cause sheet (`cargo xtask cause-sheet`, #273) swaps
    /// them in. Not a user option.
    pub sets: Option<Vec<Vec<u8>>>,
}

impl Hooks {
    fn rule(&self, cell: usize) -> CellRule {
        self.rules.get(cell).copied().unwrap_or(CellRule::FREE)
    }

    fn fixed(&self, cell: usize) -> Option<&Cell> {
        self.fixed.get(cell).and_then(Option::as_ref)
    }
}

/// The colours of a mask, ascending.
fn bits(mask: u16) -> impl Iterator<Item = u8> {
    (0..16u8).filter(move |c| mask & (1 << c) != 0)
}

/// `BackgroundRule::Search`: png2c64's multicolor background search on png2amiga's
/// picture (`lab`, in png2amiga's OKLab), with png2c64's own criterion. Per background
/// `bg`, a free cell's error is the least, over the 455 triples `{i, j, k}` of the other
/// 15 colours, of `Σ_pixels min_{c ∈ {bg, i, j, k}} |lab − pal[c]|²` (png2c64's `mse`
/// brute force, unbiased: png2amiga's cell search has no ordered threshold bias, where
/// png2c64 adds one for ordered methods); a constrained cell's is its best allowed set's;
/// fixed cells do not count. The background with the lowest sum over the cells wins,
/// ties to the lower index. The total is png2c64's `ScreenResult::total_error` before its
/// global Floyd–Steinberg replacement: it scores the search's own sets, not the colours
/// png2amiga's encoder then takes from its Floyd–Steinberg histogram. Returns the winner
/// and the 16 totals.
pub fn search_background(lab: &[Lab], pal: &[Lab; 16], hooks: &Hooks) -> (u8, [f32; 16]) {
    let mode = Mode::Multicolor;
    let n = mode.cells().0 * mode.cells().1;
    let cells: Vec<usize> = (0..n).collect();
    let tables: Vec<Option<Vec<[f32; 16]>>> = crate::par::map(&cells, |&c| {
        hooks.fixed(c).is_none().then(|| {
            cell_pixels(mode, c)
                .map(|i| pal.map(|p| dist2(lab[i], p)))
                .collect()
        })
    });
    let rules: Vec<CellRule> = cells.iter().map(|&c| hooks.rule(c)).collect();
    super::super::png2c64_ref::quantize::search_background(&tables, &rules)
}

/// `global_fs_indices`: a serpentine Floyd–Steinberg pass over the whole picture against
/// the full palette in OKLab, error clamped to ±0.12 per channel on read and on write.
pub fn global_fs_indices(lab: &[Lab], w: usize, h: usize, pal: &[Lab; 16]) -> Vec<u8> {
    const KERNEL: [(f32, f32, f32); 4] = [
        (1.0, 0.0, 7.0 / 16.0),
        (-1.0, 1.0, 3.0 / 16.0),
        (0.0, 1.0, 5.0 / 16.0),
        (1.0, 1.0, 1.0 / 16.0),
    ];
    const EC: f32 = 0.12;
    let clamp = |e: Lab| e.map(|v| v.clamp(-EC, EC));
    let mut err = vec![[0.0f32; 3]; w * h];
    let mut out = vec![0u8; w * h];
    for y in 0..h {
        let reverse = y & 1 != 0;
        let dir = if reverse { -1.0f32 } else { 1.0 };
        for step in 0..w {
            let x = if reverse { w - 1 - step } else { step };
            let idx = y * w + x;
            let ce = clamp(err[idx]);
            let adj = [
                lab[idx][0] + ce[0],
                lab[idx][1] + ce[1],
                lab[idx][2] + ce[2],
            ];
            let (mut bd, mut bi) = (f32::MAX, 0usize);
            for (c, &cl) in pal.iter().enumerate() {
                let d = dist2(adj, cl);
                if d < bd {
                    bd = d;
                    bi = c;
                }
            }
            out[idx] = bi as u8;
            let cl = pal[bi];
            let qe = [adj[0] - cl[0], adj[1] - cl[1], adj[2] - cl[2]];
            for (kx, ky, kw) in KERNEL {
                let nx = x as i64 + (kx * dir) as i64;
                let ny = y as i64 + ky as i64;
                if nx < 0 || nx as usize >= w || ny < 0 || ny as usize >= h {
                    continue;
                }
                let n = ny as usize * w + nx as usize;
                let e = err[n];
                err[n] = clamp([e[0] + qe[0] * kw, e[1] + qe[1] * kw, e[2] + qe[2] * kw]);
            }
        }
    }
    out
}

/// `global_blur_3x3`: the 3 × 3 binomial blur over the whole picture, edges replicated.
pub fn global_blur_3x3(src: &[Lab], w: usize, h: usize) -> Vec<Lab> {
    const K: [[f32; 3]; 3] = [
        [1.0 / 16.0, 2.0 / 16.0, 1.0 / 16.0],
        [2.0 / 16.0, 4.0 / 16.0, 2.0 / 16.0],
        [1.0 / 16.0, 2.0 / 16.0, 1.0 / 16.0],
    ];
    let mut out = vec![[0.0f32; 3]; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut acc = [0.0f32; 3];
            for dy in -1i64..=1 {
                let ny = (y as i64 + dy).clamp(0, h as i64 - 1) as usize;
                for dx in -1i64..=1 {
                    let nx = (x as i64 + dx).clamp(0, w as i64 - 1) as usize;
                    let wt = K[(dy + 1) as usize][(dx + 1) as usize];
                    let s = src[ny * w + nx];
                    acc = [acc[0] + wt * s[0], acc[1] + wt * s[1], acc[2] + wt * s[2]];
                }
            }
            out[y * w + x] = acc;
        }
    }
    out
}

/// A cell's pixels (row-major within the cell) as screen indices.
fn cell_pixels(mode: Mode, cell: usize) -> impl Iterator<Item = usize> {
    let (w, _) = mode.screen();
    let (cw, ch) = mode.cell();
    let cols = mode.cells().0;
    let (px, py) = ((cell % cols) * cw, (cell / cols) * ch);
    (0..ch).flat_map(move |dy| (0..cw).map(move |dx| (py + dy) * w + px + dx))
}

/// `score_cell`'s blur metric for a candidate set: each pixel's nearest candidate (in
/// the OKLab the source is held in), the rendered cell blurred with the 9-tap kernel
/// replicated at the cell's edges, against the globally blurred source.
fn blur_score(raw: &[Lab], blurred: &[Lab], cand: &[Lab], cw: usize, ch: usize) -> f32 {
    const K: [[f32; 3]; 3] = [
        [1.0 / 16.0, 2.0 / 16.0, 1.0 / 16.0],
        [2.0 / 16.0, 4.0 / 16.0, 2.0 / 16.0],
        [1.0 / 16.0, 2.0 / 16.0, 1.0 / 16.0],
    ];
    let rendered: Vec<Lab> = raw
        .iter()
        .map(|p| {
            let (mut best, mut q) = (f32::INFINITY, 0usize);
            for (k, c) in cand.iter().enumerate() {
                let d = dist2(*p, *c);
                if d < best {
                    best = d;
                    q = k;
                }
            }
            cand[q]
        })
        .collect();
    let mut err = 0.0f32;
    for py in 0..ch {
        for px in 0..cw {
            let mut b = [0.0f32; 3];
            for dy in -1i64..=1 {
                let ny = (py as i64 + dy).clamp(0, ch as i64 - 1) as usize;
                for dx in -1i64..=1 {
                    let nx = (px as i64 + dx).clamp(0, cw as i64 - 1) as usize;
                    let wt = K[(dy + 1) as usize][(dx + 1) as usize];
                    let r = rendered[ny * cw + nx];
                    b = [b[0] + wt * r[0], b[1] + wt * r[1], b[2] + wt * r[2]];
                }
            }
            let s = blurred[py * cw + px];
            err += fma_dist_sq(s[0] - b[0], s[1] - b[1], s[2] - b[2]);
        }
    }
    err
}

/// `mix_segment_score`: the distance from `t` to the OKLab segment `a`–`b` plus the
/// mixing-noise penalty `λ · u(1 − u) · |a − b|²`.
fn mix_segment_score(t: Lab, a: Lab, b: Lab, lambda: f32) -> f32 {
    let (dl, da, db) = (b[0] - a[0], b[1] - a[1], b[2] - a[2]);
    let seg_sq = fma_dist_sq(dl, da, db);
    if seg_sq <= 0.0 {
        return dist2(t, a);
    }
    let mut u = ((t[0] - a[0]) * dl + (t[1] - a[1]) * da + (t[2] - a[2]) * db) / seg_sq;
    u = u.clamp(0.0, 1.0);
    let el = t[0] - (a[0] + u * dl);
    let ea = t[1] - (a[1] + u * da);
    let eb = t[2] - (a[2] + u * db);
    fma_dist_sq(el, ea, eb) + lambda * u * (1.0 - u) * seg_sq
}

/// `kCellPairMixNoiseLambda`: the hi-res cells' mixing-noise weight.
const CELL_PAIR_LAMBDA: f32 = 0.093_75;

/// The most used colours of a cell's histogram, `k` of them, skipping `skip` (the
/// background), most used first, ties to the lower index; a constrained cell takes its
/// required colours first and the rest from its candidates.
fn top_colours(mut hist: [u16; 16], k: usize, skip: Option<u8>, rule: CellRule) -> Vec<u8> {
    let skip_mask = skip.map_or(0u16, |b| 1 << b);
    let mut own: Vec<u8> = bits(rule.required & !skip_mask).take(k).collect();
    for &c in &own {
        hist[usize::from(c)] = 0;
    }
    if let Some(b) = skip {
        hist[usize::from(b)] = 0;
    }
    while own.len() < k {
        let (mut best_cnt, mut best_c) = (0u16, None);
        for c in bits(rule.candidates & !skip_mask) {
            if hist[usize::from(c)] > best_cnt {
                best_cnt = hist[usize::from(c)];
                best_c = Some(c);
            }
        }
        let Some(c) = best_c else { break };
        own.push(c);
        hist[usize::from(c)] = 0;
    }
    own
}

/// Whether any pixel of the cell dithers (a method other than `none`).
fn cell_dithers(mode: Mode, cell: usize, px: &[PxSettings]) -> bool {
    cell_pixels(mode, cell).any(|i| px[i].method != Method::None)
}

/// The multicolor cells' colours (`[bg, c1, c2, c3]`).
fn multicolor_quads(
    lab: &[Lab],
    pal: &[Lab; 16],
    px: &[PxSettings],
    metric: Metric,
    bg: u8,
    hooks: &Hooks,
) -> Vec<[u8; 4]> {
    let mode = Mode::Multicolor;
    let (w, h) = mode.screen();
    let (cw, ch) = mode.cell();
    let n = mode.cells().0 * mode.cells().1;
    let cells: Vec<usize> = (0..n).collect();
    let dithered: Vec<bool> = cells.iter().map(|&c| cell_dithers(mode, c, px)).collect();
    let fs = dithered
        .contains(&true)
        .then(|| global_fs_indices(lab, w, h, pal));
    let blurred =
        (metric == Metric::Blur && dithered.contains(&false)).then(|| global_blur_3x3(lab, w, h));
    crate::par::map(&cells, |&c| {
        if let Some(f) = hooks.fixed(c) {
            let mut q = [0u8; 4];
            for (s, v) in q.iter_mut().zip(&f.colors) {
                *s = *v;
            }
            return q;
        }
        let rule = hooks.rule(c);
        if dithered[c] {
            let fs = fs.as_ref().expect("built when a cell dithers");
            let mut hist = [0u16; 16];
            for i in cell_pixels(mode, c) {
                hist[usize::from(fs[i])] += 1;
            }
            // `top{bg, 0, 0, 0}`: unfilled slots stay colour 0.
            let mut top = [bg, 0, 0, 0];
            for (s, v) in top[1..]
                .iter_mut()
                .zip(top_colours(hist, 3, Some(bg), rule))
            {
                *s = v;
            }
            return top;
        }
        let raw: Vec<Lab> = cell_pixels(mode, c).map(|i| lab[i]).collect();
        let blur: Vec<Lab> = match &blurred {
            Some(b) => cell_pixels(mode, c).map(|i| b[i]).collect(),
            None => Vec::new(),
        };
        // Each pixel's distance to every colour, for the mse metric.
        let d: Vec<[f32; 16]> = raw.iter().map(|&p| pal.map(|c| dist2(p, c))).collect();
        let score = |q: [u8; 4]| match metric {
            Metric::Mse => d.iter().fold(0.0f32, |t, dp| {
                let m = dp[usize::from(q[0])]
                    .min(dp[usize::from(q[1])])
                    .min(dp[usize::from(q[2])])
                    .min(dp[usize::from(q[3])]);
                t + m
            }),
            Metric::Blur => {
                let cand = q.map(|v| pal[usize::from(v)]);
                blur_score(&raw, &blur, &cand, cw, ch)
            }
        };
        let (mut best_err, mut best) = (f32::INFINITY, [0u8, 0, 0, 0]);
        let allowed = |v: u8| rule.candidates & (1 << v) != 0;
        let bgm = 1u16 << bg;
        let required = rule.required & !bgm;
        for i in 0..16u8 {
            if i == bg || !allowed(i) {
                continue;
            }
            for j in i + 1..16 {
                if j == bg || !allowed(j) {
                    continue;
                }
                for k in j + 1..16 {
                    if k == bg || !allowed(k) {
                        continue;
                    }
                    let set = (1u16 << i) | (1 << j) | (1 << k);
                    if required & !set != 0 && required.count_ones() <= 3 {
                        continue;
                    }
                    let q = [bg, i, j, k];
                    let e = score(q);
                    if e < best_err {
                        best_err = e;
                        best = q;
                    }
                }
            }
        }
        if best_err == f32::INFINITY {
            // Fewer than three allowed colours: the allowed ones, padded with 0.
            let own: Vec<u8> = bits(rule.candidates & !bgm).take(3).collect();
            best = [bg, 0, 0, 0];
            for (s, v) in best[1..].iter_mut().zip(own) {
                *s = v;
            }
        }
        best
    })
}

/// The hi-res cells' pairs.
fn hires_pairs(
    lab: &[Lab],
    pal: &[Lab; 16],
    px: &[PxSettings],
    metric: Metric,
    hooks: &Hooks,
) -> Vec<[u8; 2]> {
    let mode = Mode::Hires;
    let (w, h) = mode.screen();
    let (cw, ch) = mode.cell();
    let n = mode.cells().0 * mode.cells().1;
    let cells: Vec<usize> = (0..n).collect();
    let dithered: Vec<bool> = cells.iter().map(|&c| cell_dithers(mode, c, px)).collect();
    let need_blur = dithered.contains(&true) || metric == Metric::Blur;
    let blurred = need_blur.then(|| global_blur_3x3(lab, w, h));
    crate::par::map(&cells, |&c| {
        if let Some(f) = hooks.fixed(c) {
            return [f.colors[0], f.colors.get(1).copied().unwrap_or(f.colors[0])];
        }
        let rule = hooks.rule(c);
        let ok = |i: u8, j: u8| {
            let set = (1u16 << i) | (1 << j);
            rule.candidates & set == set
                && (rule.required & !set == 0 || rule.required.count_ones() > 2)
        };
        let (mut best_err, mut best) = (f32::INFINITY, [0u8, 0]);
        if dithered[c] {
            let b = blurred.as_ref().expect("built when a cell dithers");
            let cell: Vec<Lab> = cell_pixels(mode, c).map(|i| b[i]).collect();
            for i in 0..16u8 {
                for j in i..16 {
                    if !ok(i, j) {
                        continue;
                    }
                    let (a, bb) = (pal[usize::from(i)], pal[usize::from(j)]);
                    let total = cell.iter().fold(0.0f32, |t, &p| {
                        t + mix_segment_score(p, a, bb, CELL_PAIR_LAMBDA)
                    });
                    if total < best_err {
                        best_err = total;
                        best = [i, j];
                    }
                }
            }
        } else {
            let raw: Vec<Lab> = cell_pixels(mode, c).map(|i| lab[i]).collect();
            let blur: Vec<Lab> = match (&blurred, metric) {
                (Some(b), Metric::Blur) => cell_pixels(mode, c).map(|i| b[i]).collect(),
                _ => Vec::new(),
            };
            for i in 0..16u8 {
                for j in i + 1..16 {
                    if !ok(i, j) {
                        continue;
                    }
                    let (a, bb) = (pal[usize::from(i)], pal[usize::from(j)]);
                    let err = match metric {
                        Metric::Mse => raw
                            .iter()
                            .fold(0.0f32, |t, &p| t + dist2(p, a).min(dist2(p, bb))),
                        Metric::Blur => blur_score(&raw, &blur, &[a, bb], cw, ch),
                    };
                    if err < best_err {
                        best_err = err;
                        best = [i, j];
                    }
                }
            }
        }
        if best_err == f32::INFINITY {
            let own: Vec<u8> = bits(rule.candidates).take(2).collect();
            best = [
                own.first().copied().unwrap_or(0),
                own.get(1).or(own.first()).copied().unwrap_or(0),
            ];
        }
        best
    })
}

/// `encode_multicolor` / `encode_hires` with each pixel's own dither settings (the
/// attribute map): cell colours, then every pixel picked within its cell's colours by its
/// method. With one setting for every pixel and default hooks, this is png2amiga's.
///
/// # Panics
/// If `image` is not the mode's screen size or `px` not one entry per pixel.
pub fn encode(
    image: &Image,
    palette: &[Rgb; 16],
    mode: Mode,
    px: &[PxSettings],
    metric: Metric,
    serpentine: bool,
    hooks: &Hooks,
) -> Screen {
    let (w, h) = mode.screen();
    assert_eq!((image.w, image.h), (w, h), "image size for {mode:?}");
    assert_eq!(px.len(), w * h, "one setting per pixel");
    let pal = palette.map(linear_to_oklab);
    let lab: Vec<Lab> = image.px.iter().map(|&p| linear_to_oklab(p)).collect();
    let (cw, ch) = mode.cell();
    let cols = mode.cells().0;
    let n = mode.cells().0 * mode.cells().1;
    let bg = match hooks.background {
        Some(b) => b,
        None if mode == Mode::Multicolor && hooks.search_background => {
            search_background(&lab, &pal, hooks).0
        }
        None => 0,
    };
    let sets: Vec<Vec<u8>> = match (&hooks.sets, mode) {
        (Some(given), _) => {
            assert_eq!(given.len(), n, "one set per cell");
            given.clone()
        }
        (None, Mode::Multicolor) => multicolor_quads(&lab, &pal, px, metric, bg, hooks)
            .into_iter()
            .map(|q| q.to_vec())
            .collect(),
        (None, Mode::Hires) => hires_pairs(&lab, &pal, px, metric, hooks)
            .into_iter()
            .map(|p| p.to_vec())
            .collect(),
    };
    let set_lab: Vec<Vec<Lab>> = sets
        .iter()
        .map(|s| s.iter().map(|&v| pal[usize::from(v)]).collect())
        .collect();
    let cell_of = |x: usize, y: usize| (y / ch) * cols + x / cw;
    // Fixed cells keep their pixels: such a pixel is neither picked nor diffused.
    let mut settings = px.to_vec();
    let mut slots = vec![0u8; w * h];
    if !hooks.fixed.is_empty() {
        for y in 0..h {
            for x in 0..w {
                let c = cell_of(x, y);
                if let Some(f) = hooks.fixed(c) {
                    settings[y * w + x].method = Method::None;
                    slots[y * w + x] = f.px[(y % ch) * cw + x % cw];
                }
            }
        }
    }
    dither::diffuse(image, &settings, serpentine, |target, x, y, s| {
        let c = cell_of(x, y);
        let cp = &set_lab[c];
        if hooks.fixed(c).is_some() {
            return cp[usize::from(slots[y * w + x])];
        }
        let (fx, fy) = s.phase(x, y);
        let q = dither::pick(s.method, target, cp, fx, fy, s.strength);
        slots[y * w + x] = q;
        cp[usize::from(q)]
    });
    let cells: Vec<Cell> = (0..n)
        .map(|c| {
            let pixels: Vec<u8> = cell_pixels(mode, c).map(|i| slots[i]).collect();
            let error = cell_pixels(mode, c)
                .zip(&pixels)
                .fold(0.0f32, |t, (i, &q)| {
                    t + dist2(lab[i], set_lab[c][usize::from(q)])
                });
            Cell {
                colors: sets[c].clone(),
                px: pixels,
                error,
            }
        })
        .collect();
    Screen {
        mode,
        bg: if mode == Mode::Multicolor { bg } else { 0 },
        total_error: cells.iter().fold(0.0, |s, c| s + c.error),
        cells,
    }
}

#[cfg(test)]
mod tests {
    use super::super::colour::{PEPTO_HEX, palette_linear};
    use super::*;

    /// Every cell holds yellow and three other colours that change from cell to cell, so
    /// only a yellow background lets every cell be exact.
    fn yellow_cells() -> Image {
        let (w, h) = Mode::Multicolor.screen();
        let pal = palette_linear(&PEPTO_HEX);
        let others: Vec<u8> = (0..16).filter(|&c| c != 7).collect();
        let px = (0..w * h)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                let cell = (y / 8) * 40 + x / 4;
                let slot = (y % 8) / 2;
                let c = if slot == 0 {
                    7
                } else {
                    others[(cell * 3 + slot) % others.len()]
                };
                pal[usize::from(c)]
            })
            .collect();
        Image::new(w, h, px)
    }

    /// #273: `search_background` finds the background png2c64's criterion prefers, the
    /// encoder uses it only when asked, and Black (the default) stays colour 0.
    #[test]
    fn the_background_search_is_opt_in_and_finds_the_shared_colour() {
        let image = yellow_cells();
        let pal = palette_linear(&PEPTO_HEX);
        let lab: Vec<Lab> = image.px.iter().map(|&p| linear_to_oklab(p)).collect();
        let pal_lab = pal.map(linear_to_oklab);
        let (bg, totals) = search_background(&lab, &pal_lab, &Hooks::default());
        assert_eq!(bg, 7, "{totals:?}");
        assert!(totals[7] < 1e-6 && totals[0] > 1.0, "{totals:?}");
        let px = vec![
            PxSettings {
                method: Method::None,
                strength: 1.0,
                error_clamp: 0.35,
                origin: (0, 0),
            };
            image.w * image.h
        ];
        let encode_with = |hooks: &Hooks| {
            encode(
                &image,
                &pal,
                Mode::Multicolor,
                &px,
                Metric::Mse,
                true,
                hooks,
            )
        };
        assert_eq!(encode_with(&Hooks::default()).bg, 0);
        let searched = encode_with(&Hooks {
            search_background: true,
            ..Hooks::default()
        });
        assert_eq!(searched.bg, 7);
        assert!(searched.total_error < 1e-6, "{}", searched.total_error);
        // A pinned background wins over the search.
        let pinned = encode_with(&Hooks {
            background: Some(1),
            search_background: true,
            ..Hooks::default()
        });
        assert_eq!(pinned.bg, 1);
    }
}
