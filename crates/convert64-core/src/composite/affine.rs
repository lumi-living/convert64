//! Resampling of a **rotated** layer onto the super grid (#198, ADR-011).
//!
//! An axis-aligned layer (`rotate == 0`) takes the separable path in `resample`; a
//! rotated one cannot, so each super pixel is inverse-mapped to layer space and sampled
//! there:
//!
//! - **Kernel.** Catmull-Rom (bicubic, B = 0, C = ½), 4 × 4 taps on the premultiplied
//!   source; samples clamp to the layer's edge pixels, as on the separable path.
//! - **Prefilter.** Where the layer is scaled down, the source is first reduced to a
//!   box-filtered mip level chosen once per layer and axis (a rip-map: halved along u
//!   and v independently, an odd last pixel paired with itself). An affine map has the
//!   same footprint everywhere, so one level fits the whole layer: the level is
//!   `round(log2(footprint))` along each layer axis, where the footprint is how many
//!   source pixels one super pixel spans along it. The kernel then reads at most
//!   √2 mip pixels per super pixel, so it neither aliases much nor blurs much.
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

/// Catmull-Rom weights for the four taps around a sample `t` (0–1) past the second.
#[inline(always)]
fn catmull_rom(t: f32) -> [f32; 4] {
    let t2 = t * t;
    let t3 = t2 * t;
    [
        0.5 * (-t3 + 2.0 * t2 - t),
        0.5 * (3.0 * t3 - 5.0 * t2 + 2.0),
        0.5 * (-3.0 * t3 + 4.0 * t2 + t),
        0.5 * (t3 - t2),
    ]
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
}

