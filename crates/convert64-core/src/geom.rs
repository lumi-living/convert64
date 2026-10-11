//! Geometry shared by the compositor, masks, regions and the UI overlays.
//!
//! Four coordinate spaces, all with `+x` right and `+y` down, and pixel `(i, j)`
//! covering `[i, i + 1) × [j, j + 1)` (its centre is at `i + 0.5`):
//!
//! - **Layer space**: a layer's source pixels, at the source's own resolution.
//! - **Canvas space**: the screen in square hi-res pixels, 320 × 200 for every full-screen
//!   mode. Layer transforms (translate, scale) are in canvas pixels.
//! - **Target space**: the mode's logical pixels, 320 × 200 (hi-res) or 160 × 200
//!   (multicolor, each pixel two canvas pixels wide).
//! - **Super space**: the compositor's supersampled grid, `n ×` target space on both
//!   axes, so a multicolor super pixel is twice as wide as it is tall on screen.
//!
//! [`SuperGrid`] is the only type that knows the 2:1 multicolor aspect; everything
//! else converts through it.

use crate::mode::{Mode, PixelAspect};
use crate::model::CellPos;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::ops::{Add, Mul, Sub};

/// A point or offset in one of the spaces above, in `f32` so that positions can be
/// fractional (sub-pixel).
#[derive(Clone, Copy, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct Vec2 {
    /// Horizontal component.
    pub x: f32,
    /// Vertical component.
    pub y: f32,
}

impl Vec2 {
    /// `(0, 0)`.
    pub const ZERO: Self = Self::new(0.0, 0.0);
    /// `(1, 1)`.
    pub const ONE: Self = Self::new(1.0, 1.0);

    /// The vector `(x, y)`.
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    /// Both components are finite (not NaN or infinite).
    pub const fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }
}

impl Add for Vec2 {
    type Output = Self;
    fn add(self, o: Self) -> Self {
        Self::new(self.x + o.x, self.y + o.y)
    }
}

impl Sub for Vec2 {
    type Output = Self;
    fn sub(self, o: Self) -> Self {
        Self::new(self.x - o.x, self.y - o.y)
    }
}

impl Mul<f32> for Vec2 {
    type Output = Self;
    fn mul(self, k: f32) -> Self {
        Self::new(self.x * k, self.y * k)
    }
}

/// A half-open rectangle of whole pixels, `[x0, x1) × [y0, y1)`, in whichever space
/// the context says (usually super space).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct PxRect {
    /// Left edge (inclusive).
    pub x0: u32,
    /// Top edge (inclusive).
    pub y0: u32,
    /// Right edge (exclusive).
    pub x1: u32,
    /// Bottom edge (exclusive).
    pub y1: u32,
}

impl PxRect {
    /// The empty rectangle.
    pub const EMPTY: Self = Self::new(0, 0, 0, 0);

    /// `[x0, x1) × [y0, y1)`; an inverted rectangle is empty.
    pub const fn new(x0: u32, y0: u32, x1: u32, y1: u32) -> Self {
        Self { x0, y0, x1, y1 }
    }

    /// The rectangle `[0, width) × [0, height)`.
    pub const fn of_size(width: u32, height: u32) -> Self {
        Self::new(0, 0, width, height)
    }

    /// Covers no pixel.
    pub const fn is_empty(self) -> bool {
        self.x1 <= self.x0 || self.y1 <= self.y0
    }

    /// Width in pixels (0 when empty).
    pub const fn width(self) -> u32 {
        self.x1.saturating_sub(self.x0)
    }

    /// Height in pixels (0 when empty).
    pub const fn height(self) -> u32 {
        self.y1.saturating_sub(self.y0)
    }

    /// The smallest rectangle covering both; an empty side is ignored.
    pub fn union(self, o: Self) -> Self {
        if self.is_empty() {
            return o;
        }
        if o.is_empty() {
            return self;
        }
        Self::new(
            self.x0.min(o.x0),
            self.y0.min(o.y0),
            self.x1.max(o.x1),
            self.y1.max(o.y1),
        )
    }

