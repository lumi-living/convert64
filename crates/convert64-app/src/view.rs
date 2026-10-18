//! The working view's camera, compare mode and overlays (`docs/UX.md` §4): pure state and
//! geometry, tested without a window.
//!
//! Positions on the picture are in *screen units*: C64 hi-res pixels, 0–320 across and
//! 0–200 down in every mode, so multicolor pixels are two units wide and both modes share
//! one camera.

use egui::{Pos2, Rect, Vec2, pos2, vec2};

/// The C64 screen in screen units.
pub const SCREEN: Vec2 = vec2(320.0, 200.0);

/// The border drawn around the result in Result mode, in screen units on each side.
pub const BORDER: f32 = 32.0;

/// The margin *Fit* leaves around the picture in Source mode, as a fraction of the view on
/// each side (Figma's zoom-to-fit leaves about as much), so the transform handles on the
/// picture's edge do not sit on the view's edge (#246).
pub const FIT_MARGIN: f32 = 0.05;

/// How much of the border Result mode's *Fit* keeps in view on each side, in screen
/// units: half of it. A frame that is partly cropped still shows the edge colours against
/// the border, and it lets the picture open a step larger (#247).
pub const FIT_BORDER: f32 = BORDER / 2.0;

/// Source zoom steps, as scale factors (points per screen unit): 25 %–800 %.
pub const SOURCE_STEPS: [f32; 10] = [0.25, 0.5, 1.0, 1.5, 2.0, 3.0, 4.0, 5.0, 6.0, 8.0];

/// Result zoom steps: integers only, so pixels stay square blocks (UX §4.2).
pub const RESULT_STEPS: [u8; 6] = [1, 2, 3, 4, 6, 8];

/// What the working view shows (`Q` switches; UX §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ViewMode {
    /// The layer composite, at full precision. The default on open.
    #[default]
    Source,
    /// The C64 result, zoomed, with overlays.
    Result,
}

impl ViewMode {
    /// The other mode.
    pub fn flipped(self) -> Self {
        match self {
            Self::Source => Self::Result,
            Self::Result => Self::Source,
        }
    }

    /// The mode's name, as the status bar and the toolbar hint show it.
    pub fn label(self) -> &'static str {
        match self {
            Self::Source => "Source",
            Self::Result => "Result",
        }
    }
}

/// Where the working view looks and how close.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    /// The picture point at the centre of the view, in screen units.
    pub centre: Pos2,
    /// Source mode: points per screen unit (fractional, 0.25–8).
    pub source_zoom: f32,
    /// Result mode: points per screen unit (1–8, integer).
    pub result_zoom: u8,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            centre: (SCREEN / 2.0).to_pos2(),
            source_zoom: 2.0,
            result_zoom: 2,
        }
    }
}

impl Camera {
    /// Points per screen unit in `mode`.
    pub fn scale(&self, mode: ViewMode) -> f32 {
        match mode {
            ViewMode::Source => self.source_zoom,
            ViewMode::Result => f32::from(self.result_zoom),
        }
    }

    /// The zoom in `mode` as the status bar shows it, in percent.
    pub fn percent(&self, mode: ViewMode) -> u32 {
        (self.scale(mode) * 100.0).round() as u32
    }

    /// Where the whole screen lands in `view` (points). In Result mode the corner is
    /// rounded to whole points so pixels stay crisp.
    pub fn picture_rect(&self, view: Rect, mode: ViewMode) -> Rect {
        let s = self.scale(mode);
        let mut min = view.center() - self.centre.to_vec2() * s;
        if mode == ViewMode::Result {
            min = min.round();
        }
        Rect::from_min_size(min, SCREEN * s)
    }

    /// The picture point under `pos` (points) in `view`.
    pub fn to_picture(&self, view: Rect, mode: ViewMode, pos: Pos2) -> Pos2 {
        let r = self.picture_rect(view, mode);
        ((pos - r.min) / self.scale(mode)).to_pos2()
    }

    /// The visible part of the picture in `view`, in screen units.
    pub fn visible(&self, view: Rect, mode: ViewMode) -> Rect {
        Rect::from_min_max(
            self.to_picture(view, mode, view.min),
            self.to_picture(view, mode, view.max),
        )
    }

