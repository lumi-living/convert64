//! Separable resampling of a layer onto the super grid.
//!
//! Transforms are axis-aligned (translate, scale, flip), so every resample is two 1-D
//! passes driven by per-axis weight tables ([`AxisMap`]). Per axis, a source pixel that
//! covers at least one super pixel is sampled bilinearly; a smaller one is filtered with
//! Lanczos-3 stretched to the footprint, so downscales do not alias. Samples clamp to the
//! layer's edge pixels; the layer's own edge is instead expressed as **coverage**, the
//! exact fraction of each super pixel's footprint inside the layer's box. A layer that
//! fits the canvas exactly is therefore fully opaque, and a sub-pixel nudge moves its
//! edge coverage linearly.

use crate::scratch;
use std::f32::consts::PI;

/// A pixel value that can be accumulated with weights.
pub(crate) trait Px: Copy + Send + Sync + crate::scratch::Scratch {
    /// The additive identity.
    const ZERO: Self;
    /// `self += w × v`.
    fn madd(&mut self, w: f32, v: Self);
}

impl Px for f32 {
    const ZERO: Self = 0.0;
    #[inline]
    fn madd(&mut self, w: f32, v: Self) {
        *self += w * v;
    }
}

impl Px for [f32; 4] {
    const ZERO: Self = [0.0; 4];
    #[inline]
    fn madd(&mut self, w: f32, v: Self) {
        for i in 0..4 {
            self[i] += w * v[i];
        }
    }
}

/// One axis of a layer's placement on the super grid: which super pixels it touches,
/// their source taps and weights, and their coverage.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AxisMap {
    /// First super pixel touched.
    pub out0: u32,
    /// `taps[start[k]..start[k + 1]]` are output `out0 + k`'s taps.
    start: Vec<u32>,
    /// Source pixel index of each tap.
    src: Vec<u32>,
    /// Weight of each tap; an output's weights sum to 1.
    w: Vec<f32>,
    /// Fraction of each output's footprint inside the layer.
    pub cov: Vec<f32>,
}

/// Lanczos kernel with `a = 3`.
fn lanczos3(x: f32) -> f32 {
    let x = x.abs();
    if x < 1e-6 {
        1.0
    } else if x >= 3.0 {
        0.0
    } else {
        let px = PI * x;
        3.0 * px.sin() * (px / 3.0).sin() / (px * px)
    }
}

