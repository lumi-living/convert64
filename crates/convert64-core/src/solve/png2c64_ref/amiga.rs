//! png2amiga's dither methods on png2c64's pipeline (#275, ADR-014 §6): how the
//! reference solver runs the 32 methods png2amiga has and png2c64 lacks. Jaakko judged
//! them on #280's sheets (`cargo xtask png2amiga-methods-sheet`) and kept all 32, so the
//! reference solves them here instead of handing them to the native solver's stand-ins
//! ([`crate::layer::DitherKind::native`]). `--solver png2amiga` still runs png2amiga's
//! own versions (`solve::png2amiga_ref`).
//!
//! Every method runs on png2c64's front end ([`super::prepare`], or the compositor's)
//! and png2c64's cell stage ([`super::quantize::quantize_px`]); only the per-pixel step
//! is png2amiga's ([`crate::solve::png2amiga_ref::dither`]). How, by what the method is
//! on png2amiga's C64 path ([`Port`]):
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
//!   so both pick each pixel's nearest cell colour (png2amiga's pass, as the sheets ran
//!   them); on png2c64's pipeline that is png2c64's `none` (to within png2amiga's colour
//!   maths).
//!
//! [`convert_px`] is [`super::convert_px`] with some pixels on these methods, so a frame
//! can mix them with png2c64's own (the attribute map), under every hook of the
//! reference path (pins, locks, the interactive fast path, the divergences).

use super::dither::{Method as P2cMethod, PxSettings as P2cPx};
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
    /// it), run as png2amiga's pass runs it.
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

/// `method` on png2c64's pipeline after its front end, for every pixel: `image` is the
/// prepared picture (the mode's screen size), `strength` and `error_clamp` the dither's
/// (png2c64's baseline: 0.7 and 0.1). See the module notes for what each kind of method
/// does. [`convert_px`] with one method everywhere.
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
    let px = vec![
        P2cPx {
            method: P2cMethod::None,
            strength,
            error_clamp,
            adaptive: 0.0,
            origin: (0, 0),
        };
        w * h
    ];
    convert_px(
        image,
        palette,
        mode,
        &px,
        &vec![Some(method); w * h],
        true,
        &Hooks::default(),
    )
}

/// [`super::convert_px`] with png2amiga's methods: where `amiga[i]` is `Some`, pixel
/// `i` runs that method (its strength, error clamp and pattern anchor from `px[i]`, whose
/// own method is then ignored); elsewhere it is png2c64's, as in
/// [`super::convert_px`]. Thresholds bias the cell search with png2c64's other ordered
/// pixels (the `mix_levels` divergence quantises them too), png2c64's diffusion pixels
/// are diffused, then png2amiga's pass runs within the cells for its pixels; cells the
/// hooks fix are copied through. With no `Some`, this is [`super::convert_px`] bit for
/// bit.
///
/// # Panics
/// If `image` is not the mode's screen size or `px` and `amiga` are not one per pixel.
pub fn convert_px(
    image: &Image,
    palette: &[Rgb; 16],
    mode: Mode,
    px: &[P2cPx],
    amiga: &[Option<Method>],
    serpentine: bool,
    hooks: &Hooks,
) -> Screen {
    let (w, h) = mode.screen();
    assert_eq!(px.len(), w * h, "one setting per pixel");
    assert_eq!(
        amiga.len(),
        w * h,
        "one png2amiga method (or none) per pixel"
    );
    let levels = hooks.divergences.mix_levels;
    let anchored = |s: &P2cPx| PxSettings {
        method: Method::None,
        strength: s.strength,
        error_clamp: s.error_clamp,
        origin: s.origin,
    };
    let bias: Vec<Option<f32>> = px
        .iter()
        .zip(amiga)
        .enumerate()
        .map(|(i, (s, a))| match a {
            None => s.bias_levels(i % w, i / w, levels),
            Some(m) if Port::of(*m) == Port::Threshold => {
                let (x, y) = anchored(s).phase(i % w, i / w);
                let mut thr = ordered_threshold(*m, x, y);
                if let Some(n) = levels.filter(|&n| n > 0) {
                    let n = f32::from(n);
                    thr = (((thr + 0.5) * n).floor().min(n - 1.0) + 0.5) / n - 0.5;
                }
                Some(thr * s.strength * s.strength)
            }
            Some(_) => None,
        })
        .collect();
    let mut screen = quantize::quantize_px(image, palette, mode, &bias, hooks);
    // png2c64's diffusion, for its own pixels only.
    let own: Vec<P2cPx> = px
        .iter()
        .zip(amiga)
        .map(|(s, a)| match a {
            None => *s,
            Some(_) => P2cPx {
                method: P2cMethod::None,
                ..*s
            },
        })
        .collect();
    if own.iter().any(|s| s.method.kernel().is_some()) {
        super::dither::diffuse_px(
            image,
            &mut screen,
            palette,
            &own,
            serpentine,
            &hooks.divergences,
        );
    }
    let passes = |a: &Option<Method>| a.is_some_and(|m| Port::of(m) != Port::Threshold);
    if !amiga.iter().any(passes) {
        return screen;
    }
    // png2amiga's pass within the cells: its pixels picked by their method, every other
    // pixel a nearest pick whose result is not used.
    let settings: Vec<PxSettings> = px
        .iter()
        .zip(amiga)
        .map(|(s, a)| PxSettings {
            method: a
                .filter(|&m| Port::of(m) != Port::Threshold)
                .unwrap_or(Method::None),
            ..anchored(s)
        })
        .collect();
    let pass_hooks = c64::Hooks {
        background: (mode == Mode::Multicolor).then_some(screen.bg),
        sets: Some(screen.cells.iter().map(|c| c.colors.clone()).collect()),
        fixed: hooks.fixed.clone(),
        ..c64::Hooks::default()
    };
    let picked = c64::encode(
        image,
        palette,
        mode,
        &settings,
        c64::Metric::Mse,
        serpentine,
        &pass_hooks,
    );
    let (cw, ch) = mode.cell();
    let cols = mode.cells().0;
    for (i, a) in amiga.iter().enumerate() {
        let (x, y) = (i % w, i / w);
        let c = (y / ch) * cols + x / cw;
        if passes(a) && hooks.fixed.get(c).is_none_or(Option::is_none) {
            let p = (y % ch) * cw + x % cw;
            debug_assert_eq!(screen.cells[c].colors, picked.cells[c].colors);
            screen.cells[c].px[p] = picked.cells[c].px[p];
        }
    }
    screen
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
