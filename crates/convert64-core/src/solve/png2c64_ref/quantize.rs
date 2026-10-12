//! The cell colour search, ported from png2c64's `quantize.cpp`: the bitmap modes
//! (multicolor and hi-res) with its default `mse` metric.
//!
//! - **Distance tables.** Every pixel's squared OKLab distance to all 16 colours, after
//!   the ordered dither's threshold bias (`precompute_cell_dist`): with a threshold `t`,
//!   L moves by `0.15 t` and a, b by `0.03 t`.
//! - **Brute force.** Multicolor tries every background and, per cell, every triple of the
//!   other 15 colours (455); hi-res every pair (120). A cell's error is the sum over its
//!   pixels of the distance to the nearest of its colours; the background with the least
//!   total wins.
//! - **Global Floyd–Steinberg** (`global_fs_indices`). When no ordered threshold is given
//!   (diffusion methods and `none`), png2c64 then *replaces* every cell's colours with the
//!   most used colours of a Floyd–Steinberg dither of the whole picture against the full
//!   palette, error clamped to ±0.12, in that cell; the background stays the one the brute
//!   force chose. Odd, but ported as png2c64 does it.
//!
//! **Exactness.** png2c64 adds a cell's per-pixel minima in pixel order; the minimum of
//! four distances does not depend on which of them is the background, so each 4-set's
//! sum is computed once per cell and shared by the backgrounds it contains, in the same
//! pixel order, giving the same bits. Ties go, as in png2c64's loops, to the first
//! triple in `i < j < k` order and the first background. png2c64 sums the screen total
//! in thread completion order; this port sums in cell order, which can only matter for
//! a background tie within rounding.

use super::colour::{Lab, dist2, linear_to_oklab};
use super::{Image, Rgb};
use std::sync::OnceLock;

/// The bitmap modes of the port (png2c64's `vic2::Mode::{multicolor, hires}`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mode {
    /// 160 × 200 double-wide pixels, 4 × 8 cells, a shared background plus 3 per cell.
    Multicolor,
    /// 320 × 200, 8 × 8 cells, 2 colours per cell.
    Hires,
}

impl Mode {
    /// The picture size in (logical) pixels (`get_mode_params`).
    pub const fn screen(self) -> (usize, usize) {
        match self {
            Self::Multicolor => (160, 200),
            Self::Hires => (320, 200),
        }
    }

    /// The cell size in pixels.
    pub const fn cell(self) -> (usize, usize) {
        match self {
            Self::Multicolor => (4, 8),
            Self::Hires => (8, 8),
        }
    }

    /// Cells across and down (40 × 25).
    pub const fn cells(self) -> (usize, usize) {
        let ((w, h), (cw, ch)) = (self.screen(), self.cell());
        (w / cw, h / ch)
    }
}

/// One cell's result (`CellResult`): its colours by slot and each pixel's slot.
#[derive(Clone, Debug, PartialEq)]
pub struct Cell {
    /// Palette indices by slot: multicolor `[background, screen hi, screen lo, colour
    /// RAM]`, hi-res `[0 bit, 1 bit]`. After the global Floyd–Steinberg replacement a
    /// multicolor cell may repeat the background in unused slots, as png2c64's does.
    pub colors: Vec<u8>,
    /// Each pixel's slot, row-major within the cell.
    pub px: Vec<u8>,
    /// The cell's error (Σ squared OKLab distance) from the brute force.
    pub error: f32,
}

/// A solved screen (`ScreenResult`).
#[derive(Clone, Debug, PartialEq)]
pub struct Screen {
    /// The mode.
    pub mode: Mode,
    /// The background (multicolor; 0 for hi-res).
    pub bg: u8,
    /// The 1000 cells, row-major.
    pub cells: Vec<Cell>,
    /// Σ cell errors of the winning background.
    pub total_error: f32,
}

