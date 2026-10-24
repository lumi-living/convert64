//! The Transform group's units and presets (UX §6 item 1, usability review M6, #289).
//!
//! Every mode is composited on the same 320 × 200 grid of **screen pixels** (ADR-011:
//! the transform is in C64 target pixels, and multicolor's 2:1 is the compositor's job):
//! one screen pixel is one hi-res pixel, and a multicolor pixel is two screen pixels
//! wide. The group shows a layer's size in screen pixels, with the scale as a percentage
//! of the source and, in multicolor, the width in multicolor pixels as secondary
//! readouts.

use convert64_core::mode::Mode;
use convert64_core::{Transform, Vec2};

/// The screen the layers are placed on, in screen pixels.
pub const SCREEN: (f32, f32) = (320.0, 200.0);

/// Screen pixels per C64 pixel across: 2 in the multicolor modes, 1 in hi-res.
pub fn pixel_width(mode: Mode) -> f32 {
    match mode {
        Mode::MulticolorBitmap | Mode::MulticolorCharset => 2.0,
        _ => 1.0,
    }
}

/// The layer's size on the screen (its unrotated box), W × H in screen pixels, for a
/// source of `size`.
pub fn screen_size(t: &Transform, size: (u32, u32)) -> (f32, f32) {
    (t.scale().x * size.0 as f32, t.scale().y * size.1 as f32)
}

/// The scale that makes a source `len` pixels long `px` screen pixels long.
pub fn scale_for(px: f32, len: u32) -> f32 {
    px / len as f32
}

/// The secondary readout under W × H, a line each: the scale as a % of the source, and
/// in multicolor the width in multicolor pixels.
pub fn secondary(t: &Transform, size: (u32, u32), mode: Mode) -> Vec<String> {
    let s = t.scale();
    let mut lines = vec![format!(
        "{:.2} % × {:.2} % of the source",
        s.x * 100.0,
        s.y * 100.0
    )];
    let pw = pixel_width(mode);
    if pw > 1.0 {
        let (w, _) = screen_size(t, size);
        lines.push(format!("{:.1} multicolor px wide", w / pw));
    }
    lines
}

/// The Transform group's presets, in button order (UX §6 item 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    /// Uniform, the whole picture on the screen, centred.
    Fit,
    /// Uniform, the screen covered and the overflow cropped evenly, centred.
    Fill,
    /// One source pixel per screen pixel, on whole C64 pixels.
    OneToOne,
    /// One source pixel per multicolor pixel (2 × 1 screen pixels), on whole C64 pixels.
    TwoToOne,
}

impl Preset {
    /// Every preset, in button order.
    pub const ALL: [Self; 4] = [Self::Fit, Self::Fill, Self::OneToOne, Self::TwoToOne];