impl AxisMap {
    /// The map for a source axis of `src_len` pixels placed at `translate` with `scale`
    /// canvas pixels per source pixel (`flip` mirrors it), onto `out_len` super pixels of
    /// `step` canvas pixels each.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        reason = "indices are clamped to the super grid and the source before casting"
    )]
    pub fn new(
        src_len: u32,
        translate: f32,
        scale: f32,
        flip: bool,
        step: f32,
        out_len: u32,
    ) -> Self {
        let lo = translate;
        let hi = translate + scale * src_len as f32;
        let first = ((lo / step).floor().max(0.0) as u32).min(out_len);
        let last = ((hi / step).ceil().max(0.0) as u32).min(out_len);
        let ratio = step / scale; // source pixels per super pixel
        let mut map = Self {
            out0: first,
            start: vec![0],
            src: Vec::new(),
            w: Vec::new(),
            cov: Vec::new(),
        };
        let max_j = src_len as i64 - 1;
        for i in first..last {
            let a = i as f32 * step;
            let b = a + step;
            let cov = ((b.min(hi) - a.max(lo)) / step).clamp(0.0, 1.0);
            let c = (i as f32 + 0.5) * step;
            let mut u = (c - translate) / scale;
            if flip {
                u = src_len as f32 - u;
            }
            let n0 = map.src.len();
            if ratio <= 1.0 {
                let uu = u - 0.5;
                let j0 = uu.floor();
                let f = uu - j0;
                let j0 = j0 as i64;
                for (j, w) in [(j0, 1.0 - f), (j0 + 1, f)] {
                    map.push_tap(j.clamp(0, max_j) as u32, w, n0);
                }
            } else {
                let support = 3.0 * ratio;
                let j_lo = (u - 0.5 - support).ceil() as i64;
                let j_hi = (u - 0.5 + support).floor() as i64;
                for j in j_lo.max(0)..=j_hi.min(max_j) {
                    let w = lanczos3((j as f32 + 0.5 - u) / ratio);
                    if w != 0.0 {
                        map.src.push(j as u32);
                        map.w.push(w);
                    }
                }
                let sum: f32 = map.w[n0..].iter().sum();
                if sum.abs() < 1e-6 || map.src.len() == n0 {
                    map.src.truncate(n0);
                    map.w.truncate(n0);
                    map.src.push((u.floor() as i64).clamp(0, max_j) as u32);
                    map.w.push(1.0);
                } else {
                    for w in &mut map.w[n0..] {
                        *w /= sum;
                    }
                }
            }
            map.cov.push(cov);
            map.start.push(map.src.len() as u32);
        }
        // Trim outputs with no coverage at either end (the layer edge on a boundary).
        while map.cov.last() == Some(&0.0) {
            map.pop_back();
        }
        while map.cov.first() == Some(&0.0) {
            map.pop_front();
        }
        map
    }

    /// Adds a tap, merging it with an existing tap on the same source pixel.
    fn push_tap(&mut self, j: u32, w: f32, n0: usize) {
        if w == 0.0 {
            return;
        }
        if let Some(k) = self.src[n0..].iter().position(|&s| s == j) {
            self.w[n0 + k] += w;
        } else {
            self.src.push(j);
            self.w.push(w);
        }
    }

    fn pop_back(&mut self) {
        self.start.pop();
        let end = *self.start.last().expect("start keeps its first entry") as usize;
        self.src.truncate(end);
        self.w.truncate(end);
        self.cov.pop();
    }

    fn pop_front(&mut self) {
        let skip = self.start[1] as usize;
        self.src.drain(..skip);
        self.w.drain(..skip);
        self.start.remove(0);
        for s in &mut self.start {
            *s -= skip as u32;
        }
        self.cov.remove(0);
        self.out0 += 1;
    }

    /// Number of super pixels touched.
    pub fn len(&self) -> u32 {
        u32::try_from(self.cov.len()).expect("at most 6400 outputs")
    }

    /// Taps of output `k` (relative to `out0`).
    #[inline]
    fn taps(&self, k: usize) -> impl Iterator<Item = (u32, f32)> + '_ {
        let (a, b) = (self.start[k] as usize, self.start[k + 1] as usize);
        self.src[a..b]
            .iter()
            .copied()
            .zip(self.w[a..b].iter().copied())
    }

    /// Lowest and highest source index outputs `k0..k1` read; every output has a tap.
    fn src_range_of(&self, k0: usize, k1: usize) -> (u32, u32) {
        let taps = &self.src[self.start[k0] as usize..self.start[k1] as usize];
        let lo = taps.iter().min().expect("every output has a tap");
        let hi = taps.iter().max().expect("every output has a tap");
        (*lo, *hi)
    }
}

/// Resamples a `sw`-wide row-major source through `mx` (columns) and `my` (rows) into
/// an `mx.len() × my.len()` buffer. Coverage is not applied.
#[cfg(test)]
pub(crate) fn resample<T: Px>(src: &[T], sw: u32, mx: &AxisMap, my: &AxisMap) -> Vec<T> {
    let mut out = Vec::new();
    resample_into(src, sw, mx, my, &mut out, |_, _| ());
    out
}

/// Output rows resampled per task.
const CHUNK_ROWS: usize = 16;

/// The horizontal pass of one chunk: source rows `r0..=r1` resampled through `mx`.
fn horizontal<T: Px>(src: &[T], sw: usize, mx: &AxisMap, r0: u32, tmp: &mut [T]) {
    let ow = mx.len() as usize;
    for (r, row) in tmp.chunks_mut(ow).enumerate() {
        let line = &src[(r0 as usize + r) * sw..][..sw];
        for (k, out) in row.iter_mut().enumerate() {
            let mut acc = T::ZERO;
            for (j, w) in mx.taps(k) {
                acc.madd(w, line[j as usize]);
            }
            *out = acc;
        }
    }
}

