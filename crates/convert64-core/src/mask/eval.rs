//! Mask modifiers on the super grid (ADR-008), applied to a layer's resampled mask in
//! the fixed order Invert → Levels → SnapToCells → Feather.

use super::{Modifiers, SnapToCells};
use crate::geom::{PxRect, SuperGrid};

/// Feather σ (target px) above which the blur runs at half resolution (#76).
pub const HALF_RES_FEATHER: f32 = 8.0;

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
        if m.feather() > HALF_RES_FEATHER {
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

/// Blur at half resolution: 2 × 2 box down, blur with σ/2, bilinear back up.
fn blur_half_res(values: &mut [f32], w: usize, h: usize, sigma: f32) {
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
    let mut rows: Vec<(usize, &mut [f32])> = values.chunks_mut(w).enumerate().collect();
    crate::par::for_each(&mut rows, |(y, row)| {
        // Full-res pixel centre y + ½ is half-res coordinate (y + ½)/2, centre-aligned.
        let fy = ((*y as f32 + 0.5) / 2.0 - 0.5).clamp(0.0, (hh - 1) as f32);
        let y0 = fy.floor() as usize;
        let y1 = (y0 + 1).min(hh - 1);
        let ty = fy - y0 as f32;
        for (x, v) in row.iter_mut().enumerate() {
            let fx = ((x as f32 + 0.5) / 2.0 - 0.5).clamp(0.0, (hw - 1) as f32);
            let x0 = fx.floor() as usize;
            let x1 = (x0 + 1).min(hw - 1);
            let tx = fx - x0 as f32;
            let top = half[y0 * hw + x0] * (1.0 - tx) + half[y0 * hw + x1] * tx;
            let bot = half[y1 * hw + x0] * (1.0 - tx) + half[y1 * hw + x1] * tx;
            *v = top * (1.0 - ty) + bot * ty;
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

/// Box-filters one line of `n` values with radius `r`, edges clamped. Uses an f64 prefix
/// sum, so a window of zeros sums to exactly zero (a running sum would leave rounding
/// residue that spreads along the whole line).
fn box_line(line: &[f32], r: usize, out: &mut [f32], prefix: &mut Vec<f64>) {
    let n = line.len();
    let inv = 1.0 / (2 * r + 1) as f64;
    // prefix[k] = sum of the clamped line at indices -r .. k - r - 1.
    prefix.clear();
    prefix.push(0.0);
    let mut acc = 0.0f64;
    for k in 0..n + 2 * r {
        acc += f64::from(line[clamp_idx(k as isize - r as isize, n)]);
        prefix.push(acc);
    }
    for (i, o) in out.iter_mut().enumerate() {
        *o = ((prefix[i + 2 * r + 1] - prefix[i]) * inv) as f32;
    }
}

fn box_rows(values: &mut [f32], w: usize, _h: usize, r: usize) {
    if r == 0 {
        return;
    }
    let mut rows: Vec<&mut [f32]> = values.chunks_mut(w).collect();
    crate::par::for_each(&mut rows, |row| {
        let src = row.to_vec();
        let mut prefix = Vec::with_capacity(w + 2 * r + 1);
        box_line(&src, r, row, &mut prefix);
    });
}

fn box_cols(values: &mut [f32], w: usize, h: usize, r: usize) {
    if r == 0 {
        return;
    }
    let src = values.to_vec();
    let cols: Vec<usize> = (0..w).collect();
    let results = crate::par::map(&cols, |&x| {
        let line: Vec<f32> = (0..h).map(|y| src[y * w + x]).collect();
        let mut out = vec![0.0f32; h];
        let mut prefix = Vec::with_capacity(h + 2 * r + 1);
        box_line(&line, r, &mut out, &mut prefix);
        out
    });
    for (x, col) in results.into_iter().enumerate() {
        for (y, v) in col.into_iter().enumerate() {
            values[y * w + x] = v;
        }
    }
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

    #[test]
    fn half_res_blur_of_a_flat_field_is_flat() {
        let mut v = vec![0.7; 33 * 21];
        blur_half_res(&mut v, 33, 21, 40.0);
        assert!(v.iter().all(|x| (x - 0.7).abs() < 1e-5));
    }
}