impl Screen {
    /// The palette index of pixel `(x, y)` (logical pixels).
    pub fn index_at(&self, x: usize, y: usize) -> u8 {
        let (cw, ch) = self.mode.cell();
        let cell = &self.cells[(y / ch) * self.mode.cells().0 + x / cw];
        cell.colors[usize::from(cell.px[(y % ch) * cw + x % cw])]
    }

    /// The picture as palette indices, row-major (`render_screen` without the colours).
    pub fn indices(&self) -> Vec<u8> {
        let (w, h) = self.mode.screen();
        (0..w * h).map(|i| self.index_at(i % w, i / w)).collect()
    }
}

/// An ordered dither's threshold at `(x, y)`, already × strength (`ThresholdFn`).
pub type Threshold<'a> = &'a (dyn Fn(usize, usize) -> f32 + Sync);

/// What the divergences add to a distance (the `chroma_guard` and `max_luma_diff`
/// penalties): more than any squared OKLab distance between palette colours.
const PENALTY: f32 = 1.0;

/// One cell's distance table: per pixel, the squared distance to each of the 16 colours
/// (`precompute_cell_dist`, pixel-major). `bias[i]` is pixel `i`'s threshold offset `t`
/// (png2c64's `threshold_fn(x, y) × threshold_strength`), or `None` for no bias. Also
/// returns the OKLab L of the cell's ordered (biased) pixels, for the `bridging`
/// divergence. The divergences' penalties are added here; with none, the table is
/// png2c64's.
fn cell_table(
    image: &Image,
    mode: Mode,
    cell: usize,
    pal: &[Lab; 16],
    bias: &[Option<f32>],
    div: &super::Divergences,
) -> (Vec<[f32; 16]>, Vec<f32>) {
    let (cw, ch) = mode.cell();
    let cols = mode.cells().0;
    let (px, py) = ((cell % cols) * cw, (cell / cols) * ch);
    let mut out = Vec::with_capacity(cw * ch);
    let mut ordered = Vec::new();
    for dy in 0..ch {
        for dx in 0..cw {
            let (x, y) = (px + dx, py + dy);
            let plain = linear_to_oklab(image.at(x, y));
            let mut lab = plain;
            let b = bias[y * image.w + x];
            if let Some(t) = b {
                lab[0] += t * 0.15;
                lab[1] += t * 0.03;
                lab[2] += t * 0.03;
                ordered.push(plain[0]);
            }
            let mut d = pal.map(|c| dist2(lab, c));
            if div.chroma_guard && b.is_some() {
                for (v, &c) in d.iter_mut().zip(pal) {
                    if !crate::solve::ordered::guard_allows(plain, c) {
                        *v += PENALTY;
                    }
                }
            }
            if let Some(n) = div.max_luma_diff {
                let own = nearest_index(plain, pal);
                for (c, v) in d.iter_mut().enumerate() {
                    if levels_apart(c as u8, own) > n {
                        *v += PENALTY;
                    }
                }
            }
            out.push(d);
        }
    }
    (out, ordered)
}

/// The nearest of the 16 colours to `p` (the lowest index on a tie).
fn nearest_index(p: Lab, pal: &[Lab; 16]) -> u8 {
    let (mut best, mut idx) = (f32::MAX, 0u8);
    for (c, &cl) in pal.iter().enumerate() {
        let d = dist2(p, cl);
        if d < best {
            best = d;
            idx = c as u8;
        }
    }
    idx
}

/// Luma levels between two palette indices.
fn levels_apart(a: u8, b: u8) -> u8 {
    use crate::colour::ColourIndex;
    crate::palette::luma::level_diff(
        ColourIndex::new(a).expect("index below 16"),
        ColourIndex::new(b).expect("index below 16"),
    )
}

