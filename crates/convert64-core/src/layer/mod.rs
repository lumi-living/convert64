//! Layers: shared source pixels, a float transform, blending, a mask, adjustments and
//! the dither recipe they carry into the attribute map (`docs/TAD.md` §2, ADR-002).
//!
//! Constructors and setters validate, so an invalid layer cannot be represented.

pub mod adjust;
pub mod recipe;
pub mod snap;

pub use adjust::{
    AdjustCache, AdjustError, AdjustPlan, Adjustments, BrightnessContrast, Curve, Curves, Gamma,
    Hsl, Levels, Radius, RangeMatch, Sharpen, suggest_range_match,
};
pub use recipe::{
    CellMetric, CustomMatrix, DitherFamily, DitherKind, FitMode, Method, Metric, OrderedSemantics,
    Orientation, PatternPhase, Recipe, RecipeError,
};
pub use snap::{Snap, snap_transform};

use crate::geom::Vec2;
use crate::mask::{Mask, MaskError};
use crate::palette::srgb8_to_linear;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::Arc;

/// Largest accepted source width or height, in pixels.
pub const MAX_SOURCE_DIM: u32 = 16_384;

/// Why a layer value could not be built.
#[derive(Clone, Debug, PartialEq)]
pub enum LayerError {
    /// Width or height is zero or above [`MAX_SOURCE_DIM`].
    SourceSize {
        /// Width given.
        width: u32,
        /// Height given.
        height: u32,
    },
    /// A pixel buffer has the wrong length.
    WrongLength {
        /// What the buffer is.
        what: &'static str,
        /// Required length.
        expected: usize,
        /// Actual length.
        found: usize,
    },
    /// A value is NaN or infinite.
    NotFinite {
        /// Which value.
        what: &'static str,
    },
    /// A value is outside its range.
    OutOfRange {
        /// Which value.
        what: &'static str,
        /// The value given.
        value: f32,
    },
    /// The mask does not fit the layer.
    Mask(MaskError),
}

impl fmt::Display for LayerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SourceSize { width, height } => write!(
                f,
                "source must be 1–{MAX_SOURCE_DIM} pixels on each side, got {width}×{height}"
            ),
            Self::WrongLength {
                what,
                expected,
                found,
            } => write!(f, "{what} must have {expected} values, found {found}"),
            Self::NotFinite { what } => write!(f, "{what} must be finite"),
            Self::OutOfRange { what, value } => write!(f, "{what} is out of range: {value}"),
            Self::Mask(e) => write!(f, "mask: {e}"),
        }
    }
}

impl std::error::Error for LayerError {}

impl From<MaskError> for LayerError {
    fn from(e: MaskError) -> Self {
        Self::Mask(e)
    }
}

/// Source pixels in f32 **linear-light, straight-alpha** RGBA, decoded once.
///
/// Shared between layers by `Arc` so that `LayerViaMask` (ADR-008) creates a layer
/// without copying pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct SourceImage {
    width: u32,
    height: u32,
    pixels: Vec<[f32; 4]>,
    /// The 8-bit sRGB colour it was decoded from, when it came from 8-bit bytes and is
    /// fully opaque: png2c64's front end (#190) starts from these bytes.
    srgb8: Option<Arc<[[u8; 3]]>>,
}

impl SourceImage {
    fn check_size(width: u32, height: u32) -> Result<usize, LayerError> {
        if width == 0 || height == 0 || width > MAX_SOURCE_DIM || height > MAX_SOURCE_DIM {
            return Err(LayerError::SourceSize { width, height });
        }
        Ok(width as usize * height as usize)
    }

    /// Decodes 8-bit sRGB RGBA bytes (row-major, 4 bytes per pixel, straight alpha, as a
    /// PNG decoder returns them): colour through the sRGB LUT, alpha linearly.
    pub fn from_srgba8(width: u32, height: u32, rgba: &[u8]) -> Result<Self, LayerError> {
        let len = Self::check_size(width, height)?;
        if rgba.len() != len * 4 {
            return Err(LayerError::WrongLength {
                what: "RGBA bytes",
                expected: len * 4,
                found: rgba.len(),
            });
        }
        let pixels = rgba
            .as_chunks::<4>()
            .0
            .iter()
            .map(|p| {
                [
                    srgb8_to_linear(p[0]),
                    srgb8_to_linear(p[1]),
                    srgb8_to_linear(p[2]),
                    f32::from(p[3]) / 255.0,
                ]
            })
            .collect();
        let chunks = rgba.as_chunks::<4>().0;
        let srgb8 = chunks
            .iter()
            .all(|p| p[3] == 255)
            .then(|| chunks.iter().map(|p| [p[0], p[1], p[2]]).collect());
        Ok(Self {
            width,
            height,
            pixels,
            srgb8,
        })
    }

    /// The 8-bit sRGB colour of a fully opaque source decoded from bytes, row-major.
    pub fn srgb8(&self) -> Option<&[[u8; 3]]> {
        self.srgb8.as_deref()
    }

