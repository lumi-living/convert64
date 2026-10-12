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

    /// `Σ_k wy[k] · Σ_j wx[j] · taps[k][j]`, a 4 × 4 separable kernel on a block of
    /// pixels (the rotated path's Catmull-Rom, #198): each row summed across, then the
    /// rows down.
    #[inline(always)]
    fn taps4x4(taps: &[[Self; 4]; 4], wx: [f32; 4], wy: [f32; 4]) -> Self {
        let mut acc = Self::ZERO;
        for (line, wy) in taps.iter().zip(wy) {
            let mut r = Self::ZERO;
            for (v, wx) in line.iter().zip(wx) {
                r.madd(wx, *v);
            }
            acc.madd(wy, r);
        }
        acc
    }
}

impl Px for f32 {
    const ZERO: Self = 0.0;
    #[inline]
    fn madd(&mut self, w: f32, v: Self) {
        *self += w * v;
    }

    /// For one channel, the columns are summed down first, then across.
    #[inline(always)]
    fn taps4x4(taps: &[[Self; 4]; 4], wx: [f32; 4], wy: [f32; 4]) -> Self {
        let mut col = [0.0f32; 4];
        for (line, wy) in taps.iter().zip(wy) {
            for (c, v) in col.iter_mut().zip(line) {
                *c += wy * v;
            }
        }
        let mut acc = 0.0;
        for (c, wx) in col.iter().zip(wx) {
            acc += wx * c;
        }
        acc
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

/// `sin(π·x)` for `0 ≤ x ≤ 3` with basic IEEE operations only (no platform `sin`), so
/// resampling weights, and with them the solver's input, are the same on every OS: the
/// argument is reduced to `|r| ≤ ½` around the nearest integer and the series summed in
/// f64 (error far below f32's precision).
fn sin_pi(x: f32) -> f32 {
    let x = f64::from(x);
    let k = (x + 0.5).floor();
    let r = (x - k) * std::f64::consts::PI;
    let r2 = r * r;
    // Taylor series of sin to r^17 (|r| ≤ π/2).
    let mut term = r;
    let mut sum = r;
    for n in 1..=8 {
        let n = f64::from(n);
        term = -term * r2 / ((2.0 * n) * (2.0 * n + 1.0));
        sum += term;
    }
    #[expect(clippy::cast_possible_truncation, reason = "a sine in −1..=1")]
    let s = sum as f32;
    if (k as i64) % 2 == 0 { s } else { -s }
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
        3.0 * sin_pi(x) * sin_pi(x / 3.0) / (px * px)
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
pub(crate) const CHUNK_ROWS: usize = 32;

/// An output's taps when it has at most two (every bilinear output): source pixels and
/// weights. A one-tap output has a second tap of weight 0 on the same pixel, which adds
/// a zero to its finite sum and so leaves it unchanged (#139).
#[derive(Clone, Copy, Debug)]
struct Pair {
    j: [u32; 2],
    w: [f32; 2],
}

impl AxisMap {
    /// Every output's taps as a [`Pair`], or `None` when an output has more than two
    /// (a Lanczos downscale).
    fn pairs(&self) -> Option<Vec<Pair>> {
        self.start
            .windows(2)
            .map(|s| {
                let (a, b) = (s[0] as usize, s[1] as usize);
                match b - a {
                    1 => Some(Pair {
                        j: [self.src[a]; 2],
                        w: [self.w[a], 0.0],
                    }),
                    2 => Some(Pair {
                        j: [self.src[a], self.src[a + 1]],
                        w: [self.w[a], self.w[a + 1]],
                    }),
                    _ => None,
                }
            })
            .collect()
    }
}

/// Output `k` of `line` through `mx`: `0 + w0·v0 + w1·v1 …` in tap order.
#[inline(always)]
fn tap_sum<T: Px>(line: &[T], mx: &AxisMap, k: usize) -> T {
    let mut acc = T::ZERO;
    for (j, w) in mx.taps(k) {
        acc.madd(w, line[j as usize]);
    }
    acc
}

/// The horizontal pass of one chunk: source rows from `r0` resampled through `mx`, one
/// output row per `ow` values of `tmp`.
fn horizontal<T: Px>(
    src: &[T],
    sw: usize,
    mx: &AxisMap,
    pairs: Option<&[Pair]>,
    r0: u32,
    tmp: &mut [T],
) {
    let ow = mx.len() as usize;
    for (r, row) in tmp.chunks_mut(ow).enumerate() {
        let line = &src[(r0 as usize + r) * sw..][..sw];
        match pairs {
            // The common case on its own, so the loop has no per-output tap count.
            Some(p) => {
                for (out, &Pair { j, w }) in row.iter_mut().zip(p) {
                    let mut acc = T::ZERO;
                    acc.madd(w[0], line[j[0] as usize]);
                    acc.madd(w[1], line[j[1] as usize]);
                    *out = acc;
                }
            }
            None => {
                for (k, out) in row.iter_mut().enumerate() {
                    *out = tap_sum(line, mx, k);
                }
            }
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

/// [`vertical`] for interleaved colour, split into the planes `out` as it is written,
/// each pixel scaled by its coverage (`cov[x] × my.cov[k]`), colour clamped at 0 and
/// alpha to `0..=1`. `inner` is where `cov` is 1: on a row whose own coverage is 1
/// too, the scale there is 1 and is skipped, since `v × 1 = v` (#139).
#[inline(always)]
fn vertical_split(
    tmp: &[[f32; 4]],
    r0: u32,
    my: &AxisMap,
    k: usize,
    out: [&mut [f32]; 4],
    cov: &[f32],
    inner: std::ops::Range<usize>,
) {
    let ow = cov.len();
    let cy = my.cov[k];
    let inner = if cy == 1.0 { inner } else { 0..0 };
    let line = |j: u32| &tmp[(j - r0) as usize * ow..][..ow];
    // Each arm sums the taps at `x` as `0 + w0·v0 (+ w1·v1 …)`, in tap order.
    let mut taps = my.taps(k);
    match (taps.next(), taps.next(), taps.next()) {
        (Some((j0, w0)), None, _) => {
            let l0 = line(j0);
            split_row(
                |x| {
                    let mut acc = [0.0f32; 4];
                    acc.madd(w0, l0[x]);
                    acc
                },
                out,
                cov,
                cy,
                inner,
            );
        }
        (Some((j0, w0)), Some((j1, w1)), None) => {
            let (l0, l1) = (line(j0), line(j1));
            split_row(
                |x| {
                    let mut acc = [0.0f32; 4];
                    acc.madd(w0, l0[x]);
                    acc.madd(w1, l1[x]);
                    acc
                },
                out,
                cov,
                cy,
                inner,
            );
        }
        _ => split_row(
            |x| {
                let mut acc = [0.0f32; 4];
                for (j, w) in my.taps(k) {
                    acc.madd(w, line(j)[x]);
                }
                acc
            },
            out,
            cov,
            cy,
            inner,
        ),
    }
}

/// The body of [`vertical_split`] for one way of summing the taps.
#[inline(always)]
fn split_row(
    sum: impl Fn(usize) -> [f32; 4],
    out: [&mut [f32]; 4],
    cov: &[f32],
    cy: f32,
    inner: std::ops::Range<usize>,
) {
    let ow = cov.len();
    let [r, g, b, a] = out;
    let (r, g, b, a) = (&mut r[..ow], &mut g[..ow], &mut b[..ow], &mut a[..ow]);
    for x in (0..inner.start).chain(inner.end.max(inner.start)..ow) {
        let v = sum(x);
        let s = cov[x] * cy;
        r[x] = (v[0] * s).max(0.0);
        g[x] = (v[1] * s).max(0.0);
        b[x] = (v[2] * s).max(0.0);
        a[x] = (v[3] * s).clamp(0.0, 1.0);
    }
    let (r, g, b, a) = (
        &mut r[inner.clone()],
        &mut g[inner.clone()],
        &mut b[inner.clone()],
        &mut a[inner.clone()],
    );
    for (i, x) in inner.enumerate() {
        let v = sum(x);
        r[i] = v[0].max(0.0);
        g[i] = v[1].max(0.0);
        b[i] = v[2].max(0.0);
        a[i] = v[3].clamp(0.0, 1.0);
    }
}

/// Grows or shrinks `v` to `len`, keeping what it holds: the callers write every
/// element, so a reused buffer is not cleared first.
pub(crate) fn sized<T: Copy>(v: &mut Vec<T>, len: usize, zero: T) {
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
    let pairs = mx.pairs();
    let pairs = pairs.as_deref();
    let mut chunks: Vec<_> = out.chunks_mut(ow * CHUNK_ROWS).enumerate().collect();
    crate::par::for_each(&mut chunks, |(c, rows)| {
        let k0 = *c * CHUNK_ROWS;
        let (r0, r1) = my.src_range_of(k0, k0 + rows.len() / ow);
        scratch::with((r1 - r0 + 1) as usize * ow, |tmp| {
            horizontal(src, sw as usize, mx, pairs, r0, tmp);
            for (i, row) in rows.chunks_mut(ow).enumerate() {
                vertical(tmp, r0, my, k0 + i, row);
                finish(k0 + i, row);
            }
        });
    });
}

/// [`resample_into`] for a mask, reduced at once to half resolution: `finish` runs on
/// each full-resolution row, then `down` turns each pair of rows (the last one alone
/// when the height is odd) into a row of `half` (`⌈w/2⌉` wide). The full-resolution rows
/// are only ever in scratch, two at a time (#139); the arithmetic is unchanged.
pub(crate) fn resample_half(
    src: &[f32],
    sw: u32,
    mx: &AxisMap,
    my: &AxisMap,
    half: &mut [f32],
    finish: impl Fn(&mut [f32]) + Send + Sync,
    down: impl Fn(&[f32], Option<&[f32]>, &mut [f32]) + Send + Sync,
) {
    const {
        assert!(
            CHUNK_ROWS.is_multiple_of(2),
            "pairs of rows stay in one chunk"
        )
    };
    let (ow, oh) = (mx.len() as usize, my.len() as usize);
    if ow == 0 || oh == 0 {
        return;
    }
    let hw = ow.div_ceil(2);
    let pairs = mx.pairs();
    let pairs = pairs.as_deref();
    let mut chunks: Vec<_> = half.chunks_mut(hw * CHUNK_ROWS / 2).enumerate().collect();
    crate::par::for_each(&mut chunks, |(c, out)| {
        let k0 = *c * CHUNK_ROWS;
        let k1 = (k0 + CHUNK_ROWS).min(oh);
        let (r0, r1) = my.src_range_of(k0, k1);
        scratch::with((r1 - r0 + 1) as usize * ow + 2 * ow, |tmp| {
            let (tmp, rows) = tmp.split_at_mut(tmp.len() - 2 * ow);
            horizontal(src, sw as usize, mx, pairs, r0, tmp);
            let tmp: &[f32] = tmp;
            let (a, b) = rows.split_at_mut(ow);
            for (i, out) in out.chunks_mut(hw).enumerate() {
                let k = k0 + 2 * i;
                vertical(tmp, r0, my, k, a);
                finish(a);
                let second = if k + 1 < oh {
                    vertical(tmp, r0, my, k + 1, b);
                    finish(b);
                    Some(&*b)
                } else {
                    None
                };
                down(a, second, out);
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
    pub(crate) fn sized(&mut self, len: usize) {
        for p in [&mut self.r, &mut self.g, &mut self.b, &mut self.a] {
            sized(p, len, 0.0);
        }
    }
}

/// Resamples premultiplied colour (`sw` wide) through `mx` and `my` into planes, scaled
/// by coverage (`mx.cov × my.cov`): alpha clamped to `0..=1` and colour to `≥ 0`, since
/// Lanczos can ring slightly negative. `extent` gets, per output row, the columns outside
/// which the row is all zero.
///
/// Chunked like [`resample_into`]. The horizontal pass works on interleaved pixels; the
/// vertical pass splits each output row into the planes as it writes it, with the
/// coverage and the clamps in the same loop, which vectorises across pixels (#139).
/// Each value gets the same operations in the same order as before.
pub(crate) fn resample_planes(
    src: &[[f32; 4]],
    sw: u32,
    mx: &AxisMap,
    my: &AxisMap,
    out: &mut Planes,
    extent: &mut Vec<(u32, u32)>,
) {
    let (ow, oh) = (mx.len() as usize, my.len() as usize);
    out.sized(ow * oh);
    extent.clear();
    extent.resize(oh, (0, 0));
    if ow == 0 || oh == 0 {
        return;
    }
    let pairs = mx.pairs();
    let pairs = pairs.as_deref();
    // Where the column coverage is 1: all but a column or two at each edge.
    let inner = match (
        mx.cov.iter().position(|&c| c == 1.0),
        mx.cov.iter().rposition(|&c| c == 1.0),
    ) {
        (Some(c0), Some(c1)) if mx.cov[c0..=c1].iter().all(|&c| c == 1.0) => c0..c1 + 1,
        _ => 0..0,
    };
    let n = ow * CHUNK_ROWS;
    let Planes { r, g, b, a } = out;
    let mut chunks: Vec<_> = r
        .chunks_mut(n)
        .zip(g.chunks_mut(n))
        .zip(b.chunks_mut(n).zip(a.chunks_mut(n)))
        .zip(extent.chunks_mut(CHUNK_ROWS))
        .enumerate()
        .collect();
    crate::par::for_each(&mut chunks, |(c, (((r, g), (b, a)), extent))| {
        let k0 = *c * CHUNK_ROWS;
        let (r0, r1) = my.src_range_of(k0, k0 + extent.len());
        let plane = (r1 - r0 + 1) as usize * ow;
        scratch::with(plane, |tmp: &mut [[f32; 4]]| {
            horizontal(src, sw as usize, mx, pairs, r0, tmp);
            let tmp: &[[f32; 4]] = tmp;
            for (i, e) in extent.iter_mut().enumerate() {
                let k = k0 + i;
                let o = i * ow..(i + 1) * ow;
                let (r, g, b, a) = (
                    &mut r[o.clone()],
                    &mut g[o.clone()],
                    &mut b[o.clone()],
                    &mut a[o],
                );
                let planes = [&mut *r, &mut *g, &mut *b, &mut *a];
                vertical_split(tmp, r0, my, k, planes, &mx.cov, inner.clone());
                let on = |x: &usize| r[*x] != 0.0 || g[*x] != 0.0 || b[*x] != 0.0 || a[*x] != 0.0;
                *e = match ((0..ow).find(on), (0..ow).rfind(on)) {
                    (Some(x0), Some(x1)) => (
                        u32::try_from(x0).expect("row fits u32"),
                        u32::try_from(x1 + 1).expect("row fits u32"),
                    ),
                    _ => (0, 0),
                };
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sin_pi_matches_the_platform_sine() {
        for i in 0..=3000 {
            let x = i as f32 / 1000.0;
            let want = (f64::from(x) * std::f64::consts::PI).sin();
            assert!((f64::from(sin_pi(x)) - want).abs() < 1e-6, "{x}");
        }
    }

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

    /// The resample as written before #139, one output at a time: each output's taps in
    /// order, horizontally then vertically, from `0`.
    fn reference<T: Px>(src: &[T], sw: u32, mx: &AxisMap, my: &AxisMap) -> Vec<T> {
        let (ow, oh) = (mx.len() as usize, my.len() as usize);
        let rows: Vec<Vec<T>> = src
            .chunks(sw as usize)
            .map(|line| {
                (0..ow)
                    .map(|k| {
                        let mut acc = T::ZERO;
                        for (j, w) in mx.taps(k) {
                            acc.madd(w, line[j as usize]);
                        }
                        acc
                    })
                    .collect()
            })
            .collect();
        let mut out = vec![T::ZERO; ow * oh];
        for (k, row) in out.chunks_mut(ow.max(1)).enumerate() {
            for (j, w) in my.taps(k) {
                for (o, v) in row.iter_mut().zip(&rows[j as usize]) {
                    o.madd(w, *v);
                }
            }
        }
        out
    }

    /// Placements covering bilinear and Lanczos axes, flips, sub-pixel offsets, the grid
    /// edges and one-tap outputs: `(src len, translate, scale, flip, step, out len)`.
    fn axes() -> Vec<(u32, f32, f32, bool, f32, u32)> {
        vec![
            (37, 3.3, 1.0, false, 0.25, 400),
            (37, -2.6, 1.7, true, 0.25, 100),
            (40, 0.0, 1.0, false, 1.0, 320),
            (90, 10.1, 0.37, false, 1.0, 320),
            (90, 1.9, 0.37, true, 0.5, 60),
            (23, 5.0, 3.0, false, 0.125, 2000),
        ]
    }

    /// A deterministic source with negative colour, zeros and partial alpha.
    fn source(w: u32, h: u32) -> Vec<[f32; 4]> {
        let mut s = 0x9e37_79b9_u32;
        let mut next = move || {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            ((s >> 8) as f32 / (1u32 << 24) as f32, s)
        };
        (0..w * h)
            .map(|_| {
                let (v, bits) = next();
                let a = match bits % 5 {
                    0 => 0.0,
                    1 => 1.0,
                    _ => v,
                };
                let b = if bits % 7 == 0 { 0.0 } else { next().0 * a };
                [next().0 * a - 0.05, next().0 * a, b, a]
            })
            .collect()
    }

    #[test]
    fn colour_planes_match_the_reference_bit_for_bit() {
        for (sx, tx, kx, fx, step_x, gx) in axes() {
            for (sy, ty, ky, fy, step_y, gy) in axes() {
                let mx = AxisMap::new(sx, tx, kx, fx, step_x, gx);
                let my = AxisMap::new(sy, ty, ky, fy, step_y, gy);
                let src = source(sx, sy);
                let mut want = reference(&src, sx, &mx, &my);
                let ow = mx.len() as usize;
                for (k, row) in want.chunks_mut(ow.max(1)).enumerate() {
                    for (p, cx) in row.iter_mut().zip(&mx.cov) {
                        let s = cx * my.cov[k];
                        p[3] = (p[3] * s).clamp(0.0, 1.0);
                        for c in &mut p[..3] {
                            *c = (*c * s).max(0.0);
                        }
                    }
                }
                let (mut got, mut extent) = (Planes::default(), Vec::new());
                resample_planes(&src, sx, &mx, &my, &mut got, &mut extent);
                assert_eq!(extent.len(), my.len() as usize);
                for (i, p) in want.iter().enumerate() {
                    let q = [got.r[i], got.g[i], got.b[i], got.a[i]];
                    assert!(
                        p.iter().zip(q).all(|(a, b)| a.to_bits() == b.to_bits()),
                        "{sx} {tx} {kx} × {sy} {ty} {ky}: {i}: {p:?} {q:?}"
                    );
                }
                for (row, e) in want.chunks(ow.max(1)).zip(&extent) {
                    let on = |p: &[f32; 4]| *p != [0.0; 4];
                    let naive = match (row.iter().position(on), row.iter().rposition(on)) {
                        (Some(a), Some(b)) => (a as u32, b as u32 + 1),
                        _ => (0, 0),
                    };
                    assert_eq!(*e, naive);
                }
            }
        }
    }

    #[test]
    fn half_resolution_mask_matches_resample_then_reduce() {
        for (sx, tx, kx, fx, step_x, gx) in axes() {
            for (sy, ty, ky, fy, step_y, gy) in axes() {
                let mx = AxisMap::new(sx, tx, kx, fx, step_x, gx);
                let my = AxisMap::new(sy, ty, ky, fy, step_y, gy);
                let src: Vec<f32> = source(sx, sy).iter().map(|p| p[0] + p[3]).collect();
                let (ow, oh) = (mx.len() as usize, my.len() as usize);
                let finish = |row: &mut [f32]| {
                    for x in row {
                        *x = x.clamp(0.0, 1.0);
                    }
                };
                let mut full = reference(&src, sx, &mx, &my);
                full.chunks_mut(ow.max(1)).for_each(finish);
                let hw = ow.div_ceil(2);
                let mut want = vec![0.0; hw * oh.div_ceil(2)];
                let mut got = vec![f32::NAN; want.len()];
                let down = crate::mask::eval::down_row;
                for (y, out) in want.chunks_mut(hw.max(1)).enumerate() {
                    let r0 = &full[2 * y * ow..][..ow];
                    let r1 = (2 * y + 1 < oh).then(|| &full[(2 * y + 1) * ow..][..ow]);
                    down(r0, r1, out);
                }
                resample_half(&src, sx, &mx, &my, &mut got, finish, down);
                assert!(
                    want.iter()
                        .zip(&got)
                        .all(|(a, b)| a.to_bits() == b.to_bits()),
                    "{sx} {tx} {kx} × {sy} {ty} {ky}"
                );
            }
        }
    }
}
