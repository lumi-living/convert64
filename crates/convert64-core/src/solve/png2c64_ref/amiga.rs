//! png2amiga's dither methods on png2c64's pipeline (#275, ADR-014 §6): a sheet hook
//! (`cargo xtask png2amiga-methods-sheet`), not a user-facing path. A human judges each
//! method on the sheets; only a method adopted on that verdict becomes one the reference
//! solves, the rest stay png2amiga-solver-only.
//!
//! Every method runs on png2c64's front end ([`super::prepare`]) and png2c64's cell
//! stage ([`super::quantize::quantize_px`]); only the per-pixel step is png2amiga's
//! ([`crate::solve::png2amiga_ref::dither`]). How, by what the method is on png2amiga's
//! C64 path ([`Port`]):
//!
//! - **Threshold methods** (png2amiga's ordered, noise and effect kinds): png2amiga's
//!   threshold with png2c64's ordered semantics, exactly as png2c64 runs its own matrices:
//!   `t × strength × strength` biases the cell search (L by `0.15 t`, a and b by
//!   `0.03 t`) and each pixel is its cell's nearest colour to the biased target. The
//!   threshold is png2amiga's raw value (Truchet's reaches 0.915; nothing is clamped).
//! - **png2amiga's dither pass** (its diffusion kernels and its palette-aware pickers):
//!   png2c64's cells for a method without a threshold (the unbiased search, then the
//!   global Floyd–Steinberg replacement, png2c64's background), then png2amiga's own
//!   pass ([`crate::solve::png2amiga_ref::c64::encode`] with those cells): its sRGB
//!   residual (OKLab for `floyd-steinberg-oklab`), its error clamp on read only, its
//!   structure biases, its pickers.
//! - **Nearest pick** (`gilbert`, `dbs`): on png2amiga's C64 path neither has a kernel,
//!   so both pick each pixel's nearest cell colour; on png2c64's pipeline that is
//!   png2c64's `none` (to within png2amiga's colour maths).

use super::quantize::{self, Hooks, Mode, Screen};
use super::{Image, Rgb};
use crate::solve::png2amiga_ref::c64;
use crate::solve::png2amiga_ref::dither::{Method, PxSettings, ordered_threshold};

/// How a png2amiga method runs on png2c64's pipeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Port {
    /// Its threshold biases png2c64's cell search; the pixels are the search's own.
    Threshold,
    /// png2amiga's per-pixel pass (diffusion or a palette-aware picker) in png2c64's
    /// cells.
    Pass,
    /// A plain nearest pick in png2c64's cells (png2amiga's C64 path has no kernel for
    /// it).
    NearestPick,
}

impl Port {
    /// How `method` runs.
    pub fn of(method: Method) -> Self {
        if method.thresholds() {
            Self::Threshold
        } else if method.diffuses() || method.is_yliluoma() {
            Self::Pass
        } else {
            Self::NearestPick
        }
    }
}

/// `method` on png2c64's pipeline after its front end: `image` is the prepared picture
/// (the mode's screen size), `strength` and `error_clamp` the dither's (png2c64's
/// baseline: 0.7 and 0.1). See the module notes for what each kind of method does.
///
/// # Panics
/// If `image` is not the mode's screen size.
pub fn convert(
    image: &Image,
    palette: &[Rgb; 16],
    mode: Mode,
    method: Method,
    strength: f32,
    error_clamp: f32,
) -> Screen {
    let (w, h) = mode.screen();
    if Port::of(method) == Port::Threshold {
        let bias: Vec<Option<f32>> = (0..w * h)
            .map(|i| Some(ordered_threshold(method, i % w, i / w) * strength * strength))
            .collect();
        return quantize::quantize_px(image, palette, mode, &bias, &Hooks::default());
    }
    // png2c64's cells for a method without a threshold.
    let cells = quantize::quantize_px(image, palette, mode, &vec![None; w * h], &Hooks::default());
    let hooks = c64::Hooks {
        background: (mode == Mode::Multicolor).then_some(cells.bg),
        sets: Some(cells.cells.iter().map(|c| c.colors.clone()).collect()),
        ..c64::Hooks::default()
    };
    let px = vec![
        PxSettings {
            method,
            strength,
            error_clamp,
            origin: (0, 0),
        };
        w * h
    ];
    c64::encode(image, palette, mode, &px, c64::Metric::Mse, true, &hooks)
}

#[cfg(test)]
mod tests {
    use super::super::dither::{Method as P2c, Settings};
    use super::*;
    use crate::solve::png2amiga_ref::colour::{PEPTO_HEX, palette_linear};

    /// A 320 × 200 picture with ramps in lightness and hue.
    fn picture() -> Image {
        let (w, h) = Mode::Hires.screen();
        let px = (0..w * h)
            .map(|i| {
                let (x, y) = ((i % w) as f32 / w as f32, (i / w) as f32 / h as f32);
                [x, 0.5 * (x + y), y * y]
            })
            .collect();
        Image::new(w, h, px)
    }

    /// png2amiga's checker is png2c64's matrix, so through the threshold port it gives
    /// png2c64's checker bit for bit: the port is png2c64's ordered semantics.
    #[test]
    fn the_threshold_port_is_png2c64_s_ordered_path() {
        let im = picture();
        let pal = palette_linear(&PEPTO_HEX);
        let ours = convert(&im, &pal, Mode::Hires, Method::Checker, 0.7, 0.1);
        let theirs = super::super::convert(
            &im,
            &pal,
            Mode::Hires,
            &Settings {
                method: P2c::Checker,
                strength: 0.7,
                error_clamp: 0.1,
                serpentine: true,
                adaptive: 0.0,
            },
        );
        assert_eq!(ours.indices(), theirs.indices());
        assert_eq!(Port::of(Method::Bayer3x3), Port::Threshold);
        assert_eq!(Port::of(Method::Truchet), Port::Threshold);
        assert_eq!(Port::of(Method::StructureFs), Port::Pass);
        assert_eq!(Port::of(Method::OptChecker), Port::Pass);
        assert_eq!(Port::of(Method::Gilbert), Port::NearestPick);
        assert_eq!(Port::of(Method::Dbs), Port::NearestPick);
    }

    /// The pass keeps png2c64's cell colours; a nearest pick there is png2c64's `none`.
    #[test]
    fn the_pass_keeps_png2c64_s_cells() {
        let im = picture();
        let pal = palette_linear(&PEPTO_HEX);
        let none = super::super::convert(
            &im,
            &pal,
            Mode::Hires,
            &Settings {
                method: P2c::None,
                ..Settings::default()
            },
        );
        for m in [
            Method::FloydSteinbergOklab,
            Method::TriTone,
            Method::Gilbert,
        ] {
            let s = convert(&im, &pal, Mode::Hires, m, 0.7, 0.1);
            for (a, b) in s.cells.iter().zip(&none.cells) {
                assert_eq!(a.colors, b.colors, "{m:?}");
            }
        }
        let gilbert = convert(&im, &pal, Mode::Hires, Method::Gilbert, 0.7, 0.1).indices();
        let same = gilbert
            .iter()
            .zip(none.indices())
            .filter(|(a, b)| **a == *b)
            .count();
        assert!(same * 1000 >= gilbert.len() * 999, "{same}");
    }
}