    /// The button's text.
    pub fn label(self) -> &'static str {
        match self {
            Self::Fit => "Fit",
            Self::Fill => "Fill",
            Self::OneToOne => "1:1",
            Self::TwoToOne => "2:1",
        }
    }

    /// The button's tooltip: what the preset does, exactly.
    pub fn tip(self) -> &'static str {
        match self {
            Self::Fit => "Uniform scale, the whole picture on the screen, centred",
            Self::Fill => "Uniform scale, the screen covered, the overflow cropped evenly, centred",
            Self::OneToOne => {
                "One source pixel per screen pixel: a hi-res pixel, or half a multicolor \
                 pixel. Centred on whole C64 pixels"
            }
            Self::TwoToOne => {
                "One source pixel per multicolor pixel (2 × 1 screen pixels), for pixel \
                 art drawn at 160 × 200. Centred on whole C64 pixels"
            }
        }
    }

    /// The scale the preset gives a source of `size`.
    fn scale(self, size: (u32, u32)) -> Vec2 {
        let (w, h) = (size.0 as f32, size.1 as f32);
        let uniform = |s: f32| Vec2::new(s, s);
        match self {
            Self::Fit => uniform((SCREEN.0 / w).min(SCREEN.1 / h)),
            Self::Fill => uniform((SCREEN.0 / w).max(SCREEN.1 / h)),
            Self::OneToOne => Vec2::new(1.0, 1.0),
            Self::TwoToOne => Vec2::new(2.0, 1.0),
        }
    }

    /// The top-left corner that centres a source of `size` at `scale`: exactly for Fit
    /// and Fill (sub-pixel placement steers the solver), moved left and up onto whole
    /// C64 pixels of `mode` for the pixel presets, so pixel art stays crisp.
    fn translate(self, size: (u32, u32), scale: Vec2, mode: Mode) -> Vec2 {
        let cx = (SCREEN.0 - size.0 as f32 * scale.x) / 2.0;
        let cy = (SCREEN.1 - size.1 as f32 * scale.y) / 2.0;
        match self {
            Self::Fit | Self::Fill => Vec2::new(cx, cy),
            Self::OneToOne | Self::TwoToOne => {
                let step = pixel_width(mode);
                Vec2::new((cx / step).floor() * step, cy.floor())
            }
        }
    }

    /// `t` with the preset's scale and position for a source of `size` in `mode`; the
    /// rotation, anchor and flips are kept.
    pub fn apply(self, t: &Transform, size: (u32, u32), mode: Mode) -> Option<Transform> {
        let scale = self.scale(size);
        let translate = self.translate(size, scale, mode);
        Transform::new(translate, scale, t.flip_h(), t.flip_v())
            .and_then(|n| n.with_rotation(t.rotate()))
            .and_then(|n| n.with_anchor(t.anchor()))
            .ok()
    }

    /// The preset `t` is at, if any (scale and position, within a rounding error).
    /// Where presets give the same transform (a 320 × 200 source is at 1:1, Fill and Fit
    /// at once), the most specific one: 1:1 or 2:1, then Fill, then Fit (#306).
    pub fn matching(t: &Transform, size: (u32, u32), mode: Mode) -> Option<Self> {
        let close =
            |a: Vec2, b: Vec2, eps: f32| (a.x - b.x).abs() <= eps && (a.y - b.y).abs() <= eps;
        [Self::OneToOne, Self::TwoToOne, Self::Fill, Self::Fit]
            .into_iter()
            .find(|p| {
                let scale = p.scale(size);
                close(t.scale(), scale, 1e-5 * scale.x.max(scale.y))
                    && close(t.translate(), p.translate(size, scale, mode), 1e-3)
            })
    }
}

/// The preset a picture opened in the app gets (UX §6 item 1, #289, #306): [`Preset::OneToOne`]
/// when it fits inside the screen, so a small picture is neither enlarged nor blurred,
/// and [`Preset::Fill`] otherwise, uniform with the screen covered.
pub fn opening(size: (u32, u32)) -> Preset {
    if size.0 as f32 <= SCREEN.0 && size.1 as f32 <= SCREEN.1 {
        Preset::OneToOne
    } else {
        Preset::Fill
    }
}

