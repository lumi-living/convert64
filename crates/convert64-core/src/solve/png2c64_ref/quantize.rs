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

/// One cell's distance table: per pixel, the squared distance to each of the 16 colours
/// (`precompute_cell_dist`, pixel-major). `bias[i]` is pixel `i`'s threshold offset `t`
/// (png2c64's `threshold_fn(x, y) × threshold_strength`), or `None` for no bias.
fn cell_table(
    image: &Image,
    mode: Mode,
    cell: usize,
    pal: &[Lab; 16],
    bias: &[Option<f32>],
) -> Vec<[f32; 16]> {
    let (cw, ch) = mode.cell();
    let cols = mode.cells().0;
    let (px, py) = ((cell % cols) * cw, (cell / cols) * ch);
    let mut out = Vec::with_capacity(cw * ch);
    for dy in 0..ch {
        for dx in 0..cw {
            let (x, y) = (px + dx, py + dy);
            let mut lab = linear_to_oklab(image.at(x, y));
            if let Some(t) = bias[y * image.w + x] {
                lab[0] += t * 0.15;
                lab[1] += t * 0.03;
                lab[2] += t * 0.03;
            }
            out.push(pal.map(|c| dist2(lab, c)));
        }
    }
    out
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

/// What the cell search may be told beyond png2c64's own inputs. The default changes
/// nothing: the search is png2c64's.
#[derive(Clone, Debug, Default)]
pub struct Hooks {
    /// Search only this background (multicolor); `None` tries all 16, as png2c64.
    pub background: Option<u8>,
}

/// [`quantize`] with a per-pixel threshold offset: `bias[i]` is pixel `i`'s `t`
/// (`threshold × strength × strength` for an ordered pixel), `None` for a pixel without
/// one. A cell none of whose pixels has an offset gets the global Floyd–Steinberg
/// replacement; with one offset for every pixel, or none, this is png2c64's `quantize`.
///
/// # Panics
/// If `image` is not the mode's screen size or `bias` not one entry per pixel.
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
    let tables: Vec<Vec<[f32; 16]>> =
        crate::par::map(&cells, |&i| cell_table(image, mode, i, &pal, bias));
    let mut screen = match mode {
        Mode::Multicolor => {
            let per_cell: Vec<[([u8; 3], f32); 16]> =
                crate::par::map(&tables, |t| best_triples(&errors4(t)));
            let (mut best_bg, mut best_total) = (0u8, f32::MAX);
            for bg in 0..16u8 {
                if hooks.background.is_some_and(|b| b != bg) {
                    continue;
                }
                let total = per_cell
                    .iter()
                    .fold(0.0f32, |s, c| s + c[usize::from(bg)].1);
                if total < best_total {
                    best_total = total;
                    best_bg = bg;
                }
            }
            let out = crate::par::map(&cells, |&i| {
                let [a, b, c] = per_cell[i][usize::from(best_bg)].0;
                let colors = vec![best_bg, a, b, c];
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
            let out = crate::par::map(&tables, |t| {
                let errs = errors2(t);
                let (mut best, mut pick, mut r) = (f32::MAX, (0u8, 1u8), 0);
                for i in 0..16u8 {
                    for j in i + 1..16 {
                        if errs[r] < best {
                            best = errs[r];
                            pick = (i, j);
                        }
                        r += 1;
                    }
                }
                let colors = vec![pick.0, pick.1];
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
    let unbiased: Vec<bool> = cells
        .iter()
        .map(|&i| {
            let (px, py) = ((i % cols) * cw, (i / cols) * ch);
            (0..ch).all(|dy| (0..cw).all(|dx| bias[(py + dy) * image.w + px + dx].is_none()))
        })
        .collect();
    if unbiased.contains(&true) {
        fs_replace(image, &pal, &mut screen, &unbiased);
    }
    screen
}

/// The global-FS palette replacement (`quantize_multicolor_bruteforce` and
/// `quantize_hires` when `tfn` is empty): each cell's colours become the most used
/// colours of [`global_fs_indices`] in it, the background excluded and fixed in slot 0
/// (multicolor), most used first, ties to the lower index. Only cells marked in `which`.
fn fs_replace(image: &Image, pal: &[Lab; 16], screen: &mut Screen, which: &[bool]) {
    let mode = screen.mode;
    let global = global_fs_indices(image, pal);
    let hists = histograms(mode, &global);
    let bg = screen.bg;
    let replaced: Vec<Vec<u8>> = crate::par::map(&hists, |&hist| {
        let mut hist = hist;
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
    let idx: Vec<usize> = (0..screen.cells.len()).collect();
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
