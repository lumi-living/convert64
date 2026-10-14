//! Separable Lanczos-3 resampling widened by the shrink ratio, ported from png2amiga's
//! `scale.cpp`.
//!
//! What differs from png2c64's Mitchell bicubic (`png2c64_ref::scale`):
//!
//! - **The kernel** is Lanczos-3, `sinc(x) · sinc(x / 3)` for `|x| < 3`.
//! - **Widened when shrinking.** `filter_scale = max(1, src / dst)`: the kernel is
//!   stretched to cover one destination pixel's worth of source, so a large shrink
//!   averages instead of aliasing (#235). Its support is `⌈3 · filter_scale⌉` taps on
//!   each side of the centre.
//! - **Edges cut, not clamped.** Taps outside the source are dropped (png2c64 repeats
//!   the edge pixel); the remaining weights are renormalised.
//! - **Weights are normalised first** (`w · (1 / Σw)`) and the sum is `Σ src · w`, where
//!   png2c64 sums raw weights and scales the sum.
//! - **Clamped to `0..=1` after each pass**, where png2c64 keeps the negative lobes.
//!
//! `std::sin` on `float` is computed in `f64` and rounded once; macOS's `sinf` agrees on
//! every float the scaler can pass it (`docs/reference/png2amiga-reference.md`).

use super::colour::{clamped, sinf};
use super::{Image, Rgb};

/// Lanczos-3's half-width, `kLanczosA`.
pub const LANCZOS_A: i32 = 3;

/// `lanczos3`.
#[inline]
pub fn lanczos3(x: f32) -> f32 {
    if x == 0.0 {
        return 1.0;
    }
    let a = LANCZOS_A as f32;
    if x <= -a || x >= a {
        return 0.0;
    }
    let pix = std::f32::consts::PI * x;
    let pix_a = pix / a;
    (sinf(pix) / pix) * (sinf(pix_a) / pix_a)
}

/// One destination pixel's taps (`KernelRow`): the first source index and the
/// normalised weights.
#[derive(Clone, Debug, PartialEq)]
pub struct KernelRow {
    /// First source index (already clamped to the source).
    pub first: usize,
    /// The weights, normalised to sum to 1 (as png2amiga normalises them).
    pub w: Vec<f32>,
}

/// `filter_scale` and `support` for a `ratio` of source per destination pixel.
#[expect(
    clippy::cast_possible_truncation,
    reason = "the support is a small positive integer"
)]
pub fn support(ratio: f32) -> (f32, i64) {
    let filter_scale = ratio.max(1.0);
    let support = (LANCZOS_A as f32 * filter_scale).ceil() as i64;
    (filter_scale, support)
}

/// The taps of one destination pixel whose centre is at source position `src_center`
/// (`(i + ½) · ratio − ½` for png2amiga), on a source axis of `src_len` pixels:
/// `build_kernel_rows`' body.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "indices are clamped to the source before casting"
)]
pub fn kernel_row(src_center: f32, src_len: usize, filter_scale: f32, support: i64) -> KernelRow {
    let center = src_center.floor() as i64;
    let lo = (center - support + 1).max(0);
    let hi = (center + support).min(src_len as i64 - 1);
    let count = (hi - lo + 1).max(0) as usize;
    let mut w = Vec::with_capacity(count);
    let mut wsum = 0.0f32;
    for k in 0..count {
        let sx = lo + k as i64;
        let dist = (src_center - sx as f32) / filter_scale;
        let v = lanczos3(dist);
        w.push(v);
        wsum += v;
    }
    if wsum > 0.0 {
        let inv = 1.0 / wsum;
        for v in &mut w {
            *v *= inv;
        }
    }
    KernelRow {
        first: lo.max(0) as usize,
        w,
    }
}

/// `build_kernel_rows(src_size, dst_size)`.
pub fn kernel_rows(src_size: usize, dst_size: usize) -> Vec<KernelRow> {
    let ratio = src_size as f32 / dst_size as f32;
    let (filter_scale, sup) = support(ratio);
    (0..dst_size)
        .map(|i| kernel_row((i as f32 + 0.5) * ratio - 0.5, src_size, filter_scale, sup))
        .collect()
}

/// `Σ tap(first + k) · w[k]` from zero, in tap order (png2amiga's `sum += src * w`).
#[inline]
fn apply(row: &KernelRow, tap: impl Fn(usize) -> Rgb) -> Rgb {
    let mut sum = [0.0f32; 3];
    for (k, &w) in row.w.iter().enumerate() {
        let v = tap(row.first + k);
        sum = [sum[0] + v[0] * w, sum[1] + v[1] * w, sum[2] + v[2] * w];
    }
    clamped(sum)
}

/// `resample`: the horizontal pass to `dst_w`, then the vertical to `dst_h`, each
/// clamped to `0..=1`.
///
/// # Panics
/// If either target size is 0.
pub fn resample(src: &Image, dst_w: usize, dst_h: usize) -> Image {
    assert!(dst_w > 0 && dst_h > 0, "target dimensions must be non-zero");
    let rows = kernel_rows(src.w, dst_w);
    let mut mid = Vec::with_capacity(dst_w * src.h);
    for y in 0..src.h {
        for r in &rows {
            mid.push(apply(r, |s| src.at(s, y)));
        }
    }
    let rows = kernel_rows(src.h, dst_h);
    let mut out = vec![[0.0; 3]; dst_w * dst_h];
    for (y, r) in rows.iter().enumerate() {
        for x in 0..dst_w {
            out[y * dst_w + x] = apply(r, |s| mid[s * dst_w + x]);
        }
    }
    Image::new(dst_w, dst_h, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_kernel_widens_with_the_shrink() {
        // 1:1 keeps three taps a side; a 12.8× shrink 39.
        assert_eq!(support(1.0), (1.0, 3));
        assert_eq!(support(0.5).1, 3);
        assert_eq!(support(12.8).1, 39);
        let rows = kernel_rows(4096, 320);
        assert!(rows.iter().all(|r| (r.w.iter().sum::<f32>() - 1.0).abs() < 1e-4));
        assert_eq!(rows[160].w.len(), 78);
    }

    #[test]
    fn a_flat_source_stays_flat_and_in_range() {
        let src = Image::new(7, 5, vec![[0.25, 0.5, 1.0]; 35]);
        let out = resample(&src, 3, 4);
        for p in &out.px {
            for (a, b) in p.iter().zip([0.25f32, 0.5, 1.0]) {
                assert!((a - b).abs() < 1e-6);
            }
        }
    }
}