/// The transform a picture opened in the app gives its first layer: [`opening`]'s preset.
/// A 16:10 picture of at least 320 × 200 gets png2c64's stretch to 320 × 200, so it keeps
/// png2c64's front end.
pub fn opened(size: (u32, u32), mode: Mode) -> Transform {
    opening(size)
        .apply(&Transform::IDENTITY, size, mode)
        .unwrap_or(Transform::IDENTITY)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODES: [Mode; 2] = [Mode::MulticolorBitmap, Mode::HiresBitmap];

    fn t(translate: (f32, f32), scale: (f32, f32)) -> Transform {
        Transform::new(
            Vec2::new(translate.0, translate.1),
            Vec2::new(scale.0, scale.1),
            false,
            false,
        )
        .unwrap()
    }

    /// W × H are screen pixels: scale × source size, and back.
    #[test]
    fn sizes_are_in_screen_pixels() {
        let size = (1024, 637);
        let layer = t((0.0, 0.0), (0.3125, 0.25));
        let (w, h) = screen_size(&layer, size);
        assert!(
            (w - 320.0).abs() < 1e-4 && (h - 159.25).abs() < 1e-4,
            "{w} {h}"
        );
        assert_eq!(scale_for(320.0, 1024), 0.3125);
        assert_eq!(scale_for(160.0, 640), 0.25);
        // 160 × 100 screen px: half the screen each way.
        let half = t((0.0, 0.0), (scale_for(160.0, 320), scale_for(100.0, 200)));
        assert_eq!(screen_size(&half, (320, 200)), (160.0, 100.0));
    }

    /// A multicolor pixel is two screen pixels wide; the readout says so, and gives the
    /// scale as a % of the source in every mode.
    #[test]
    fn the_secondary_readout_names_its_units() {
        assert_eq!(pixel_width(Mode::MulticolorBitmap), 2.0);
        assert_eq!(pixel_width(Mode::HiresBitmap), 1.0);
        let layer = t((0.0, 0.0), (0.3125, 0.3125));
        assert_eq!(
            secondary(&layer, (1024, 640), Mode::MulticolorBitmap),
            [
                "31.25 % × 31.25 % of the source",
                "160.0 multicolor px wide"
            ]
        );
        assert_eq!(
            secondary(&layer, (1024, 640), Mode::HiresBitmap),
            ["31.25 % × 31.25 % of the source"]
        );
    }

    /// Fit and Fill are uniform and centred, the same in both modes; on a 16:10 source
    /// both fill the screen exactly.
    #[test]
    fn fit_and_fill_are_uniform_and_centred_in_both_modes() {
        let size = (1024, 637);
        for mode in MODES {
            let fit = Preset::Fit.apply(&Transform::IDENTITY, size, mode).unwrap();
            assert_eq!(fit.scale(), Vec2::new(0.3125, 0.3125));
            let (w, h) = screen_size(&fit, size);
            assert_eq!(w, 320.0);
            assert!((fit.translate().y - (200.0 - h) / 2.0).abs() < 1e-5);
            assert!(h <= 200.0 && fit.translate().x == 0.0);

            let fill = Preset::Fill
                .apply(&Transform::IDENTITY, size, mode)
                .unwrap();
            let s = 200.0 / 637.0;
            assert_eq!(fill.scale(), Vec2::new(s, s));
            let (w, h) = screen_size(&fill, size);
            assert!((h - 200.0).abs() < 1e-4 && w > 320.0, "{w} {h}");
            assert!((fill.translate().x - (320.0 - w) / 2.0).abs() < 1e-5);
            assert!(fill.translate().y.abs() < 1e-4);

            // 16:10: both are png2c64's stretch.
            for p in [Preset::Fit, Preset::Fill] {
                let n = p.apply(&Transform::IDENTITY, (640, 400), mode).unwrap();
                assert_eq!(n.scale(), Vec2::new(320.0 / 640.0, 200.0 / 400.0));
                assert_eq!(n.translate(), Vec2::ZERO);
            }
        }
    }

    /// 1:1 is (1, 1) and 2:1 is (2, 1) in both modes, centred and then moved onto whole
    /// C64 pixels: an even x in multicolor.
    #[test]
    fn the_pixel_presets_land_on_whole_c64_pixels() {
        let cases = [
            // (preset, source, mode, scale, translate)
            (
                Preset::OneToOne,
                (101, 51),
                Mode::HiresBitmap,
                (1.0, 1.0),
                (109.0, 74.0),
            ),
            (
                Preset::OneToOne,
                (101, 51),
                Mode::MulticolorBitmap,
                (1.0, 1.0),
                (108.0, 74.0),
            ),
            (
                Preset::OneToOne,
                (320, 200),
                Mode::MulticolorBitmap,
                (1.0, 1.0),
                (0.0, 0.0),
            ),
            (
                Preset::TwoToOne,
                (160, 200),
                Mode::MulticolorBitmap,
                (2.0, 1.0),
                (0.0, 0.0),
            ),
            (
                Preset::TwoToOne,
                (160, 200),
                Mode::HiresBitmap,
                (2.0, 1.0),
                (0.0, 0.0),
            ),
            (
                Preset::TwoToOne,
                (81, 100),
                Mode::MulticolorBitmap,
                (2.0, 1.0),
                (78.0, 50.0),
            ),
            (
                Preset::TwoToOne,
                (81, 100),
                Mode::HiresBitmap,
                (2.0, 1.0),
                (79.0, 50.0),
            ),
            // Bigger than the screen: still centred, on whole pixels.
            (
                Preset::OneToOne,
                (1023, 637),
                Mode::MulticolorBitmap,
                (1.0, 1.0),
                (-352.0, -219.0),
            ),
        ];
        for (p, size, mode, scale, at) in cases {
            let n = p.apply(&Transform::IDENTITY, size, mode).unwrap();
            assert_eq!(
                n.scale(),
                Vec2::new(scale.0, scale.1),
                "{p:?} {size:?} {mode:?}"
            );
            assert_eq!(
                n.translate(),
                Vec2::new(at.0, at.1),
                "{p:?} {size:?} {mode:?}"
            );
            // It reads as that preset (or one that gives the same transform: 1:1 is Fit
            // on a 320 × 200 source).
            let m = Preset::matching(&n, size, mode).expect("a preset");
            assert_eq!(m.apply(&Transform::IDENTITY, size, mode), Some(n));
        }
    }

    /// A preset keeps the rotation, the anchor and the flips.
    #[test]
    fn presets_keep_rotation_anchor_and_flips() {
        let before = Transform::new(Vec2::new(5.0, 7.0), Vec2::new(0.1, 0.2), true, false)
            .and_then(|t| t.with_rotation(12.5))
            .and_then(|t| t.with_anchor(Vec2::new(0.0, 1.0)))
            .unwrap();
        for p in Preset::ALL {
            let n = p
                .apply(&before, (640, 480), Mode::MulticolorBitmap)
                .unwrap();
            assert_eq!(n.rotate(), 12.5);
            assert_eq!(n.anchor(), Vec2::new(0.0, 1.0));
            assert!(n.flip_h() && !n.flip_v());
        }
    }

    /// Only a transform at a preset's scale and position matches it.
    #[test]
    fn matching_needs_scale_and_position() {
        let size = (1024, 637);
        let mode = Mode::MulticolorBitmap;
        let fill = opened(size, mode);
        assert_eq!(Preset::matching(&fill, size, mode), Some(Preset::Fill));
        let moved = fill.nudged(Vec2::new(1.0, 0.0)).unwrap();
        assert_eq!(Preset::matching(&moved, size, mode), None);
        // png2c64's stretch is not uniform, so no preset.
        let stretch = t((0.0, 0.0), (320.0 / 1024.0, 200.0 / 637.0));
        assert_eq!(Preset::matching(&stretch, size, mode), None);
    }

    /// A picture opens with a uniform scale: Fill when it is larger than the screen.
    #[test]
    fn an_opened_picture_has_a_uniform_scale() {
        for size in [
            (1024, 637),
            (480, 640),
            (640, 400),
            (400, 150),
            (3000, 1000),
        ] {
            let o = opened(size, Mode::MulticolorBitmap);
            assert_eq!(o.scale().x, o.scale().y, "{size:?}");
            assert_eq!(opening(size), Preset::Fill, "{size:?}");
            assert_eq!(
                Preset::matching(&o, size, Mode::MulticolorBitmap),
                Some(Preset::Fill),
                "{size:?}"
            );
            assert_eq!(
                Preset::Fill.apply(&Transform::IDENTITY, size, Mode::MulticolorBitmap),
                Some(o)
            );
        }
    }

    /// #306: a picture that fits inside the screen opens at 1:1, centred on whole C64
    /// pixels, in both modes; a larger one in either dimension at Fill.
    #[test]
    fn a_small_picture_opens_at_1_to_1() {
        for mode in MODES {
            for (size, at) in [
                ((160, 100), (80.0, 50.0)),
                ((320, 200), (0.0, 0.0)),
                ((64, 40), (128.0, 80.0)),
                ((320, 1), (0.0, 99.0)),
                (
                    (1, 200),
                    (
                        if mode == Mode::HiresBitmap {
                            159.0
                        } else {
                            158.0
                        },
                        0.0,
                    ),
                ),
                (
                    (101, 51),
                    (
                        if mode == Mode::HiresBitmap {
                            109.0
                        } else {
                            108.0
                        },
                        74.0,
                    ),
                ),
            ] {
                assert_eq!(opening(size), Preset::OneToOne, "{size:?}");
                let o = opened(size, mode);
                assert_eq!(o.scale(), Vec2::new(1.0, 1.0), "{size:?} {mode:?}");
                assert_eq!(o.translate(), Vec2::new(at.0, at.1), "{size:?} {mode:?}");
                assert_eq!(o.translate().x % pixel_width(mode), 0.0);
                assert_eq!(o.translate().y.fract(), 0.0);
                // The group shows 1:1, also where it is Fill and Fit too (320 × 200).
                assert_eq!(
                    Preset::matching(&o, size, mode),
                    Some(Preset::OneToOne),
                    "{size:?} {mode:?}"
                );
            }
            assert_eq!(
                opened((320, 200), mode),
                Preset::Fill
                    .apply(&Transform::IDENTITY, (320, 200), mode)
                    .unwrap()
            );
        }
        for size in [(321, 200), (320, 201), (400, 150), (1024, 637)] {
            assert_eq!(opening(size), Preset::Fill, "{size:?}");
        }
    }
}