/// What the `pair_bias` and `bridging` divergences add to a set's search score: the
/// bias per same-luma pair in it, and 0.01 × each ordered pixel's lightness bracket.
fn set_extra(mask: u16, ordered: &[f32], pal: &[Lab; 16], div: &super::Divergences) -> f32 {
    let mut extra = 0.0f32;
    if div.pair_bias != 0.0 {
        let set: crate::colour::ColourSet = crate::colour::ColourSet::from_bits(mask);
        extra += crate::solve::metric::luma_pair_bias(set, div.pair_bias);
    }
    if div.bridging {
        for &l in ordered {
            let (mut lo, mut hi) = (f32::NEG_INFINITY, f32::INFINITY);
            for c in bits(mask) {
                let cl = pal[usize::from(c)][0];
                if cl <= l && cl > lo {
                    lo = cl;
                }
                if cl >= l && cl < hi {
                    hi = cl;
                }
            }
            extra += 0.01
                * if lo.is_finite() && hi.is_finite() {
                    (hi - lo) * (hi - lo)
                } else {
                    let end = if lo.is_finite() { lo } else { hi };
                    0.5 + (l - end).abs()
                };
        }
    }
    extra
}

/// The bit mask of a cell's slots.
fn mask_of(slots: &[u8]) -> u16 {
    slots.iter().fold(0, |m, &c| m | (1 << c))
}

/// The lexicographic rank of each 4-colour set among the 1820, by bit mask.
fn rank4() -> &'static [u16] {
    static RANK: OnceLock<Vec<u16>> = OnceLock::new();
    RANK.get_or_init(|| {
        let mut rank = vec![u16::MAX; 1 << 16];
        let mut r = 0u16;
        for a in 0..16 {
            for b in a + 1..16 {
                for c in b + 1..16 {
                    for d in c + 1..16 {
                        rank[(1usize << a) | (1 << b) | (1 << c) | (1 << d)] = r;
                        r += 1;
                    }
                }
            }
        }
        rank
    })
}

/// `cell_error_4` for all 1820 four-colour sets of one cell, by [`rank4`].
fn errors4(table: &[[f32; 16]]) -> Vec<f32> {
    let mut out = vec![0.0f32; 1820];
    let mut m = vec![0.0f32; table.len()];
    let mut r = 0usize;
    for a in 0..16 {
        for b in a + 1..16 {
            for c in b + 1..16 {
                if c == 15 {
                    continue;
                }
                for (mp, d) in m.iter_mut().zip(table) {
                    *mp = d[a].min(d[b]).min(d[c]);
                }
                // One running sum per fourth colour, each in pixel order.
                let mut acc = [0.0f32; 16];
                for (&mp, d) in m.iter().zip(table) {
                    for k in 0..16 {
                        acc[k] += mp.min(d[k]);
                    }
                }
                let n = 15 - c;
                out[r..r + n].copy_from_slice(&acc[c + 1..]);
                r += n;
            }
        }
    }
    out
}

/// `cell_error_2` for all 120 pairs of one cell, in `i < j` order.
fn errors2(table: &[[f32; 16]]) -> Vec<f32> {
    let mut out = Vec::with_capacity(120);
    for i in 0..16 {
        let mut acc = [0.0f32; 16];
        for d in table {
            for j in 0..16 {
                acc[j] += d[i].min(d[j]);
            }
        }
        out.extend_from_slice(&acc[i + 1..]);
    }
    out
}

/// `evaluate_cell_full`: each pixel's nearest slot (the first on a tie) and the sum of
/// those distances in pixel order.
fn evaluate(table: &[[f32; 16]], slots: &[u8]) -> (Vec<u8>, f32) {
    let mut px = Vec::with_capacity(table.len());
    let mut error = 0.0f32;
    for d in table {
        let (mut best, mut idx) = (f32::MAX, 0u8);
        for (s, &c) in slots.iter().enumerate() {
            let v = d[usize::from(c)];
            if v < best {
                best = v;
                idx = s as u8;
            }
        }
        px.push(idx);
        error += best;
    }
    (px, error)
}

