//! Colour conversions and the Pepto palette, ported from png2c64's `color_space.hpp` and
//! `palette.hpp`.

use super::Rgb;
use std::sync::OnceLock;

/// OKLab coordinates `[L, a, b]`, png2c64's `color_space::OKLab`.
pub type Lab = [f32; 3];

/// `std::pow` on `float`, correctly rounded (computed in `f64`, rounded once).
#[expect(
    clippy::cast_possible_truncation,
    reason = "the f64 result is rounded to f32 on purpose"
)]
pub(crate) fn powf(x: f32, y: f32) -> f32 {
    f64::from(x).powf(f64::from(y)) as f32
}

/// `std::cbrt` on `float`, correctly rounded.
#[expect(
    clippy::cast_possible_truncation,
    reason = "the f64 result is rounded to f32 on purpose"
)]
pub(crate) fn cbrtf(x: f32) -> f32 {
    f64::from(x).cbrt() as f32
}

/// One sRGB channel in `0..=1` to linear light (`srgb_to_linear`).
pub fn srgb_to_linear(s: f32) -> f32 {
    if s <= 0.04045 {
        s / 12.92
    } else {
        powf((s + 0.055) / 1.055, 2.4)
    }
}

/// One linear channel to sRGB in `0..=1` (`linear_to_srgb`).
pub fn linear_to_srgb(l: f32) -> f32 {
    if l <= 0.003_130_8 {
        l * 12.92
    } else {
        1.055 * powf(l, 1.0 / 2.4) - 0.055
    }
}

/// The 256-entry byte → linear table (`srgb_lut`).
fn lut() -> &'static [f32; 256] {
    static LUT: OnceLock<[f32; 256]> = OnceLock::new();
    LUT.get_or_init(|| {
        let mut t = [0.0; 256];
        for (i, v) in t.iter_mut().enumerate() {
            *v = srgb_to_linear(f32::from(i as u8) / 255.0);
        }
        t
    })
}

/// An 8-bit sRGB colour to linear light (`srgb_u8_to_linear`).
pub fn srgb8_to_linear(r: u8, g: u8, b: u8) -> Rgb {
    let t = lut();
    [t[usize::from(r)], t[usize::from(g)], t[usize::from(b)]]
}

/// A `0xRRGGBB` colour to linear light (`srgb_hex_to_linear`).
pub fn hex_to_linear(hex: u32) -> Rgb {
    let [_, r, g, b] = hex.to_be_bytes();
    srgb8_to_linear(r, g, b)
}

/// Linear RGB to OKLab (`linear_to_oklab`).
#[expect(
    clippy::excessive_precision,
    reason = "png2c64's literals, kept digit for digit; each rounds to the same f32"
)]
pub fn linear_to_oklab(c: Rgb) -> Lab {
    let l = 0.412_221_470_8 * c[0] + 0.536_332_536_3 * c[1] + 0.051_445_992_9 * c[2];
    let m = 0.211_903_498_2 * c[0] + 0.680_699_545_1 * c[1] + 0.107_396_956_6 * c[2];
    let s = 0.088_302_461_9 * c[0] + 0.281_718_837_6 * c[1] + 0.629_978_700_5 * c[2];
    let (l, m, s) = (cbrtf(l), cbrtf(m), cbrtf(s));
    [
        0.210_454_255_3 * l + 0.793_617_785_0 * m - 0.004_072_046_8 * s,
        1.977_998_495_1 * l - 2.428_592_205_0 * m + 0.450_593_709_9 * s,
        0.025_904_037_1 * l + 0.782_771_766_2 * m - 0.808_675_766_0 * s,
    ]
}

/// OKLab to linear RGB (`oklab_to_linear`).
#[expect(
    clippy::excessive_precision,
    reason = "png2c64's literals, kept digit for digit; each rounds to the same f32"
)]
pub fn oklab_to_linear(lab: Lab) -> Rgb {
    let [ll, a, b] = lab;
    let l = ll + 0.396_337_777_4 * a + 0.215_803_757_3 * b;
    let m = ll - 0.105_561_345_8 * a - 0.063_854_172_8 * b;
    let s = ll - 0.089_484_177_5 * a - 1.291_485_548_0 * b;
    let (l, m, s) = (l * l * l, m * m * m, s * s * s);
    [
        4.076_741_662_1 * l - 3.307_711_591_3 * m + 0.230_969_929_2 * s,
        -1.268_438_004_6 * l + 2.609_757_401_1 * m - 0.341_319_396_5 * s,
        -0.004_196_086_3 * l - 0.703_418_614_7 * m + 1.707_614_701_0 * s,
    ]
}