    /// Moves the picture by `delta` points.
    pub fn pan(&mut self, mode: ViewMode, delta: Vec2) {
        self.centre -= delta / self.scale(mode);
        self.clamp();
    }

    /// Keeps the centre on the picture.
    fn clamp(&mut self) {
        self.centre = self.centre.clamp(Pos2::ZERO, SCREEN.to_pos2());
    }

    /// One step in or out (`Cmd =` / `Cmd −`), about the view centre.
    pub fn step(&mut self, mode: ViewMode, zoom_in: bool) {
        match mode {
            ViewMode::Source => self.source_zoom = step(&SOURCE_STEPS, self.source_zoom, zoom_in),
            ViewMode::Result => {
                let steps = RESULT_STEPS.map(f32::from);
                self.result_zoom = step(&steps, f32::from(self.result_zoom), zoom_in) as u8;
            }
        }
    }

    /// Zooms by `factor` keeping the picture point under `anchor` where it is (wheel
    /// and pinch). Result mode moves to the nearest integer step in that direction.
    pub fn zoom_about(&mut self, view: Rect, mode: ViewMode, factor: f32, anchor: Pos2) {
        if factor == 1.0 {
            return;
        }
        let before = self.to_picture(view, mode, anchor);
        match mode {
            ViewMode::Source => {
                self.source_zoom = (self.source_zoom * factor)
                    .clamp(SOURCE_STEPS[0], SOURCE_STEPS[SOURCE_STEPS.len() - 1]);
            }
            ViewMode::Result => self.step(mode, factor > 1.0),
        }
        let after = self.to_picture(view, mode, anchor);
        self.centre += before - after;
        self.clamp();
    }

    /// Fits the screen in `view` (`Cmd 0`, and on open): fractional in Source mode,
    /// leaving [`FIT_MARGIN`] of the view free on each side; in Result mode the largest
    /// integer step at which the picture fits with [`FIT_BORDER`] of the border around it
    /// when the `border` is drawn, or with the Source margin when it is not. Screen units
    /// already carry multicolor's 2:1 pixels, so both modes fit alike.
    pub fn fit(&mut self, view: Rect, mode: ViewMode, border: bool) {
        self.centre = (SCREEN / 2.0).to_pos2();
        match mode {
            ViewMode::Source => {
                let room = view.size() * (1.0 - 2.0 * FIT_MARGIN);
                let s = (room.x / SCREEN.x).min(room.y / SCREEN.y);
                self.source_zoom = s.clamp(SOURCE_STEPS[0], SOURCE_STEPS[SOURCE_STEPS.len() - 1]);
            }
            ViewMode::Result => {
                let (room, framed) = if border {
                    (view.size(), SCREEN + Vec2::splat(2.0 * FIT_BORDER))
                } else {
                    (view.size() * (1.0 - 2.0 * FIT_MARGIN), SCREEN)
                };
                let s = (room.x / framed.x).min(room.y / framed.y);
                self.result_zoom = RESULT_STEPS
                    .into_iter()
                    .rev()
                    .find(|&z| f32::from(z) <= s)
                    .unwrap_or(1);
            }
        }
    }

    /// 100 % (`Cmd 1`).
    pub fn actual(&mut self, mode: ViewMode) {
        match mode {
            ViewMode::Source => self.source_zoom = 1.0,
            ViewMode::Result => self.result_zoom = 1,
        }
    }
}

/// The next step after `now` in `steps` (ascending), clamped at both ends.
fn step(steps: &[f32], now: f32, up: bool) -> f32 {
    let found = if up {
        steps.iter().copied().find(|&z| z > now + 1e-3)
    } else {
        steps.iter().rev().copied().find(|&z| z < now - 1e-3)
    };
    found.unwrap_or(if up { steps[steps.len() - 1] } else { steps[0] })
}

/// How the working view compares source and result (`,` cycles; UX §4.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Compare {
    /// One image (hold `\` to flip to the other).
    #[default]
    Off,
    /// Source left of a draggable divider, result right.
    Split,
    /// Source and result alternating at 2 Hz.
    Flicker,
    /// Source and result side by side, same place, linked cursor.
    SideBySide,
}

