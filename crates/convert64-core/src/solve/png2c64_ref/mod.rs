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
pub mod dither;
pub mod preprocess;
pub mod quantize;
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

/// png2c64's front end (`png_io::load`, `scale::bicubic`, `preprocess::apply`,
/// `match_palette_range`): 8-bit sRGB pixels, `w × h`, scaled to `mode`'s screen with the
/// Mitchell kernel (unless already that size), preprocessed with `pre`, and with
/// `match_range` remapped to `palette`'s OKLab extent.
pub fn prepare(
    rgb: &[[u8; 3]],
    (w, h): (usize, usize),
    mode: quantize::Mode,
    pre: &preprocess::Settings,
    match_range: Option<&[Rgb; 16]>,
) -> Image {
    let src = Image::from_srgb8(w, h, rgb);
    let (tw, th) = mode.screen();
    let mut im = if (w, h) == (tw, th) {
        src
    } else {
        scale::bicubic(&src, tw, th, scale::Kernel::MitchellNetravali)
    };
    preprocess::apply(&mut im, pre);
    if let Some(palette) = match_range {
        preprocess::match_palette_range(&mut im, palette, 0.01, 0.05);
    }
    im
}

/// png2c64's bitmap pipeline after preprocessing (`main.cpp`): an ordered method's
/// threshold (× strength) biases the cell search and its nearest colours are the
/// picture; a diffusion method gets the search's sets (replaced by the global
/// Floyd–Steinberg choice) and then [`dither::apply`]; `none` is the search alone.
pub fn convert(
    image: &Image,
    palette: &[Rgb; 16],
    mode: quantize::Mode,
    settings: &dither::Settings,
) -> quantize::Screen {
    let method = settings.method;
    let strength = settings.strength;
    let threshold = move |x: usize, y: usize| dither::ordered_threshold(method, x, y) * strength;
    let ordered = method.is_ordered() && method != dither::Method::None;
    let mut screen = quantize::quantize(
        image,
        palette,
        mode,
        ordered.then_some(&threshold as quantize::Threshold<'_>),
        strength,
    );
    if !method.is_ordered() {
        dither::apply(image, &mut screen, palette, settings);
    }
    screen
}
