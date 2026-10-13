//! Mask modifiers on the super grid (ADR-008), applied to a layer's resampled mask in
//! the fixed order Invert → Levels → SnapToCells → Feather.

use super::{Modifiers, SnapToCells};
use crate::geom::{PxRect, SuperGrid};
use crate::scratch;

/// Feather σ (target px) from which the blur runs at half resolution (#76, #98).
pub const HALF_RES_FEATHER: f32 = 4.0;

/// Whether a feather of σ `feather` target px is blurred at half resolution.
pub(crate) fn half_res(feather: f32) -> bool {
    feather >= HALF_RES_FEATHER
}

/// σ in super pixels below which an exact Gaussian kernel is used; above it, three box
/// blurs (the W3C `feGaussianBlur` approximation, O(1) per pixel for any σ).
const EXACT_SIGMA: f32 = 4.0;

/// Applies `m` in place to `values`, a mask covering super rectangle `rect` of `grid`.
#[cfg(test)]
pub(crate) fn apply_modifiers(values: &mut [f32], rect: PxRect, grid: SuperGrid, m: &Modifiers) {
    apply_modifiers_with(values, rect, grid, m, false);
}

/// [`apply_modifiers`]; with `full_res`, every feather is blurred at full resolution
/// (the reference the half-resolution blur is measured against).
pub(crate) fn apply_modifiers_with(
    values: &mut [f32],
    rect: PxRect,
    grid: SuperGrid,
    m: &Modifiers,
    full_res: bool,
) {
    point_ops(values, m);
    spatial_ops(values, rect, grid, m, full_res);
}

/// Snap then feather: the modifiers that need neighbours, after [`point_ops`].
pub(crate) fn spatial_ops(
    values: &mut [f32],
    rect: PxRect,
    grid: SuperGrid,
    m: &Modifiers,
    full_res: bool,
) {
    if m.snap() != SnapToCells::Off {
        snap(values, rect, grid, m.snap());
    }
    if m.feather() > 0.0 {
        let sigma = m.feather() * f32::from(grid.n());
        let (w, h) = (rect.width() as usize, rect.height() as usize);
        if half_res(m.feather()) && !full_res {
            blur_half_res(values, w, h, sigma);
        } else {
            blur(values, w, h, sigma);
        }
    }
}

/// Invert then levels, value by value: the modifiers that need no neighbours.
pub(crate) fn point_ops(values: &mut [f32], m: &Modifiers) {
    let levels = m.levels();
    let identity_levels = levels == crate::layer::Levels::IDENTITY;
    if m.invert() || !identity_levels {
        for v in values.iter_mut() {
            if m.invert() {
                *v = 1.0 - *v;
            }
            if !identity_levels {
                *v = levels.eval(*v);
            }
        }
    }
}

/// When `m` ends in a feather blurred at half resolution with no snap before it, the
/// feather's σ in super pixels: then [`point_ops`], [`down_row`] and [`blur_and_up`]
/// apply `m`, and the full-resolution mask is only ever read two rows at a time (#139).
pub(crate) fn half_res_sigma(m: &Modifiers, grid: SuperGrid, full_res: bool) -> Option<f32> {
    let half = m.snap() == SnapToCells::Off && m.feather() > 0.0 && half_res(m.feather());
    (half && !full_res).then(|| m.feather() * f32::from(grid.n()))
}