/// `quantize_cell_multicolor` for every background, from the cell's 4-set errors: the
/// best triple per background and its error.
fn best_triples(err4: &[f32]) -> [([u8; 3], f32); 16] {
    let rank = rank4();
    let mut out = [([0u8; 3], 0.0f32); 16];
    for (bg, slot) in out.iter_mut().enumerate() {
        let (mut best, mut pick) = (f32::MAX, [0u8, 0, 0]);
        for i in 0..16 {
            if i == bg {
                continue;
            }
            for j in i + 1..16 {
                if j == bg {
                    continue;
                }
                for k in j + 1..16 {
                    if k == bg {
                        continue;
                    }
                    let e = err4[usize::from(rank[(1 << bg) | (1 << i) | (1 << j) | (1 << k)])];
                    if e < best {
                        best = e;
                        pick = [i as u8, j as u8, k as u8];
                    }
                }
            }
        }
        *slot = (pick, best);
    }
    out
}

/// `quantize_cell_multicolor` for one background only: the best triple and its error,
/// computed from the 455 four-colour sets that contain `bg` (the same sums, in the same
/// pixel order, as [`errors4`] gives them).
fn best_triple_for(table: &[[f32; 16]], bg: usize) -> ([u8; 3], f32) {
    let (mut best, mut pick) = (f32::MAX, [0u8, 0, 0]);
    let mut m = vec![0.0f32; table.len()];
    for i in 0..16 {
        if i == bg {
            continue;
        }
        for j in i + 1..16 {
            if j == bg {
                continue;
            }
            for (mp, d) in m.iter_mut().zip(table) {
                *mp = d[bg].min(d[i]).min(d[j]);
            }
            let mut acc = [0.0f32; 16];
            for (&mp, d) in m.iter().zip(table) {
                for k in 0..16 {
                    acc[k] += mp.min(d[k]);
                }
            }
            for (k, &e) in acc.iter().enumerate().skip(j + 1) {
                if k != bg && e < best {
                    best = e;
                    pick = [i as u8, j as u8, k as u8];
                }
            }
        }
    }
    (pick, best)
}