    /// The overlap of both, or [`Self::EMPTY`].
    pub fn intersect(self, o: Self) -> Self {
        let r = Self::new(
            self.x0.max(o.x0),
            self.y0.max(o.y0),
            self.x1.min(o.x1),
            self.y1.min(o.y1),
        );
        if r.is_empty() { Self::EMPTY } else { r }
    }

    /// Grown by `by` pixels on every side, clipped to `bounds`.
    pub fn grow(self, by: u32, bounds: Self) -> Self {
        if self.is_empty() {
            return self;
        }
        Self::new(
            self.x0.saturating_sub(by),
            self.y0.saturating_sub(by),
            self.x1.saturating_add(by),
            self.y1.saturating_add(by),
        )
        .intersect(bounds)
    }

    /// Whether pixel `(x, y)` is inside.
    pub const fn contains(self, x: u32, y: u32) -> bool {
        x >= self.x0 && x < self.x1 && y >= self.y0 && y < self.y1
    }
}

/// A set of cells on the 40 × 25 screen (every compositor target has that cell grid),
/// as a bit set. The answer to "which cells did this change touch?".
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct CellSet([u64; 16]);

impl CellSet {
    /// Cells across.
    pub const COLS: u16 = 40;
    /// Cells down.
    pub const ROWS: u16 = 25;

    /// No cells.
    pub const EMPTY: Self = Self([0; 16]);

    /// Every cell.
    pub fn all() -> Self {
        let mut s = Self::EMPTY;
        s.insert_rect(0, 0, Self::COLS, Self::ROWS);
        s
    }

    const fn bit(c: CellPos) -> Option<usize> {
        if c.x < Self::COLS && c.y < Self::ROWS {
            Some(c.y as usize * Self::COLS as usize + c.x as usize)
        } else {
            None
        }
    }

    /// Adds a cell; cells off the screen are ignored.
    pub fn insert(&mut self, c: CellPos) {
        if let Some(b) = Self::bit(c) {
            self.0[b / 64] |= 1 << (b % 64);
        }
    }

    /// Adds the cells in columns `x0..x1` and rows `y0..y1` (clipped to the screen).
    pub fn insert_rect(&mut self, x0: u16, y0: u16, x1: u16, y1: u16) {
        for y in y0..y1.min(Self::ROWS) {
            for x in x0..x1.min(Self::COLS) {
                self.insert(CellPos::new(x, y));
            }
        }
    }

    /// Whether the cell is in the set.
    pub const fn contains(&self, c: CellPos) -> bool {
        match Self::bit(c) {
            Some(b) => self.0[b / 64] & (1 << (b % 64)) != 0,
            None => false,
        }
    }

    /// Number of cells.
    pub fn len(&self) -> usize {
        self.0.iter().map(|w| w.count_ones() as usize).sum()
    }

    /// No cells.
    pub fn is_empty(&self) -> bool {
        self.0.iter().all(|&w| w == 0)
    }

    /// Cells in either set.
    pub fn union(mut self, o: Self) -> Self {
        for (a, b) in self.0.iter_mut().zip(o.0) {
            *a |= b;
        }
        self
    }

    /// Cells in both sets.
    pub fn intersection(mut self, o: Self) -> Self {
        for (a, b) in self.0.iter_mut().zip(o.0) {
            *a &= b;
        }
        self
    }

    /// The cells, row by row.
    pub fn iter(&self) -> impl Iterator<Item = CellPos> + '_ {
        (0..Self::ROWS).flat_map(move |y| {
            (0..Self::COLS)
                .map(move |x| CellPos::new(x, y))
                .filter(|c| self.contains(*c))
        })
    }

    /// The cells a super-grid rectangle overlaps.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "cell coordinates are below 40 × 25"
    )]
    pub fn from_super_rect(grid: SuperGrid, r: PxRect) -> Self {
        let mut s = Self::EMPTY;
        let r = r.intersect(grid.bounds());
        if r.is_empty() {
            return s;
        }
        let (cw, ch) = grid.cell_size_super();
        s.insert_rect(
            (r.x0 / cw) as u16,
            (r.y0 / ch) as u16,
            r.x1.div_ceil(cw) as u16,
            r.y1.div_ceil(ch) as u16,
        );
        s
    }
}

