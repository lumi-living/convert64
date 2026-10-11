//! The composite on the target grid: what the solver consumes.

use super::attr::{AttributeMap, TargetAttr};
use crate::geom::{PxRect, SuperGrid};

/// Box-filters target pixel `tx` from `band`, the `n` premultiplied super rows (each `w`
/// wide) of its target row: straight colour and coverage.
pub(crate) fn reduce_px(band: &[[f32; 4]], w: usize, n: u32, tx: usize) -> ([f32; 3], f32) {
    let n = n as usize;
    let mut acc = [0.0f32; 4];
    for sy in 0..n {
        let base = sy * w + tx * n;
        for p in &band[base..base + n] {
            for i in 0..4 {
                acc[i] += p[i];
            }
        }
    }
    let a = acc[3] * (1.0 / (n * n) as f32);
    let colour = if acc[3] > 0.0 {
        [acc[0] / acc[3], acc[1] / acc[3], acc[2] / acc[3]]
    } else {
        [0.0; 3]
    };
    (colour, a.clamp(0.0, 1.0))
}

/// The supersampled composite box-filtered to the target grid (320 × 200 or 160 × 200):
/// per target pixel the straight (unpremultiplied) linear RGB, the coverage (mean
/// composite alpha, 0 where no layer is) and the attribute map.
#[derive(Clone, Debug, PartialEq)]
pub struct TargetFrame {
    grid: SuperGrid,
    colour: Vec<[f32; 3]>,
    coverage: Vec<f32>,
    attributes: AttributeMap,
}

impl TargetFrame {
    /// An empty frame (nothing covered) for `grid`.
    pub(crate) fn empty(grid: SuperGrid) -> Self {
        let (w, h) = grid.target_size();
        let len = w as usize * h as usize;
        Self {
            grid,
            colour: vec![[0.0; 3]; len],
            coverage: vec![0.0; len],
            attributes: AttributeMap::empty(grid),
        }
    }

    /// The grid.
    pub const fn grid(&self) -> SuperGrid {
        self.grid
    }

    /// Row-major straight linear RGB per target pixel (0 where coverage is 0).
    pub fn colour(&self) -> &[[f32; 3]] {
        &self.colour
    }

    /// Row-major coverage per target pixel, `0..=1`.
    pub fn coverage(&self) -> &[f32] {
        &self.coverage
    }

    /// The attribute map.
    pub const fn attributes(&self) -> &AttributeMap {
        &self.attributes
    }

    pub(crate) fn attributes_mut(&mut self) -> &mut AttributeMap {
        &mut self.attributes
    }

    /// Whether target pixel `i` (row-major) is identical in both frames.
    fn same_px(&self, other: &Self, i: usize) -> bool {
        self.colour[i] == other.colour[i]
            && self.coverage[i] == other.coverage[i]
            && self.attributes.pixels()[i] == other.attributes.pixels()[i]
    }

    /// Target pixels in `area` (target coordinates) that differ from `other`.
    pub(crate) fn differing(&self, other: &Self, area: PxRect) -> Vec<(u32, u32)> {
        let (w, _) = self.grid.target_size();
        let mut out = Vec::new();
        for y in area.y0..area.y1 {
            for x in area.x0..area.x1 {
                if !self.same_px(other, (y * w + x) as usize) {
                    out.push((x, y));
                }
            }
        }
        out
    }

    /// The colour, coverage and attribute pixels, for the compositor's fused
    /// fold-and-filter.
    pub(crate) fn parts_mut(&mut self) -> (&mut [[f32; 3]], &mut [f32], &mut [TargetAttr]) {
        (
            &mut self.colour,
            &mut self.coverage,
            self.attributes.pixels_mut(),
        )
    }

    /// The attribute of target pixel `(x, y)`.
    pub fn attr(&self, x: u32, y: u32) -> TargetAttr {
        self.attributes.get(x, y)
    }
}
