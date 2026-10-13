//! Separable bicubic resampling in linear light, ported from png2c64's `scale.cpp`.

use super::{Image, Rgb};

/// The cubic kernel (`scale::Kernel`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kernel {
    /// Mitchell–Netravali, B = C = 1/3 (png2c64's default).
    #[default]
    MitchellNetravali,
    /// Catmull–Rom, B = 0, C = 1/2.
    CatmullRom,
}

/// The Mitchell–Netravali kernel, B = C = 1/3: the compositor resamples every layer with it
/// too (ADR-012), so a layer fitted to the screen is scaled bit for bit as png2c64 scales.
pub(crate) fn mitchell(x: f32) -> f32 {
    let x = x.abs();
    if x < 1.0 {
        (7.0 * x * x * x - 12.0 * x * x + 16.0 / 3.0) / 6.0
    } else if x < 2.0 {
        (-7.0 / 3.0 * x * x * x + 12.0 * x * x - 20.0 * x + 32.0 / 3.0) / 6.0
    } else {
        0.0
    }
}

fn catmull_rom(x: f32) -> f32 {
    let x = x.abs();
    if x < 1.0 {
        1.5 * x * x * x - 2.5 * x * x + 1.0
    } else if x < 2.0 {
        -0.5 * x * x * x + 2.5 * x * x - 4.0 * x + 2.0
    } else {
        0.0
    }
}

/// One output sample from four taps along an axis (`scale_horizontal`'s inner loop):
/// `tap(i)` is the source pixel at clamped index `i`.
fn sample(pos: f32, len: usize, kernel: fn(f32) -> f32, tap: impl Fn(usize) -> Rgb) -> Rgb {
    let center = pos.floor() as i64;
    let mut sum = [0.0f32; 3];
    let mut weight_sum = 0.0f32;
    for k in -1..=2 {
        let s = (center + k).clamp(0, len as i64 - 1);
        let w = kernel(pos - s as f32);
        let v = tap(s as usize);
        for c in 0..3 {
            sum[c] += v[c] * w;
        }
        weight_sum += w;
    }
    if weight_sum > 0.0 {
        let inv = 1.0 / weight_sum;
        sum = sum.map(|v| v * inv);
    }
    sum
}

/// `bicubic`: horizontal pass to `dst_w`, then vertical to `dst_h`.
///
/// # Panics
/// If either target size is 0.
pub fn bicubic(src: &Image, dst_w: usize, dst_h: usize, kernel: Kernel) -> Image {
    assert!(dst_w > 0 && dst_h > 0, "target dimensions must be non-zero");
    let f: fn(f32) -> f32 = match kernel {
        Kernel::MitchellNetravali => mitchell,
        Kernel::CatmullRom => catmull_rom,
    };
    let ratio = src.w as f32 / dst_w as f32;
    let mut mid = Vec::with_capacity(dst_w * src.h);
    for y in 0..src.h {
        for x in 0..dst_w {
            let pos = (x as f32 + 0.5) * ratio - 0.5;
            mid.push(sample(pos, src.w, f, |s| src.at(s, y)));
        }
    }
    let ratio = src.h as f32 / dst_h as f32;
    let mut out = vec![[0.0; 3]; dst_w * dst_h];
    for y in 0..dst_h {
        let pos = (y as f32 + 0.5) * ratio - 0.5;
        for x in 0..dst_w {
            out[y * dst_w + x] = sample(pos, src.h, f, |s| mid[s * dst_w + x]);
        }
    }
    Image::new(dst_w, dst_h, out)
}

#[cfg(test)]
mod tests {
    use super::super::testdata as t;
    use super::*;

    fn image(w: usize, h: usize, bits: &[[u32; 3]]) -> Image {
        Image::new(w, h, bits.iter().map(|p| p.map(f32::from_bits)).collect())
    }

    #[test]
    fn mitchell_and_catmull_rom_match_png2c64_bit_for_bit() {
        let src = image(7, 5, t::SCALE_SRC);
        let a = bicubic(&src, 3, 4, Kernel::MitchellNetravali);
        for (i, p) in a.px.iter().enumerate() {
            assert_eq!(
                p.map(f32::to_bits),
                t::SCALE_MITCHELL_3X4[i],
                "mitchell {i}"
            );
        }
        let b = bicubic(&src, 9, 6, Kernel::CatmullRom);
        for (i, p) in b.px.iter().enumerate() {
            assert_eq!(
                p.map(f32::to_bits),
                t::SCALE_CATMULL_9X6[i],
                "catmull-rom {i}"
            );
        }
    }
}