impl fmt::Debug for CellSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_set()
            .entries(self.iter().map(|c| (c.x, c.y)))
            .finish()
    }
}

impl FromIterator<CellPos> for CellSet {
    fn from_iter<I: IntoIterator<Item = CellPos>>(it: I) -> Self {
        let mut s = Self::EMPTY;
        for c in it {
            s.insert(c);
        }
        s
    }
}

/// Why a [`SuperGrid`] could not be built.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GridError {
    /// The supersample factor is outside 1–8.
    Factor(u8),
    /// The mode is not a full-screen bitmap or charset mode with a specification.
    Mode(Mode),
}

impl fmt::Display for GridError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Factor(n) => write!(f, "supersample factor must be 1–8, got {n}"),
            Self::Mode(m) => write!(f, "{m} is not a full-screen target the compositor supports"),
        }
    }
}

impl std::error::Error for GridError {}

#[derive(Deserialize)]
struct SuperGridData {
    n: u8,
    target: Mode,
}

/// The compositor's supersampled grid: `n ×` the target mode's logical resolution
/// (default 4×, so 1280 × 800 for hi-res and 640 × 800 for multicolor).
///
/// Converts between the spaces in the [module docs](self); the multicolor 2:1 pixel
/// aspect lives here and nowhere else. Masks, regions, the cursor and the cell-badge
/// overlay all map through these helpers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "SuperGridData")]
pub struct SuperGrid {
    n: u8,
    target: Mode,
    /// Target size, cell size and aspect copied from the mode spec at construction.
    #[serde(skip)]
    geom: TargetGeom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
struct TargetGeom {
    width: u16,
    height: u16,
    cell_width: u8,
    cell_height: u8,
    /// Canvas pixels per target pixel, horizontally (1 or 2).
    aspect: u8,
}

impl TryFrom<SuperGridData> for SuperGrid {
    type Error = GridError;
    fn try_from(d: SuperGridData) -> Result<Self, GridError> {
        Self::new(d.target, d.n)
    }
}

impl SuperGrid {
    /// The default supersample factor (`docs/TAD.md` §2).
    pub const DEFAULT_N: u8 = 4;

    /// The grid for `target` at `n ×` (1–8). Only full-screen bitmap and charset modes
    /// are compositor targets; sprites and the reserved M3 modes are rejected.
    pub fn new(target: Mode, n: u8) -> Result<Self, GridError> {
        if !(1..=8).contains(&n) {
            return Err(GridError::Factor(n));
        }
        let spec = target.spec().map_err(|_| GridError::Mode(target))?;
        let aspect = match spec.aspect {
            PixelAspect::Square => 1,
            PixelAspect::DoubleWide => 2,
            PixelAspect::Mixed => return Err(GridError::Mode(target)),
        };
        if target.is_sprite() || u32::from(spec.width) * u32::from(aspect) != 320 {
            return Err(GridError::Mode(target));
        }
        Ok(Self {
            n,
            target,
            geom: TargetGeom {
                width: spec.width,
                height: spec.height,
                cell_width: spec.cell_width,
                cell_height: spec.cell_height,
                aspect,
            },
        })
    }

    /// The grid for `target` at the default 4×.
    pub fn with_default_factor(target: Mode) -> Result<Self, GridError> {
        Self::new(target, Self::DEFAULT_N)
    }

    /// The supersample factor.
    pub const fn n(self) -> u8 {
        self.n
    }

    /// The target mode.
    pub const fn target(self) -> Mode {
        self.target
    }

    /// Target size in logical pixels: 320 × 200 or 160 × 200.
    pub const fn target_size(self) -> (u32, u32) {
        (self.geom.width as u32, self.geom.height as u32)
    }