/// The vertical pass for output row `k` from `tmp`, the chunk's horizontal pass that
/// starts at source row `r0`.
fn vertical<T: Px>(tmp: &[T], r0: u32, my: &AxisMap, k: usize, row: &mut [T]) {
    let ow = row.len();
    let line = |j: u32| &tmp[(j - r0) as usize * ow..][..ow];
    let mut taps = my.taps(k);
    match (taps.next(), taps.next(), taps.next()) {
        // One or two taps (every bilinear output) in one pass; the sums are the same as
        // the general case's, `0 + w0·v0 (+ w1·v1)`.
        (Some((j0, w0)), None, _) => {
            for (o, v) in row.iter_mut().zip(line(j0)) {
                let mut acc = T::ZERO;
                acc.madd(w0, *v);
                *o = acc;
            }
        }
        (Some((j0, w0)), Some((j1, w1)), None) => {
            for ((o, a), b) in row.iter_mut().zip(line(j0)).zip(line(j1)) {
                let mut acc = T::ZERO;
                acc.madd(w0, *a);
                acc.madd(w1, *b);
                *o = acc;
            }
        }
        _ => {
            row.fill(T::ZERO);
            for (j, w) in my.taps(k) {
                for (o, v) in row.iter_mut().zip(line(j)) {
                    o.madd(w, *v);
                }
            }
        }
    }
}

/// Grows or shrinks `v` to `len`, keeping what it holds: the callers write every
/// element, so a reused buffer is not cleared first.
fn sized<T: Copy>(v: &mut Vec<T>, len: usize, zero: T) {
    if v.len() != len {
        v.clear();
        v.resize(len, zero);
    }
}

/// [`resample`] into `out`, a reused buffer, so a layer resampled again (a drag) does not
/// page in fresh memory; `finish` then runs on each output row, given its index, while
/// the row is still in cache.
///
/// Rows are done in chunks: each chunk resamples horizontally just the source rows its
/// vertical taps read, then filters vertically. A source row shared by two chunks is
/// filtered twice, with the same arithmetic, so the result does not depend on the
/// chunking (or the thread count).
pub(crate) fn resample_into<T: Px>(
    src: &[T],
    sw: u32,
    mx: &AxisMap,
    my: &AxisMap,
    out: &mut Vec<T>,
    finish: impl Fn(usize, &mut [T]) + Send + Sync,
) {
    let (ow, oh) = (mx.len() as usize, my.len() as usize);
    sized(out, ow * oh, T::ZERO);
    if ow == 0 || oh == 0 {
        return;
    }
    let mut chunks: Vec<_> = out.chunks_mut(ow * CHUNK_ROWS).enumerate().collect();
    crate::par::for_each(&mut chunks, |(c, rows)| {
        let k0 = *c * CHUNK_ROWS;
        let (r0, r1) = my.src_range_of(k0, k0 + rows.len() / ow);
        scratch::with((r1 - r0 + 1) as usize * ow, |tmp| {
            horizontal(src, sw as usize, mx, r0, tmp);
            for (i, row) in rows.chunks_mut(ow).enumerate() {
                vertical(tmp, r0, my, k0 + i, row);
                finish(k0 + i, row);
            }
        });
    });
}

/// Premultiplied colour stored one plane per channel, row-major.
#[derive(Debug, Default)]
pub(crate) struct Planes {
    pub r: Vec<f32>,
    pub g: Vec<f32>,
    pub b: Vec<f32>,
    pub a: Vec<f32>,
}

impl Planes {
    /// `len` transparent pixels.
    pub fn new(len: usize) -> Self {
        Self {
            r: vec![0.0; len],
            g: vec![0.0; len],
            b: vec![0.0; len],
            a: vec![0.0; len],
        }
    }

    /// Grows or shrinks every plane to `len`.
    fn sized(&mut self, len: usize) {
        for p in [&mut self.r, &mut self.g, &mut self.b, &mut self.a] {
            sized(p, len, 0.0);
        }
    }
}