/// Makes every target cell uniform: Expand takes the cell's maximum, Contract its
/// minimum, Nearest the maximum when the mean is at least ½ and the minimum otherwise.
/// Only the part of a cell inside `rect` (the layer's footprint) is considered.
fn snap(values: &mut [f32], rect: PxRect, grid: SuperGrid, mode: SnapToCells) {
    let (cw, ch) = grid.cell_size_super();
    let w = rect.width() as usize;
    let mut cy = rect.y0 / ch * ch;
    while cy < rect.y1 {
        let mut cx = rect.x0 / cw * cw;
        while cx < rect.x1 {
            let cell = PxRect::new(cx, cy, cx + cw, cy + ch).intersect(rect);
            let idx = |x: u32, y: u32| (y - rect.y0) as usize * w + (x - rect.x0) as usize;
            let (mut lo, mut hi, mut sum) = (f32::INFINITY, f32::NEG_INFINITY, 0.0f32);
            for y in cell.y0..cell.y1 {
                for x in cell.x0..cell.x1 {
                    let v = values[idx(x, y)];
                    lo = lo.min(v);
                    hi = hi.max(v);
                    sum += v;
                }
            }
            let mean = sum / (cell.width() * cell.height()) as f32;
            let v = match mode {
                SnapToCells::Off => unreachable!("checked by the caller"),
                SnapToCells::Expand => hi,
                SnapToCells::Contract => lo,
                SnapToCells::Nearest => {
                    if mean >= 0.5 {
                        hi
                    } else {
                        lo
                    }
                }
            };
            for y in cell.y0..cell.y1 {
                for x in cell.x0..cell.x1 {
                    values[idx(x, y)] = v;
                }
            }
            cx += cw;
        }
        cy += ch;
    }
}

/// Gaussian blur with σ in pixels, edges clamped.
pub(crate) fn blur(values: &mut [f32], w: usize, h: usize, sigma: f32) {
    if w == 0 || h == 0 || sigma <= 0.0 {
        return;
    }
    if sigma < EXACT_SIGMA {
        let kernel = gaussian_kernel(sigma);
        convolve_rows(values, w, h, &kernel);
        convolve_cols(values, w, h, &kernel);
    } else {
        for r in box_radii(sigma) {
            box_rows(values, w, h, r);
            box_cols(values, w, h, r);
        }
    }
}

/// Where full-resolution index `i` samples a half-resolution axis of `n` values: the two
/// neighbours and the weight of the second. Pixel centre `i + ½` is half-resolution
/// coordinate `(i + ½) / 2`, centre-aligned.
fn half_tap(i: usize, n: usize) -> (usize, usize, f32) {
    let f = ((i as f32 + 0.5) / 2.0 - 0.5).clamp(0.0, (n - 1) as f32);
    let i0 = f.floor() as usize;
    (i0, (i0 + 1).min(n - 1), f - i0 as f32)
}

/// Blur at half resolution: 2 × 2 box down, blur with σ/2, bilinear back up.
fn blur_half_res(values: &mut [f32], w: usize, h: usize, sigma: f32) {
    let (hw, hh) = (w.div_ceil(2), h.div_ceil(2));
    scratch::with(hw * hh, |half| blur_half_res_in(values, w, h, sigma, half));
}

/// [`blur_half_res`] with `half` (`⌈w/2⌉ × ⌈h/2⌉`) as scratch.
fn blur_half_res_in(values: &mut [f32], w: usize, h: usize, sigma: f32, half: &mut [f32]) {
    let hw = w.div_ceil(2);
    {
        let src: &[f32] = values;
        let mut rows: Vec<(usize, &mut [f32])> = half.chunks_mut(hw).enumerate().collect();
        crate::par::for_each(&mut rows, |(y, out)| {
            let r0 = &src[2 * *y * w..][..w];
            let r1 = (2 * *y + 1 < h).then(|| &src[(2 * *y + 1) * w..][..w]);
            down_row(r0, r1, out);
        });
    }
    blur_and_up(values, w, h, sigma, half);
}

/// One half-resolution row: the mean of each 2 × 2 block of full-resolution rows `r0`
/// and `r1` (`r0` alone at an odd bottom edge), summed in the order (0, 0), (1, 0),
/// (0, 1), (1, 1).
pub(crate) fn down_row(r0: &[f32], r1: Option<&[f32]>, out: &mut [f32]) {
    let w = r0.len();
    // Full 2 × 2 blocks first, branch-free so it vectorises.
    let full = if r1.is_some() { w / 2 } else { 0 };
    if let Some(r1) = r1 {
        let pairs = r0.as_chunks::<2>().0.iter().zip(r1.as_chunks::<2>().0);
        for (o, (a, b)) in out[..full].iter_mut().zip(pairs) {
            let mut sum = 0.0f32 + a[0];
            sum += a[1];
            sum += b[0];
            sum += b[1];
            *o = sum / 4.0;
        }
    }
    for (x, o) in out.iter_mut().enumerate().skip(full) {
        let sx = 2 * x;
        let right = sx + 1 < w;
        let mut sum = 0.0f32 + r0[sx];
        let mut n = 1.0f32;
        if right {
            sum += r0[sx + 1];
            n += 1.0;
        }
        if let Some(r1) = r1 {
            sum += r1[sx];
            n += 1.0;
            if right {
                sum += r1[sx + 1];
                n += 1.0;
            }
        }
        *o = sum / n;
    }
}

