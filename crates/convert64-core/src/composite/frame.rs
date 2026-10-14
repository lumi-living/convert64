//! The composite on the target grid: what the solver consumes.

use super::attr::{AttributeMap, TargetAttr};
use crate::geom::SuperGrid;

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
    finish(acc, n)
}

/// Straight colour and coverage from a target pixel's premultiplied sum over `n²` super
/// pixels.
fn finish(acc: [f32; 4], n: usize) -> ([f32; 3], f32) {
    let a = acc[3] * (1.0 / (n * n) as f32);
    let colour = if acc[3] > 0.0 {
        [acc[0] / acc[3], acc[1] / acc[3], acc[2] / acc[3]]
    } else {
        [0.0; 3]
    };
    (colour, a.clamp(0.0, 1.0))
}

/// Target pixels this many at a time in [`reduce_row`]: their sums are independent, so
/// they overlap instead of waiting on one long chain of additions.
const LANES: usize = 4;

/// [`reduce_px`] for the target pixels `txs` of one target row, passing each result to
/// `out`. Every pixel's sum runs in the same order as in [`reduce_px`].
pub(crate) fn reduce_row(
    band: &[[f32; 4]],
    w: usize,
    n: u32,
    txs: std::ops::Range<usize>,
    out: impl FnMut(usize, [f32; 3], f32),
) {
    // Specialised for the common factors, so the inner loops have constant bounds.
    match n {
        2 => reduce_row_n(band, w, 2, txs, out),
        4 => reduce_row_n(band, w, 4, txs, out),
        8 => reduce_row_n(band, w, 8, txs, out),
        n => reduce_row_n(band, w, n as usize, txs, out),
    }
}

#[inline(always)]
fn reduce_row_n(
    band: &[[f32; 4]],
    w: usize,
    n: usize,
    txs: std::ops::Range<usize>,
    mut out: impl FnMut(usize, [f32; 3], f32),
) {
    let mut tx = txs.start;
    while tx + LANES <= txs.end {
        let mut acc = [[0.0f32; 4]; LANES];
        for sy in 0..n {
            let row = &band[sy * w + tx * n..][..LANES * n];
            for sx in 0..n {
                for (l, a) in acc.iter_mut().enumerate() {
                    let p = row[l * n + sx];
                    for i in 0..4 {
                        a[i] += p[i];
                    }
                }
            }
        }
        for (l, a) in acc.into_iter().enumerate() {
            let (c, v) = finish(a, n);
            out(tx + l, c, v);
        }
        tx += LANES;
    }
    for tx in tx..txs.end {
        let (c, v) = reduce_px(band, w, n as u32, tx);
        out(tx, c, v);
    }
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
    /// The colour has been through the reference's preprocessing already (png2amiga's,
    /// ADR-013), which the reference solver then does not repeat: png2amiga's prepared
    /// image of a plain layer (#190), or a composite, whose every layer went through it
    /// (ADR-012).
    prepared: bool,
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
            prepared: false,
        }
    }

    /// Whether the colour has been through the reference's preprocessing (png2amiga's
    /// gamma and OKLab round trip, ADR-013): png2amiga's prepared image of one plain
    /// layer (#190), or the compositor's composite, whose layers each went through it
    /// (ADR-012).
    pub const fn prepared(&self) -> bool {
        self.prepared
    }

    /// Replaces the colour with png2amiga's prepared image, or (`None`) keeps the
    /// compositor's composite; both have been through png2amiga's preprocessing.
    pub(crate) fn set_prepared(&mut self, colour: Option<&[[f32; 3]]>) {
        if let Some(c) = colour {
            self.colour.copy_from_slice(c);
        }
        self.prepared = true;
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
    #[cfg(test)]
    fn same_px(&self, other: &Self, i: usize) -> bool {
        self.colour[i] == other.colour[i]
            && self.coverage[i] == other.coverage[i]
            && self.attributes.pixels()[i] == other.attributes.pixels()[i]
    }

    /// Target pixels in `area` (target coordinates) that differ from `other`. The fold
    /// finds these itself as it writes; tests check it against this.
    #[cfg(test)]
    pub(crate) fn differing(&self, other: &Self, area: crate::geom::PxRect) -> Vec<(u32, u32)> {
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