impl AffineMap {
    /// The map for a source of `size` placed by `t` onto `grid`.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the box is clamped to the grid before casting"
    )]
    pub fn new(t: &Transform, size: (u32, u32), grid: SuperGrid) -> Self {
        let [a, b, c, d, e, f] = t.canvas_to_layer_affine(size);
        let step = grid.super_px_in_canvas();
        let (sx, sy) = (f64::from(step.x), f64::from(step.y));
        // Super (i, j) has its centre at canvas ((i + ½)·sx, (j + ½)·sy).
        let u = [a * sx, b * sy, a * sx * 0.5 + b * sy * 0.5 + c];
        let v = [d * sx, e * sy, d * sx * 0.5 + e * sy * 0.5 + f];
        // The footprint (one super pixel) spans u[0] + u[1] along u, v[0] + v[1] along v.
        let span = (u[0].abs() + u[1].abs(), v[0].abs() + v[1].abs());
        let level = |s: f64| {
            let mut l = 0;
            while l < 14 && s / f64::from(1u32 << l) > std::f64::consts::SQRT_2 {
                l += 1;
            }
            l
        };
        let (lu, lv) = (level(span.0), level(span.1));
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
        Self {
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
    /// the mip and the kernel: two mip pixels each way.
    pub fn reach_src(&self) -> f64 {
        f64::from(2u32 << self.level.0.max(self.level.1))
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

    /// The Catmull-Rom sample of `src` (mip level [`Self::level`], `mw × mh`) at layer
    /// `(u, v)` (level-0 coordinates), edges clamped. The 4 × 4 taps are summed row by
    /// row, `0 + w0·v0 + … + w3·v3`, the same way whether or not they needed clamping.
    #[inline(always)]
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "coordinates are clamped to the source before indexing"
    )]
    fn sample<T: Px>(&self, src: &[T], mw: u32, mh: u32, u: f64, v: f64) -> T {
        let x = u * self.inv_mip.0 - 0.5;
        let y = v * self.inv_mip.1 - 0.5;
        let (fx, fy) = (x.floor(), y.floor());
        let wx = catmull_rom((x - fx) as f32);
        let wy = catmull_rom((y - fy) as f32);
        let (x0, y0) = (fx as i64 - 1, fy as i64 - 1);
        let (mw, mh) = (i64::from(mw), i64::from(mh));
        let taps: [[T; 4]; 4] = if x0 >= 0 && y0 >= 0 && x0 + 3 < mw && y0 + 3 < mh {
            // Inside: four runs of four pixels.
            let w = mw as usize;
            let base = y0 as usize * w + x0 as usize;
            std::array::from_fn(|k| src[base + k * w..][..4].try_into().expect("four taps"))
        } else {
            let xs: [usize; 4] = std::array::from_fn(|k| (x0 + k as i64).clamp(0, mw - 1) as usize);
            std::array::from_fn(|k| {
                let r = (y0 + k as i64).clamp(0, mh - 1) as usize * mw as usize;
                xs.map(|x| src[r + x])
            })
        };
        T::taps4x4(&taps, wx, wy)
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

/// `src` (`w × h`, row-major) box-reduced by 2 along x `lx` times and along y `ly`
/// times; an odd last pixel is averaged with itself (the edge clamp). Returns the
/// reduced pixels and their size.
pub(crate) fn mip<T: Px>(src: &[T], w: u32, h: u32, lx: u32, ly: u32) -> (Vec<T>, (u32, u32)) {
    let (mut px, mut w, mut h) = (src.to_vec(), w as usize, h as usize);
    for _ in 0..lx {
        let nw = w.div_ceil(2);
        let mut out = vec![T::ZERO; nw * h];
        for (o, line) in out.chunks_mut(nw).zip(px.chunks(w)) {
            for (i, o) in o.iter_mut().enumerate() {
                o.madd(0.5, line[2 * i]);
                o.madd(0.5, line[(2 * i + 1).min(w - 1)]);
            }
        }
        (px, w) = (out, nw);
    }
    for _ in 0..ly {
        let nh = h.div_ceil(2);
        let mut out = vec![T::ZERO; w * nh];
        for (y, o) in out.chunks_mut(w).enumerate() {
            let (a, b) = (2 * y, (2 * y + 1).min(h - 1));
            for (x, o) in o.iter_mut().enumerate() {
                o.madd(0.5, px[a * w + x]);
                o.madd(0.5, px[b * w + x]);
            }
        }
        (px, h) = (out, nh);
    }
    let size = (
        u32::try_from(w).expect("mip of a u32 size"),
        u32::try_from(h).expect("mip of a u32 size"),
    );
    (px, size)
}

/// Resamples premultiplied colour (the mip level `map` samples, `mw × mh`) onto
/// `map.rect` into planes, scaled by coverage, alpha clamped to `0..=1` and colour to
/// `≥ 0` (Catmull-Rom rings): the rotated counterpart of `resample::resample_planes`,
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
/// `map.rect` into `out`, clamped to `0..=1`. Like the separable path, no coverage is
/// applied: outside the quad the mask continues its edge values, and the colour's
/// coverage hides it.
pub(crate) fn resample_mask_affine(
    src: &[f32],
    (mw, mh): (u32, u32),
    map: &AffineMap,
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
            map.row(j, 0..ow, |k, u, v| {
                row[k] = map.sample(src, mw, mh, u, v).clamp(0.0, 1.0);
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Vec2;
    use crate::mode::Mode;

    #[test]
    fn catmull_rom_interpolates_and_sums_to_one() {
        assert_eq!(catmull_rom(0.0), [0.0, 1.0, 0.0, 0.0]);
        for i in 0..=16 {
            let t = i as f32 / 16.0;
            let w = catmull_rom(t);
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
                let map = AffineMap::new(&t, size, grid);
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

    #[test]
    fn mip_halves_each_axis_and_keeps_flat_fields() {
        let src = vec![[0.5f32, 0.25, 1.0, 1.0]; 7 * 5];
        let (px, size) = mip(&src, 7, 5, 2, 1);
        assert_eq!(size, (2, 3));
        assert!(px.iter().all(|p| *p == [0.5, 0.25, 1.0, 1.0]));
        let ramp: Vec<f32> = (0..8).map(|x| x as f32).collect();
        let (px, size) = mip(&ramp, 8, 1, 1, 0);
        assert_eq!((px, size), (vec![0.5, 2.5, 4.5, 6.5], (4, 1)));
    }

    #[test]
    fn mip_level_follows_the_downscale() {
        let grid = SuperGrid::new(Mode::HiresBitmap, 1).unwrap();
        for (scale, want) in [(1.0, 0), (0.5, 1), (0.3, 2), (0.1, 3), (2.0, 0)] {
            let t = Transform::new(Vec2::ZERO, Vec2::new(scale, 1.0), false, false)
                .unwrap()
                .with_rotation(90.0)
                .unwrap();
            let map = AffineMap::new(&t, (100, 100), grid);
            // Turned 90°, the canvas's y runs along the layer's u, which is scaled.
            assert_eq!(map.level, (want, 0), "{scale}");
        }
    }
}
