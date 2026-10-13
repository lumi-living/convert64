//! Resampling of a **rotated** layer onto the super grid (#198, ADR-011).
//!
//! An axis-aligned layer (`rotate == 0`) takes the separable path in `resample`; a
//! rotated one cannot, so each super pixel is inverse-mapped to layer space and sampled
//! there:
//!
//! - **Kernel.** png2c64's Mitchell bicubic (B = C = ⅓, ADR-012, #227), the separable
//!   path's kernel: 4 × 4 taps on the premultiplied source around the inverse-mapped
//!   position, weights normalised to sum to 1; samples clamp to the layer's edge pixels.
//!   Like png2c64's scaler and the separable path, it is not widened when the layer is
//!   scaled down, so a turned layer is as crisp as an upright one. Shrunk by more than
//!   4× along a layer axis, under `Downscale::Auto` it samples the source's mip level on
//!   that axis instead (#235, `mip`), the rule the separable path follows.
//! - **Coverage.** The layer's edge is antialiased by the fraction of each super pixel
//!   inside the rotated quad, computed **exactly**: the super pixel's footprint is a
//!   parallelogram in layer space, clipped against the box `[0, w] × [0, h]`
//!   (Sutherland–Hodgman) and its area taken, in f64. Pixels whose footprint lies wholly
//!   inside or outside the box skip the clip. The fraction is invariant under the affine
//!   map, so it is the canvas-space coverage too.
//!
//! Every output depends only on its own position, so the result does not depend on the
//! chunking or the thread count.

use super::resample::{CHUNK_ROWS, Planes, Px, sized};
use crate::geom::{PxRect, SuperGrid};
use crate::layer::Transform;
use crate::scratch;

/// Mitchell weights for the four taps around a sample `t` (0–1) past the second,
/// normalised to sum to 1.
#[cfg(test)]
fn mitchell4(t: f32) -> [f32; 4] {
    weights(t + 1.0, [0, 1, 2, 3])
}

/// Mitchell weights of the taps at source pixels `taps` (clamped) for a sample at `pos`,
/// normalised to sum to 1: png2c64's `sample` weighting.
#[inline(always)]
#[expect(
    clippy::cast_precision_loss,
    reason = "pixel indices are far below 2^24"
)]
fn weights(pos: f32, taps: [i64; 4]) -> [f32; 4] {
    use crate::solve::png2c64_ref::scale::mitchell;
    let w = taps.map(|s| mitchell(pos - s as f32));
    let sum = w[0] + w[1] + w[2] + w[3];
    let inv = if sum > 0.0 { 1.0 / sum } else { 1.0 };
    w.map(|v| v * inv)
}

/// A rotated layer's placement on the super grid: the inverse map from super pixels to
/// layer space, the mip level it samples, and the super rectangle it touches.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AffineMap {
    /// Layer `u` of super pixel `(i, j)`'s centre: `u[0]·i + u[1]·j + u[2]`.
    u: [f64; 3],
    /// Layer `v`, likewise.
    v: [f64; 3],
    /// The source size in pixels.
    size: (u32, u32),
    /// Mip level per layer axis: the source is halved `level.0` times along u and
    /// `level.1` times along v.
    pub level: (u32, u32),
    /// `2^−level`: level-0 layer coordinates to the mip's.
    inv_mip: (f64, f64),
    /// Half the footprint's extent along u and v around its centre.
    half: (f64, f64),
    /// The super pixels the layer's quad can touch: its bounding box on the grid.
    pub rect: PxRect,
    /// Every super pixel's centre lands on a source pixel's centre at 1:1 (a turn by a
    /// multiple of 90° on whole pixels): samples are copies, as the separable path copies
    /// an axis at 1:1 (ADR-012).
    lattice: bool,
    /// The map turns by a multiple of 90° (any scale): separable, so a mask's 4× grid
    /// samples it with Mitchell like the separable path, not bilinearly.
    pub(crate) quarter: bool,
}

