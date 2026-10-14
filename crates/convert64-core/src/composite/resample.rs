//! Separable resampling of a layer onto the super grid.
//!
//! An axis-aligned transform (translate, scale, flip; `rotate == 0`) is resampled in two
//! 1-D passes driven by per-axis weight tables ([`AxisMap`]); a rotated one goes through
//! `affine` instead (#198).
//!
//! **Colour** is resampled with png2amiga's Lanczos-3 widened by the shrink (ADR-013,
//! #261; `solve::png2amiga_ref::scale`), exactly as its `scale::resample` does: per
//! output pixel the source taps within `⌈3 · max(1, ratio)⌉` of the sample position, cut
//! at the source's edges (not clamped), weighted by `lanczos3(distance / max(1, ratio))`
//! and normalised before the sum, the horizontal pass first, each pass clamped to
//! `0..=1`. **Masks** keep png2c64's Mitchell bicubic (ADR-012): four taps around the
//! position, clamped to the source, the sum divided by the weights' sum, as png2c64's
//! `scale::bicubic` does; they are coverage, and Lanczos would ring on a hard edge.
//!
//! Sub-pixel placement is the kernel's phase. A layer that spans a whole number of output
//! pixels from 0 (one fitted to the screen) samples at png2amiga's positions,
//! `(x + ½) · src / dst − ½`, so it is resampled bit for bit as png2amiga scales. An
//! axis at 1:1 with no sub-pixel offset is copied when the other axis is too, as
//! png2amiga copies a source that is already the target's size; one 1:1 axis beside a
//! scaled one is filtered, as png2amiga filters it. Sample positions clamp to the
//! layer's edge pixels; the layer's own edge is instead expressed as **coverage**, the
//! exact fraction of each super pixel's footprint inside the layer's box, so a sub-pixel
//! nudge moves its edge coverage linearly.

use crate::scratch;
use crate::solve::png2amiga_ref::scale::{kernel_row_into, support};
use crate::solve::png2c64_ref::scale::mitchell;

/// A pixel value that can be accumulated with weights.
pub(crate) trait Px: Copy + Send + Sync + crate::scratch::Scratch {
    /// The additive identity.
    const ZERO: Self;
    /// `self += w × v`.
    fn madd(&mut self, w: f32, v: Self);
    /// `self × s`, each channel.
    fn scaled(self, s: f32) -> Self;
    /// Each channel clamped to `0..=1`, as png2amiga clamps each pass of its scaler.
    fn clamped01(self) -> Self;

    /// `Σ_k wy[k] · Σ_j wx[j] · taps[k][j]`, a 4 × 4 separable kernel on a block of
    /// pixels (the rotated path's Mitchell, #198, ADR-012): each row summed across, then the
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
    #[inline]
    fn scaled(self, s: f32) -> Self {
        self * s
    }
    #[inline]
    fn clamped01(self) -> Self {
        self.clamp(0.0, 1.0)
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
    #[inline]
    fn scaled(self, s: f32) -> Self {
        self.map(|v| v * s)
    }
    #[inline]
    fn clamped01(self) -> Self {
        self.map(|v| v.clamp(0.0, 1.0))
    }
}

/// The kernel an [`AxisMap`] resamples with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kernel {
    /// png2amiga's Lanczos-3 widened by the shrink, each pass clamped to `0..=1` (colour,
    /// ADR-013). With `copy`, an axis at 1:1 on whole pixels is copied; the caller asks
    /// for that only when the other axis is copied too (png2amiga copies a picture that
    /// is already the target's size and filters both axes of any other).
    Lanczos {
        /// Copy an axis at 1:1 on whole pixels.
        copy: bool,
    },
    /// png2c64's Mitchell bicubic (masks, ADR-012); an axis at 1:1 on whole pixels is
    /// copied.
    Mitchell,
}

impl Kernel {
    /// The colour kernel, copying an axis at 1:1.
    pub const COLOUR: Self = Self::Lanczos { copy: true };
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
    /// Weight of each tap: Mitchell's as the kernel gives it, Lanczos' normalised.
    w: Vec<f32>,
    /// Per output, `1 / Σ w`: the taps' sum is scaled by it afterwards, as png2c64 does;
    /// 1 (exact) for Lanczos, whose weights are normalised already, as png2amiga's are.
    inv: Vec<f32>,
    /// Every output is clamped to `0..=1` (Lanczos: png2amiga clamps each pass).
    pub clamp: bool,
    /// Every output is a copy of one source pixel (1:1 on whole pixels).
    pub copy: bool,
    /// Fraction of each output's footprint inside the layer.
    pub cov: Vec<f32>,
    /// The source's mip level the taps index (#235); 0 for the source itself.
    pub level: u32,
}