    /// Wraps linear straight-alpha pixels; every channel must be finite and alpha in
    /// `0..=1` (colour may exceed 1 for HDR sources).
    pub fn from_linear(width: u32, height: u32, pixels: Vec<[f32; 4]>) -> Result<Self, LayerError> {
        let len = Self::check_size(width, height)?;
        if pixels.len() != len {
            return Err(LayerError::WrongLength {
                what: "pixels",
                expected: len,
                found: pixels.len(),
            });
        }
        for p in &pixels {
            if !p.iter().all(|v| v.is_finite()) {
                return Err(LayerError::NotFinite { what: "pixel" });
            }
            if !(0.0..=1.0).contains(&p[3]) {
                return Err(LayerError::OutOfRange {
                    what: "alpha",
                    value: p[3],
                });
            }
        }
        Ok(Self {
            width,
            height,
            pixels,
            srgb8: None,
        })
    }

    /// Width in pixels.
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Height in pixels.
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// `(width, height)`.
    pub const fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Row-major pixels.
    pub fn pixels(&self) -> &[[f32; 4]] {
        &self.pixels
    }

    /// The pixel at `(x, y)`; panics outside the image.
    pub fn pixel(&self, x: u32, y: u32) -> [f32; 4] {
        assert!(
            x < self.width && y < self.height,
            "({x}, {y}) is outside the source"
        );
        self.pixels[y as usize * self.width as usize + x as usize]
    }

    /// Whether any pixel is less than fully opaque.
    pub fn has_alpha(&self) -> bool {
        self.pixels.iter().any(|p| p[3] < 1.0)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TransformData {
    #[serde(default)]
    translate: Vec2,
    #[serde(default = "unit_scale")]
    scale: Vec2,
    #[serde(default)]
    rotate: f32,
    #[serde(default = "centre")]
    anchor: Vec2,
    #[serde(default)]
    flip_h: bool,
    #[serde(default)]
    flip_v: bool,
}

const fn unit_scale() -> Vec2 {
    Vec2::ONE
}

const fn centre() -> Vec2 {
    Transform::CENTRE
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if passes a reference"
)]
fn is_zero(v: &f32) -> bool {
    *v == 0.0
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if passes a reference"
)]
fn is_centre(v: &Vec2) -> bool {
    *v == Transform::CENTRE
}

/// `(sin, cos)` of `deg` degrees with basic IEEE operations only (no platform `sin`), so
/// a rotated layer composites the same on every OS (as `composite::resample`'s
/// `sin_pi`): the angle is reduced to `|r| ≤ 45°` around a multiple of 90° and the
/// series summed in f64. A multiple of 90° is exact (`sin 90° = 1`, `cos 90° = 0`).
fn sin_cos_deg(deg: f32) -> (f64, f64) {
    let d = f64::from(deg).rem_euclid(360.0);
    let k = (d / 90.0).round();
    let r = (d - 90.0 * k).to_radians();
    let r2 = r * r;
    // Taylor series to r^17 / r^18 (|r| ≤ π/4).
    let (mut s, mut ts) = (r, r);
    let (mut c, mut tc) = (1.0, 1.0);
    for n in 1..=9 {
        let n = f64::from(n);
        ts = -ts * r2 / ((2.0 * n) * (2.0 * n + 1.0));
        tc = -tc * r2 / ((2.0 * n - 1.0) * (2.0 * n));
        s += ts;
        c += tc;
    }
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "k is 0..=4"
    )]
    match (k as u32) % 4 {
        0 => (s, c),
        1 => (c, -s),
        2 => (-s, -c),
        _ => (-c, s),
    }
}

/// Where a layer sits on the canvas, in square canvas pixels (`geom` module docs), y
/// down (ADR-011, amended 2026-10-13):
///
/// ```text
/// canvas = translate + c + R(rotate) · (S · F(p) − c),   c = S · (anchor ⊙ size)
/// ```
///
/// `F` mirrors within the layer's own box (`x → width − x` for `flip_h`), `S` scales
/// per axis, and `R` turns by `rotate` degrees, **positive clockwise on screen** (with
/// y down, `R = [cos −sin; sin cos]`). `translate` is the top-left corner of the
/// *unrotated* box, and the anchor (0–1 in layer space, default the centre) is only the
/// pivot the box turns about. With `rotate == 0` the anchor has no effect and this is
/// the axis-aligned `translate + S · F(p)` the compositor resamples separably.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "TransformData")]
pub struct Transform {
    translate: Vec2,
    scale: Vec2,
    #[serde(default, skip_serializing_if = "is_zero")]
    rotate: f32,
    #[serde(default = "centre", skip_serializing_if = "is_centre")]
    anchor: Vec2,
    flip_h: bool,
    flip_v: bool,
}