impl AffineMap {
    /// The map for a source of `size` placed by `t` onto `grid`, sampling the source's
    /// mip `level` along each layer axis ((0, 0): the source itself; #235).
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the box is clamped to the grid before casting"
    )]
    pub fn new(t: &Transform, size: (u32, u32), grid: SuperGrid, level: (u32, u32)) -> Self {
        let [a, b, c, d, e, f] = t.canvas_to_layer_affine(size);
        let step = grid.super_px_in_canvas();
        let (sx, sy) = (f64::from(step.x), f64::from(step.y));
        // Super (i, j) has its centre at canvas ((i + ½)·sx, (j + ½)·sy).
        let u = [a * sx, b * sy, a * sx * 0.5 + b * sy * 0.5 + c];
        let v = [d * sx, e * sy, d * sx * 0.5 + e * sy * 0.5 + f];
        // The footprint (one super pixel) spans u[0] + u[1] along u, v[0] + v[1] along v.
        let span = (u[0].abs() + u[1].abs(), v[0].abs() + v[1].abs());
        let (lu, lv) = level;
        let (lo, hi) = t.canvas_bounds(size);
        let a0 = grid.canvas_to_super(lo);
        let a1 = grid.canvas_to_super(hi);
        let (gw, gh) = grid.size();
        let clamp = |v: f32, max: u32| v.clamp(0.0, max as f32);
        let rect = PxRect::new(
            clamp(a0.x, gw).floor() as u32,
            clamp(a0.y, gh).floor() as u32,
            clamp(a1.x, gw).ceil() as u32,
            clamp(a1.y, gh).ceil() as u32,
        );
        // A signed permutation at 1:1 whose centres land on source centres.
        let unit = |x: f64| x == 0.0 || x.abs() == 1.0;
        let centred = |x: f64| (x - 0.5).fract() == 0.0;
        // A turn by a multiple of 90°: each axis of the grid maps onto one of the layer's.
        let quarter = (u[0] == 0.0) != (u[1] == 0.0)
            && (v[0] == 0.0) != (v[1] == 0.0)
            && (u[0] == 0.0) == (v[1] == 0.0);
        let lattice = quarter
            && level == (0, 0)
            && [u[0], u[1], v[0], v[1]].into_iter().all(unit)
            && centred(u[2])
            && centred(v[2]);
        Self {
            lattice,
            quarter,
            u,
            v,
            size,
            level: (lu, lv),
            inv_mip: (1.0 / f64::from(1u32 << lu), 1.0 / f64::from(1u32 << lv)),
            half: (span.0 * 0.5, span.1 * 0.5),
            rect: if rect.is_empty() { PxRect::EMPTY } else { rect },
        }
    }

    /// How far (in source pixels of level 0) one source pixel's change can reach through
    /// the kernel: two pixels each way, and on a mip level (#235) two of its pixels and
    /// the one it was averaged into.
    pub fn reach_src(&self) -> f64 {
        let l = self.level.0.max(self.level.1);
        if l == 0 { 2.0 } else { f64::from(3u32 << l) }
    }

    /// Source pixels (of level 0) one pixel of `grid` spans along each layer axis, for a
    /// source of `size` placed by `t`: the shrink the mip level is chosen from (#235).
    /// For an unrotated layer it is `step / scale` on each axis.
    pub fn footprint(t: &Transform, size: (u32, u32), step: crate::geom::Vec2) -> (f64, f64) {
        let [a, b, _, d, e, _] = t.canvas_to_layer_affine(size);
        let (sx, sy) = (f64::from(step.x), f64::from(step.y));
        (
            (a * sx).abs() + (b * sy).abs(),
            (d * sx).abs() + (e * sy).abs(),
        )
    }

    /// The fraction of super pixel `(i, j)` inside the layer's box, whose footprint is
    /// centred on layer `(u, v)`.
    #[inline(always)]
    fn coverage(&self, u: f64, v: f64) -> f32 {
        let (w, h) = (f64::from(self.size.0), f64::from(self.size.1));
        let (hu, hv) = self.half;
        if u - hu >= 0.0 && u + hu <= w && v - hv >= 0.0 && v + hv <= h {
            return 1.0;
        }
        if u + hu <= 0.0 || u - hu >= w || v + hv <= 0.0 || v - hv >= h {
            return 0.0;
        }
        self.clip_coverage(u, v)
    }

    /// [`Self::coverage`] by clipping the footprint parallelogram against the box.
    #[expect(clippy::cast_possible_truncation, reason = "a fraction in 0..=1")]
    fn clip_coverage(&self, u: f64, v: f64) -> f32 {
        let (w, h) = (f64::from(self.size.0), f64::from(self.size.1));
        let (ex, ey) = (
            (self.u[0] * 0.5, self.v[0] * 0.5),
            (self.u[1] * 0.5, self.v[1] * 0.5),
        );
        let mut poly = [(0.0f64, 0.0f64); 8];
        poly[0] = (u - ex.0 - ey.0, v - ex.1 - ey.1);
        poly[1] = (u + ex.0 - ey.0, v + ex.1 - ey.1);
        poly[2] = (u + ex.0 + ey.0, v + ex.1 + ey.1);
        poly[3] = (u - ex.0 + ey.0, v - ex.1 + ey.1);
        let mut n = 4;
        let full = area(&poly[..4]).abs();
        if full == 0.0 {
            return 0.0;
        }
        // Keep the side where `inside(p) ≥ 0`, an axis-aligned half-plane each.
        let planes: [(usize, f64, f64); 4] =
            [(0, 1.0, 0.0), (0, -1.0, w), (1, 1.0, 0.0), (1, -1.0, h)];
        for (axis, sign, off) in planes {
            let dist = |p: (f64, f64)| sign * if axis == 0 { p.0 } else { p.1 } + off;
            let mut out = [(0.0f64, 0.0f64); 8];
            let mut m = 0;
            for k in 0..n {
                let (p, q) = (poly[k], poly[(k + 1) % n]);
                let (dp, dq) = (dist(p), dist(q));
                if dp >= 0.0 {
                    out[m] = p;
                    m += 1;
                }
                if (dp >= 0.0) != (dq >= 0.0) {
                    let s = dp / (dp - dq);
                    out[m] = (p.0 + s * (q.0 - p.0), p.1 + s * (q.1 - p.1));
                    m += 1;
                }
            }
            poly = out;
            n = m;
            if n == 0 {
                return 0.0;
            }
        }
        (area(&poly[..n]).abs() / full).clamp(0.0, 1.0) as f32
    }

    /// Layer `(u, v)` of super-pixel index `(i, j)` (pixel `(i, j)`'s centre when whole).
    #[inline(always)]
    fn at(&self, i: f64, j: f64) -> (f64, f64) {
        (
            self.u[0] * i + self.u[1] * j + self.u[2],
            self.v[0] * i + self.v[1] * j + self.v[2],
        )
    }

    /// Calls `f(k, u, v)` for outputs `ks` of super row `j` (indices into the columns of
    /// `rect`), with the layer coordinates of each one's centre.
    #[inline(always)]
    #[expect(clippy::cast_precision_loss, reason = "column indices are below 2^16")]
    fn row(&self, j: u32, ks: std::ops::Range<usize>, mut f: impl FnMut(usize, f64, f64)) {
        let jf = f64::from(j);
        let (ur, vr) = (self.u[1] * jf + self.u[2], self.v[1] * jf + self.v[2]);
        let x0 = f64::from(self.rect.x0);
        for k in ks {
            let i = x0 + k as f64;
            f(k, self.u[0] * i + ur, self.v[0] * i + vr);
        }
    }

    /// The Mitchell sample of `src` (mip level [`Self::level`], `mw × mh`) at layer
    /// `(u, v)` (level-0 coordinates), edges clamped. The 4 × 4 taps are summed row by
    /// row, `0 + w0·v0 + … + w3·v3`, the same way whether or not they needed clamping.
    #[inline(always)]
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "coordinates are clamped to the source before indexing"
    )]
    fn sample<T: Px>(&self, src: &[T], mw: u32, mh: u32, u: f64, v: f64) -> T {
        // An edge output's centre can lie past the layer: it samples the edge, as on the
        // separable path.
        let x = (u * self.inv_mip.0 - 0.5).clamp(-0.5, f64::from(mw) - 0.5);
        let y = (v * self.inv_mip.1 - 0.5).clamp(-0.5, f64::from(mh) - 0.5);
        let (fx, fy) = (x.floor(), y.floor());
        if self.lattice {
            let (mw, mh) = (i64::from(mw), i64::from(mh));
            let (cx, cy) = ((fx as i64).clamp(0, mw - 1), (fy as i64).clamp(0, mh - 1));
            return src[cy as usize * mw as usize + cx as usize];
        }
        let (x0, y0) = (fx as i64 - 1, fy as i64 - 1);
        let (mw, mh) = (i64::from(mw), i64::from(mh));
        // png2c64's edge rule, as on the separable path: a tap clamped to the edge is
        // weighted by its distance to the pixel it reads.
        let xs: [i64; 4] = std::array::from_fn(|k| (x0 + k as i64).clamp(0, mw - 1));
        let ys: [i64; 4] = std::array::from_fn(|k| (y0 + k as i64).clamp(0, mh - 1));
        let wx = weights(x as f32, xs);
        let wy = weights(y as f32, ys);
        let taps: [[T; 4]; 4] = if x0 >= 0 && y0 >= 0 && x0 + 3 < mw && y0 + 3 < mh {
            // Inside: four runs of four pixels.
            let w = mw as usize;
            let base = y0 as usize * w + x0 as usize;
            std::array::from_fn(|k| src[base + k * w..][..4].try_into().expect("four taps"))
        } else {
            std::array::from_fn(|k| {
                let r = ys[k] as usize * mw as usize;
                xs.map(|x| src[r + x as usize])
            })
        };
        T::taps4x4(&taps, wx, wy)
    }

    /// [`Self::sample`] with a bilinear kernel: what a mask's 4× grid samples on the
    /// rotated path (ADR-012), where Mitchell's sixteen taps per fine pixel cost more
    /// than the box reduction after them can tell apart.
    fn sample_bilinear(&self, src: &[f32], mw: u32, mh: u32, u: f64, v: f64) -> f32 {
        let x = (u * self.inv_mip.0 - 0.5).clamp(0.0, f64::from(mw) - 1.0);
        let y = (v * self.inv_mip.1 - 0.5).clamp(0.0, f64::from(mh) - 1.0);
        let (fx, fy) = (x.floor(), y.floor());
        let (tx, ty) = ((x - fx) as f32, (y - fy) as f32);
        let (x0, y0) = (fx as usize, fy as usize);
        let (x1, y1) = ((x0 + 1).min(mw as usize - 1), (y0 + 1).min(mh as usize - 1));
        let w = mw as usize;
        let row = |r: usize| src[r * w + x0] + (src[r * w + x1] - src[r * w + x0]) * tx;
        let (a, b) = (row(y0), row(y1));
        a + (b - a) * ty
    }
}