/// [`resample_into`] for colour, into planes: `finish` runs on each row interleaved,
/// then the row is split into the planes; what `finish` returns goes to `extra`, one per
/// row.
pub(crate) fn resample_planes<X: Copy + Default + Send>(
    src: &[[f32; 4]],
    sw: u32,
    mx: &AxisMap,
    my: &AxisMap,
    out: &mut Planes,
    extra: &mut Vec<X>,
    finish: impl Fn(usize, &mut [[f32; 4]]) -> X + Send + Sync,
) {
    let (ow, oh) = (mx.len() as usize, my.len() as usize);
    out.sized(ow * oh);
    extra.clear();
    extra.resize(oh, X::default());
    if ow == 0 || oh == 0 {
        return;
    }
    let n = ow * CHUNK_ROWS;
    let Planes { r, g, b, a } = out;
    let mut chunks: Vec<_> = r
        .chunks_mut(n)
        .zip(g.chunks_mut(n))
        .zip(b.chunks_mut(n).zip(a.chunks_mut(n)))
        .zip(extra.chunks_mut(CHUNK_ROWS))
        .enumerate()
        .collect();
    crate::par::for_each(&mut chunks, |(c, (((r, g), (b, a)), extra))| {
        let k0 = *c * CHUNK_ROWS;
        let (r0, r1) = my.src_range_of(k0, k0 + extra.len());
        scratch::with((r1 - r0 + 1) as usize * ow + ow, |tmp| {
            let (tmp, row) = tmp.split_at_mut(tmp.len() - ow);
            horizontal(src, sw as usize, mx, r0, tmp);
            for (i, x) in extra.iter_mut().enumerate() {
                vertical(tmp, r0, my, k0 + i, row);
                *x = finish(k0 + i, row);
                let o = i * ow;
                let planes = r[o..o + ow]
                    .iter_mut()
                    .zip(&mut g[o..o + ow])
                    .zip(b[o..o + ow].iter_mut().zip(&mut a[o..o + ow]));
                for (((r, g), (b, a)), p) in planes.zip(&*row) {
                    [*r, *g, *b, *a] = *p;
                }
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sum_weights(m: &AxisMap, k: usize) -> f32 {
        m.taps(k).map(|(_, w)| w).sum()
    }

    #[test]
    fn identity_at_1x_is_one_tap_per_pixel() {
        let m = AxisMap::new(10, 0.0, 1.0, false, 1.0, 320);
        assert_eq!((m.out0, m.len()), (0, 10));
        for k in 0..10 {
            let taps: Vec<_> = m.taps(k).filter(|t| t.1 != 0.0).collect();
            assert_eq!(taps, vec![(k as u32, 1.0)]);
            assert_eq!(m.cov[k], 1.0);
        }
    }

    #[test]
    fn weights_sum_to_one_and_coverage_tracks_the_edge() {
        for (scale, step) in [(1.0, 0.25), (3.0, 0.5), (0.1, 0.25), (0.37, 1.0)] {
            let m = AxisMap::new(50, 2.3, scale, false, step, 2000);
            for k in 0..m.len() as usize {
                assert!(
                    (sum_weights(&m, k) - 1.0).abs() < 1e-5,
                    "{scale} {step} {k}"
                );
            }
            let total: f32 = m.cov.iter().sum::<f32>() * step;
            assert!(
                (total - 50.0 * scale).abs() < 1e-3,
                "{scale} {step}: {total}"
            );
        }
    }

    #[test]
    fn flip_mirrors_taps() {
        let a = AxisMap::new(8, 0.0, 1.0, false, 0.5, 100);
        let b = AxisMap::new(8, 0.0, 1.0, true, 0.5, 100);
        let n = a.len() as usize;
        for k in 0..n {
            let ta: Vec<_> = a.taps(k).collect();
            // Output k of the plain layer reads the pixels output n−1−k of the mirrored one does.
            let tb: Vec<_> = b.taps(n - 1 - k).collect();
            let mut ta = ta;
            let mut tb = tb;
            ta.sort_by_key(|t| t.0);
            tb.sort_by_key(|t| t.0);
            assert_eq!(ta.len(), tb.len());
            for (x, y) in ta.iter().zip(&tb) {
                assert_eq!(x.0, y.0);
                assert!((x.1 - y.1).abs() < 1e-6);
            }
        }
    }

    #[test]
    fn clipped_to_the_grid() {
        let m = AxisMap::new(10, -5.0, 1.0, false, 1.0, 3);
        assert_eq!((m.out0, m.len()), (0, 3));
        let m = AxisMap::new(10, 400.0, 1.0, false, 1.0, 320);
        assert_eq!(m.len(), 0);
    }

    #[test]
    fn lanczos_downscale_preserves_flat_fields() {
        let src = vec![[0.5, 0.25, 1.0, 1.0]; 40 * 40];
        let mx = AxisMap::new(40, 0.0, 0.25, false, 1.0, 320);
        let my = AxisMap::new(40, 0.0, 0.25, false, 1.0, 200);
        let out = resample(&src, 40, &mx, &my);
        assert_eq!(out.len(), 100);
        for p in out {
            for (a, b) in p.iter().zip([0.5, 0.25, 1.0, 1.0]) {
                assert!((a - b).abs() < 1e-5);
            }
        }
    }
}