impl TryFrom<TransformData> for Transform {
    type Error = LayerError;
    fn try_from(d: TransformData) -> Result<Self, LayerError> {
        Self::new(d.translate, d.scale, d.flip_h, d.flip_v)?
            .with_rotation(d.rotate)?
            .with_anchor(d.anchor)
    }
}

impl Default for Transform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Transform {
    /// The default anchor: the layer's centre.
    pub const CENTRE: Vec2 = Vec2::new(0.5, 0.5);

    /// Source pixel `(0, 0)` at canvas `(0, 0)`, one source pixel per canvas pixel.
    pub const IDENTITY: Self = Self {
        translate: Vec2::ZERO,
        scale: Vec2::ONE,
        rotate: 0.0,
        anchor: Self::CENTRE,
        flip_h: false,
        flip_v: false,
    };

    /// An unrotated transform anchored at the centre; `translate` must be finite and both
    /// scale factors finite and > 0 (mirroring is the flips' job).
    pub fn new(
        translate: Vec2,
        scale: Vec2,
        flip_h: bool,
        flip_v: bool,
    ) -> Result<Self, LayerError> {
        if !translate.is_finite() {
            return Err(LayerError::NotFinite { what: "translate" });
        }
        if !scale.is_finite() {
            return Err(LayerError::NotFinite { what: "scale" });
        }
        for v in [scale.x, scale.y] {
            if v <= 0.0 {
                return Err(LayerError::OutOfRange {
                    what: "scale",
                    value: v,
                });
            }
        }
        Ok(Self {
            translate,
            scale,
            rotate: 0.0,
            anchor: Self::CENTRE,
            flip_h,
            flip_v,
        })
    }

    /// Identity with a translation.
    pub fn translated(translate: Vec2) -> Result<Self, LayerError> {
        Self::new(translate, Vec2::ONE, false, false)
    }

    /// This transform turned to `degrees` (finite; positive is clockwise on screen) about
    /// its anchor. The angle is kept as given: 360 is not folded to 0.
    pub fn with_rotation(self, degrees: f32) -> Result<Self, LayerError> {
        if !degrees.is_finite() {
            return Err(LayerError::NotFinite { what: "rotate" });
        }
        Ok(Self {
            rotate: degrees,
            ..self
        })
    }

    /// This transform with its pivot at `anchor` (0–1 in layer space on both axes). With
    /// a rotation, the picture turns about the new pivot, so it moves; see
    /// [`Self::reanchored`] to keep it in place.
    pub fn with_anchor(self, anchor: Vec2) -> Result<Self, LayerError> {
        if !anchor.is_finite() {
            return Err(LayerError::NotFinite { what: "anchor" });
        }
        for v in [anchor.x, anchor.y] {
            if !(0.0..=1.0).contains(&v) {
                return Err(LayerError::OutOfRange {
                    what: "anchor",
                    value: v,
                });
            }
        }
        Ok(Self { anchor, ..self })
    }