/// Twice the signed area of a polygon (shoelace).
fn area(p: &[(f64, f64)]) -> f64 {
    let n = p.len();
    (0..n)
        .map(|k| {
            let (a, b) = (p[k], p[(k + 1) % n]);
            a.0 * b.1 - b.0 * a.1
        })
        .sum()
}

/// Resamples premultiplied colour (the mip level `map` samples, `mw × mh`) onto
/// `map.rect` into planes, scaled by coverage, alpha clamped to `0..=1` and colour
/// left with Mitchell's rings (png2c64 keeps them; the front end's round trip clamps):
/// the rotated counterpart of `resample::resample_planes`,
/// with the same per-row `extent`.
pub(crate) fn resample_planes_affine(
    src: &[[f32; 4]],
    (mw, mh): (u32, u32),
    map: &AffineMap,
    out: &mut Planes,
    extent: &mut Vec<(u32, u32)>,
) {
    let (ow, oh) = (map.rect.width() as usize, map.rect.height() as usize);
    out.sized(ow * oh);
    extent.clear();
    extent.resize(oh, (0, 0));
    if ow == 0 || oh == 0 {
        return;
    }
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
        scratch::with(ow, |cov: &mut [f32]| {
            scratch::with(ow, |px: &mut [[f32; 4]]| {
                for (i, e) in extent.iter_mut().enumerate() {
                    let j = map.rect.y0 + u32::try_from(*c * CHUNK_ROWS + i).expect("rows fit u32");
                    let o = i * ow..(i + 1) * ow;
                    let (r, g, b, a) = (
                        &mut r[o.clone()],
                        &mut g[o.clone()],
                        &mut b[o.clone()],
                        &mut a[o],
                    );
                    // The coverage first, then samples only where it is above zero.
                    map.row(j, 0..ow, |k, u, v| cov[k] = map.coverage(u, v));
                    let span = match (
                        cov.iter().position(|&c| c > 0.0),
                        cov.iter().rposition(|&c| c > 0.0),
                    ) {
                        (Some(k0), Some(k1)) => k0..k1 + 1,
                        _ => 0..0,
                    };
                    map.row(j, span.clone(), |k, u, v| {
                        px[k] = map.sample(src, mw, mh, u, v);
                    });
                    for k in 0..ow {
                        let (p, s) = if span.contains(&k) {
                            (px[k], cov[k])
                        } else {
                            ([0.0; 4], 0.0)
                        };
                        r[k] = (p[0] * s).max(0.0);
                        g[k] = (p[1] * s).max(0.0);
                        b[k] = (p[2] * s).max(0.0);
                        a[k] = (p[3] * s).clamp(0.0, 1.0);
                    }
                    // Where coverage is above zero: the row is zero outside it.
                    *e = (
                        u32::try_from(span.start).expect("row fits u32"),
                        u32::try_from(span.end).expect("row fits u32"),
                    );
                }
            });
        });
    });
}