impl AxisMap {
    /// The map for a source axis of `src_len` pixels placed at `translate` with `scale`
    /// canvas pixels per source pixel (`flip` mirrors it), onto `out_len` super pixels of
    /// `step` canvas pixels each, its taps on the source's mip `level` along this axis
    /// (#235; 0 for the source itself), with `kernel`. The placement, the coverage and
    /// the sample positions are the original's; a level only divides the positions by
    /// `2^level` (exact in f32) and limits the taps to the level's length, and Lanczos is
    /// widened by the shrink left on the level (`ratio / 2^level`).
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        reason = "indices are clamped to the super grid and the source before casting"
    )]
    #[expect(
        clippy::too_many_arguments,
        reason = "the axis' placement, the grid, the level and the kernel"
    )]
    pub fn new(
        src_len: u32,
        translate: f32,
        scale: f32,
        flip: bool,
        step: f32,
        out_len: u32,
        level: u32,
        kernel: Kernel,
    ) -> Self {
        // Output pixels the layer spans, and source pixels per output pixel: png2amiga's
        // `src / dst` when the span is a whole number (a layer fitted to the screen), whose
        // end is then exactly on an output edge (`320 / 637 · 637` rounds just short).
        let span = f64::from(scale) * f64::from(src_len) / f64::from(step);
        let whole = span.round();
        let fitted = (span - whole).abs() < 1e-4 && whole >= 1.0;
        let ratio = if fitted {
            src_len as f32 / whole as f32
        } else {
            (f64::from(src_len) / span) as f32
        };
        let lo = translate;
        let hi = if fitted {
            translate + whole as f32 * step
        } else {
            translate + scale * src_len as f32
        };
        let first = ((lo / step).floor().max(0.0) as u32).min(out_len);
        let last = ((hi / step).ceil().max(0.0) as u32).min(out_len);
        // The layer's offset in output pixels; 0 for a layer at the origin, so its
        // positions are png2amiga's `(x + ½) · ratio`.
        let offset = translate / step;
        let identity = ratio == 1.0
            && offset.fract() == 0.0
            && level == 0
            && kernel != (Kernel::Lanczos { copy: false });
        let mut map = Self {
            out0: first,
            start: vec![0],
            src: Vec::new(),
            w: Vec::new(),
            inv: Vec::new(),
            clamp: matches!(kernel, Kernel::Lanczos { .. }),
            copy: identity,
            cov: Vec::new(),
            level,
        };
        let len = super::mip::level_len(src_len, level);
        // `2^−level`, exact.
        let inv_mip = 1.0 / (1u32 << level) as f32;
        let max_j = len as i64 - 1;
        // Lanczos' widening and reach for the shrink left on the level: png2amiga's
        // `filter_scale` and `support` for the ratio on the original.
        let (filter_scale, reach) = support(if level > 0 { ratio * inv_mip } else { ratio });
        for i in first..last {
            let a = i as f32 * step;
            let b = a + step;
            let cov = ((b.min(hi) - a.max(lo)) / step).clamp(0.0, 1.0);
            let mut u = (i as f32 + 0.5 - offset) * ratio;
            if flip {
                u = src_len as f32 - u;
            }
            if level > 0 {
                u *= inv_mip;
            }
            if identity {
                // 1:1 and on whole pixels: a copy, as png2amiga copies a source that is
                // already the target's size.
                map.src.push((u.floor() as i64).clamp(0, max_j) as u32);
                map.w.push(1.0);
                map.inv.push(1.0);
            } else if map.clamp {
                // png2amiga's `build_kernel_rows`: the taps within `reach` of `pos`, cut
                // at the source's edges, weighted by Lanczos-3 widened by `filter_scale`
                // and normalised. An edge output's centre can lie past the layer (its
                // coverage says how much it overlaps): it samples at the edge, as on the
                // Mitchell path. A fitted layer's positions are always inside, so this
                // changes nothing there.
                let pos = (u - 0.5).clamp(-0.5, len as f32 - 0.5);
                let first = kernel_row_into(pos, len as usize, filter_scale, reach, &mut map.w);
                let n = map.w.len() - map.src.len();
                map.src.extend((0..n).map(|k| (first + k) as u32));
                map.inv.push(1.0);
            } else {
                // png2c64's `sample`: the four pixels around `pos`, clamped, each weighted
                // by the kernel at its distance; the sum is divided by the weights' sum.
                // An edge output's centre can lie past the layer (its coverage says how
                // much it overlaps): it samples the edge, as png2c64 never samples outside.
                // A fitted layer's positions are always inside, so this changes nothing
                // there.
                let pos = (u - 0.5).clamp(-0.5, len as f32 - 0.5);
                let center = pos.floor() as i64;
                let mut weight_sum = 0.0f32;
                for k in -1..=2 {
                    let s = (center + k).clamp(0, max_j);
                    let w = mitchell(pos - s as f32);
                    map.src.push(s as u32);
                    map.w.push(w);
                    weight_sum += w;
                }
                map.inv.push(if weight_sum > 0.0 {
                    1.0 / weight_sum
                } else {
                    1.0
                });
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

    fn pop_back(&mut self) {
        self.start.pop();
        let end = *self.start.last().expect("start keeps its first entry") as usize;
        self.src.truncate(end);
        self.w.truncate(end);
        self.inv.pop();
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
        self.inv.remove(0);
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

/// A colour layer's two axis maps, `x(kernel)` and `y(kernel)` building each: Lanczos,
/// an axis at 1:1 on whole pixels copied only when the other one is too. png2amiga
/// copies a picture that is already the target's size and filters both axes of any
/// other, and at 1:1 its kernel is not quite a copy (`lanczos3(±1)` is about `−2e−8` in
/// f32), so a fitted layer with one axis at 1:1 is resampled bit for bit as it is.
pub(crate) fn colour_pair(
    x: impl Fn(Kernel) -> AxisMap,
    y: impl Fn(Kernel) -> AxisMap,
) -> (AxisMap, AxisMap) {
    let filtered = Kernel::Lanczos { copy: false };
    let (mx, my) = (x(Kernel::COLOUR), y(Kernel::COLOUR));
    match (mx.copy, my.copy) {
        (true, false) => (x(filtered), my),
        (false, true) => (mx, y(filtered)),
        _ => (mx, my),
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

/// An output's taps when it has at most two (an axis copied at 1:1): source pixels and
/// weights. A one-tap output has a second tap of weight 0 on the same pixel, which adds
/// a zero to its finite sum and so leaves it unchanged (#139). Such outputs have
/// `inv == 1`.
#[derive(Clone, Copy, Debug)]
struct Pair {
    j: [u32; 2],
    w: [f32; 2],
}

impl AxisMap {
    /// Every output's taps as a [`Pair`], or `None` when an output has more than two
    /// (the Mitchell kernel's four).
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

/// Output `k` of `line` through `mx`: `(0 + w0·v0 + w1·v1 …) · inv` in tap order, as
/// png2c64's `sample` sums.
#[inline(always)]
fn tap_sum<T: Px>(line: &[T], mx: &AxisMap, k: usize) -> T {
    let mut acc = T::ZERO;
    for (j, w) in mx.taps(k) {
        acc.madd(w, line[j as usize]);
    }
    acc.scaled(mx.inv[k])
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
        if mx.clamp {
            // png2amiga clamps its horizontal pass before the vertical one reads it.
            for out in row.iter_mut() {
                *out = out.clamped01();
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
            let inv = my.inv[k];
            for o in row.iter_mut() {
                *o = o.scaled(inv);
            }
        }
    }
    if my.clamp {
        for o in row.iter_mut() {
            *o = o.clamped01();
        }
    }
}

/// [`vertical`] for interleaved colour, split into the planes `out` as it is written,
/// each pixel clamped to `0..=1` when `my` clamps (Lanczos, as png2amiga's vertical
/// pass), then scaled by its coverage (`cov[x] × my.cov[k]`), alpha clamped to `0..=1`,
/// or exactly the coverage when the source is `opaque`. `inner` is where `cov` is 1: on
/// a row whose own coverage is 1 too, the scale there is 1 and is skipped, since
/// `v × 1 = v` (#139).
#[inline(always)]
#[expect(
    clippy::too_many_arguments,
    reason = "the pass's inputs, its outputs and whether the source is opaque"
)]
fn vertical_split(
    tmp: &[[f32; 4]],
    r0: u32,
    my: &AxisMap,
    k: usize,
    out: [&mut [f32]; 4],
    cov: &[f32],
    inner: std::ops::Range<usize>,
    opaque: bool,
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
                opaque,
                my.clamp,
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
                opaque,
                my.clamp,
            );
        }
        _ => split_row(
            |x| {
                let mut acc = [0.0f32; 4];
                for (j, w) in my.taps(k) {
                    acc.madd(w, line(j)[x]);
                }
                acc.scaled(my.inv[k])
            },
            out,
            cov,
            cy,
            inner,
            opaque,
            my.clamp,
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
    opaque: bool,
    clamp: bool,
) {
    let ow = cov.len();
    let [r, g, b, a] = out;
    // png2amiga clamps its vertical pass (Lanczos); Mitchell keeps its lobes.
    let sum = |x: usize| if clamp { sum(x).clamped01() } else { sum(x) };
    let (r, g, b, a) = (&mut r[..ow], &mut g[..ow], &mut b[..ow], &mut a[..ow]);
    for x in (0..inner.start).chain(inner.end.max(inner.start)..ow) {
        let v = sum(x);
        let s = cov[x] * cy;
        // Colour is clamped already (Lanczos, as png2amiga); a Mitchell sum would keep
        // its slight negative ringing, as png2c64's scaler does.
        r[x] = v[0] * s;
        g[x] = v[1] * s;
        b[x] = v[2] * s;
        // An opaque source's alpha is its coverage, exactly: the kernel's sum of ones
        // can round to just under 1.
        a[x] = if opaque {
            s
        } else {
            (v[3] * s).clamp(0.0, 1.0)
        };
    }
    let (r, g, b, a) = (
        &mut r[inner.clone()],
        &mut g[inner.clone()],
        &mut b[inner.clone()],
        &mut a[inner.clone()],
    );
    for (i, x) in inner.enumerate() {
        let v = sum(x);
        r[i] = v[0];
        g[i] = v[1];
        b[i] = v[2];
        a[i] = if opaque { 1.0 } else { v[3].clamp(0.0, 1.0) };
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
/// by coverage (`mx.cov × my.cov`): alpha clamped to `0..=1`. With Lanczos maps every
/// channel is clamped to `0..=1` after each pass, as png2amiga's scaler clamps (for an
/// opaque source exactly png2amiga's); with Mitchell maps colour is left as the kernel
/// gives it, slight negative ringing included. With `opaque` (every source alpha 1) the alpha
/// is the coverage itself. `extent` gets, per output row, the columns outside
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
    opaque: bool,
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
                vertical_split(tmp, r0, my, k, planes, &mx.cov, inner.clone(), opaque);
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

    /// The weights of output `k` as applied: the kernel's, scaled by `1 / Σ w`.
    fn sum_weights(m: &AxisMap, k: usize) -> f32 {
        m.taps(k).map(|(_, w)| w).sum::<f32>() * m.inv[k]
    }

    /// Both kernels: colour's Lanczos and the masks' Mitchell.
    const KERNELS: [Kernel; 2] = [Kernel::COLOUR, Kernel::Mitchell];

    #[test]
    fn identity_at_1x_is_one_tap_per_pixel() {
        for kernel in KERNELS {
            let m = AxisMap::new(10, 0.0, 1.0, false, 1.0, 320, 0, kernel);
            assert_eq!((m.out0, m.len()), (0, 10));
            assert!(m.copy);
            for k in 0..10 {
                let taps: Vec<_> = m.taps(k).filter(|t| t.1 != 0.0).collect();
                assert_eq!(taps, vec![(k as u32, 1.0)]);
                assert_eq!(m.cov[k], 1.0);
            }
        }
        // Filtered at 1:1, Lanczos is not quite a copy (png2amiga's `sin(π)` in f32).
        let filtered = Kernel::Lanczos { copy: false };
        let m = AxisMap::new(10, 0.0, 1.0, false, 1.0, 320, 0, filtered);
        assert!(!m.copy && m.taps(5).count() == 6);
    }

    /// A fitted layer's colour maps on a `tw`-wide grid of `step` canvas px (both modes'
    /// geometry), as the compositor builds them.
    fn fitted_maps(w: usize, h: usize, tw: u32, step: f32) -> (AxisMap, AxisMap) {
        colour_pair(
            |k| AxisMap::new(w as u32, 0.0, 320.0 / w as f32, false, step, tw, 0, k),
            |k| AxisMap::new(h as u32, 0.0, 200.0 / h as f32, false, 1.0, 200, 0, k),
        )
    }

    /// ADR-013 (#261): a layer fitted to the target is resampled bit for bit as
    /// png2amiga's `scale::resample` scales (widened Lanczos-3, edges cut, each pass
    /// clamped), in both modes' geometry, shrunk (up to 12.8×), enlarged, and with one
    /// axis at 1:1 beside a scaled one (png2amiga filters it too).
    #[test]
    fn a_fitted_layer_is_png2amiga_s_resample_bit_for_bit() {
        use crate::solve::png2amiga_ref::{Image, scale};
        for (w, h, tw, step) in [
            (637usize, 398usize, 160u32, 2.0f32),
            (1024, 637, 320, 1.0),
            (1024, 640, 160, 2.0),
            (2048, 1280, 160, 2.0),
            (200, 125, 160, 2.0),
            (97, 61, 160, 2.0),
            (320, 150, 320, 1.0),
            (160, 211, 160, 2.0),
            (320, 200, 320, 1.0),
        ] {
            // Steps that overshoot under the kernel, ramps, and the extremes.
            let px: Vec<[f32; 3]> = (0..w * h)
                .map(|i| {
                    let (x, y) = (i % w, i / w);
                    [
                        (x as f32 * 0.013).fract(),
                        if (x / 7 + y / 5) % 2 == 0 { 1.0 } else { 0.0 },
                        ((x + y) as f32 * 0.051).fract(),
                    ]
                })
                .collect();
            let im = Image::new(w, h, px.clone());
            let want = if (w, h) == (tw as usize, 200) {
                im
            } else {
                scale::resample(&im, tw as usize, 200)
            };
            let src: Vec<[f32; 4]> = px.iter().map(|p| [p[0], p[1], p[2], 1.0]).collect();
            let (mx, my) = fitted_maps(w, h, tw, step);
            let (mut got, mut extent) = (Planes::default(), Vec::new());
            resample_planes(&src, w as u32, &mx, &my, &mut got, &mut extent, true);
            assert_eq!(got.r.len(), want.px.len());
            for (i, p) in want.px.iter().enumerate() {
                assert_eq!(
                    [got.r[i], got.g[i], got.b[i]].map(f32::to_bits),
                    p.map(f32::to_bits),
                    "{w}×{h} → {tw}: {i}"
                );
                assert_eq!(got.a[i], 1.0);
            }
        }
    }

    #[test]
    fn weights_sum_to_one_and_coverage_tracks_the_edge() {
        for kernel in KERNELS {
            for (scale, step) in [(1.0, 0.25), (3.0, 0.5), (0.1, 0.25), (0.37, 1.0)] {
                let m = AxisMap::new(50, 2.3, scale, false, step, 2000, 0, kernel);
                for k in 0..m.len() as usize {
                    assert!(
                        (sum_weights(&m, k) - 1.0).abs() < 1e-5,
                        "{kernel:?} {scale} {step} {k}: {:?} {}",
                        m.taps(k).collect::<Vec<_>>(),
                        m.inv[k]
                    );
                }
                let total: f32 = m.cov.iter().sum::<f32>() * step;
                assert!(
                    (total - 50.0 * scale).abs() < 1e-3,
                    "{kernel:?} {scale} {step}: {total}"
                );
            }
        }
    }

    /// Lanczos is widened by the shrink: 3 taps a side at 1:1 and when enlarging,
    /// `⌈3 · shrink⌉` when shrinking, and on a mip level by the shrink left there.
    #[test]
    fn lanczos_widens_with_the_shrink_left_on_the_level() {
        let taps = |m: &AxisMap, k| m.taps(k).count();
        let fit = 320.0 / 4096.0;
        let m = AxisMap::new(4096, 0.0, fit, false, 1.0, 320, 0, Kernel::COLOUR);
        assert_eq!(taps(&m, 160), 78);
        // The 512 px level: a shrink of 1.6 left, ⌈4.8⌉ = 5 a side.
        let m = AxisMap::new(4096, 0.0, fit, false, 1.0, 320, 3, Kernel::COLOUR);
        assert_eq!(taps(&m, 160), 10);
        let m = AxisMap::new(50, 0.3, 3.0, false, 1.0, 320, 0, Kernel::COLOUR);
        assert_eq!(taps(&m, 60), 6);
    }

    #[test]
    fn flip_mirrors_taps() {
        for kernel in KERNELS {
            let a = AxisMap::new(8, 0.0, 1.0, false, 0.5, 100, 0, kernel);
            let b = AxisMap::new(8, 0.0, 1.0, true, 0.5, 100, 0, kernel);
            let n = a.len() as usize;
            // Output k of the plain layer reads the pixels output n−1−k of the mirrored
            // one does (a tap of weight 0 at the end of Lanczos' reach aside).
            let nonzero = |m: &AxisMap, k| -> Vec<(u32, f32)> {
                let mut t: Vec<_> = m.taps(k).filter(|t| t.1 != 0.0).collect();
                t.sort_by_key(|t| t.0);
                t
            };
            for k in 0..n {
                let (ta, tb) = (nonzero(&a, k), nonzero(&b, n - 1 - k));
                assert_eq!(ta.len(), tb.len(), "{kernel:?} {k}");
                for (x, y) in ta.iter().zip(&tb) {
                    assert_eq!(x.0, y.0);
                    assert!((x.1 - y.1).abs() < 1e-6);
                }
            }
        }
    }

    #[test]
    fn clipped_to_the_grid() {
        for kernel in KERNELS {
            let m = AxisMap::new(10, -5.0, 1.0, false, 1.0, 3, 0, kernel);
            assert_eq!((m.out0, m.len()), (0, 3));
            let m = AxisMap::new(10, 400.0, 1.0, false, 1.0, 320, 0, kernel);
            assert_eq!(m.len(), 0);
        }
    }

    #[test]
    fn a_downscale_preserves_flat_fields() {
        for kernel in KERNELS {
            let src = vec![[0.5, 0.25, 1.0, 1.0]; 40 * 40];
            let mx = AxisMap::new(40, 0.0, 0.25, false, 1.0, 320, 0, kernel);
            let my = AxisMap::new(40, 0.0, 0.25, false, 1.0, 200, 0, kernel);
            let out = resample(&src, 40, &mx, &my);
            assert_eq!(out.len(), 100);
            for p in out {
                for (a, b) in p.iter().zip([0.5, 0.25, 1.0, 1.0]) {
                    assert!((a - b).abs() < 1e-5);
                }
            }
        }
    }

    /// The resample as written before #139, one output at a time: each output's taps in
    /// order, horizontally then vertically, from `0`, each pass clamped when its map
    /// clamps.
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
                        let v = acc.scaled(mx.inv[k]);
                        if mx.clamp { v.clamped01() } else { v }
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
            for o in row.iter_mut() {
                *o = o.scaled(my.inv[k]);
                if my.clamp {
                    *o = o.clamped01();
                }
            }
        }
        out
    }

    /// Placements covering upscaled and downscaled axes, flips, sub-pixel offsets, the grid
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
        for kernel in KERNELS.into_iter().chain([Kernel::Lanczos { copy: false }]) {
            colour_planes_match_with(kernel);
        }
    }

    fn colour_planes_match_with(kernel: Kernel) {
        for (sx, tx, kx, fx, step_x, gx) in axes() {
            for (sy, ty, ky, fy, step_y, gy) in axes() {
                let mx = AxisMap::new(sx, tx, kx, fx, step_x, gx, 0, kernel);
                let my = AxisMap::new(sy, ty, ky, fy, step_y, gy, 0, kernel);
                let src = source(sx, sy);
                let mut want = reference(&src, sx, &mx, &my);
                let ow = mx.len() as usize;
                for (k, row) in want.chunks_mut(ow.max(1)).enumerate() {
                    for (p, cx) in row.iter_mut().zip(&mx.cov) {
                        let s = cx * my.cov[k];
                        p[3] = (p[3] * s).clamp(0.0, 1.0);
                        for c in &mut p[..3] {
                            *c *= s;
                        }
                    }
                }
                let (mut got, mut extent) = (Planes::default(), Vec::new());
                resample_planes(&src, sx, &mx, &my, &mut got, &mut extent, false);
                assert_eq!(extent.len(), my.len() as usize);
                for (i, p) in want.iter().enumerate() {
                    let q = [got.r[i], got.g[i], got.b[i], got.a[i]];
                    assert!(
                        p.iter().zip(q).all(|(a, b)| a.to_bits() == b.to_bits()),
                        "{kernel:?} {sx} {tx} {kx} × {sy} {ty} {ky}: {i}: {p:?} {q:?}"
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
                let mx = AxisMap::new(sx, tx, kx, fx, step_x, gx, 0, Kernel::Mitchell);
                let my = AxisMap::new(sy, ty, ky, fy, step_y, gy, 0, Kernel::Mitchell);
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