/// The rest of the half-resolution feather once `half` (`⌈w/2⌉ × ⌈h/2⌉`) holds the
/// mask's 2 × 2 means: blur it with σ/2, then interpolate it bilinearly back up into
/// `values` (`w × h`), every value of which it writes.
pub(crate) fn blur_and_up(values: &mut [f32], w: usize, h: usize, sigma: f32, half: &mut [f32]) {
    let (hw, hh) = (w.div_ceil(2), h.div_ceil(2));
    blur(half, hw, hh, sigma / 2.0);
    let half: &[f32] = half;
    let cols: Vec<(usize, usize, f32, f32)> = (0..w)
        .map(|x| {
            let (x0, x1, tx) = half_tap(x, hw);
            (x0, x1, 1.0 - tx, tx)
        })
        .collect();
    // Interior columns come in pairs 2i, 2i + 1 (1 ≤ i ≤ hw − 2) that read half-res
    // pixels i − 1, i and i, i + 1 with weights ¼ ¾ and ¾ ¼: the taps above, exactly,
    // without the gathers.
    let pairs = 1..hw.saturating_sub(1);
    debug_assert!(pairs.clone().all(|i| {
        cols[2 * i] == (i - 1, i, 0.25, 0.75) && cols[2 * i + 1] == (i, i + 1, 0.75, 0.25)
    }));
    // A row of horizontally interpolated half-res row `j`.
    let interpolate = |j: usize, out: &mut [f32]| {
        let a = &half[j * hw..][..hw];
        // The columns outside the pairs: a few at each edge (#139: visit only those).
        let edges = if pairs.is_empty() {
            [0..w, 0..0]
        } else {
            [0..2 * pairs.start, 2 * pairs.end..w]
        };
        for x in edges.into_iter().flatten() {
            let (x0, x1, sx, tx) = cols[x];
            out[x] = a[x0] * sx + a[x1] * tx;
        }
        if !pairs.is_empty() {
            let inner = &mut out[2 * pairs.start..2 * pairs.end];
            let (l, c, r) = (
                &a[pairs.start - 1..],
                &a[pairs.start..],
                &a[pairs.start + 1..],
            );
            for (k, o) in inner.as_chunks_mut::<2>().0.iter_mut().enumerate() {
                o[0] = l[k] * 0.25 + c[k] * 0.75;
                o[1] = c[k] * 0.75 + r[k] * 0.25;
            }
        }
    };
    // Rows by chunk: each interpolates the half-res rows it reads once, then mixes
    // them, `top × (1 − ty) + bottom × ty` as before.
    const CHUNK: usize = 16;
    let mut chunks: Vec<(usize, &mut [f32])> = values.chunks_mut(w * CHUNK).enumerate().collect();
    crate::par::for_each(&mut chunks, |(c, rows)| {
        let y0 = *c * CHUNK;
        let n = rows.len() / w;
        let (j0, _, _) = half_tap(y0, hh);
        let (_, j1, _) = half_tap(y0 + n - 1, hh);
        scratch::with((j1 - j0 + 1) * w, |lines| {
            for (j, line) in lines.chunks_mut(w).enumerate() {
                interpolate(j0 + j, line);
            }
            for (i, row) in rows.chunks_mut(w).enumerate() {
                let (ya, yb, ty) = half_tap(y0 + i, hh);
                let (top, bot) = (&lines[(ya - j0) * w..][..w], &lines[(yb - j0) * w..][..w]);
                for ((v, t), b) in row.iter_mut().zip(top).zip(bot) {
                    *v = t * (1.0 - ty) + b * ty;
                }
            }
        });
    });
}