/// Resamples a mask's layer-space values (the mip level `map` samples, `mw × mh`) onto
/// `map.rect` into `out`, clamped to `0..=1`, with Mitchell or, for a mask's 4× grid,
/// `bilinear`ly ([`AffineMap::sample_bilinear`]) unless the map turns by a multiple of
/// 90° (then it matches the separable path's Mitchell). Like the separable path, no coverage is
/// applied: outside the quad the mask continues its edge values, and the colour's
/// coverage hides it.
pub(crate) fn resample_mask_affine(
    src: &[f32],
    (mw, mh): (u32, u32),
    map: &AffineMap,
    bilinear: bool,
    out: &mut Vec<f32>,
) {
    let (ow, oh) = (map.rect.width() as usize, map.rect.height() as usize);
    sized(out, ow * oh, 0.0);
    if ow == 0 || oh == 0 {
        return;
    }
    let mut chunks: Vec<_> = out.chunks_mut(ow * CHUNK_ROWS).enumerate().collect();
    crate::par::for_each(&mut chunks, |(c, rows)| {
        for (i, row) in rows.chunks_mut(ow).enumerate() {
            let j = map.rect.y0 + u32::try_from(*c * CHUNK_ROWS + i).expect("rows fit u32");
            if bilinear && !map.quarter {
                map.row(j, 0..ow, |k, u, v| {
                    row[k] = map.sample_bilinear(src, mw, mh, u, v).clamp(0.0, 1.0);
                });
            } else {
                map.row(j, 0..ow, |k, u, v| {
                    row[k] = map.sample(src, mw, mh, u, v).clamp(0.0, 1.0);
                });
            }
        }
    });
}

