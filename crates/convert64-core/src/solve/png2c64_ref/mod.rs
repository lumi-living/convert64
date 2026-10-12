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

/// Our extras that png2c64 does not have, as named options on the reference path, each
/// off by default (#181 step 7). With the default, the pipeline is png2c64's. A human
/// judges each on the sheet (`cargo xtask dither-sheet`, the divergence rows) before it
/// can become a default.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Divergences {
    /// #157's chroma guard: an ordered pixel's search and pick avoid colours that add
    /// chroma the pixel lacks (a neutral pixel keeps to near-greys, a tinted one to
    /// colours of a close hue).
    pub chroma_guard: bool,
    /// Dither mixes stay within this many of the nine luma levels of the pixel's nearest
    /// colour (0: same-luma only, PAL-blend safe); `None`: unrestricted.
    pub max_luma_diff: Option<u8>,
    /// Added to a cell set's search score per same-luma pair in it (positive
    /// penalises, negative rewards); 0: off.
    pub pair_bias: f32,
    /// Diffused error does not cross into a cell that lacks the colour it pulls
    /// towards (E5's cell clamp); png2c64 lets it flow everywhere.
    pub cell_clamp: bool,
    /// An ordered pixel's threshold is quantised to this many levels (so a pair mixes
    /// at one of `n + 1` ratios); `None`: the matrix as is.
    pub mix_levels: Option<u8>,
    /// #157's bridging term: a cell set's search score also counts, per ordered pixel,
    /// 0.01 × the squared width of the OKLab-L bracket the set gives it (½ plus its
    /// distance to the nearest end when unbracketed).
    pub bridging: bool,
}

impl Divergences {
    /// Whether all are off (png2c64's pipeline).
    pub fn none(&self) -> bool {
        *self == Self::default()
    }
}

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
    let (w, h) = mode.screen();
    let px = vec![dither::PxSettings::from(settings); w * h];
    convert_px(
        image,
        palette,
        mode,
        &px,
        settings.serpentine,
        &quantize::Hooks::default(),
    )
}

/// [`convert`] with each pixel's own dither settings (the attribute map): ordered pixels
/// bias the search, cells without any get the global Floyd–Steinberg choice, and the
/// diffusion pixels are then diffused. With the same settings for every pixel this is
/// [`convert`], bit for bit.
pub fn convert_px(
    image: &Image,
    palette: &[Rgb; 16],
    mode: quantize::Mode,
    px: &[dither::PxSettings],
    serpentine: bool,
    hooks: &quantize::Hooks,
) -> quantize::Screen {
    let (w, _) = mode.screen();
    let levels = hooks.divergences.mix_levels;
    let bias: Vec<Option<f32>> = px
        .iter()
        .enumerate()
        .map(|(i, s)| s.bias_levels(i % w, i / w, levels))
        .collect();
    let mut screen = quantize::quantize_px(image, palette, mode, &bias, hooks);
    if px.iter().any(|s| s.method.kernel().is_some()) {
        dither::diffuse_px(
            image,
            &mut screen,
            palette,
            px,
            serpentine,
            &hooks.divergences,
        );
    }
    screen
}
