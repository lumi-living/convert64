//! The reference solver since ADR-013: a faithful port of png2amiga's C64 bitmap path
//! (#261).
//!
//! Ported from [png2amiga](https://github.com/tinic/png2amiga) by Tinic Uro, MIT licence,
//! at commit `4a8777653536ac07074a650ee86860347c8755fb` (v1.111.2); see `THIRD_PARTY.md`.
//! png2amiga's C64 encoder began as a merge of png2c64 (`c64.hpp`), so this module shares
//! what is literally unchanged with [`super::png2c64_ref`] (the sRGB table, the image
//! type, the L-plane sharpen arithmetic, the cell layout) and copies and changes the
//! rest. Each submodule names the file it ports and lists what differs from png2c64.
//! Odd choices are ported too and marked "as png2amiga".
//!
//! **Float semantics.** The reference is png2amiga built with strict IEEE arithmetic
//! (GCC 15, `-ffp-contract=off` appended after png2amiga's own `-ffp-contract=fast`) and
//! a maths shim that computes `powf`, `sinf`, `cosf`, `atan2f` and `expf` in double and
//! rounds once (`docs/reference/png2amiga-reference.md`). Rust does the same: no
//! implicit fused multiply-adds, png2amiga's explicit `std::fma` as [`f32::mul_add`],
//! and those functions in `f64`.

pub mod c64;
pub mod colour;
pub mod dither;
pub mod preprocess;
pub mod scale;

pub use super::png2c64_ref::{Image, Rgb};
pub use super::png2c64_ref::quantize::{Cell, CellRule, Mode, Screen};

/// png2amiga's front end for an 8-bit picture (`png_io::load`, `scale::resample`,
/// `preprocess::apply`, `match_palette_range`): the bytes, `w × h`, scaled to `mode`'s
/// screen with the widened Lanczos-3 (unless already that size), preprocessed with `pre`,
/// and with `match_range = Some((palette, amount))` the chroma stretched onto the
/// palette's hull.
pub fn prepare(
    rgb: &[[u8; 3]],
    (w, h): (usize, usize),
    mode: Mode,
    pre: &preprocess::Settings,
    match_range: Option<(&[Rgb; 16], f32)>,
) -> Image {
    let src = Image::from_srgb8(w, h, rgb);
    let (tw, th) = mode.screen();
    let mut im = if (w, h) == (tw, th) {
        src
    } else {
        scale::resample(&src, tw, th)
    };
    preprocess::apply(&mut im, pre);
    if let Some((palette, amount)) = match_range {
        preprocess::match_palette_range(&mut im, palette, amount);
    }
    im
}

/// png2amiga's C64 settings for one picture: its `dither::Settings` (method, strength,
/// error clamp, serpentine) and the cell metric.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    /// The dither method.
    pub method: dither::Method,
    /// 0 = no dithering, 1 = full.
    pub strength: f32,
    /// Maximum received error per channel.
    pub error_clamp: f32,
    /// Alternate the scan direction per row (error diffusion).
    pub serpentine: bool,
    /// The `none` brute force's cell metric.
    pub metric: c64::Metric,
}

impl Default for Settings {
    /// png2amiga's command-line defaults for C64: Floyd–Steinberg, strength 1, error
    /// clamp 0.35, blur metric.
    fn default() -> Self {
        Self {
            method: dither::Method::FloydSteinberg,
            strength: 1.0,
            error_clamp: 0.35,
            serpentine: true,
            metric: c64::Metric::Blur,
        }
    }
}

/// png2amiga's C64 encode of a prepared picture with one setting for every pixel.
pub fn convert(image: &Image, palette: &[Rgb; 16], mode: Mode, settings: &Settings) -> Screen {
    let (w, h) = mode.screen();
    let px = vec![
        dither::PxSettings {
            method: settings.method,
            strength: settings.strength,
            error_clamp: settings.error_clamp,
            origin: (0, 0),
        };
        w * h
    ];
    c64::encode(
        image,
        palette,
        mode,
        &px,
        settings.metric,
        settings.serpentine,
        &c64::Hooks::default(),
    )
}