/// Each channel clamped to `0..=1` (`Color3f::clamped`).
pub fn clamped(c: Rgb) -> Rgb {
    c.map(|v| v.clamp(0.0, 1.0))
}

/// Squared OKLab distance, summed as png2c64 sums it: `dL² + da² + db²` left to right.
#[inline]
pub fn dist2(p: Lab, c: Lab) -> f32 {
    let (dl, da, db) = (p[0] - c[0], p[1] - c[1], p[2] - c[2]);
    dl * dl + da * da + db * db
}

/// png2c64's Pepto palette (`palette::detail::pepto_hex`), equal to
/// [`crate::palette::Palette::pepto_pal`]'s colours.
pub const PEPTO_HEX: [u32; 16] = [
    0x000000, 0xFFFFFF, 0x68372B, 0x70A4B2, 0x6F3D86, 0x588D43, 0x352879, 0xB8C76F, 0x6F4F25,
    0x433900, 0x9A6759, 0x444444, 0x6C6C6C, 0x9AD284, 0x6C5EB5, 0x959595,
];

/// A palette's 16 colours in linear light, png2c64's `to_linear_palette`.
pub fn palette_linear(hex: &[u32; 16]) -> [Rgb; 16] {
    hex.map(hex_to_linear)
}

/// The linear palette of one of our palettes, through png2c64's table (its sRGB bytes).
pub fn palette_of(p: &crate::palette::Palette) -> [Rgb; 16] {
    p.rgb_table().map(|c| srgb8_to_linear(c.r, c.g, c.b))
}

#[cfg(test)]
mod tests {
    use super::super::testdata as t;
    use super::*;

    fn bits(v: [f32; 3]) -> [u32; 3] {
        v.map(f32::to_bits)
    }

    #[test]
    fn the_srgb_table_is_png2c64_s_bit_for_bit() {
        for (i, &want) in t::LUT.iter().enumerate() {
            let got = srgb8_to_linear(i as u8, 0, 0)[0];
            assert_eq!(
                got.to_bits(),
                want,
                "{i}: {got} vs {}",
                f32::from_bits(want)
            );
        }
    }

    #[test]
    fn the_pepto_palette_is_png2c64_s_bit_for_bit() {
        let lin = palette_linear(&PEPTO_HEX);
        for (i, c) in lin.iter().enumerate() {
            assert_eq!(bits(*c), t::PALETTE_LINEAR[i], "colour {i} linear");
            assert_eq!(
                bits(linear_to_oklab(*c)),
                t::PALETTE_OKLAB[i],
                "colour {i} OKLab"
            );
        }
        assert_eq!(
            palette_of(&crate::palette::Palette::pepto_pal()),
            lin,
            "pepto-pal is png2c64's pepto"
        );
    }

    #[test]
    fn oklab_of_random_srgb_matches_png2c64() {
        for &(rgb, want) in t::RANDOM {
            let got = linear_to_oklab(srgb8_to_linear(rgb[0], rgb[1], rgb[2]));
            for k in 0..3 {
                let w = f32::from_bits(want[k]);
                assert!(
                    (got[k] - w).abs() <= 1e-5,
                    "{rgb:?}[{k}]: {} vs {w}",
                    got[k]
                );
            }
            assert_eq!(bits(got), want, "{rgb:?} bit for bit");
        }
    }

    #[test]
    fn oklab_round_trips() {
        for &(rgb, _) in t::RANDOM {
            let lin = srgb8_to_linear(rgb[0], rgb[1], rgb[2]);
            let back = oklab_to_linear(linear_to_oklab(lin));
            for k in 0..3 {
                assert!((back[k] - lin[k]).abs() < 1e-5, "{rgb:?}");
            }
        }
    }
}
