//! The large-source fixtures of #235 and their rows on the front-end sheet.
//!
//! Generated at sheet time into `target/aliasing/`, never committed (4K PNGs):
//!
//! - `zoneplate-4096.png`: a 4096 × 2560 zone plate whose rings reach the source's
//!   Nyquist frequency at its corners. Shrunk to 320 × 200, everything beyond the centre
//!   is finer than the target can show, so the correct picture there is flat grey.
//! - `title-4096.png` and `title-grain-4096.png`: the `title` fixture upscaled to 4096 px
//!   (bilinear), without and with Gaussian grain of σ = 14/255 from a seeded xorshift
//!   PRNG, one value per pixel added to all three channels (luminance grain: with it,
//!   png2c64's own sampling leaks 24.4 %, the evidence's 24.5 %; independent grain per
//!   channel leaks 30 %). The share of output pixels that change between the two is the
//!   grain that leaks into the dither.
//!
//! Each is solved three ways at the baselines' recipe (multicolor, checker 0.7, error
//! clamp 0.1, gamma 1.5, pepto): **exact** (`Downscale::Exact`: png2c64's four Mitchell
//! taps on the original, png2c64's own front end), **auto** (`Downscale::Auto`: the mip
//! level, #235) and the **reference**, the source box-averaged in linear light to the
//! 320 × 200 grid first (8-bit sRGB, as a file would hold it), then the same front end
//! and solver. The grain leakage is also given for the source box-averaged on its 8-bit
//! sRGB values, as an image editor's box filter does: that is how the evidence's 2.8 %
//! was made (it measures 2.5 % here).

use crate::frontend::{layer_from_rgba, render_layers};
use crate::repo_root;
use convert64_core::layer::{DitherKind, Downscale};
use convert64_core::mode::Mode;
use convert64_core::palette::{linear_to_srgb8, srgb8_to_linear};
use image::{RgbImage, RgbaImage};
use std::path::PathBuf;

/// The grain: σ in 8-bit steps (14/255), and the PRNG seed.
const GRAIN_SIGMA: f64 = 14.0;
const GRAIN_SEED: u64 = 0x2350_6a1e_5eed_0235;

/// The gate (#235): at most this share of output pixels may change with the grain under
/// Auto, in percent.
pub(crate) const LEAKAGE_GATE: f64 = 5.0;

/// The baselines' recipe for these rows.
const MODE: Mode = Mode::MulticolorBitmap;
const KIND: DitherKind = DitherKind::Checker;
const STRENGTH: f32 = 0.7;

/// A `w × h` zone plate, 8-bit sRGB grey: `0.5 + 0.5 cos(k r²)` with `r` from the
/// centre, `k` such that the local frequency is 0.5 cycles per pixel at the corners.
pub(crate) fn zone_plate(w: u32, h: u32) -> RgbaImage {
    let (cx, cy) = (f64::from(w) / 2.0, f64::from(h) / 2.0);
    let rmax = cx.hypot(cy);
    let k = std::f64::consts::PI / (2.0 * rmax);
    RgbaImage::from_fn(w, h, |x, y| {
        let (dx, dy) = (f64::from(x) + 0.5 - cx, f64::from(y) + 0.5 - cy);
        let v = 0.5 + 0.5 * (k * (dx * dx + dy * dy)).cos();
        let b = (v * 255.0).round() as u8;
        image::Rgba([b, b, b, 255])
    })
}

/// `src` scaled to `w` pixels wide (the aspect kept), bilinear on the 8-bit values.
pub(crate) fn upscaled(src: &RgbaImage, w: u32) -> RgbaImage {
    let (sw, sh) = src.dimensions();
    let h = (u64::from(sh) * u64::from(w) / u64::from(sw)) as u32;
    let (rx, ry) = (f64::from(sw) / f64::from(w), f64::from(sh) / f64::from(h));
    let at = |x: i64, y: i64| {
        src.get_pixel(
            x.clamp(0, i64::from(sw) - 1) as u32,
            y.clamp(0, i64::from(sh) - 1) as u32,
        )
        .0
    };
    RgbaImage::from_fn(w, h, |x, y| {
        let (u, v) = (
            (f64::from(x) + 0.5) * rx - 0.5,
            (f64::from(y) + 0.5) * ry - 0.5,
        );
        let (x0, y0) = (u.floor(), v.floor());
        let (tx, ty) = (u - x0, v - y0);
        let (x0, y0) = (x0 as i64, y0 as i64);
        let (a, b, c, d) = (
            at(x0, y0),
            at(x0 + 1, y0),
            at(x0, y0 + 1),
            at(x0 + 1, y0 + 1),
        );
        let mut out = [255u8; 4];
        for i in 0..3 {
            let top = f64::from(a[i]) + (f64::from(b[i]) - f64::from(a[i])) * tx;
            let bottom = f64::from(c[i]) + (f64::from(d[i]) - f64::from(c[i])) * tx;
            out[i] = (top + (bottom - top) * ty).round().clamp(0.0, 255.0) as u8;
        }
        image::Rgba(out)
    })
}