impl Compare {
    /// The next mode in the `,` cycle.
    pub fn next(self) -> Self {
        match self {
            Self::Off => Self::Split,
            Self::Split => Self::Flicker,
            Self::Flicker => Self::SideBySide,
            Self::SideBySide => Self::Off,
        }
    }

    /// The mode's name.
    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Split => "Split",
            Self::Flicker => "A/B flicker",
            Self::SideBySide => "Side by side",
        }
    }
}

/// The working view's overlays (UX §4.1–§4.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Overlays {
    /// The cell grid (`Cmd '`).
    pub grid: bool,
    /// The colour-limited cells (the core's clashes) as a heatmap, and as dots on the
    /// result panel (`Shift C`; off by default, #243).
    pub clashes: bool,
    /// Pinned and locked cells.
    pub steering: bool,
    /// The other image at 30 % over this one.
    pub ghost: bool,
    /// The border around the result (`Cmd B`).
    pub border: bool,
}

impl Default for Overlays {
    fn default() -> Self {
        Self {
            grid: false,
            clashes: false,
            steering: true,
            ghost: false,
            border: true,
        }
    }
}

/// The picture point `p` (screen units) as a target pixel and cell, or `None` off the
/// screen.
pub fn target_at(grid: convert64_core::SuperGrid, p: Pos2) -> Option<((u32, u32), (u32, u32))> {
    if !(p.x >= 0.0 && p.y >= 0.0 && p.x < SCREEN.x && p.y < SCREEN.y) {
        return None;
    }
    let (tw, _) = grid.target_size();
    let per = SCREEN.x / tw as f32;
    let (x, y) = ((p.x / per) as u32, p.y as u32);
    let cell = grid.target_px_to_cell(x, y)?;
    Some(((x, y), (u32::from(cell.x), u32::from(cell.y))))
}

/// A cell's rectangle in screen units.
pub fn cell_rect(grid: convert64_core::SuperGrid, cell: (u32, u32)) -> Rect {
    let (cw, ch) = grid.cell_size();
    let (tw, _) = grid.target_size();
    let per = SCREEN.x / tw as f32;
    let (w, h) = (cw as f32 * per, ch as f32);
    Rect::from_min_size(pos2(cell.0 as f32 * w, cell.1 as f32 * h), vec2(w, h))
}

#[cfg(test)]
mod tests {
    use super::*;
    use convert64_core::SuperGrid;
    use convert64_core::mode::Mode;

    fn view() -> Rect {
        Rect::from_min_size(pos2(10.0, 20.0), vec2(800.0, 600.0))
    }

    #[test]
    fn the_centre_stays_put_and_zoom_about_keeps_the_anchor() {
        let mut cam = Camera::default();
        let r = cam.picture_rect(view(), ViewMode::Source);
        assert_eq!(r.center(), view().center());
        let anchor = pos2(300.0, 200.0);
        let before = cam.to_picture(view(), ViewMode::Source, anchor);
        cam.zoom_about(view(), ViewMode::Source, 1.5, anchor);
        let after = cam.to_picture(view(), ViewMode::Source, anchor);
        assert!((before - after).length() < 1e-3, "{before:?} → {after:?}");
        assert!((cam.source_zoom - 3.0).abs() < 1e-6);
    }

    #[test]
    fn result_zoom_is_integer_and_fit_leaves_room_for_the_border() {
        let mut cam = Camera::default();
        cam.step(ViewMode::Result, true);
        assert_eq!(cam.result_zoom, 3);
        cam.zoom_about(view(), ViewMode::Result, 1.1, view().center());
        assert_eq!(cam.result_zoom, 4);
        cam.fit(view(), ViewMode::Result, true);
        // (320 + 32) × 2 = 704 ≤ 800 and (200 + 32) × 2 = 464 ≤ 600; × 3 does not fit.
        assert_eq!(cam.result_zoom, 2);
        cam.fit(view(), ViewMode::Source, true);
        // 800 × 600 less 5 % on each side is 720 × 540: 720 / 320 = 2.25.
        assert!((cam.source_zoom - 2.25).abs() < 1e-6);
        assert_eq!(cam.percent(ViewMode::Source), 225);
    }

