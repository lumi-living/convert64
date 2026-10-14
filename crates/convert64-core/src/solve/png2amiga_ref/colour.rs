//! Colour conversions, ported from png2amiga's `color_space.hpp`.
//!
//! What differs from png2c64's (`super::super::png2c64_ref::colour`):
//!
//! - **The cube root** is png2amiga's `fast_cbrt4`: a bit-trick seed refined by two
//!   Newton steps (`y ← ⅓ (2y + x / y²)`), not a correctly rounded `cbrt`. Its NEON,
//!   SSE2, WASM and scalar arms compute the same IEEE operations in the same order, so
//!   one scalar port matches all of them.
//! - **Fused multiply-adds.** png2amiga calls `std::fma` explicitly in the OKLab matrix
//!   rows (`fma_dot3`), in `oklab_to_linear` and in every squared distance
//!   (`fma_dist_sq`). An explicit `std::fma` is fused even in a strict
//!   (`-ffp-contract=off`) build; Rust's [`f32::mul_add`] is the same correctly rounded
//!   operation. Everything else stays unfused, as in the strict build.
//! - The linear → LMS step is three vector multiplies and two adds, unfused.
//!
//! The 8-bit sRGB table, `srgb_to_linear` and `linear_to_srgb` are png2c64's, literally
//! identical, and shared.

use super::super::png2c64_ref::colour as p2c;
use super::Rgb;

pub use p2c::{hex_to_linear, linear_to_srgb, srgb_to_linear, srgb8_to_linear};

/// OKLab coordinates `[L, a, b]`, png2amiga's `color_space::OKLab`.
pub type Lab = [f32; 3];

/// `fast_cbrt4`'s scalar arm: the Walczyk-style seed (shift-and-add ⅓ of the exponent
/// bits) and two Newton iterations; 0 stays exactly 0.
#[inline]
pub fn fast_cbrt(x: f32) -> f32 {
    let b = x.to_bits();
    let sign = b & 0x8000_0000;
    let absbits = b & 0x7fff_ffff;
    let mut t = (absbits >> 2).wrapping_add(absbits >> 4);
    t = t.wrapping_add(t >> 4);
    t = t.wrapping_add(t >> 8);
    let seed = 0x2a51_37a0_u32.wrapping_add(t);
    let absx = f32::from_bits(absbits);
    let mut y = f32::from_bits(seed);
    // png2amiga's 0.33333333f.
    let third = 0.333_333_33_f32;
    y = third * (2.0 * y + absx / (y * y));
    y = third * (2.0 * y + absx / (y * y));
    let mask = if absbits != 0 { u32::MAX } else { 0 };
    f32::from_bits((y.to_bits() & mask) | sign)
}

/// `fma_dot3`: `c0·x0 + c1·x1 + c2·x2` as `fma(c0, x0, fma(c1, x1, c2·x2))`.
#[inline]
pub fn fma_dot3(c0: f32, x0: f32, c1: f32, x1: f32, c2: f32, x2: f32) -> f32 {
    c0.mul_add(x0, c1.mul_add(x1, c2 * x2))
}

/// `fma_dist_sq(dx, dy, dz)`: `fma(dx, dx, fma(dy, dy, dz·dz))`.
#[inline]
pub fn fma_dist_sq(dx: f32, dy: f32, dz: f32) -> f32 {
    dx.mul_add(dx, dy.mul_add(dy, dz * dz))
}

/// `fma_dist_sq(OKLab a, OKLab b)`.
#[inline]
pub fn dist2(a: Lab, b: Lab) -> f32 {
    fma_dist_sq(a[0] - b[0], a[1] - b[1], a[2] - b[2])
}

/// `lms_cbrt_to_oklab`.
#[inline]
#[expect(
    clippy::excessive_precision,
    reason = "png2amiga's literals, kept digit for digit; each rounds to the same f32"
)]
fn lms_cbrt_to_oklab(l: f32, m: f32, s: f32) -> Lab {
    [
        fma_dot3(0.210_454_255_3, l, 0.793_617_785_0, m, -0.004_072_046_8, s),
        fma_dot3(1.977_998_495_1, l, -2.428_592_205_0, m, 0.450_593_709_9, s),
        fma_dot3(0.025_904_037_1, l, 0.782_771_766_2, m, -0.808_675_766_0, s),
    ]
}

