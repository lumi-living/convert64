//! Opt-in snapping of a layer transform to the target's pixels or cells (ADR-011).
//!
//! Snapping is a pure function from a proposed [`Transform`] to a snapped one, applied
//! by the commands that write a transform. The setting lives in the project; the
//! transform never records that it was snapped.

use super::Transform;
use crate::geom::Vec2;
use crate::mode::{Mode, PixelAspect};
use serde::{Deserialize, Serialize};

/// What a transform snaps to (ADR-011 §3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Snap {
    /// Fully continuous: sub-pixel translate, scale and rotation (the default).
    #[default]
    Off,
    /// `translate` and the scaled, unrotated box's edges on whole target pixels
    /// (multicolor: whole 2:1 pixels, 2 canvas px across); rotation in 0.5° steps.
    C64Pixel,
    /// `translate` and the box's edges on cell boundaries (multicolor 4 × 8 target px =
    /// 8 × 8 canvas px, hi-res 8 × 8); rotation in 15° steps.
    Cell,
}

impl Snap {
    /// Every setting, in the order the UI lists them.
    pub const ALL: [Self; 3] = [Self::Off, Self::C64Pixel, Self::Cell];

    /// The grid in canvas pixels `(x, y)` and the rotation step in degrees for `mode`, or
    /// `None` for [`Snap::Off`]. A mode without a specification snaps like hi-res.
    pub fn steps(self, mode: Mode) -> Option<(Vec2, f32)> {
        let (aspect, cell) = match mode.spec() {
            Ok(s) => (
                if s.aspect == PixelAspect::DoubleWide {
                    2.0
                } else {
                    1.0
                },
                (f32::from(s.cell_width), f32::from(s.cell_height)),
            ),
            Err(_) => (1.0, (8.0, 8.0)),
        };
        match self {
            Self::Off => None,
            Self::C64Pixel => Some((Vec2::new(aspect, 1.0), 0.5)),
            Self::Cell => Some((Vec2::new(cell.0 * aspect, cell.1), 15.0)),
        }
    }

    /// `t` for a source of `size`, snapped for `mode`: `translate` and the far edge of the
    /// scaled, unrotated box rounded to the nearest grid line on each axis (the box at
    /// least one grid step across), and the rotation rounded to its step. The anchor and
    /// the flips are kept. [`Snap::Off`] returns `t` unchanged.
    #[expect(clippy::cast_precision_loss, reason = "source sizes are at most 16384")]
    pub fn apply(self, t: Transform, size: (u32, u32), mode: Mode) -> Transform {
        let Some((grid, step)) = self.steps(mode) else {
            return t;
        };
        let axis = |origin: f32, scale: f32, len: u32, unit: f32| {
            let x0 = (origin / unit).round() * unit;
            let x1 = ((origin + scale * len as f32) / unit).round() * unit;
            let x1 = if x1 > x0 { x1 } else { x0 + unit };
            (x0, (x1 - x0) / len as f32)
        };
        let tr = t.translate();
        let sc = t.scale();
        let (x0, sx) = axis(tr.x, sc.x, size.0, grid.x);
        let (y0, sy) = axis(tr.y, sc.y, size.1, grid.y);
        let rotate = (t.rotate() / step).round() * step;
        Transform {
            translate: Vec2::new(x0, y0),
            scale: Vec2::new(sx, sy),
            rotate,
            ..t
        }
    }
}

/// [`Snap::apply`] as a free function: `t` (a source of `size`) snapped for `mode`.
pub fn snap_transform(t: Transform, size: (u32, u32), mode: Mode, snap: Snap) -> Transform {
    snap.apply(t, size, mode)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on_grid(v: f32, unit: f32) -> bool {
        (v / unit - (v / unit).round()).abs() < 1e-4
    }

    #[test]
    fn off_changes_nothing() {
        let t = Transform::new(Vec2::new(3.3, -1.7), Vec2::new(0.37, 1.9), true, false)
            .unwrap()
            .with_rotation(12.34)
            .unwrap();
        for mode in [Mode::MulticolorBitmap, Mode::HiresBitmap] {
            assert_eq!(Snap::Off.apply(t, (97, 41), mode), t);
        }
    }

    /// Snapped transforms put the unrotated box's edges on whole target pixels (2 canvas
    /// px across in multicolor) or on cell boundaries, in both modes, and round the
    /// rotation to its step.
    #[test]
    fn edges_land_on_pixels_and_cells_in_both_modes() {
        let size = (97, 41);
        let proposals = [
            (Vec2::new(3.3, -1.7), Vec2::new(0.37, 1.9), 12.34),
            (Vec2::new(-20.6, 150.2), Vec2::new(2.71, 0.53), -97.8),
            (Vec2::new(0.49, 0.51), Vec2::new(0.001, 0.001), 7.6),
        ];
        let cases = [
            (Mode::MulticolorBitmap, Snap::C64Pixel, (2.0, 1.0), 0.5),
            (Mode::HiresBitmap, Snap::C64Pixel, (1.0, 1.0), 0.5),
            (Mode::MulticolorBitmap, Snap::Cell, (8.0, 8.0), 15.0),
            (Mode::HiresBitmap, Snap::Cell, (8.0, 8.0), 15.0),
        ];
        for (mode, snap, (ux, uy), step) in cases {
            for (tr, sc, rot) in proposals {
                let t = Transform::new(tr, sc, false, true)
                    .unwrap()
                    .with_rotation(rot)
                    .unwrap()
                    .with_anchor(Vec2::new(0.25, 1.0))
                    .unwrap();
                let s = snap.apply(t, size, mode);
                let unrotated = s.with_rotation(0.0).unwrap();
                let (a, b) = unrotated.canvas_bounds(size);
                for (v, u) in [(a.x, ux), (b.x, ux), (a.y, uy), (b.y, uy)] {
                    assert!(on_grid(v, u), "{mode} {snap:?}: edge {v} not on {u}");
                }
                assert!(b.x - a.x >= ux - 1e-4 && b.y - a.y >= uy - 1e-4);
                // Each edge moves at most half a grid step (or to the minimum size).
                let (pa, pb) = t.with_rotation(0.0).unwrap().canvas_bounds(size);
                assert!((a.x - pa.x).abs() <= ux / 2.0 + 1e-4);
                assert!((a.y - pa.y).abs() <= uy / 2.0 + 1e-4);
                if pb.x - pa.x >= ux {
                    assert!((b.x - pb.x).abs() <= ux / 2.0 + 1e-4);
                }
                assert!(on_grid(s.rotate(), step), "{}", s.rotate());
                assert!((s.rotate() - rot).abs() <= step / 2.0);
                assert_eq!(s.anchor(), t.anchor());
                assert_eq!((s.flip_h(), s.flip_v()), (false, true));
            }
        }
    }

    #[test]
    fn snap_serialises_kebab_case() {
        #[derive(Serialize, Deserialize, PartialEq, Debug)]
        struct S {
            snap: Snap,
        }
        let text = toml::to_string(&S {
            snap: Snap::C64Pixel,
        })
        .unwrap();
        assert_eq!(text.trim(), "snap = \"c64-pixel\"");
        let s: S = toml::from_str("snap = \"cell\"").unwrap();
        assert_eq!(s.snap, Snap::Cell);
        assert_eq!(Snap::default(), Snap::Off);
    }
}