fn gaussian_kernel(sigma: f32) -> Vec<f32> {
    let r = (3.0 * sigma).ceil() as i32;
    let mut k: Vec<f32> = (-r..=r)
        .map(|i| (-(i * i) as f32 / (2.0 * sigma * sigma)).exp())
        .collect();
    let sum: f32 = k.iter().sum();
    for v in &mut k {
        *v /= sum;
    }
    k
}

/// Radii of three box blurs whose cascade approximates a Gaussian of `sigma`
/// (Kutskir's "boxes for Gauss").
fn box_radii(sigma: f32) -> [usize; 3] {
    let n = 3.0f32;
    let w_ideal = (12.0 * sigma * sigma / n + 1.0).sqrt();
    let mut wl = w_ideal.floor() as i32;
    if wl % 2 == 0 {
        wl -= 1;
    }
    let wu = wl + 2;
    let wl_f = wl as f32;
    let m_ideal =
        (12.0 * sigma * sigma - n * wl_f * wl_f - 4.0 * n * wl_f - 3.0 * n) / (-4.0 * wl_f - 4.0);
    let m = m_ideal.round() as i32;
    std::array::from_fn(|i| {
        let size = if (i as i32) < m { wl } else { wu };
        (size.max(1) as usize - 1) / 2
    })
}

#[inline]
fn clamp_idx(i: isize, n: usize) -> usize {
    i.clamp(0, n as isize - 1) as usize
}

fn convolve_rows(values: &mut [f32], w: usize, _h: usize, k: &[f32]) {
    let r = (k.len() / 2) as isize;
    let mut rows: Vec<&mut [f32]> = values.chunks_mut(w).collect();
    crate::par::for_each(&mut rows, |row| {
        scratch::with(w, |src| {
            src.copy_from_slice(row);
            for (x, out) in row.iter_mut().enumerate() {
                let mut acc = 0.0;
                for (j, kw) in k.iter().enumerate() {
                    acc += kw * src[clamp_idx(x as isize + j as isize - r, w)];
                }
                *out = acc;
            }
        });
    });
}

fn convolve_cols(values: &mut [f32], w: usize, h: usize, k: &[f32]) {
    let r = (k.len() / 2) as isize;
    scratch::with(values.len(), |src| {
        src.copy_from_slice(values);
        let src: &[f32] = src;
        let mut rows: Vec<(usize, &mut [f32])> = values.chunks_mut(w).enumerate().collect();
        crate::par::for_each(&mut rows, |(y, row)| {
            row.fill(0.0);
            for (j, kw) in k.iter().enumerate() {
                let sy = clamp_idx(*y as isize + j as isize - r, h);
                for (o, s) in row.iter_mut().zip(&src[sy * w..(sy + 1) * w]) {
                    *o += kw * s;
                }
            }
        });
    });
}

/// Box-filters `lanes` interleaved lines of `n` values each with radius `r`, edges
/// clamped: value `k` of lane `j` is `src[k * stride + j]`, and the result goes to
/// `out[k * lanes + j]`.
///
/// Each line uses an f64 prefix sum, so a window of zeros sums to exactly zero (a running
/// sum would leave rounding residue that spreads along the whole line): `prefix[k]` is
/// the sum of the clamped line at indices `-r .. k - r - 1`, accumulated in order, and
/// output `i` is `(prefix[i + 2r + 1] − prefix[i]) / (2r + 1)`. The lanes are
/// independent, so the inner loops vectorise across them, and only the last `2r + 1`
/// prefix rows are kept.
fn box_lanes(src: &[f32], stride: usize, lanes: usize, n: usize, r: usize, out: &mut [f32]) {
    let span = 2 * r + 1;
    // Slot `p mod span` holds prefix[p] until step p + span reads it as prefix[i] and
    // overwrites it; prefix[0] is the zero slot 0 starts with.
    scratch::with(span * lanes + lanes, |ring| {
        ring.fill(0.0);
        let (ring, acc) = ring.split_at_mut(span * lanes);
        match lanes {
            ROW_GROUP => box_lanes_fixed::<ROW_GROUP>(src, stride, n, r, out, ring),
            COL_STRIP => box_lanes_fixed::<COL_STRIP>(src, stride, n, r, out, ring),
            _ => box_lanes_in(src, stride, lanes, n, r, out, ring, acc),
        }
    });
}