/// Linear RGB to OKLab (`linear_to_oklab`).
#[inline]
#[expect(
    clippy::excessive_precision,
    reason = "png2amiga's literals, kept digit for digit; each rounds to the same f32"
)]
pub fn linear_to_oklab(c: Rgb) -> Lab {
    // `f32x4{..} * r + f32x4{..} * g + f32x4{..} * b`, lane by lane, unfused.
    let l = 0.412_221_470_8 * c[0] + 0.536_332_536_3 * c[1] + 0.051_445_992_9 * c[2];
    let m = 0.211_903_498_2 * c[0] + 0.680_699_545_1 * c[1] + 0.107_396_956_6 * c[2];
    let s = 0.088_302_461_9 * c[0] + 0.281_718_837_6 * c[1] + 0.629_978_700_5 * c[2];
    lms_cbrt_to_oklab(fast_cbrt(l), fast_cbrt(m), fast_cbrt(s))
}

/// OKLab to linear RGB (`oklab_to_linear`).
#[inline]
#[expect(
    clippy::excessive_precision,
    reason = "png2amiga's literals, kept digit for digit; each rounds to the same f32"
)]
pub fn oklab_to_linear(lab: Lab) -> Rgb {
    let [ll, a, b] = lab;
    let l_ = 0.396_337_777_4_f32.mul_add(a, 0.215_803_757_3_f32.mul_add(b, ll));
    let m_ = (-0.105_561_345_8_f32).mul_add(a, (-0.063_854_172_8_f32).mul_add(b, ll));
    let s_ = (-0.089_484_177_5_f32).mul_add(a, (-1.291_485_548_0_f32).mul_add(b, ll));
    let (l, m, s) = (l_ * l_ * l_, m_ * m_ * m_, s_ * s_ * s_);
    [
        fma_dot3(4.076_741_662_1, l, -3.307_711_591_3, m, 0.230_969_929_2, s),
        fma_dot3(-1.268_438_004_6, l, 2.609_757_401_1, m, -0.341_319_396_5, s),
        fma_dot3(-0.004_196_086_3, l, -0.703_418_614_7, m, 1.707_614_701_0, s),
    ]
}

/// Each channel clamped to `0..=1` (`Color3f::clamped`).
#[inline]
pub fn clamped(c: Rgb) -> Rgb {
    c.map(|v| v.clamp(0.0, 1.0))
}

/// `linear_to_srgb(Color3f)`, per channel.
#[inline]
pub fn linear_to_srgb3(c: Rgb) -> Rgb {
    c.map(linear_to_srgb)
}

/// `srgb_to_linear(Color3f)`, per channel.
#[inline]
pub fn srgb_to_linear3(c: Rgb) -> Rgb {
    c.map(srgb_to_linear)
}

/// `std::pow` on `float`, as png2c64's port computes it (correctly rounded).
pub(crate) use p2c::powf;

/// `std::sin` on `float`, correctly rounded (computed in `f64`, rounded once).
#[expect(
    clippy::cast_possible_truncation,
    reason = "the f64 result is rounded to f32 on purpose"
)]
#[inline]
pub(crate) fn sinf(x: f32) -> f32 {
    f64::from(x).sin() as f32
}

/// `std::cos` on `float`, correctly rounded.
#[expect(
    clippy::cast_possible_truncation,
    reason = "the f64 result is rounded to f32 on purpose"
)]
#[inline]
pub(crate) fn cosf(x: f32) -> f32 {
    f64::from(x).cos() as f32
}

/// `std::atan2` on `float`, correctly rounded.
#[expect(
    clippy::cast_possible_truncation,
    reason = "the f64 result is rounded to f32 on purpose"
)]
#[inline]
pub(crate) fn atan2f(y: f32, x: f32) -> f32 {
    f64::from(y).atan2(f64::from(x)) as f32
}

/// png2amiga's `kC64Pepto` table, equal to png2c64's Pepto and to
/// [`crate::palette::Palette::pepto_pal`].
pub const PEPTO_HEX: [u32; 16] = p2c::PEPTO_HEX;

/// A palette's 16 colours in linear light (`palette_linear`).
pub fn palette_linear(hex: &[u32; 16]) -> [Rgb; 16] {
    hex.map(hex_to_linear)
}

/// The linear palette of one of our palettes, through the sRGB table.
pub fn palette_of(p: &crate::palette::Palette) -> [Rgb; 16] {
    p2c::palette_of(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fast_cbrt_is_close_to_the_cube_root_and_keeps_zero() {
        assert_eq!(fast_cbrt(0.0).to_bits(), 0);
        for i in 1..2000 {
            let x = i as f32 / 1000.0;
            let r = fast_cbrt(x);
            assert!((r - x.cbrt()).abs() < 1e-5, "{x}: {r}");
        }
    }

    #[test]
    fn oklab_round_trips() {
        for &c in &[[0.2f32, 0.5, 0.9], [1.0, 1.0, 1.0], [0.01, 0.0, 0.3]] {
            let back = oklab_to_linear(linear_to_oklab(c));
            for k in 0..3 {
                assert!((back[k] - c[k]).abs() < 1e-4, "{c:?} → {back:?}");
            }
        }
    }
}