    /// Super-grid size in super pixels: `n ×` the target size.
    pub const fn size(self) -> (u32, u32) {
        let (w, h) = self.target_size();
        (w * self.n as u32, h * self.n as u32)
    }

    /// The whole super grid as a rectangle.
    pub const fn bounds(self) -> PxRect {
        let (w, h) = self.size();
        PxRect::of_size(w, h)
    }

    /// Canvas size in square pixels: always 320 × 200 for the supported targets.
    pub fn canvas_size(self) -> Vec2 {
        let (w, h) = self.target_size();
        Vec2::new((w * u32::from(self.geom.aspect)) as f32, h as f32)
    }

    /// Cell size in target pixels: 8 × 8 (hi-res) or 4 × 8 (multicolor).
    pub const fn cell_size(self) -> (u32, u32) {
        (self.geom.cell_width as u32, self.geom.cell_height as u32)
    }

    /// Cell size in super pixels.
    pub const fn cell_size_super(self) -> (u32, u32) {
        let (w, h) = self.cell_size();
        (w * self.n as u32, h * self.n as u32)
    }

    /// Number of cells across and down (40 × 25 for every supported target).
    pub const fn cells(self) -> (u32, u32) {
        let (w, h) = self.target_size();
        let (cw, ch) = self.cell_size();
        (w / cw, h / ch)
    }

    /// The size of one super pixel in canvas pixels: `(1/n, 1/n)` for hi-res and
    /// `(2/n, 1/n)` for multicolor.
    pub fn super_px_in_canvas(self) -> Vec2 {
        let n = f32::from(self.n);
        Vec2::new(f32::from(self.geom.aspect) / n, 1.0 / n)
    }

    /// Target-space point to super space.
    pub fn target_to_super(self, p: Vec2) -> Vec2 {
        p * f32::from(self.n)
    }

    /// Super-space point to target space.
    pub fn super_to_target(self, p: Vec2) -> Vec2 {
        p * (1.0 / f32::from(self.n))
    }

    /// Canvas-space point to target space.
    pub fn canvas_to_target(self, p: Vec2) -> Vec2 {
        Vec2::new(p.x / f32::from(self.geom.aspect), p.y)
    }

    /// Target-space point to canvas space.
    pub fn target_to_canvas(self, p: Vec2) -> Vec2 {
        Vec2::new(p.x * f32::from(self.geom.aspect), p.y)
    }

    /// Canvas-space point to super space.
    pub fn canvas_to_super(self, p: Vec2) -> Vec2 {
        self.target_to_super(self.canvas_to_target(p))
    }

    /// Super-space point to canvas space.
    pub fn super_to_canvas(self, p: Vec2) -> Vec2 {
        self.target_to_canvas(self.super_to_target(p))
    }

    /// The centre of super pixel `(x, y)` in canvas space.
    pub fn super_px_centre_in_canvas(self, x: u32, y: u32) -> Vec2 {
        self.super_to_canvas(Vec2::new(x as f32 + 0.5, y as f32 + 0.5))
    }

    /// The target pixel containing super pixel `(x, y)`.
    pub const fn super_px_to_target_px(self, x: u32, y: u32) -> (u32, u32) {
        (x / self.n as u32, y / self.n as u32)
    }

    /// The super pixels covering target pixel `(x, y)`.
    pub const fn target_px_to_super_rect(self, x: u32, y: u32) -> PxRect {
        let n = self.n as u32;
        PxRect::new(x * n, y * n, (x + 1) * n, (y + 1) * n)
    }