/// [`box_lanes_in`] for `L` lanes, a constant, so the running sums stay in registers and
/// the loops are unrolled (#139). The same operations in the same order.
fn box_lanes_fixed<const L: usize>(
    src: &[f32],
    stride: usize,
    n: usize,
    r: usize,
    out: &mut [f32],
    ring: &mut [f64],
) {
    let inv = 1.0 / (2 * r + 1) as f64;
    let span = 2 * r + 1;
    let mut acc = [0.0f64; L];
    let mut slot = 0;
    for k in 0..n + 2 * r {
        let line = src[clamp_idx(k as isize - r as isize, n) * stride..]
            .first_chunk::<L>()
            .expect("a whole line");
        let p = k + 1;
        slot = if slot + 1 == span { 0 } else { slot + 1 };
        let prefix = ring[slot * L..]
            .first_chunk_mut::<L>()
            .expect("a whole ring slot");
        if p >= span {
            let out = out[(p - span) * L..]
                .first_chunk_mut::<L>()
                .expect("a whole line");
            for (((a, old), &v), o) in acc.iter_mut().zip(prefix).zip(line).zip(out) {
                *a += f64::from(v);
                *o = ((*a - *old) * inv) as f32;
                *old = *a;
            }
        } else {
            for ((a, old), &v) in acc.iter_mut().zip(prefix).zip(line) {
                *a += f64::from(v);
                *old = *a;
            }
        }
    }
}

/// [`box_lanes`] with zeroed scratch: `ring` (`(2r + 1) × lanes`) and `acc` (`lanes`).
#[expect(
    clippy::too_many_arguments,
    reason = "the lines' layout and two scratch buffers"
)]
fn box_lanes_in(
    src: &[f32],
    stride: usize,
    lanes: usize,
    n: usize,
    r: usize,
    out: &mut [f32],
    ring: &mut [f64],
    acc: &mut [f64],
) {
    let inv = 1.0 / (2 * r + 1) as f64;
    let span = 2 * r + 1;
    let mut slot = 0;
    for k in 0..n + 2 * r {
        let line = &src[clamp_idx(k as isize - r as isize, n) * stride..][..lanes];
        let p = k + 1;
        slot = if slot + 1 == span { 0 } else { slot + 1 };
        let prefix = &mut ring[slot * lanes..][..lanes];
        if p >= span {
            let out = &mut out[(p - span) * lanes..][..lanes];
            for (((a, old), &v), o) in acc.iter_mut().zip(prefix).zip(line).zip(out) {
                *a += f64::from(v);
                *o = ((*a - *old) * inv) as f32;
                *old = *a;
            }
        } else {
            for ((a, old), &v) in acc.iter_mut().zip(prefix).zip(line) {
                *a += f64::from(v);
                *old = *a;
            }
        }
    }
}

/// Rows box-filtered per group of this many, transposed so the lines are lanes.
const ROW_GROUP: usize = 16;
/// Columns box-filtered per strip.
const COL_STRIP: usize = 32;

fn box_rows(values: &mut [f32], w: usize, _h: usize, r: usize) {
    if r == 0 {
        return;
    }
    let mut groups: Vec<&mut [f32]> = values.chunks_mut(w * ROW_GROUP).collect();
    crate::par::for_each(&mut groups, |rows| {
        let g = rows.len() / w;
        // A whole group is transposed in tiles (#139); the last one may be shorter.
        if g == ROW_GROUP {
            scratch::with(2 * w * g, |t| {
                let (t, out) = t.split_at_mut(w * g);
                transpose_in::<ROW_GROUP>(rows, w, t);
                box_lanes(t, g, g, w, r, out);
                transpose_out::<ROW_GROUP>(out, w, rows);
            });
            return;
        }
        scratch::with(2 * w * g, |t| {
            let (t, out) = t.split_at_mut(w * g);
            for (x, lanes) in t.chunks_exact_mut(g).enumerate() {
                for (j, v) in lanes.iter_mut().enumerate() {
                    *v = rows[j * w + x];
                }
            }
            box_lanes(t, g, g, w, r, out);
            for (x, lanes) in out.chunks_exact(g).enumerate() {
                for (j, &v) in lanes.iter().enumerate() {
                    rows[j * w + x] = v;
                }
            }
        });
    });
}

