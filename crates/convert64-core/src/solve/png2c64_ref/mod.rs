//! The reference solver: a faithful port of png2c64's bitmap pipeline (#181).
//!
//! Ported from [png2c64](https://github.com/tinic/png2c64) by tinic, MIT licence, at
//! commit `1265329a573b1dfa3c2e0f3473d2aa15ffd7ac3f` (version 1.0.0.76); see
//! `THIRD_PARTY.md`. Each submodule names the file it ports. The port follows the source
//! operation for operation, in single precision, in the same order, so that our output
//! is pixel-identical to png2c64's; where png2c64 does something odd it is ported anyway
//! and marked "as png2c64". Divergences are separate, named options elsewhere.
//!
//! **Float semantics.** The reference is png2c64 built with strict IEEE arithmetic
//! (GCC 15, `-ffp-contract=off`), which is what Rust does: no fused multiply-adds, every
//! operation rounded to `f32`. png2c64's `std::cbrt` and `std::pow` on `float` are
//! computed here in `f64` and rounded once, which is correctly rounded and therefore the
//! same on every platform; macOS's `cbrtf` is correctly rounded too, and the reference
//! renders do not depend on the last bit of `powf` (`docs/reference/png2c64-reference.md`).

pub mod colour;
pub mod preprocess;
pub mod scale;

#[cfg(test)]
mod testdata;

/// Linear-light RGB, png2c64's `Color3f`.
pub type Rgb = [f32; 3];

/// An owned image of linear RGB pixels, png2c64's `Image`.
#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    /// Width in pixels.
    pub w: usize,
    /// Height in pixels.
    pub h: usize,
    /// Row-major pixels.
    pub px: Vec<Rgb>,
}

impl Image {
    /// An image of `w × h` pixels; `px` must hold `w × h` of them.
    ///
    /// # Panics
    /// If `px.len() != w * h`.
    pub fn new(w: usize, h: usize, px: Vec<Rgb>) -> Self {
        assert_eq!(px.len(), w * h, "image size");
        Self { w, h, px }
    }

    /// An image decoded from 8-bit sRGB, as png2c64's `png_io::load` does.
    ///
    /// # Panics
    /// If `rgb.len() != w * h`.
    pub fn from_srgb8(w: usize, h: usize, rgb: &[[u8; 3]]) -> Self {
        Self::new(
            w,
            h,
            rgb.iter()
                .map(|&[r, g, b]| colour::srgb8_to_linear(r, g, b))
                .collect(),
        )
    }

    /// Pixel `(x, y)`.
    pub fn at(&self, x: usize, y: usize) -> Rgb {
        self.px[y * self.w + x]
    }
}
