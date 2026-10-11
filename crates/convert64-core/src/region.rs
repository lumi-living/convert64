//! Region overrides (UX §7): a soft mask painted over the composite that carries its own
//! recipe, for "only the dither differs here" without a new layer (UX §5a).
//!
//! A region's mask is in **canvas space** (320 × 200 square pixels), so it stays put when
//! layers move and survives switching between hi-res and multicolor targets. Regions are
//! applied to the attribute map after all layers, with the same top-most rule: where the
//! mask is > 0 the region's recipe wins, its strength multiplied by the mask.

use crate::geom::Vec2;
use crate::layer::Recipe;
use crate::mask::{MaskError, MaskRaster};

/// Canvas width in pixels; a region mask is this wide.
pub const CANVAS_WIDTH: u32 = 320;
/// Canvas height in pixels; a region mask is this tall.
pub const CANVAS_HEIGHT: u32 = 200;

/// A recipe override over part of the picture.
#[derive(Clone, Debug, PartialEq)]
pub struct Region {
    name: String,
    mask: MaskRaster,
    recipe: Recipe,
}

impl Region {
    /// A region; `mask` must be 320 × 200 (canvas space).
    pub fn new(
        name: impl Into<String>,
        mask: MaskRaster,
        recipe: Recipe,
    ) -> Result<Self, MaskError> {
        if mask.size() != (CANVAS_WIDTH, CANVAS_HEIGHT) {
            return Err(MaskError::Size {
                expected: (CANVAS_WIDTH, CANVAS_HEIGHT),
                found: mask.size(),
            });
        }
        Ok(Self {
            name: name.into(),
            mask,
            recipe,
        })
    }

    /// Display name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The canvas-space mask.
    pub const fn mask(&self) -> &MaskRaster {
        &self.mask
    }

    /// The recipe it imposes.
    pub const fn recipe(&self) -> &Recipe {
        &self.recipe
    }

    /// Replaces the recipe.
    pub fn set_recipe(&mut self, recipe: Recipe) {
        self.recipe = recipe;
    }

    /// The mask at canvas point `p`, bilinear with clamped edges.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "coordinates are clamped to the raster first"
    )]
    pub fn sample(&self, p: Vec2) -> f32 {
        let (w, h) = self.mask.size();
        let fx = (p.x - 0.5).clamp(0.0, (w - 1) as f32);
        let fy = (p.y - 0.5).clamp(0.0, (h - 1) as f32);
        let (x0, y0) = (fx.floor() as u32, fy.floor() as u32);
        let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
        let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
        let g = |x, y| self.mask.get(x, y);
        let top = g(x0, y0) * (1.0 - tx) + g(x1, y0) * tx;
        let bot = g(x0, y1) * (1.0 - tx) + g(x1, y1) * tx;
        top * (1.0 - ty) + bot * ty
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn region_mask_must_be_canvas_sized() {
        let bad = MaskRaster::filled(160, 200, 1.0).unwrap();
        assert!(Region::new("r", bad, Recipe::default()).is_err());
        let r = Region::new(
            "r",
            MaskRaster::filled(320, 200, 0.25).unwrap(),
            Recipe::default(),
        )
        .unwrap();
        assert_eq!(r.sample(Vec2::new(10.0, 10.0)), 0.25);
        assert_eq!(r.sample(Vec2::new(-5.0, 500.0)), 0.25);
    }
}