    /// This transform with its pivot moved to `anchor` and `translate` adjusted so the
    /// picture (a source of `size`) stays where it is: `translate' = translate + (I − R)
    /// (c − c')`. Without a rotation only the anchor changes.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "canvas coordinates are well within f32"
    )]
    pub fn reanchored(self, anchor: Vec2, size: (u32, u32)) -> Result<Self, LayerError> {
        let moved = self.with_anchor(anchor)?;
        if self.rotate == 0.0 {
            return Ok(moved);
        }
        let (c0, c1) = (self.pivot(size), moved.pivot(size));
        let (dx, dy) = (c0.0 - c1.0, c0.1 - c1.1);
        let (s, c) = sin_cos_deg(self.rotate);
        let shift = (dx - (c * dx - s * dy), dy - (s * dx + c * dy));
        let translate = Vec2::new(
            (f64::from(self.translate.x) + shift.0) as f32,
            (f64::from(self.translate.y) + shift.1) as f32,
        );
        if !translate.is_finite() {
            return Err(LayerError::NotFinite { what: "translate" });
        }
        Ok(Self { translate, ..moved })
    }

    /// This transform scaled to `scale` about its anchor (a source of `size`): the pivot
    /// `translate + S · (anchor ⊙ size)` stays where it is, which is how the app's
    /// Transform tool scales (UX §6–§7). [`Self::new`] and a plain scale change keep the
    /// top-left corner instead.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "canvas coordinates are well within f32"
    )]
    pub fn scaled_about_anchor(self, scale: Vec2, size: (u32, u32)) -> Result<Self, LayerError> {
        let t = Self::new(self.translate, scale, self.flip_h, self.flip_v)?;
        let t = Self {
            rotate: self.rotate,
            anchor: self.anchor,
            ..t
        };
        let (c0, c1) = (self.pivot(size), t.pivot(size));
        let translate = Vec2::new(
            (f64::from(self.translate.x) + c0.0 - c1.0) as f32,
            (f64::from(self.translate.y) + c0.1 - c1.1) as f32,
        );
        if !translate.is_finite() {
            return Err(LayerError::NotFinite { what: "translate" });
        }
        Ok(Self { translate, ..t })
    }

    /// Offset of the unrotated box's top-left corner, in canvas pixels.
    pub const fn translate(&self) -> Vec2 {
        self.translate
    }

    /// Canvas pixels per source pixel, per axis.
    pub const fn scale(&self) -> Vec2 {
        self.scale
    }

    /// Rotation in degrees about the anchor, positive clockwise on screen.
    pub const fn rotate(&self) -> f32 {
        self.rotate
    }

    /// The pivot of the rotation, 0–1 in layer space.
    pub const fn anchor(&self) -> Vec2 {
        self.anchor
    }

    /// Whether the layer's box stays axis-aligned (`rotate == 0`): the compositor's
    /// separable fast path.
    pub fn is_axis_aligned(&self) -> bool {
        self.rotate == 0.0
    }

    /// Mirrored left–right.
    pub const fn flip_h(&self) -> bool {
        self.flip_h
    }

    /// Mirrored top–bottom.
    pub const fn flip_v(&self) -> bool {
        self.flip_v
    }

    /// This transform moved by `by` canvas pixels.
    pub fn nudged(&self, by: Vec2) -> Result<Self, LayerError> {
        let t = Self::new(self.translate + by, self.scale, self.flip_h, self.flip_v)?;
        Ok(Self {
            rotate: self.rotate,
            anchor: self.anchor,
            ..t
        })
    }

    /// The pivot `c = S · (anchor ⊙ size)`, relative to `translate`, in f64.
    fn pivot(&self, size: (u32, u32)) -> (f64, f64) {
        (
            f64::from(self.scale.x) * f64::from(self.anchor.x) * f64::from(size.0),
            f64::from(self.scale.y) * f64::from(self.anchor.y) * f64::from(size.1),
        )
    }

    /// The map from canvas to layer space (source of `size`) as `[a, b, c, d, e, f]`:
    /// `u = a·x + b·y + c`, `v = d·x + e·y + f`. For the compositor's rotated path.
    pub(crate) fn canvas_to_layer_affine(&self, size: (u32, u32)) -> [f64; 6] {
        let (s, c) = sin_cos_deg(self.rotate);
        let (px, py) = self.pivot(size);
        let (tx, ty) = (
            f64::from(self.translate.x) + px,
            f64::from(self.translate.y) + py,
        );
        // q = pivot + Rᵀ (p − translate − pivot), then F(q / S).
        let (kx, ox) = if self.flip_h {
            (-1.0 / f64::from(self.scale.x), f64::from(size.0))
        } else {
            (1.0 / f64::from(self.scale.x), 0.0)
        };
        let (ky, oy) = if self.flip_v {
            (-1.0 / f64::from(self.scale.y), f64::from(size.1))
        } else {
            (1.0 / f64::from(self.scale.y), 0.0)
        };
        [
            kx * c,
            kx * s,
            kx * (px - c * tx - s * ty) + ox,
            -ky * s,
            ky * c,
            ky * (py + s * tx - c * ty) + oy,
        ]
    }

    /// A layer-space point (source of size `size`) to canvas space.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "canvas coordinates are well within f32"
    )]
    pub fn layer_to_canvas(&self, size: (u32, u32), p: Vec2) -> Vec2 {
        let fx = if self.flip_h {
            size.0 as f32 - p.x
        } else {
            p.x
        };
        let fy = if self.flip_v {
            size.1 as f32 - p.y
        } else {
            p.y
        };
        if self.rotate == 0.0 {
            return Vec2::new(
                self.translate.x + self.scale.x * fx,
                self.translate.y + self.scale.y * fy,
            );
        }
        let (px, py) = self.pivot(size);
        let dx = f64::from(self.scale.x) * f64::from(fx) - px;
        let dy = f64::from(self.scale.y) * f64::from(fy) - py;
        let (s, c) = sin_cos_deg(self.rotate);
        Vec2::new(
            (f64::from(self.translate.x) + px + c * dx - s * dy) as f32,
            (f64::from(self.translate.y) + py + s * dx + c * dy) as f32,
        )
    }

    /// A canvas-space point to layer space (source of size `size`).
    #[expect(
        clippy::cast_possible_truncation,
        reason = "layer coordinates are well within f32"
    )]
    pub fn canvas_to_layer(&self, size: (u32, u32), p: Vec2) -> Vec2 {
        if self.rotate == 0.0 {
            let fx = (p.x - self.translate.x) / self.scale.x;
            let fy = (p.y - self.translate.y) / self.scale.y;
            return Vec2::new(
                if self.flip_h { size.0 as f32 - fx } else { fx },
                if self.flip_v { size.1 as f32 - fy } else { fy },
            );
        }
        let [a, b, c, d, e, f] = self.canvas_to_layer_affine(size);
        let (x, y) = (f64::from(p.x), f64::from(p.y));
        Vec2::new((a * x + b * y + c) as f32, (d * x + e * y + f) as f32)
    }

    /// The layer's four corners on the canvas, in the order of source `(0, 0)`,
    /// `(w, 0)`, `(w, h)`, `(0, h)`: the rotated quad the app draws handles on.
    pub fn canvas_quad(&self, size: (u32, u32)) -> [Vec2; 4] {
        let (w, h) = (size.0 as f32, size.1 as f32);
        [
            Vec2::new(0.0, 0.0),
            Vec2::new(w, 0.0),
            Vec2::new(w, h),
            Vec2::new(0.0, h),
        ]
        .map(|p| self.layer_to_canvas(size, p))
    }

    /// The layer's axis-aligned bounding box on the canvas (of the rotated quad when
    /// rotated): top-left and bottom-right corners.
    pub fn canvas_bounds(&self, size: (u32, u32)) -> (Vec2, Vec2) {
        if self.rotate == 0.0 {
            let min = self.translate;
            let max = Vec2::new(
                self.translate.x + self.scale.x * size.0 as f32,
                self.translate.y + self.scale.y * size.1 as f32,
            );
            return (min, max);
        }
        let q = self.canvas_quad(size);
        let min = q
            .iter()
            .fold(Vec2::new(f32::INFINITY, f32::INFINITY), |m, p| {
                Vec2::new(m.x.min(p.x), m.y.min(p.y))
            });
        let max = q
            .iter()
            .fold(Vec2::new(f32::NEG_INFINITY, f32::NEG_INFINITY), |m, p| {
                Vec2::new(m.x.max(p.x), m.y.max(p.y))
            });
        (min, max)
    }
}