/// `t[x × L + j] = rows[j × w + x]` for `L` rows of `w`, in 4 × 4 tiles, which the
/// compiler turns into vector shuffles (#139).
fn transpose_in<const L: usize>(rows: &[f32], w: usize, t: &mut [f32]) {
    const { assert!(L.is_multiple_of(4), "whole tiles of rows") };
    let x4 = w / 4 * 4;
    for jb in (0..L).step_by(4) {
        let src: [&[f32]; 4] = std::array::from_fn(|i| &rows[(jb + i) * w..][..w]);
        for x in (0..x4).step_by(4) {
            let tile: [[f32; 4]; 4] =
                std::array::from_fn(|i| *src[i][x..].first_chunk::<4>().expect("4 values"));
            for c in 0..4 {
                let dst = t[(x + c) * L + jb..]
                    .first_chunk_mut::<4>()
                    .expect("4 lanes");
                *dst = [tile[0][c], tile[1][c], tile[2][c], tile[3][c]];
            }
        }
        for x in x4..w {
            for (i, s) in src.iter().enumerate() {
                t[x * L + jb + i] = s[x];
            }
        }
    }
}

/// The inverse of [`transpose_in`]: `rows[j × w + x] = t[x × L + j]`.
fn transpose_out<const L: usize>(t: &[f32], w: usize, rows: &mut [f32]) {
    const { assert!(L.is_multiple_of(4), "whole tiles of rows") };
    let x4 = w / 4 * 4;
    for (jb, block) in rows
        .chunks_exact_mut(4 * w)
        .enumerate()
        .map(|(b, r)| (4 * b, r))
    {
        let (r0, rest) = block.split_at_mut(w);
        let (r1, rest) = rest.split_at_mut(w);
        let (r2, r3) = rest.split_at_mut(w);
        let mut dst = [r0, r1, r2, r3];
        for x in (0..x4).step_by(4) {
            let tile: [[f32; 4]; 4] = std::array::from_fn(|c| {
                *t[(x + c) * L + jb..].first_chunk::<4>().expect("4 lanes")
            });
            for (i, d) in dst.iter_mut().enumerate() {
                *d[x..].first_chunk_mut::<4>().expect("4 values") =
                    [tile[0][i], tile[1][i], tile[2][i], tile[3][i]];
            }
        }
        for x in x4..w {
            for (i, d) in dst.iter_mut().enumerate() {
                d[x] = t[x * L + jb + i];
            }
        }
    }
}

