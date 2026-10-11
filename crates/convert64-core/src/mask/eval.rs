//! Mask modifiers on the super grid (ADR-008), applied to a layer's resampled mask in
//! the fixed order Invert → Levels → SnapToCells → Feather.

use super::{Modifiers, SnapToCells};
use crate::geom::{PxRect, SuperGrid};

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
pub(crate) fn apply_modifiers(values: &mut [f32], rect: PxRect, grid: SuperGrid, m: &Modifiers) {
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
    if m.snap() != SnapToCells::Off {
        snap(values, rect, grid, m.snap());
    }
    if m.feather() > 0.0 {
        let sigma = m.feather() * f32::from(grid.n());
        let (w, h) = (rect.width() as usize, rect.height() as usize);
        if half_res(m.feather()) {
            blur_half_res(values, w, h, sigma);
        } else {
            blur(values, w, h, sigma);
        }
    }
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
    let mut half = vec![0.0f32; hw * hh];
    {
        let src: &[f32] = values;
        let mut rows: Vec<(usize, &mut [f32])> = half.chunks_mut(hw).enumerate().collect();
        crate::par::for_each(&mut rows, |(y, out)| {
            let r0 = &src[2 * *y * w..][..w];
            let r1 = (2 * *y + 1 < h).then(|| &src[(2 * *y + 1) * w..][..w]);
            // The mean of the up to four pixels, summed in the order (0, 0), (1, 0),
            // (0, 1), (1, 1). Full 2 × 2 blocks first, branch-free so it vectorises.
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
        });
    }
    blur(&mut half, hw, hh, sigma / 2.0);
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
        for (x, o) in out.iter_mut().enumerate() {
            if pairs.contains(&(x / 2)) {
                continue;
            }
            let (x0, x1, sx, tx) = cols[x];
            *o = a[x0] * sx + a[x1] * tx;
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
        let mut lines = vec![0.0f32; (j1 - j0 + 1) * w];
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
        let src = row.to_vec();
        for (x, out) in row.iter_mut().enumerate() {
            let mut acc = 0.0;
            for (j, kw) in k.iter().enumerate() {
                acc += kw * src[clamp_idx(x as isize + j as isize - r, w)];
            }
            *out = acc;
        }
    });
}

fn convolve_cols(values: &mut [f32], w: usize, h: usize, k: &[f32]) {
    let r = (k.len() / 2) as isize;
    let src = values.to_vec();
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
}

/// Box-filters `lanes` interleaved lines of `n` values each with radius `r`, edges
/// clamped: value `k` of lane `j` is `src[k * stride + j]`, and the result goes to
/// `out[k * lanes + j]`.
///
/// Each line uses an f64 prefix sum, so a window of zeros sums to exactly zero (a running
/// sum would leave rounding residue that spreads along the whole line): `prefix[k]` is
/// the sum of the clamped line at indices `-r .. k - r - 1`, accumulated in order, and
/// output `i` is `(prefix[i + 2r + 1] − prefix[i]) / (2r + 1)`. The lanes are
/// independent, so the inner loops vectorise across them, and only the last `2r + 2`
/// prefix rows are kept.
fn box_lanes(src: &[f32], stride: usize, lanes: usize, n: usize, r: usize, out: &mut [f32]) {
    let inv = 1.0 / (2 * r + 1) as f64;
    let span = 2 * r + 1;
    let ring_len = span + 1;
    // Slot `p % ring_len` holds prefix[p]; prefix[0] is zero.
    let mut ring = vec![0.0f64; ring_len * lanes];
    let mut acc = vec![0.0f64; lanes];
    for k in 0..n + 2 * r {
        let line = &src[clamp_idx(k as isize - r as isize, n) * stride..][..lanes];
        for (a, &v) in acc.iter_mut().zip(line) {
            *a += f64::from(v);
        }
        let p = k + 1;
        ring[(p % ring_len) * lanes..][..lanes].copy_from_slice(&acc);
        if p >= span {
            let i = p - span;
            let hi = &ring[(p % ring_len) * lanes..][..lanes];
            let lo = &ring[(i % ring_len) * lanes..][..lanes];
            for ((o, a), b) in out[i * lanes..][..lanes].iter_mut().zip(hi).zip(lo) {
                *o = ((a - b) * inv) as f32;
            }
        }
    }
}

/// Rows box-filtered per group of this many, transposed so the lines are lanes.
const ROW_GROUP: usize = 8;
/// Columns box-filtered per strip.
const COL_STRIP: usize = 32;

fn box_rows(values: &mut [f32], w: usize, _h: usize, r: usize) {
    if r == 0 {
        return;
    }
    let mut groups: Vec<&mut [f32]> = values.chunks_mut(w * ROW_GROUP).collect();
    crate::par::for_each(&mut groups, |rows| {
        let g = rows.len() / w;
        let mut t = vec![0.0f32; w * g];
        for (j, row) in rows.chunks(w).enumerate() {
            for (x, &v) in row.iter().enumerate() {
                t[x * g + j] = v;
            }
        }
        let mut out = vec![0.0f32; w * g];
        box_lanes(&t, g, g, w, r, &mut out);
        for (j, row) in rows.chunks_mut(w).enumerate() {
            for (x, v) in row.iter_mut().enumerate() {
                *v = out[x * g + j];
            }
        }
    });
}

fn box_cols(values: &mut [f32], w: usize, h: usize, r: usize) {
    if r == 0 {
        return;
    }
    let strips: Vec<usize> = (0..w).step_by(COL_STRIP).collect();
    let src: &[f32] = values;
    let results = crate::par::map(&strips, |&x0| {
        let lanes = COL_STRIP.min(w - x0);
        let mut out = vec![0.0f32; lanes * h];
        box_lanes(&src[x0..], w, lanes, h, r, &mut out);
        out
    });
    let mut rows: Vec<(usize, &mut [f32])> = values.chunks_mut(w).enumerate().collect();
    crate::par::for_each(&mut rows, |(y, row)| {
        for (&x0, strip) in strips.iter().zip(&results) {
            let lanes = COL_STRIP.min(w - x0);
            row[x0..x0 + lanes].copy_from_slice(&strip[*y * lanes..][..lanes]);
        }
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
}