/// xorshift64*: a deterministic PRNG with no dependency.
struct Rng(u64);

impl Rng {
    /// Uniform in (0, 1].
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        let v = self.0.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 11;
        (v as f64 + 1.0) / (1u64 << 53) as f64
    }

    /// Standard normal (Box–Muller).
    fn gauss(&mut self) -> f64 {
        let (u1, u2) = (self.next(), self.next());
        (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
    }
}

/// `img` with Gaussian grain of σ = `sigma` 8-bit steps, one value per pixel added to
/// its three channels.
pub(crate) fn grained(img: &RgbaImage, sigma: f64, seed: u64) -> RgbaImage {
    let mut rng = Rng(seed | 1);
    let mut out = img.clone();
    for p in out.pixels_mut() {
        let g = sigma * rng.gauss();
        for c in &mut p.0[..3] {
            *c = (f64::from(*c) + g).round().clamp(0.0, 255.0) as u8;
        }
    }
    out
}

/// Where [`area_averaged`] averages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Space {
    /// Linear light, as the compositor and the mip levels do.
    Linear,
    /// The 8-bit sRGB values themselves, as an image editor's box filter does.
    Srgb,
}

/// `img` box-averaged to `tw × th` (each target pixel the mean of the source area it
/// covers, partial pixels weighted by their overlap) in `space`, as 8-bit sRGB.
pub(crate) fn area_averaged(img: &RgbaImage, tw: u32, th: u32, space: Space) -> RgbaImage {
    let (sw, sh) = img.dimensions();
    // Per target index, the source pixels it overlaps and how much.
    let spans = |s: u32, t: u32| -> Vec<Vec<(usize, f64)>> {
        let r = f64::from(s) / f64::from(t);
        (0..t)
            .map(|i| {
                let (a, b) = (f64::from(i) * r, f64::from(i + 1) * r);
                (a.floor() as u32..(b.ceil() as u32).min(s))
                    .map(|j| {
                        let w = (b.min(f64::from(j + 1)) - a.max(f64::from(j))).max(0.0);
                        (j as usize, w / r)
                    })
                    .collect()
            })
            .collect()
    };
    let (xs, ys) = (spans(sw, tw), spans(sh, th));
    let decode = |v: u8| match space {
        Space::Linear => f64::from(srgb8_to_linear(v)),
        Space::Srgb => f64::from(v) / 255.0,
    };
    let encode = |v: f64| match space {
        Space::Linear => linear_to_srgb8(v as f32),
        Space::Srgb => (v * 255.0).round().clamp(0.0, 255.0) as u8,
    };
    let px: Vec<[f64; 3]> = img
        .pixels()
        .map(|p| [0, 1, 2].map(|i| decode(p.0[i])))
        .collect();
    // Across first, then down.
    let rows: Vec<Vec<[f64; 3]>> = (0..sh as usize)
        .map(|y| {
            xs.iter()
                .map(|span| {
                    let mut acc = [0.0; 3];
                    for &(x, w) in span {
                        let p = px[y * sw as usize + x];
                        for i in 0..3 {
                            acc[i] += w * p[i];
                        }
                    }
                    acc
                })
                .collect()
        })
        .collect();
    RgbaImage::from_fn(tw, th, |x, y| {
        let mut acc = [0.0; 3];
        for &(r, w) in &ys[y as usize] {
            let p = rows[r][x as usize];
            for i in 0..3 {
                acc[i] += w * p[i];
            }
        }
        let [r, g, b] = acc.map(encode);
        image::Rgba([r, g, b, 255])
    })
}

/// The share of pixels (in percent) that differ between two solves.
pub(crate) fn changed(a: &RgbImage, b: &RgbImage) -> f64 {
    let n = a.pixels().zip(b.pixels()).filter(|(x, y)| x != y).count();
    100.0 * n as f64 / f64::from(a.width() * a.height())
}

/// `img` (or, for a reference, `img` box-averaged to 320 × 200 in `space` first) fitted
/// to the screen, sampled as `d` says, and solved: `(prepared, solved)`.
fn render(
    name: &str,
    img: &RgbaImage,
    d: Downscale,
    reference: Option<Space>,
) -> Result<(RgbImage, RgbImage), String> {
    let averaged = reference.map(|s| area_averaged(img, 320, 200, s));
    let layer = layer_from_rgba(name, averaged.as_ref().unwrap_or(img), KIND, STRENGTH, d)?;
    render_layers(&[layer], MODE, true)
}

/// One source solved the three ways: `(prepared, solved)` for exact, auto and the
/// linear-light area-averaged reference.
pub(crate) fn three_ways(name: &str, img: &RgbaImage) -> Result<[(RgbImage, RgbImage); 3], String> {
    Ok([
        render(name, img, Downscale::Exact, None)?,
        render(name, img, Downscale::Auto, None)?,
        render(name, img, Downscale::Auto, Some(Space::Linear))?,
    ])
}