fn box_cols(values: &mut [f32], w: usize, h: usize, r: usize) {
    if r == 0 {
        return;
    }
    // The strips' results, one after another: strip `s` (columns from `s × COL_STRIP`)
    // is `lanes × h` values from `s × COL_STRIP × h`.
    scratch::with(w * h, |results| {
        {
            let src: &[f32] = values;
            let mut strips: Vec<(usize, &mut [f32])> =
                results.chunks_mut(COL_STRIP * h).enumerate().collect();
            crate::par::for_each(&mut strips, |(s, out)| {
                let x0 = *s * COL_STRIP;
                let lanes = COL_STRIP.min(w - x0);
                box_lanes(&src[x0..], w, lanes, h, r, out);
            });
        }
        let results: &[f32] = results;
        let mut rows: Vec<(usize, &mut [f32])> = values.chunks_mut(w).enumerate().collect();
        crate::par::for_each(&mut rows, |(y, row)| {
            for (strip, x0) in results.chunks(COL_STRIP * h).zip((0..w).step_by(COL_STRIP)) {
                let lanes = COL_STRIP.min(w - x0);
                row[x0..x0 + lanes].copy_from_slice(&strip[*y * lanes..][..lanes]);
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Variance of the blur's impulse response, measured.
    fn impulse_sigma(sigma: f32) -> f32 {
        let n = 401;
        let mut v = vec![0.0; n];
        v[200] = 1.0;
        blur(&mut v, n, 1, sigma);
        let sum: f32 = v.iter().sum();
        let var: f32 = v
            .iter()
            .enumerate()
            .map(|(i, x)| x * (i as f32 - 200.0).powi(2))
            .sum::<f32>()
            / sum;
        var.sqrt()
    }

    #[test]
    fn blur_has_the_requested_sigma_and_preserves_mass() {
        for sigma in [0.5, 1.0, 1.9, 2.0, 4.0, 16.0, 32.0] {
            let got = impulse_sigma(sigma);
            // A sampled Gaussian kernel at σ < 1 is slightly narrower than the continuous one.
            assert!(
                (got - sigma).abs() < (0.06 * sigma).max(0.05),
                "σ {sigma}: measured {got}"
            );
        }
        let mut flat = vec![0.3; 50 * 40];
        blur(&mut flat, 50, 40, 7.0);
        assert!(flat.iter().all(|v| (v - 0.3).abs() < 1e-5));
    }

    /// The blur as first written (#76): one line at a time, gathered columns. The
    /// restructured passes must match it bit for bit.
    mod reference {
        use super::super::{EXACT_SIGMA, box_radii};
        use super::super::{clamp_idx, convolve_cols, convolve_rows, gaussian_kernel};

        fn box_line(line: &[f32], r: usize, out: &mut [f32]) {
            let n = line.len();
            let inv = 1.0 / (2 * r + 1) as f64;
            let mut prefix = vec![0.0f64];
            let mut acc = 0.0f64;
            for k in 0..n + 2 * r {
                acc += f64::from(line[clamp_idx(k as isize - r as isize, n)]);
                prefix.push(acc);
            }
            for (i, o) in out.iter_mut().enumerate() {
                *o = ((prefix[i + 2 * r + 1] - prefix[i]) * inv) as f32;
            }
        }

        pub fn blur(values: &mut [f32], w: usize, h: usize, sigma: f32) {
            if sigma < EXACT_SIGMA {
                let kernel = gaussian_kernel(sigma);
                convolve_rows(values, w, h, &kernel);
                convolve_cols(values, w, h, &kernel);
                return;
            }
            for r in box_radii(sigma) {
                if r == 0 {
                    continue;
                }
                for row in values.chunks_mut(w) {
                    let src = row.to_vec();
                    box_line(&src, r, row);
                }
                for x in 0..w {
                    let line: Vec<f32> = (0..h).map(|y| values[y * w + x]).collect();
                    let mut out = vec![0.0f32; h];
                    box_line(&line, r, &mut out);
                    for (y, v) in out.into_iter().enumerate() {
                        values[y * w + x] = v;
                    }
                }
            }
        }

        pub fn blur_half_res(values: &mut [f32], w: usize, h: usize, sigma: f32) {
            let (hw, hh) = (w.div_ceil(2), h.div_ceil(2));
            let mut half = vec![0.0f32; hw * hh];
            for y in 0..hh {
                for x in 0..hw {
                    let mut sum = 0.0;
                    let mut n = 0.0;
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let (sx, sy) = (2 * x + dx, 2 * y + dy);
                        if sx < w && sy < h {
                            sum += values[sy * w + sx];
                            n += 1.0;
                        }
                    }
                    half[y * hw + x] = sum / n;
                }
            }
            blur(&mut half, hw, hh, sigma / 2.0);
            for y in 0..h {
                let fy = ((y as f32 + 0.5) / 2.0 - 0.5).clamp(0.0, (hh - 1) as f32);
                let y0 = fy.floor() as usize;
                let y1 = (y0 + 1).min(hh - 1);
                let ty = fy - y0 as f32;
                for x in 0..w {
                    let fx = ((x as f32 + 0.5) / 2.0 - 0.5).clamp(0.0, (hw - 1) as f32);
                    let x0 = fx.floor() as usize;
                    let x1 = (x0 + 1).min(hw - 1);
                    let tx = fx - x0 as f32;
                    let top = half[y0 * hw + x0] * (1.0 - tx) + half[y0 * hw + x1] * tx;
                    let bot = half[y1 * hw + x0] * (1.0 - tx) + half[y1 * hw + x1] * tx;
                    values[y * w + x] = top * (1.0 - ty) + bot * ty;
                }
            }
        }
    }

    /// A deterministic field with hard edges, zero runs and noise.
    fn field(w: usize, h: usize) -> Vec<f32> {
        let mut s = 0x2545_f491_u32;
        (0..w * h)
            .map(|i| {
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                let (x, y) = (i % w, i / w);
                if x < w / 3 {
                    0.0
                } else if (x + y) % 23 < 9 {
                    1.0
                } else {
                    (s >> 8) as f32 / (1u32 << 24) as f32
                }
            })
            .collect()
    }

    #[test]
    fn restructured_blur_matches_the_reference_bit_for_bit() {
        for (w, h) in [(1, 1), (7, 3), (33, 21), (101, 67), (640, 37)] {
            for sigma in [0.7, 3.9, 4.0, 9.5, 32.0, 128.0] {
                let mut a = field(w, h);
                let mut b = a.clone();
                blur(&mut a, w, h, sigma);
                reference::blur(&mut b, w, h, sigma);
                assert!(
                    a.iter().zip(&b).all(|(x, y)| x.to_bits() == y.to_bits()),
                    "blur {w}×{h} σ {sigma}"
                );
                let mut a = field(w, h);
                let mut b = a.clone();
                blur_half_res(&mut a, w, h, sigma);
                reference::blur_half_res(&mut b, w, h, sigma);
                assert!(
                    a.iter().zip(&b).all(|(x, y)| x.to_bits() == y.to_bits()),
                    "half-res blur {w}×{h} σ {sigma}"
                );
            }
        }
    }

    #[test]
    fn half_res_blur_of_a_flat_field_is_flat() {
        let mut v = vec![0.7; 33 * 21];
        blur_half_res(&mut v, 33, 21, 40.0);
        assert!(v.iter().all(|x| (x - 0.7).abs() < 1e-5));
    }

    /// Largest difference between the half- and full-resolution blur of `mask`.
    fn half_res_error(mask: &[f32], w: usize, h: usize, sigma: f32) -> f32 {
        let mut full = mask.to_vec();
        blur(&mut full, w, h, sigma);
        let mut half = mask.to_vec();
        blur_half_res(&mut half, w, h, sigma);
        full.iter()
            .zip(&half)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0, f32::max)
    }

    /// #98: the half-resolution blur, used from σ = 4 target px, against the
    /// full-resolution blur on hard edges (on and off the 2 × 2 grid) and a small square,
    /// at every supersample factor and up to the largest feather. The error is in mask
    /// units (0–1); it is largest at σ = 4 target px on a 1× grid (1.4 %, where the
    /// half-resolution blur is an exact Gaussian of σ = 2 against three box blurs), and
    /// below one 8-bit step (0.37 % at most) at 4× and 8×.
    #[test]
    fn half_res_blur_error_on_hard_edges() {
        let mut worst = 0.0f32;
        for n in [1u8, 2, 4, 8] {
            for feather in [HALF_RES_FEATHER, 6.0, 8.0, 16.0, 32.0] {
                let sigma = feather * f32::from(n);
                let (w, h) = (8 * sigma as usize + 64, 8 * sigma as usize + 64);
                let mut err = 0.0f32;
                for offset in [0, 1] {
                    let edge: Vec<f32> = (0..w * h)
                        .map(|i| f32::from(u8::from(i % w < w / 2 + offset)))
                        .collect();
                    err = err.max(half_res_error(&edge, w, h, sigma));
                }
                let side = f32::from(n) * 3.0;
                let square: Vec<f32> = (0..w * h)
                    .map(|i| {
                        let (x, y) = ((i % w) as f32, (i / w) as f32);
                        let inside =
                            (x - w as f32 / 2.0).abs() < side && (y - h as f32 / 2.0).abs() < side;
                        f32::from(u8::from(inside))
                    })
                    .collect();
                err = err.max(half_res_error(&square, w, h, sigma));
                println!("half-res feather error: {n}x, σ {feather} target px: {err:.5}");
                // Below one 8-bit step at the default 4× and above.
                if n >= 4 {
                    assert!(err < 1.0 / 255.0, "{n}x σ {feather}: {err}");
                }
                worst = worst.max(err);
            }
        }
        println!("half-res feather error: worst {worst:.5}");
        assert!(worst < 0.015, "half-res feather error {worst}");
    }
}