    #[test]
    fn source_fit_leaves_a_margin_of_four_to_six_percent() {
        for size in [vec2(800.0, 600.0), vec2(1100.0, 400.0), vec2(500.0, 900.0)] {
            let view = Rect::from_min_size(pos2(10.0, 20.0), size);
            let mut cam = Camera {
                centre: pos2(3.0, 4.0),
                source_zoom: 8.0,
                result_zoom: 8,
            };
            cam.fit(view, ViewMode::Source, true);
            let picture = cam.picture_rect(view, ViewMode::Source);
            assert!(view.contains_rect(picture), "{size:?}: {picture:?}");
            assert_eq!(picture.center(), view.center(), "{size:?}");
            // On the axis that binds, the gap on each side is 4–6 % of the view.
            let gap = ((view.width() - picture.width()) / view.width())
                .min((view.height() - picture.height()) / view.height())
                / 2.0;
            assert!((0.04..=0.06).contains(&gap), "{size:?}: {gap}");
        }
    }

    #[test]
    fn result_fit_takes_the_largest_integer_zoom_that_fits() {
        let fit = |w: f32, h: f32, border: bool| {
            let view = Rect::from_min_size(pos2(10.0, 20.0), vec2(w, h));
            let mut cam = Camera::default();
            cam.fit(view, ViewMode::Result, border);
            let picture = cam.picture_rect(view, ViewMode::Result);
            assert!(view.contains_rect(picture), "{w}×{h}: {picture:?}");
            cam.result_zoom
        };
        // The review's 1440 × 900 browser window leaves a working view of about
        // 1120 × 816: 3× (960 × 600) fits with most of the border; the whole border
        // (1152 wide) would not, which kept it at 2×.
        assert_eq!(fit(1120.0, 816.0, true), 3);
        assert_eq!(fit(1120.0, 816.0, false), 3);
        // The border counts only when it is drawn: 680 high fits 3× (600 and 5 % above
        // and below) without it, but not with half of it (3 × 232 = 696).
        assert_eq!(fit(1300.0, 680.0, true), 2);
        assert_eq!(fit(1300.0, 680.0, false), 3);
        // Large views go up the integer steps (no 5×); small ones stop at 1×.
        assert_eq!(fit(2600.0, 1800.0, true), 6);
        assert_eq!(fit(400.0, 260.0, true), 1);
        // Multicolor's 2:1: one multicolor pixel (two screen units) is 2z × z points.
        let mut cam = Camera::default();
        cam.fit(view(), ViewMode::Result, true);
        let z = f32::from(cam.result_zoom);
        let picture = cam.picture_rect(view(), ViewMode::Result);
        assert_eq!(picture.width() / 160.0, 2.0 * z);
        assert_eq!(picture.height() / 200.0, z);
    }

    #[test]
    fn source_steps_clamp() {
        let mut cam = Camera::default();
        for _ in 0..20 {
            cam.step(ViewMode::Source, true);
        }
        assert_eq!(cam.source_zoom, 8.0);
        for _ in 0..20 {
            cam.step(ViewMode::Source, false);
        }
        assert_eq!(cam.source_zoom, 0.25);
    }

    #[test]
    fn screen_units_map_to_target_pixels_and_cells() {
        let mc = SuperGrid::new(Mode::MulticolorBitmap, 4).unwrap();
        assert_eq!(target_at(mc, pos2(9.5, 17.0)), Some(((4, 17), (1, 2))));
        assert_eq!(
            cell_rect(mc, (1, 2)),
            Rect::from_min_size(pos2(8.0, 16.0), vec2(8.0, 8.0))
        );
        let hi = SuperGrid::new(Mode::HiresBitmap, 4).unwrap();
        assert_eq!(target_at(hi, pos2(9.5, 17.0)), Some(((9, 17), (1, 2))));
        assert_eq!(target_at(hi, pos2(320.0, 0.0)), None);
    }

    #[test]
    fn compare_cycles_through_every_mode() {
        let mut c = Compare::Off;
        for want in [
            Compare::Split,
            Compare::Flicker,
            Compare::SideBySide,
            Compare::Off,
        ] {
            c = c.next();
            assert_eq!(c, want);
        }
    }
}