    /// The cell containing target pixel `(x, y)`, or `None` outside the screen.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "cell coordinates are below 40 × 25"
    )]
    pub const fn target_px_to_cell(self, x: u32, y: u32) -> Option<CellPos> {
        let (w, h) = self.target_size();
        if x >= w || y >= h {
            return None;
        }
        let (cw, ch) = self.cell_size();
        Some(CellPos::new((x / cw) as u16, (y / ch) as u16))
    }

    /// The cell under a canvas-space point (the cursor), or `None` off screen.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the point is checked to be on screen first"
    )]
    pub fn canvas_to_cell(self, p: Vec2) -> Option<CellPos> {
        let t = self.canvas_to_target(p);
        let (w, h) = self.target_size();
        if !(t.x >= 0.0 && t.y >= 0.0 && t.x < w as f32 && t.y < h as f32) {
            return None;
        }
        self.target_px_to_cell(t.x as u32, t.y as u32)
    }

    /// The target pixels of a cell.
    pub const fn cell_target_rect(self, cell: CellPos) -> PxRect {
        let (cw, ch) = self.cell_size();
        let (x, y) = (cell.x as u32 * cw, cell.y as u32 * ch);
        PxRect::new(x, y, x + cw, y + ch)
    }

    /// The super pixels of a cell.
    pub const fn cell_super_rect(self, cell: CellPos) -> PxRect {
        let (cw, ch) = self.cell_size_super();
        let (x, y) = (cell.x as u32 * cw, cell.y as u32 * ch);
        PxRect::new(x, y, x + cw, y + ch)
    }

    /// The cell's top-left corner in canvas space (for the cell-badge overlay).
    pub fn cell_origin_in_canvas(self, cell: CellPos) -> Vec2 {
        let r = self.cell_target_rect(cell);
        self.target_to_canvas(Vec2::new(r.x0 as f32, r.y0 as f32))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_for_both_aspects() {
        let hi = SuperGrid::new(Mode::HiresBitmap, 4).unwrap();
        assert_eq!(hi.size(), (1280, 800));
        assert_eq!(hi.cell_size_super(), (32, 32));
        assert_eq!(hi.cells(), (40, 25));
        let mc = SuperGrid::new(Mode::MulticolorBitmap, 4).unwrap();
        assert_eq!(mc.size(), (640, 800));
        assert_eq!(mc.cell_size_super(), (16, 32));
        assert_eq!(mc.cells(), (40, 25));
        assert_eq!(mc.canvas_size(), Vec2::new(320.0, 200.0));
        assert_eq!(mc.super_px_in_canvas(), Vec2::new(0.5, 0.25));
    }

    #[test]
    fn rejects_bad_factor_and_modes() {
        assert_eq!(
            SuperGrid::new(Mode::HiresBitmap, 0),
            Err(GridError::Factor(0))
        );
        assert_eq!(
            SuperGrid::new(Mode::HiresBitmap, 9),
            Err(GridError::Factor(9))
        );
        for m in [
            Mode::HiresSprite,
            Mode::MulticolorSprite,
            Mode::OverlaySprite,
            Mode::Fli,
        ] {
            assert_eq!(SuperGrid::new(m, 4), Err(GridError::Mode(m)), "{m}");
        }
        for m in [
            Mode::MulticolorBitmap,
            Mode::HiresBitmap,
            Mode::MulticolorCharset,
            Mode::HiresCharset,
            Mode::EcmCharset,
        ] {
            assert!(SuperGrid::new(m, 1).is_ok(), "{m}");
        }
    }

    #[test]
    fn round_trips_between_spaces() {
        for mode in [Mode::HiresBitmap, Mode::MulticolorBitmap] {
            for n in 1..=8 {
                let g = SuperGrid::new(mode, n).unwrap();
                for p in [
                    Vec2::new(0.0, 0.0),
                    Vec2::new(13.25, 7.5),
                    Vec2::new(319.9, 199.75),
                ] {
                    let back = g.super_to_canvas(g.canvas_to_super(p));
                    assert!((back.x - p.x).abs() < 1e-4 && (back.y - p.y).abs() < 1e-4);
                    let t = g.canvas_to_target(p);
                    assert_eq!(g.target_to_canvas(t), p);
                    let back = g.super_to_target(g.target_to_super(t));
                    assert!((back.x - t.x).abs() < 1e-4 && (back.y - t.y).abs() < 1e-4);
                }
            }
        }
    }

    #[test]
    fn multicolor_pixels_are_double_wide_on_canvas() {
        let g = SuperGrid::new(Mode::MulticolorBitmap, 2).unwrap();
        // Target pixel 10 spans canvas x 20..22.
        assert_eq!(g.target_to_canvas(Vec2::new(10.0, 0.0)).x, 20.0);
        // Super pixel (1, 1) centre: target (0.75, 0.75) → canvas (1.5, 0.75).
        assert_eq!(g.super_px_centre_in_canvas(1, 1), Vec2::new(1.5, 0.75));
        assert_eq!(
            g.canvas_to_cell(Vec2::new(8.5, 8.0)),
            Some(CellPos::new(1, 1))
        );
        assert_eq!(g.canvas_to_cell(Vec2::new(320.0, 0.0)), None);
        assert_eq!(g.canvas_to_cell(Vec2::new(-0.1, 0.0)), None);
        assert_eq!(
            g.cell_origin_in_canvas(CellPos::new(1, 1)),
            Vec2::new(8.0, 8.0)
        );
    }

    #[test]
    fn cell_rects_tile_the_grid() {
        for mode in [Mode::HiresBitmap, Mode::MulticolorBitmap] {
            let g = SuperGrid::new(mode, 3).unwrap();
            let (cx, cy) = g.cells();
            let last = g.cell_super_rect(CellPos::new((cx - 1) as u16, (cy - 1) as u16));
            assert_eq!((last.x1, last.y1), g.size());
            let r = g.target_px_to_super_rect(5, 9);
            assert_eq!(g.super_px_to_target_px(r.x0, r.y0), (5, 9));
            assert_eq!(g.super_px_to_target_px(r.x1 - 1, r.y1 - 1), (5, 9));
        }
    }

    #[test]
    fn cell_sets() {
        let g = SuperGrid::new(Mode::MulticolorBitmap, 4).unwrap();
        // Super x 15..17 straddles the cell boundary at 16 (multicolor cells are 16 wide).
        let s = CellSet::from_super_rect(g, PxRect::new(15, 31, 17, 33));
        let want: CellSet = [(0, 0), (1, 0), (0, 1), (1, 1)]
            .into_iter()
            .map(|(x, y)| CellPos::new(x, y))
            .collect();
        assert_eq!(s, want);
        assert_eq!(s.len(), 4);
        assert_eq!(CellSet::all().len(), 1000);
        assert!(CellSet::from_super_rect(g, PxRect::EMPTY).is_empty());
        assert_eq!(s.intersection(CellSet::EMPTY), CellSet::EMPTY);
        assert_eq!(s.union(CellSet::all()), CellSet::all());
        let mut t = CellSet::EMPTY;
        t.insert(CellPos::new(40, 0));
        assert!(t.is_empty());
    }

    #[test]
    fn rect_algebra() {
        let a = PxRect::new(0, 0, 10, 10);
        let b = PxRect::new(5, 5, 20, 8);
        assert_eq!(a.union(b), PxRect::new(0, 0, 20, 10));
        assert_eq!(a.intersect(b), PxRect::new(5, 5, 10, 8));
        assert!(a.intersect(PxRect::new(10, 0, 12, 2)).is_empty());
        assert_eq!(PxRect::EMPTY.union(b), b);
        assert_eq!(
            b.grow(3, PxRect::of_size(21, 21)),
            PxRect::new(2, 2, 21, 11)
        );
    }

    #[test]
    fn serde_goes_through_the_constructor() {
        let g: SuperGrid = toml::from_str("n = 4\ntarget = \"multicolor-bitmap\"").unwrap();
        assert_eq!(g, SuperGrid::new(Mode::MulticolorBitmap, 4).unwrap());
        assert_eq!(g.size(), (640, 800));
        assert!(toml::from_str::<SuperGrid>("n = 9\ntarget = \"hires-bitmap\"").is_err());
        assert!(toml::from_str::<SuperGrid>("n = 2\ntarget = \"hires-sprite\"").is_err());
        assert_eq!(
            toml::to_string(&g).unwrap(),
            "n = 4\ntarget = \"multicolor-bitmap\"\n"
        );
    }
}
