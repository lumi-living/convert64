//! Layers: shared source pixels, a float transform, blending, a mask, adjustments and
//! the dither recipe they carry into the attribute map (`docs/TAD.md` §2, ADR-002).
//!
//! Constructors and setters validate, so an invalid layer cannot be represented.

pub mod adjust;
pub mod recipe;

pub use adjust::{
    AdjustCache, AdjustError, AdjustPlan, Adjustments, BrightnessContrast, Curve, Curves, Gamma,
    Hsl, Levels, RangeMatch, suggest_range_match,
};
pub use recipe::{
    CellMetric, CustomMatrix, DitherFamily, DitherKind, FitMode, Method, Metric, Orientation,
    PatternPhase, Recipe, RecipeError,
};

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
        Ok(Self {
            width,
            height,
            pixels,
        })
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
struct TransformData {
    #[serde(default)]
    translate: Vec2,
    #[serde(default = "unit_scale")]
    scale: Vec2,
    #[serde(default)]
    flip_h: bool,
    #[serde(default)]
    flip_v: bool,
}

const fn unit_scale() -> Vec2 {
    Vec2::ONE
}

/// Where a layer sits on the canvas: `canvas = translate + scale ⊙ flip(layer)`, in
/// square canvas pixels (`geom` module docs). Flips mirror within the layer's own box
/// before scaling. No rotation: the transform stays axis-aligned, which keeps
/// resampling separable.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "TransformData")]
pub struct Transform {
    translate: Vec2,
    scale: Vec2,
    flip_h: bool,
    flip_v: bool,
}

impl TryFrom<TransformData> for Transform {
    type Error = LayerError;
    fn try_from(d: TransformData) -> Result<Self, LayerError> {
        Self::new(d.translate, d.scale, d.flip_h, d.flip_v)
    }
}

impl Default for Transform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Transform {
    /// Source pixel `(0, 0)` at canvas `(0, 0)`, one source pixel per canvas pixel.
    pub const IDENTITY: Self = Self {
        translate: Vec2::ZERO,
        scale: Vec2::ONE,
        flip_h: false,
        flip_v: false,
    };

    /// A transform; `translate` must be finite and both scale factors finite and > 0
    /// (mirroring is the flips' job).
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
            flip_h,
            flip_v,
        })
    }

    /// Identity with a translation.
    pub fn translated(translate: Vec2) -> Result<Self, LayerError> {
        Self::new(translate, Vec2::ONE, false, false)
    }

    /// Offset of the layer's top-left corner, in canvas pixels.
    pub const fn translate(&self) -> Vec2 {
        self.translate
    }

    /// Canvas pixels per source pixel, per axis.
    pub const fn scale(&self) -> Vec2 {
        self.scale
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
        Self::new(self.translate + by, self.scale, self.flip_h, self.flip_v)
    }

    /// A layer-space point (source of size `size`) to canvas space.
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
        Vec2::new(
            self.translate.x + self.scale.x * fx,
            self.translate.y + self.scale.y * fy,
        )
    }

    /// A canvas-space point to layer space (source of size `size`).
    pub fn canvas_to_layer(&self, size: (u32, u32), p: Vec2) -> Vec2 {
        let fx = (p.x - self.translate.x) / self.scale.x;
        let fy = (p.y - self.translate.y) / self.scale.y;
        Vec2::new(
            if self.flip_h { size.0 as f32 - fx } else { fx },
            if self.flip_v { size.1 as f32 - fy } else { fy },
        )
    }

    /// The layer's box on the canvas: top-left and bottom-right corners.
    pub fn canvas_bounds(&self, size: (u32, u32)) -> (Vec2, Vec2) {
        let min = self.translate;
        let max = Vec2::new(
            self.translate.x + self.scale.x * size.0 as f32,
            self.translate.y + self.scale.y * size.1 as f32,
        );
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