/// `global_fs_indices`: a serpentine Floyd–Steinberg dither of the whole picture
/// against the full palette in OKLab, error clamped to ±0.12 per channel on read and on
/// write. Returns each pixel's palette index.
pub fn global_fs_indices(image: &Image, pal: &[Lab; 16]) -> Vec<u8> {
    const KERNEL: [(f32, f32, f32); 4] = [
        (1.0, 0.0, 7.0 / 16.0),
        (-1.0, 1.0, 3.0 / 16.0),
        (0.0, 1.0, 5.0 / 16.0),
        (1.0, 1.0, 1.0 / 16.0),
    ];
    const EC: f32 = 0.12;
    let clamp = |e: Lab| e.map(|v| v.clamp(-EC, EC));
    let (w, h) = (image.w, image.h);
    let lab: Vec<Lab> = image.px.iter().map(|&p| linear_to_oklab(p)).collect();
    let mut err = vec![[0.0f32; 3]; w * h];
    let mut out = vec![0u8; w * h];
    for y in 0..h {
        let reverse = y & 1 != 0;
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
                let sx = if reverse { -kx } else { kx };
                let nx = x as i64 + sx as i64;
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

/// Each cell's histogram of a full-picture index map.
fn histograms(mode: Mode, global: &[u8]) -> Vec<[u16; 16]> {
    let (w, _) = mode.screen();
    let (cw, ch) = mode.cell();
    let cols = mode.cells().0;
    (0..mode.cells().0 * mode.cells().1)
        .map(|i| {
            let (px, py) = ((i % cols) * cw, (i / cols) * ch);
            let mut hist = [0u16; 16];
            for dy in 0..ch {
                for dx in 0..cw {
                    hist[usize::from(global[(py + dy) * w + px + dx])] += 1;
                }
            }
            hist
        })
        .collect()
}

/// A cell's pixels assigned to the nearest of `colors` (unbiased; the first slot on a
/// tie), as png2c64's global-FS passes do.
fn assign(image: &Image, mode: Mode, cell: usize, pal: &[Lab; 16], colors: &[u8]) -> Vec<u8> {
    let (cw, ch) = mode.cell();
    let cols = mode.cells().0;
    let (px, py) = ((cell % cols) * cw, (cell / cols) * ch);
    let mut out = Vec::with_capacity(cw * ch);
    for dy in 0..ch {
        for dx in 0..cw {
            let lab = linear_to_oklab(image.at(px + dx, py + dy));
            if colors.len() == 2 {
                let d0 = dist2(lab, pal[usize::from(colors[0])]);
                let d1 = dist2(lab, pal[usize::from(colors[1])]);
                out.push(u8::from(d1 < d0));
            } else {
                let (mut bd, mut bs) = (f32::MAX, 0u8);
                for (s, &c) in colors.iter().enumerate() {
                    let d = dist2(lab, pal[usize::from(c)]);
                    if d < bd {
                        bd = d;
                        bs = s as u8;
                    }
                }
                out.push(bs);
            }
        }
    }
    out
}

/// `quantize` for the bitmap modes with the `mse` metric: brute-force cell colours (and,
/// multicolor, the background), then the global Floyd–Steinberg replacement when there
/// is no ordered threshold. `threshold` is png2c64's `ThresholdFn` (the ordered
/// threshold × strength) and `threshold_strength` the factor `precompute_cell_dist`
/// applies on top of it (png2c64 passes the dither strength again).
///
/// # Panics
/// If `image` is not the mode's screen size.
pub fn quantize(
    image: &Image,
    palette: &[Rgb; 16],
    mode: Mode,
    threshold: Option<Threshold<'_>>,
    threshold_strength: f32,
) -> Screen {
    let (w, h) = mode.screen();
    let bias: Vec<Option<f32>> = (0..w * h)
        .map(|i| threshold.map(|f| f(i % w, i / w) * threshold_strength))
        .collect();
    quantize_px(image, palette, mode, &bias, &Hooks::default())
}

/// A cell's search constraints as colour bit masks: the colours its set may use (a
/// filter: allow, deny, recipes' allowed colours, a full pin) and the colours it must
/// use (pins). [`CellRule::FREE`] is png2c64's unconstrained search.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellRule {
    /// Colours the set may use.
    pub candidates: u16,
    /// Colours the set must contain.
    pub required: u16,
}

impl CellRule {
    /// No constraint.
    pub const FREE: Self = Self {
        candidates: u16::MAX,
        required: 0,
    };
}

impl Default for CellRule {
    fn default() -> Self {
        Self::FREE
    }
}

/// What the cell search may be told beyond png2c64's own inputs. The default changes
/// nothing: the search is png2c64's.
#[derive(Clone, Debug, Default)]
pub struct Hooks {
    /// Search only this background (multicolor); `None` tries all 16, as png2c64.
    pub background: Option<u8>,
    /// Per cell, its constraints (empty: none). A free cell is searched exactly as
    /// png2c64 searches it.
    pub rules: Vec<CellRule>,
    /// Per cell, a result copied through untouched: locked cells, and the clean cells
    /// of an interactive re-solve (empty: none). A fixed cell is not searched and does
    /// not count towards the background choice.
    pub fixed: Vec<Option<Cell>>,
    /// Our named extras (all off: png2c64's search).
    pub divergences: super::Divergences,
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

/// A constrained cell's best colours for background `bg` (`None`: hi-res), `k` of them
/// at most: every required colour (up to `k`), then the best combination of the other
/// candidates, by the same error as the free search; ties to the first combination in
/// ascending order. Returns the cell's slots (padded with the background, or the first
/// colour in hi-res) and their error.
fn constrained(table: &[[f32; 16]], bg: Option<u8>, k: usize, rule: CellRule) -> (Vec<u8>, f32) {
    let bgm = bg.map_or(0u16, |b| 1 << b);
    let required: Vec<u8> = bits(rule.required & !bgm).take(k).collect();
    let free: Vec<u8> = bits(rule.candidates & !rule.required & !bgm).collect();
    let picks = (k - required.len()).min(free.len());
    let slots_of = |combo: &[usize]| {
        let mut own: Vec<u8> = required
            .iter()
            .copied()
            .chain(combo.iter().map(|&j| free[j]))
            .collect();
        own.sort_unstable();
        let pad = bg.or_else(|| own.first().copied()).unwrap_or(0);
        while own.len() < k {
            own.push(pad);
        }
        bg.into_iter().chain(own).collect::<Vec<u8>>()
    };
    let mut idx: Vec<usize> = (0..picks).collect();
    let mut best = {
        let slots = slots_of(&idx);
        let e = evaluate(table, &slots).1;
        (slots, e)
    };
    while let Some(pos) = (0..picks).rev().find(|&p| idx[p] < free.len() - picks + p) {
        idx[pos] += 1;
        for q in pos + 1..picks {
            idx[q] = idx[q - 1] + 1;
        }
        let slots = slots_of(&idx);
        let e = evaluate(table, &slots).1;
        if e < best.1 {
            best = (slots, e);
        }
    }
    best
}

/// [`quantize`] with a per-pixel threshold offset: `bias[i]` is pixel `i`'s `t`
/// (`threshold × strength × strength` for an ordered pixel), `None` for a pixel without
/// one. A cell none of whose pixels has an offset gets the global Floyd–Steinberg
/// replacement; with one offset for every pixel, or none, and default `hooks`, this is
/// png2c64's `quantize`.
///
/// # Panics
/// If `image` is not the mode's screen size or `bias` not one entry per pixel, or if a
/// cell is fixed in multicolor without `hooks.background`.
pub fn quantize_px(
    image: &Image,
    palette: &[Rgb; 16],
    mode: Mode,
    bias: &[Option<f32>],
    hooks: &Hooks,
) -> Screen {
    assert_eq!((image.w, image.h), mode.screen(), "image size for {mode:?}");
    assert_eq!(bias.len(), image.w * image.h, "one bias per pixel");
    let pal = palette.map(linear_to_oklab);
    let (cols, rows) = mode.cells();
    let cells: Vec<usize> = (0..cols * rows).collect();
    let div = &hooks.divergences;
    let extras = div.pair_bias != 0.0 || div.bridging;
    let tabled: Vec<(Vec<[f32; 16]>, Vec<f32>)> = crate::par::map(&cells, |&i| {
        if hooks.fixed(i).is_some() {
            (Vec::new(), Vec::new())
        } else {
            cell_table(image, mode, i, &pal, bias, div)
        }
    });
    let (tables, ordered): (Vec<Vec<[f32; 16]>>, Vec<Vec<f32>>) = tabled.into_iter().unzip();
    // A set's search score: its error plus the divergences' extras (none: png2c64's).
    let score = |i: usize, slots: &[u8], e: f32| {
        if extras {
            e + set_extra(mask_of(slots), &ordered[i], &pal, div)
        } else {
            e
        }
    };
    let mut screen = match mode {
        Mode::Multicolor => {
            assert!(
                hooks.background.is_some() || !hooks.fixed.iter().any(Option::is_some),
                "fixed cells need a fixed background"
            );
            // Per cell and background: its slots and error (fixed cells: none).
            let per_cell: Vec<Vec<(Vec<u8>, f32)>> = crate::par::map(&cells, |&i| {
                if hooks.fixed(i).is_some() {
                    return Vec::new();
                }
                let rule = hooks.rule(i);
                if rule == CellRule::FREE && extras {
                    // The divergences' extras: every 4-set's error plus its extra.
                    let rank = rank4();
                    let mut err4 = errors4(&tables[i]);
                    for a in 0..16usize {
                        for b in a + 1..16 {
                            for c in b + 1..16 {
                                for d in c + 1..16 {
                                    let m = (1u16 << a) | (1 << b) | (1 << c) | (1 << d);
                                    err4[usize::from(rank[usize::from(m)])] +=
                                        set_extra(m, &ordered[i], &pal, div);
                                }
                            }
                        }
                    }
                    best_triples(&err4)
                        .iter()
                        .enumerate()
                        .map(|(bg, &([a, b, c], e))| (vec![bg as u8, a, b, c], e))
                        .collect()
                } else if rule == CellRule::FREE {
                    if let Some(b) = hooks.background {
                        // One background: only its 455 sets.
                        return (0..16u8)
                            .map(|bg| {
                                if bg == b {
                                    let ([x, y, z], e) =
                                        best_triple_for(&tables[i], usize::from(b));
                                    (vec![bg, x, y, z], e)
                                } else {
                                    (Vec::new(), f32::MAX)
                                }
                            })
                            .collect();
                    }
                    best_triples(&errors4(&tables[i]))
                        .iter()
                        .enumerate()
                        .map(|(bg, &([a, b, c], e))| (vec![bg as u8, a, b, c], e))
                        .collect()
                } else {
                    (0..16u8)
                        .map(|bg| {
                            if hooks.background.is_some_and(|b| b != bg) {
                                (Vec::new(), f32::MAX)
                            } else {
                                let (slots, e) = constrained(&tables[i], Some(bg), 3, rule);
                                let s = score(i, &slots, e);
                                (slots, s)
                            }
                        })
                        .collect()
                }
            });
            let (mut best_bg, mut best_total) = (hooks.background.unwrap_or(0), f32::MAX);
            for bg in 0..16u8 {
                if hooks.background.is_some_and(|b| b != bg) {
                    continue;
                }
                let total = per_cell
                    .iter()
                    .filter(|c| !c.is_empty())
                    .fold(0.0f32, |s, c| s + c[usize::from(bg)].1);
                if total < best_total {
                    best_total = total;
                    best_bg = bg;
                }
            }
            let out = crate::par::map(&cells, |&i| {
                if let Some(c) = hooks.fixed(i) {
                    return c.clone();
                }
                let colors = per_cell[i][usize::from(best_bg)].0.clone();
                let (px, error) = evaluate(&tables[i], &colors);
                Cell { colors, px, error }
            });
            Screen {
                mode,
                bg: best_bg,
                total_error: out.iter().fold(0.0, |s, c| s + c.error),
                cells: out,
            }
        }
        Mode::Hires => {
            let out = crate::par::map(&cells, |&i| {
                if let Some(c) = hooks.fixed(i) {
                    return c.clone();
                }
                let t = &tables[i];
                let rule = hooks.rule(i);
                let colors = if rule == CellRule::FREE {
                    let errs = errors2(t);
                    let (mut best, mut pick, mut r) = (f32::MAX, (0u8, 1u8), 0);
                    for a in 0..16u8 {
                        for b in a + 1..16 {
                            let e = score(i, &[a, b], errs[r]);
                            if e < best {
                                best = e;
                                pick = (a, b);
                            }
                            r += 1;
                        }
                    }
                    vec![pick.0, pick.1]
                } else {
                    constrained(t, None, 2, rule).0
                };
                let (px, error) = evaluate(t, &colors);
                Cell { colors, px, error }
            });
            Screen {
                mode,
                bg: 0,
                total_error: out.iter().fold(0.0, |s, c| s + c.error),
                cells: out,
            }
        }
    };
    let (cw, ch) = mode.cell();
    let replace: Vec<bool> = cells
        .iter()
        .map(|&i| {
            let (px, py) = ((i % cols) * cw, (i / cols) * ch);
            hooks.fixed(i).is_none()
                && (0..ch).all(|dy| (0..cw).all(|dx| bias[(py + dy) * image.w + px + dx].is_none()))
        })
        .collect();
    if replace.contains(&true) {
        fs_replace(image, &pal, &mut screen, &replace, hooks);
    }
    screen
}

/// The global-FS palette replacement (`quantize_multicolor_bruteforce` and
/// `quantize_hires` when `tfn` is empty): each cell's colours become the most used
/// colours of [`global_fs_indices`] in it, the background excluded and fixed in slot 0
/// (multicolor), most used first, ties to the lower index. Only cells marked in `which`;
/// a constrained cell takes its required colours first and the others from its
/// candidates only.
fn fs_replace(image: &Image, pal: &[Lab; 16], screen: &mut Screen, which: &[bool], hooks: &Hooks) {
    let mode = screen.mode;
    let global = global_fs_indices(image, pal);
    let hists = histograms(mode, &global);
    let bg = screen.bg;
    let idx: Vec<usize> = (0..screen.cells.len()).collect();
    let replaced: Vec<Vec<u8>> = crate::par::map(&idx, |&i| {
        let mut hist = hists[i];
        let rule = hooks.rule(i);
        if rule != CellRule::FREE {
            return fs_constrained(hist, mode, bg, rule);
        }
        match mode {
            Mode::Multicolor => {
                hist[usize::from(bg)] = 0;
                let mut top = [bg; 3];
                for slot in &mut top {
                    let (mut best_cnt, mut best_c) = (0u16, bg);
                    for (c, &n) in hist.iter().enumerate() {
                        if n > best_cnt {
                            best_cnt = n;
                            best_c = c as u8;
                        }
                    }
                    if best_cnt == 0 {
                        break;
                    }
                    *slot = best_c;
                    hist[usize::from(best_c)] = 0;
                }
                vec![bg, top[0], top[1], top[2]]
            }
            Mode::Hires => {
                let mut top = [0u8, 1];
                for slot in &mut top {
                    let (mut best_cnt, mut best_c) = (0u16, 0u8);
                    for (c, &n) in hist.iter().enumerate() {
                        if n > best_cnt {
                            best_cnt = n;
                            best_c = c as u8;
                        }
                    }
                    *slot = best_c;
                    hist[usize::from(best_c)] = 0;
                }
                top.to_vec()
            }
        }
    });
    let pixels = crate::par::map(&idx, |&i| {
        if which[i] {
            assign(image, mode, i, pal, &replaced[i])
        } else {
            Vec::new()
        }
    });
    for (((cell, colors), px), &on) in screen.cells.iter_mut().zip(replaced).zip(pixels).zip(which)
    {
        if on {
            cell.colors = colors;
            cell.px = px;
        }
    }
}

/// [`fs_replace`]'s choice for a constrained cell: its required colours (other than the
/// background), then its most used candidates, padded like a free cell.
fn fs_constrained(mut hist: [u16; 16], mode: Mode, bg: u8, rule: CellRule) -> Vec<u8> {
    let (k, bgm) = match mode {
        Mode::Multicolor => (3, 1u16 << bg),
        Mode::Hires => (2, 0),
    };
    let mut own: Vec<u8> = bits(rule.required & !bgm).take(k).collect();
    for &c in &own {
        hist[usize::from(c)] = 0;
    }
    while own.len() < k {
        let (mut best_cnt, mut best_c) = (0u16, None);
        for c in bits(rule.candidates & !bgm) {
            if hist[usize::from(c)] > best_cnt {
                best_cnt = hist[usize::from(c)];
                best_c = Some(c);
            }
        }
        let Some(c) = best_c else { break };
        own.push(c);
        hist[usize::from(c)] = 0;
    }
    match mode {
        Mode::Multicolor => {
            while own.len() < 3 {
                own.push(bg);
            }
            std::iter::once(bg).chain(own).collect()
        }
        Mode::Hires => {
            let first = own.first().copied().unwrap_or(0);
            while own.len() < 2 {
                own.push(first);
            }
            own
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One background's triple from its 455 sets equals the 1820-set search's, bit for
    /// bit, on random tables.
    #[test]
    fn one_background_search_matches_the_full_one() {
        let mut st = 11u32;
        let mut rnd = || {
            st = st.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            (st >> 8) as f32 / 16_777_216.0
        };
        for _ in 0..20 {
            let table: Vec<[f32; 16]> = (0..32).map(|_| std::array::from_fn(|_| rnd())).collect();
            let all = best_triples(&errors4(&table));
            for (bg, &(want, we)) in all.iter().enumerate() {
                let (pick, e) = best_triple_for(&table, bg);
                assert_eq!((pick, e.to_bits()), (want, we.to_bits()), "bg {bg}");
            }
        }
    }
}