/// The grain leakage (percent of output pixels the grain changes) of exact, auto, the
/// linear-light reference and the sRGB-space one, from the clean and grainy sources'
/// three-way solves and the sources themselves.
pub(crate) fn leakage(
    (clean, clean3): (&RgbaImage, &[(RgbImage, RgbImage); 3]),
    (grainy, grainy3): (&RgbaImage, &[(RgbImage, RgbImage); 3]),
) -> Result<[f64; 4], String> {
    let srgb = |img| render("title", img, Downscale::Auto, Some(Space::Srgb));
    let (a, b) = (srgb(clean)?, srgb(grainy)?);
    Ok([
        changed(&clean3[0].1, &grainy3[0].1),
        changed(&clean3[1].1, &grainy3[1].1),
        changed(&clean3[2].1, &grainy3[2].1),
        changed(&a.1, &b.1),
    ])
}

/// The fixtures `(zone plate, title, title with grain)`, also written to
/// `target/aliasing/` (for a look, or for png2c64).
pub(crate) fn fixtures() -> Result<(RgbaImage, RgbaImage, RgbaImage), String> {
    let (zone, title, grain) = sources(4096)?;
    let dir: PathBuf = repo_root().join("target/aliasing");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for (img, name) in [
        (&zone, "zoneplate-4096.png"),
        (&title, "title-4096.png"),
        (&grain, "title-grain-4096.png"),
    ] {
        let p = dir.join(name);
        img.save(&p).map_err(|e| format!("{}: {e}", p.display()))?;
    }
    Ok((zone, title, grain))
}

/// The sources at `width` px: the zone plate (at 16:10), the title upscaled, and that
/// with grain.
fn sources(width: u32) -> Result<(RgbaImage, RgbaImage, RgbaImage), String> {
    let path = repo_root().join("fixtures/title/source.png");
    let title = image::open(&path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .to_rgba8();
    let zone = zone_plate(width, width * 5 / 8);
    let title = upscaled(&title, width);
    let grain = grained(&title, GRAIN_SIGMA, GRAIN_SEED);
    Ok((zone, title, grain))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn area_averaging_keeps_flat_fields_and_halves_pairs() {
        let flat = RgbaImage::from_pixel(37, 23, image::Rgba([90, 140, 200, 255]));
        for space in [Space::Linear, Space::Srgb] {
            let out = area_averaged(&flat, 10, 7, space);
            assert!(out.pixels().all(|p| p.0 == [90, 140, 200, 255]));
        }
        let stripes = RgbaImage::from_fn(4, 1, |x, _| {
            let v = if x % 2 == 0 { 0 } else { 255 };
            image::Rgba([v, v, v, 255])
        });
        // Black and white average to linear 0.5, sRGB 188; to 128 on the sRGB values.
        let out = area_averaged(&stripes, 2, 1, Space::Linear);
        assert_eq!(out.get_pixel(0, 0).0[0], linear_to_srgb8(0.5));
        let out = area_averaged(&stripes, 2, 1, Space::Srgb);
        assert_eq!(out.get_pixel(0, 0).0[0], 128);
    }

    #[test]
    fn grain_is_deterministic_and_has_its_sigma() {
        let grey = RgbaImage::from_pixel(256, 256, image::Rgba([128, 128, 128, 255]));
        let (a, b) = (
            grained(&grey, GRAIN_SIGMA, 7),
            grained(&grey, GRAIN_SIGMA, 7),
        );
        assert_eq!(a, b);
        let n = f64::from(256 * 256);
        let sd = (a
            .pixels()
            .map(|p| (f64::from(p.0[0]) - 128.0).powi(2))
            .sum::<f64>()
            / n)
            .sqrt();
        assert!((sd - GRAIN_SIGMA).abs() < 0.3, "{sd}");
        assert!(a.pixels().all(|p| p.0[0] == p.0[1] && p.0[1] == p.0[2]));
    }

    /// #235's gate on the sheet's sources (4096 px, a 12.8× shrink): under Auto at most
    /// 5 % of the output pixels change with the grain, against 24 % under Exact.
    #[test]
    fn grain_leakage_under_auto_is_within_the_gate() {
        let (_, title, grain) = sources(4096).unwrap();
        let clean = three_ways("title", &title).unwrap();
        let grainy = three_ways("title", &grain).unwrap();
        let [exact, auto, reference, srgb] = leakage((&title, &clean), (&grain, &grainy)).unwrap();
        assert!(
            auto <= LEAKAGE_GATE && auto < exact / 4.0,
            "exact {exact:.1} %, auto {auto:.1} %, reference {reference:.1} % (sRGB {srgb:.1} %)"
        );
    }
}