/// How a layer's colour combines with what is below it (W3C Compositing Level 1
/// separable modes), evaluated in linear light.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Blend {
    /// Source over.
    #[default]
    Normal,
    /// `Cb × Cs`.
    Multiply,
    /// `Cb + Cs − Cb × Cs`.
    Screen,
    /// Hard light with the layers swapped.
    Overlay,
}

/// One layer of a picture.
#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    name: String,
    source: Arc<SourceImage>,
    transform: Transform,
    opacity: f32,
    blend: Blend,
    visible: bool,
    mask: Option<Mask>,
    adjustments: Adjustments,
    recipe: Recipe,
}

impl Layer {
    /// A visible, unmasked, unadjusted layer at the identity transform, Normal blend,
    /// full opacity and the default recipe.
    pub fn new(name: impl Into<String>, source: Arc<SourceImage>) -> Self {
        Self {
            name: name.into(),
            source,
            transform: Transform::IDENTITY,
            opacity: 1.0,
            blend: Blend::Normal,
            visible: true,
            mask: None,
            adjustments: Adjustments::default(),
            recipe: Recipe::default(),
        }
    }

    /// Display name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Renames the layer.
    pub fn set_name(&mut self, name: impl Into<String>) {
        self.name = name.into();
    }

    /// The shared source pixels.
    pub fn source(&self) -> &Arc<SourceImage> {
        &self.source
    }

    /// Placement on the canvas.
    pub const fn transform(&self) -> Transform {
        self.transform
    }

    /// Moves, scales or flips the layer.
    pub fn set_transform(&mut self, t: Transform) {
        self.transform = t;
    }

    /// Opacity, `0..=1`.
    pub const fn opacity(&self) -> f32 {
        self.opacity
    }

    /// Sets the opacity; it must be in `0..=1`.
    pub fn set_opacity(&mut self, opacity: f32) -> Result<(), LayerError> {
        if !(0.0..=1.0).contains(&opacity) {
            return Err(LayerError::OutOfRange {
                what: "opacity",
                value: opacity,
            });
        }
        self.opacity = opacity;
        Ok(())
    }

    /// Blend mode.
    pub const fn blend(&self) -> Blend {
        self.blend
    }

    /// Sets the blend mode.
    pub fn set_blend(&mut self, blend: Blend) {
        self.blend = blend;
    }

    /// Whether the layer takes part in compositing.
    pub const fn visible(&self) -> bool {
        self.visible
    }