/// [`resample_mask_affine`] straight at half resolution, for a feather blurred there
/// (`mask::eval::half_res_sigma`): `half` (`⌈w/2⌉ × ⌈h/2⌉` of `map.rect`) gets, per 2 × 2
/// block of super pixels, the sample at the block's centre (at an odd edge, the centre
/// of the one or two pixels there), clamped to `0..=1`. The separable path averages the
/// block's four samples instead; under a feather of at least 4 target px (16 half-res
/// px at 4×) the two differ by far less than an 8-bit step, and this takes a quarter
/// of the samples.
pub(crate) fn resample_mask_affine_half(
    src: &[f32],
    (mw, mh): (u32, u32),
    map: &AffineMap,
    half: &mut [f32],
) {
    let (w, h) = (map.rect.width() as usize, map.rect.height() as usize);
    let (hw, hh) = (w.div_ceil(2), h.div_ceil(2));
    debug_assert_eq!(half.len(), hw * hh);
    if hw == 0 || hh == 0 {
        return;
    }
    // A block's centre in super-pixel index space (pixel `i`'s centre is at `i`).
    let centre = |b: usize, len: usize, origin: u32| {
        let offset = if 2 * b + 1 < len { 0.5 } else { 0.0 };
        f64::from(origin) + f64::from(u32::try_from(2 * b).expect("fits u32")) + offset
    };
    let mut chunks: Vec<_> = half.chunks_mut(hw * CHUNK_ROWS).enumerate().collect();
    crate::par::for_each(&mut chunks, |(c, rows)| {
        for (i, row) in rows.chunks_mut(hw).enumerate() {
            let j = centre(*c * CHUNK_ROWS + i, h, map.rect.y0);
            for (k, o) in row.iter_mut().enumerate() {
                let x = centre(k, w, map.rect.x0);
                let (u, v) = map.at(x, j);
                *o = map.sample(src, mw, mh, u, v).clamp(0.0, 1.0);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Vec2;
    use crate::mode::Mode;

    #[test]
    fn mitchell_weights_sum_to_one_and_keep_a_ramp() {
        // png2c64's kernel at t = 0: 1/18, 16/18, 1/18 (it smooths; it does not
        // interpolate).
        let w = mitchell4(0.0);
        assert!((w[0] - 1.0 / 18.0).abs() < 1e-6 && (w[1] - 16.0 / 18.0).abs() < 1e-6);
        for i in 0..=16 {
            let t = i as f32 / 16.0;
            let w = mitchell4(t);
            assert!((w.iter().sum::<f32>() - 1.0).abs() < 1e-6, "{t}");
            // Reproduces a linear ramp exactly.
            let lin: f32 = w.iter().enumerate().map(|(k, w)| w * k as f32).sum();
            assert!((lin - (1.0 + t)).abs() < 1e-5, "{t}: {lin}");
        }
    }

    fn rotated(deg: f32, scale: f32) -> Transform {
        Transform::new(Vec2::new(40.3, 20.7), Vec2::new(scale, scale), false, false)
            .unwrap()
            .with_rotation(deg)
            .unwrap()
    }

    /// The coverage of every super pixel sums to the quad's area, whatever the angle.
    #[test]
    fn coverage_sums_to_the_quad_area() {
        for mode in [Mode::HiresBitmap, Mode::MulticolorBitmap] {
            let grid = SuperGrid::new(mode, 4).unwrap();
            let step = grid.super_px_in_canvas();
            for deg in [0.25, 17.0, 45.0, 90.0, 133.3, -60.0] {
                let t = rotated(deg, 1.7);
                let size = (37, 23);
                let map = AffineMap::new(&t, size, grid, (0, 0));
                let mut total = 0.0f64;
                for j in map.rect.y0..map.rect.y1 {
                    let w = map.rect.width() as usize;
                    map.row(j, 0..w, |_, u, v| total += f64::from(map.coverage(u, v)));
                }
                let got = total * f64::from(step.x) * f64::from(step.y);
                let want = 37.0 * 23.0 * 1.7 * 1.7;
                assert!(
                    (got - want).abs() < 1e-3 * want,
                    "{mode} {deg}: {got} {want}"
                );
            }
        }
    }

    /// #235: the shrink a level is chosen from is the source pixels one grid pixel spans
    /// along each layer axis: `1 / scale` unrotated or turned by 90°, more in between.
    #[test]
    fn the_footprint_is_the_shrink_along_each_layer_axis() {
        let step = Vec2::new(1.0, 1.0);
        for (deg, want) in [(0.0, 10.0), (90.0, 10.0), (45.0, 10.0 * 2f64.sqrt())] {
            let t = Transform::new(Vec2::ZERO, Vec2::new(0.1, 0.5), false, false)
                .unwrap()
                .with_rotation(deg)
                .unwrap();
            let (fu, _) = AffineMap::footprint(&t, (100, 100), step);
            assert!((fu - want).abs() < 1e-6, "{deg}: {fu}");
        }
    }
}