    /// Shows or hides the layer.
    pub fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
    }

    /// The mask, if any.
    pub const fn mask(&self) -> Option<&Mask> {
        self.mask.as_ref()
    }

    /// Mutable mask, if any. A mask's raster size cannot change, so it keeps fitting.
    pub fn mask_mut(&mut self) -> Option<&mut Mask> {
        self.mask.as_mut()
    }

    /// Sets or removes the mask; its raster must be the source's size.
    pub fn set_mask(&mut self, mask: Option<Mask>) -> Result<(), LayerError> {
        if let Some(m) = &mask
            && m.size() != self.source.size()
        {
            return Err(MaskError::Size {
                expected: self.source.size(),
                found: m.size(),
            }
            .into());
        }
        self.mask = mask;
        Ok(())
    }

    /// Colour adjustments.
    pub const fn adjustments(&self) -> &Adjustments {
        &self.adjustments
    }

    /// Replaces the colour adjustments.
    pub fn set_adjustments(&mut self, adjustments: Adjustments) {
        self.adjustments = adjustments;
    }

    /// Dither recipe.
    pub const fn recipe(&self) -> &Recipe {
        &self.recipe
    }

    /// Replaces the dither recipe.
    pub fn set_recipe(&mut self, recipe: Recipe) {
        self.recipe = recipe;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::palette::linear_to_srgb8;

    fn src(w: u32, h: u32) -> Arc<SourceImage> {
        Arc::new(SourceImage::from_srgba8(w, h, &vec![255; (w * h * 4) as usize]).unwrap())
    }

    #[test]
    fn source_rejects_bad_sizes_and_values() {
        for (w, h) in [(0, 1), (1, 0), (MAX_SOURCE_DIM + 1, 1)] {
            assert_eq!(
                SourceImage::from_srgba8(w, h, &[]),
                Err(LayerError::SourceSize {
                    width: w,
                    height: h
                })
            );
        }
        assert!(matches!(
            SourceImage::from_srgba8(2, 1, &[0; 7]),
            Err(LayerError::WrongLength { expected: 8, .. })
        ));
        assert!(matches!(
            SourceImage::from_linear(1, 1, vec![]),
            Err(LayerError::WrongLength { .. })
        ));
        assert_eq!(
            SourceImage::from_linear(1, 1, vec![[f32::NAN, 0.0, 0.0, 1.0]]),
            Err(LayerError::NotFinite { what: "pixel" })
        );
        assert_eq!(
            SourceImage::from_linear(1, 1, vec![[0.0, 0.0, 0.0, 1.5]]),
            Err(LayerError::OutOfRange {
                what: "alpha",
                value: 1.5
            })
        );
        assert!(SourceImage::from_linear(1, 1, vec![[2.0, 0.0, 0.0, 1.0]]).is_ok());
    }

    #[test]
    fn source_decodes_srgb_and_keeps_alpha_linear() {
        let s = SourceImage::from_srgba8(2, 1, &[255, 128, 0, 255, 10, 20, 30, 128]).unwrap();
        let p = s.pixel(0, 0);
        assert_eq!(p[0], 1.0);
        assert!((p[1] - 0.215_861).abs() < 1e-5, "{}", p[1]);
        assert_eq!(p[2], 0.0);
        assert_eq!(p[3], 1.0);
        let q = s.pixel(1, 0);
        assert_eq!(q[3], 128.0 / 255.0);
        assert_eq!([q[0], q[1], q[2]].map(linear_to_srgb8), [10, 20, 30]);
        assert!(s.has_alpha());
    }

    #[test]
    fn transform_rejects_non_finite_and_non_positive() {
        let nan = Vec2::new(f32::NAN, 0.0);
        assert_eq!(
            Transform::new(nan, Vec2::ONE, false, false),
            Err(LayerError::NotFinite { what: "translate" })
        );
        assert_eq!(
            Transform::new(Vec2::ZERO, Vec2::new(1.0, f32::INFINITY), false, false),
            Err(LayerError::NotFinite { what: "scale" })
        );
        for bad in [0.0, -1.0] {
            assert_eq!(
                Transform::new(Vec2::ZERO, Vec2::new(bad, 1.0), false, false),
                Err(LayerError::OutOfRange {
                    what: "scale",
                    value: bad
                })
            );
        }
        assert!(
            Transform::IDENTITY
                .nudged(Vec2::new(f32::INFINITY, 0.0))
                .is_err()
        );
    }

    #[test]
    fn transform_maps_both_ways_with_flips() {
        let size = (10, 20);
        for (fh, fv) in [(false, false), (true, false), (false, true), (true, true)] {
            let t = Transform::new(Vec2::new(3.25, -1.5), Vec2::new(2.0, 0.5), fh, fv).unwrap();
            for p in [
                Vec2::new(0.0, 0.0),
                Vec2::new(4.5, 7.25),
                Vec2::new(10.0, 20.0),
            ] {
                let back = t.canvas_to_layer(size, t.layer_to_canvas(size, p));
                assert!((back.x - p.x).abs() < 1e-5 && (back.y - p.y).abs() < 1e-5);
            }
        }
        let t = Transform::new(Vec2::new(1.0, 2.0), Vec2::new(2.0, 3.0), true, false).unwrap();
        // Flipped: source x = 0 lands on the right edge of the box.
        assert_eq!(t.layer_to_canvas(size, Vec2::ZERO), Vec2::new(21.0, 2.0));
        assert_eq!(
            t.canvas_bounds(size),
            (Vec2::new(1.0, 2.0), Vec2::new(21.0, 62.0))
        );
    }

    #[test]
    fn transform_deserialises_through_the_constructor() {
        let t: Transform = toml::from_str("translate = { x = 0.25, y = 1.0 }\n").unwrap();
        assert_eq!(t, Transform::translated(Vec2::new(0.25, 1.0)).unwrap());
        assert!(toml::from_str::<Transform>("scale = { x = 0.0, y = 1.0 }\n").is_err());
    }

    fn close(a: Vec2, b: Vec2) -> bool {
        (a.x - b.x).abs() < 1e-4 && (a.y - b.y).abs() < 1e-4
    }

    /// #198: positive angles turn clockwise on screen (y down) about the anchor, and
    /// `translate` stays the unrotated box's top-left corner.
    #[test]
    fn rotation_turns_clockwise_about_the_anchor() {
        let size = (10, 20);
        let t = Transform::translated(Vec2::ZERO)
            .unwrap()
            .with_rotation(90.0)
            .unwrap();
        // The box (centre (5, 10)) turned a quarter clockwise: the source's top-left
        // lands on the turned box's top-right.
        assert_eq!(t.layer_to_canvas(size, Vec2::ZERO), Vec2::new(15.0, 5.0));
        assert_eq!(
            t.layer_to_canvas(size, Vec2::new(5.0, 10.0)),
            Vec2::new(5.0, 10.0)
        );
        assert_eq!(
            t.canvas_bounds(size),
            (Vec2::new(-5.0, 5.0), Vec2::new(15.0, 15.0))
        );
        // About the top-left corner instead, it swings to the left of it.
        let t = t.with_anchor(Vec2::ZERO).unwrap();
        assert_eq!(t.layer_to_canvas(size, Vec2::ZERO), Vec2::ZERO);
        assert_eq!(
            t.layer_to_canvas(size, Vec2::new(0.0, 20.0)),
            Vec2::new(-20.0, 0.0)
        );
        // Unrotated, the anchor changes nothing.
        let a = Transform::new(Vec2::new(1.0, 2.0), Vec2::new(2.0, 3.0), true, false).unwrap();
        let b = a.with_anchor(Vec2::new(1.0, 0.0)).unwrap();
        for p in [Vec2::ZERO, Vec2::new(3.5, 7.25)] {
            assert_eq!(a.layer_to_canvas(size, p), b.layer_to_canvas(size, p));
            assert_eq!(a.canvas_to_layer(size, p), b.canvas_to_layer(size, p));
        }
        assert_eq!(a.canvas_bounds(size), b.canvas_bounds(size));
    }

    #[test]
    fn rotated_transform_maps_both_ways() {
        let size = (37, 23);
        for (fh, fv) in [(false, false), (true, false), (false, true), (true, true)] {
            for deg in [0.25, 33.0, -90.0, 181.5, 360.0] {
                let t = Transform::new(Vec2::new(3.25, -1.5), Vec2::new(2.0, 0.5), fh, fv)
                    .unwrap()
                    .with_rotation(deg)
                    .unwrap()
                    .with_anchor(Vec2::new(0.2, 0.9))
                    .unwrap();
                for p in [Vec2::ZERO, Vec2::new(4.5, 7.25), Vec2::new(37.0, 23.0)] {
                    let back = t.canvas_to_layer(size, t.layer_to_canvas(size, p));
                    assert!(close(back, p), "{deg} {fh} {fv}: {back:?} {p:?}");
                }
                // The bounding box holds the quad and touches it on every side.
                let (lo, hi) = t.canvas_bounds(size);
                let q = t.canvas_quad(size);
                for p in q {
                    assert!(p.x >= lo.x && p.x <= hi.x && p.y >= lo.y && p.y <= hi.y);
                }
                assert!(q.iter().any(|p| p.x == lo.x) && q.iter().any(|p| p.y == hi.y));
            }
        }
    }

    /// The anchor helper moves the pivot and keeps the picture where it is.
    #[test]
    fn reanchoring_keeps_the_picture_in_place() {
        let size = (37, 23);
        let t = Transform::new(Vec2::new(10.0, 20.0), Vec2::new(1.5, 0.75), false, true)
            .unwrap()
            .with_rotation(-41.0)
            .unwrap();
        for anchor in [Vec2::ZERO, Vec2::ONE, Vec2::new(0.1, 0.7)] {
            let r = t.reanchored(anchor, size).unwrap();
            assert_eq!(r.anchor(), anchor);
            assert_eq!(r.rotate(), t.rotate());
            for p in [Vec2::ZERO, Vec2::new(11.0, 5.5), Vec2::new(37.0, 23.0)] {
                assert!(close(
                    r.layer_to_canvas(size, p),
                    t.layer_to_canvas(size, p)
                ));
            }
        }
        assert!(t.reanchored(Vec2::new(1.5, 0.0), size).is_err());
        // Scaling about the anchor keeps the anchor's point on the canvas.
        let pivot =
            |t: &Transform| t.layer_to_canvas(size, Vec2::new(0.1 * 37.0, (1.0 - 0.7) * 23.0));
        let a = t.reanchored(Vec2::new(0.1, 0.7), size).unwrap();
        let b = a.scaled_about_anchor(Vec2::new(0.5, 3.0), size).unwrap();
        assert_eq!(b.scale(), Vec2::new(0.5, 3.0));
        assert!(
            close(pivot(&a), pivot(&b)),
            "{:?} {:?}",
            pivot(&a),
            pivot(&b)
        );
        assert!(a.scaled_about_anchor(Vec2::new(0.0, 1.0), size).is_err());
        let flat = Transform::translated(Vec2::new(4.0, 5.0)).unwrap();
        assert_eq!(
            flat.reanchored(Vec2::ZERO, size).unwrap().translate(),
            flat.translate()
        );
    }

    #[test]
    fn rotation_and_anchor_are_validated() {
        let t = Transform::IDENTITY;
        for bad in [f32::NAN, f32::INFINITY] {
            assert_eq!(
                t.with_rotation(bad),
                Err(LayerError::NotFinite { what: "rotate" })
            );
        }
        assert_eq!(
            t.with_anchor(Vec2::new(-0.1, 0.5)),
            Err(LayerError::OutOfRange {
                what: "anchor",
                value: -0.1
            })
        );
        assert!(t.with_anchor(Vec2::new(0.5, f32::NAN)).is_err());
        // A nudge keeps the rotation and the anchor.
        let r = t
            .with_rotation(12.0)
            .unwrap()
            .with_anchor(Vec2::ONE)
            .unwrap();
        let n = r.nudged(Vec2::new(0.25, 0.0)).unwrap();
        assert_eq!((n.rotate(), n.anchor()), (12.0, Vec2::ONE));
        assert!(!n.is_axis_aligned() && t.is_axis_aligned());
    }

    /// Old files load with no rotation, anchored at the centre; an unrotated, centred
    /// transform writes exactly the fields it always did (project bytes and goldens stay
    /// the same).
    #[test]
    fn rotation_and_anchor_are_optional_in_files() {
        let t: Transform =
            toml::from_str("translate = { x = 1.0, y = 2.0 }\nflip_h = true\n").unwrap();
        assert_eq!((t.rotate(), t.anchor()), (0.0, Transform::CENTRE));
        let text = toml::to_string(&t).unwrap();
        assert!(
            !text.contains("rotate") && !text.contains("anchor"),
            "{text}"
        );
        let r = t
            .with_rotation(-12.5)
            .unwrap()
            .with_anchor(Vec2::new(0.0, 1.0))
            .unwrap();
        let text = toml::to_string(&r).unwrap();
        assert!(
            text.contains("rotate = -12.5") && text.contains("anchor"),
            "{text}"
        );
        assert_eq!(toml::from_str::<Transform>(&text).unwrap(), r);
        assert!(toml::from_str::<Transform>("anchor = { x = 2.0, y = 0.0 }\n").is_err());
        assert!(
            toml::from_str::<Transform>("rotation = 5.0\n").is_err(),
            "a typo"
        );
    }

    #[test]
    fn sin_cos_is_exact_on_right_angles_and_close_elsewhere() {
        assert_eq!(sin_cos_deg(0.0), (0.0, 1.0));
        assert_eq!(sin_cos_deg(90.0), (1.0, 0.0));
        assert_eq!(sin_cos_deg(-90.0), (-1.0, 0.0));
        assert_eq!(sin_cos_deg(180.0), (0.0, -1.0));
        assert_eq!(sin_cos_deg(360.0), (0.0, 1.0));
        for i in -3600..=3600 {
            let d = i as f32 * 0.37;
            let (s, c) = sin_cos_deg(d);
            let r = f64::from(d).to_radians();
            assert!(
                (s - r.sin()).abs() < 1e-12 && (c - r.cos()).abs() < 1e-12,
                "{d}"
            );
        }
    }

    #[test]
    fn layer_setters_validate() {
        let mut l = Layer::new("a", src(4, 3));
        for bad in [-0.1, 1.1, f32::NAN] {
            assert!(l.set_opacity(bad).is_err());
        }
        assert_eq!(l.opacity(), 1.0);
        l.set_opacity(0.5).unwrap();
        assert_eq!(l.opacity(), 0.5);
        assert_eq!(
            l.set_mask(Some(Mask::opaque(3, 4).unwrap())),
            Err(LayerError::Mask(MaskError::Size {
                expected: (4, 3),
                found: (3, 4)
            }))
        );
        l.set_mask(Some(Mask::opaque(4, 3).unwrap())).unwrap();
        assert!(l.mask().is_some());
    }

    #[test]
    fn layers_share_source_pixels() {
        let s = src(8, 8);
        let a = Layer::new("a", Arc::clone(&s));
        let b = Layer::new("b", Arc::clone(&s));
        assert!(Arc::ptr_eq(a.source(), b.source()));
        assert_eq!(Arc::strong_count(&s), 3);
    }
}
