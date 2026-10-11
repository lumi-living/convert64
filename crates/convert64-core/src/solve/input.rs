//! The solver's view of a composite: per target pixel its colour in its own metric's
//! coordinates, its coverage weight and its recipe; per cell a table of weighted
//! distances to all 16 colours, built once and shared by every background tried.

use super::cell::{CellChoice, Constraints, search};
use super::metric::{Metric, MetricTables};
use crate::colour::{ColourIndex, ColourSet};
use crate::composite::{AttrSource, TargetFrame};
use crate::geom::SuperGrid;
use crate::layer::{DitherKind, FitMode, Orientation, PatternPhase, Recipe};
use crate::model::CellPos;

/// One target pixel as the solver sees it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Px {
    /// The composite colour in `metric`'s coordinates.
    pub coords: [f32; 3],
    /// The metric this pixel is judged by (from its recipe).
    pub metric: Metric,
    /// Coverage weight, `0..=1` (0 where no layer is).
    pub weight: f32,
    /// Dither method.
    pub kind: DitherKind,
    /// Orientation preference.
    pub orientation: Orientation,
    /// Dither strength, `0..=1`, already scaled by alpha (attribute map).
    pub strength: f32,
    /// Screen- or layer-locked pattern.
    pub phase: PatternPhase,
    /// Force or Adaptive.
    pub fit: FitMode,
    /// Which layer or region the recipe came from.
    pub source: AttrSource,
}

/// Solver options that shape the per-pixel input.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InputOptions {
    /// Luma weight for pixels whose recipe uses the Pepto Y'UV metric.
    pub luma_weight: f32,
}

impl Default for InputOptions {
    fn default() -> Self {
        Self {
            luma_weight: Metric::DEFAULT_LUMA_WEIGHT,
        }
    }
}

/// The prepared input of one solve.
#[derive(Clone, Debug)]
pub struct Input {
    pub(crate) grid: SuperGrid,
    /// Target width and height in logical pixels.
    pub(crate) size: (usize, usize),
    /// Cells across and down.
    pub(crate) cells: (usize, usize),
    /// Cell width and height in target pixels.
    pub(crate) cell: (usize, usize),
    /// Row-major target pixels.
    pub(crate) px: Vec<Px>,
    /// Per cell, `16 × P` weighted distances (`weight × distance`), colour-major, pixels
    /// in row-major order within the cell.
    pub(crate) dist: Vec<f32>,
    /// Per cell, the colours every covered pixel's recipe allows (a filter, never a
    /// request: a recipe can only exclude colours from a cell).
    pub(crate) allowed: Vec<ColourSet>,
    /// For each layer or region with Force pixels, its intended colours: the top
    /// [`FORCE_TOP_K`] of its own nearest-colour histogram.
    pub(crate) intended: Vec<(AttrSource, ColourSet)>,
}

/// How much more a Force pixel counts than an Adaptive one in the cell-set search
/// (TAD §3.1: Force weights a region's intended colours heavily; neighbours pay).
pub const FORCE_WEIGHT: f32 = 8.0;

/// How many of a Force layer's most frequent colours count as its intended colours.
pub const FORCE_TOP_K: usize = 4;

/// What a Force pixel pays, on top of the real distance, for a colour outside its
/// layer's intended colours: about the largest squared distance in any metric, so a set
/// that drops an intended colour its Force pixels need loses to one that keeps it.
pub const FORCE_PENALTY: f32 = 1.0;

impl Input {
    /// Prepares `frame` for solving with the palette in `tables`.
    pub fn new(frame: &TargetFrame, tables: &MetricTables, opts: InputOptions) -> Self {
        let grid = frame.grid();
        let (w, h) = grid.target_size();
        let (cw, ch) = grid.cell_size();
        let (cx, cy) = grid.cells();
        let (w, h, cw, ch, cx, cy) = (
            w as usize,
            h as usize,
            cw as usize,
            ch as usize,
            cx as usize,
            cy as usize,
        );
        let attrs = frame.attributes();
        let default_recipe = Recipe::default();
        let px: Vec<Px> = (0..w * h)
            .map(|i| {
                let a = attrs.pixels()[i];
                let recipe = attrs.recipe(a.source).unwrap_or(&default_recipe);
                let metric = Metric::from_recipe(recipe.metric(), opts.luma_weight);
                let method = recipe.method();
                Px {
                    coords: metric.coords(frame.colour()[i]),
                    metric,
                    weight: frame.coverage()[i],
                    kind: method.kind,
                    orientation: method.orientation,
                    strength: a.strength,
                    phase: recipe.phase(),
                    fit: recipe.fit_mode(),
                    source: a.source,
                }
            })
            .collect();
        let intended = intended_colours(&px, tables);
        let dist = build_dist(&px, tables, w, (cw, ch), (cx, cy), &intended);
        let mut allowed = vec![ColourSet::ALL; cx * cy];
        for (ci, set) in allowed.iter_mut().enumerate() {
            let (gx, gy) = (ci % cx, ci / cx);
            for (x, y) in cell_pixels(gx, gy, cw, ch) {
                let a = attrs.pixels()[y * w + x];
                if frame.coverage()[y * w + x] > 0.0
                    && let Some(r) = attrs.recipe(a.source)
                {
                    *set = set.intersection(r.allowed());
                }
            }
        }
        Self {
            grid,
            size: (w, h),
            cells: (cx, cy),
            cell: (cw, ch),
            px,
            dist,
            allowed,
            intended,
        }
    }

    /// The same input with every pixel's colour moved by the error it received in a
    /// dithering pass: what the refinement re-picks cell sets from (TAD §3.1).
    pub(crate) fn retarget(&self, received: &[[f32; 3]], tables: &MetricTables) -> Self {
        let px: Vec<Px> = self
            .px
            .iter()
            .zip(received)
            .map(|(q, e)| Px {
                coords: [q.coords[0] + e[0], q.coords[1] + e[1], q.coords[2] + e[2]],
                ..*q
            })
            .collect();
        let dist = build_dist(
            &px,
            tables,
            self.size.0,
            self.cell,
            self.cells,
            &self.intended,
        );
        Self {
            px,
            dist,
            ..self.clone()
        }
    }

    /// The grid (target mode and geometry).
    pub const fn grid(&self) -> SuperGrid {
        self.grid
    }

    /// Whether the target has a global background (multicolor): cells then choose
    /// 3 colours besides it; hi-res cells choose 2 and have no background.
    pub fn has_background(&self) -> bool {
        self.grid
            .target()
            .spec()
            .is_ok_and(|s| !s.globals.is_empty())
    }

    /// Free colours per cell: 3 (multicolor, besides the background) or 2 (hi-res).
    pub fn k(&self) -> usize {
        if self.has_background() { 3 } else { 2 }
    }

    /// The colours cell `cell`'s recipes allow (the intersection over its covered
    /// pixels). A filter, never a request.
    pub fn allowed(&self, cell: CellPos) -> ColourSet {
        self.allowed[self.index(cell)]
    }

    /// The lowest-error legal set for `cell` with background `bg` (multicolor) or `None`
    /// (hi-res), within `extra` (steering) and the attribute map's allowed sets, plus
    /// `bias` per same-luma pair.
    pub fn best_set(
        &self,
        cell: CellPos,
        bg: Option<ColourIndex>,
        extra: &Constraints,
        bias: f32,
    ) -> CellChoice {
        let i = self.index(cell);
        let c = Constraints {
            candidates: extra.candidates.intersection(self.allowed[i]),
            required: extra.required,
        };
        search(self.dist(i), self.per_cell(), bg, self.k(), &c, bias)
    }

    /// Weighted wanted colours of `cell`: each covered pixel's nearest of the 16, summed
    /// by colour, heaviest first (ties by index).
    pub fn wanted_colours(&self, cell: CellPos) -> Vec<(ColourIndex, f32)> {
        let mut w = [0.0f32; 16];
        for (c, wt) in self.wanted(self.index(cell)) {
            w[c.as_usize()] += wt;
        }
        let mut out: Vec<(ColourIndex, f32)> = ColourIndex::ALL
            .into_iter()
            .zip(w)
            .filter(|(_, wt)| *wt > 0.0)
            .collect();
        out.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        out
    }

    /// The cell at index `i` (row-major).
    #[expect(clippy::cast_possible_truncation, reason = "cells are below 40 × 25")]
    pub(crate) const fn cell_at(&self, i: usize) -> CellPos {
        CellPos::new((i % self.cells.0) as u16, (i / self.cells.0) as u16)
    }

    /// Cell index of `cell`.
    pub(crate) const fn index(&self, cell: CellPos) -> usize {
        cell.y as usize * self.cells.0 + cell.x as usize
    }

    /// Pixels per cell.
    pub(crate) const fn per_cell(&self) -> usize {
        self.cell.0 * self.cell.1
    }

    /// Cell index `i`'s weighted distance table (`16 × P`).
    pub(crate) fn dist(&self, i: usize) -> &[f32] {
        let n = 16 * self.per_cell();
        &self.dist[i * n..(i + 1) * n]
    }

    /// Target pixel coordinates of cell `i`, row-major within the cell.
    pub(crate) fn pixels_of(&self, i: usize) -> impl Iterator<Item = (usize, usize)> + use<> {
        cell_pixels(i % self.cells.0, i / self.cells.0, self.cell.0, self.cell.1)
    }

    /// The colour each pixel of cell `i` would take with no constraint: its nearest of
    /// the 16, with its weight. Ignores pixels with zero weight.
    pub(crate) fn wanted(&self, i: usize) -> impl Iterator<Item = (ColourIndex, f32)> + '_ {
        let p = self.per_cell();
        let d = self.dist(i);
        self.pixels_of(i)
            .enumerate()
            .filter_map(move |(k, (x, y))| {
                let w = self.px[y * self.size.0 + x].weight;
                (w > 0.0).then(|| {
                    let best = (0..16)
                        .min_by(|&a, &b| d[a * p + k].total_cmp(&d[b * p + k]))
                        .expect("16 colours");
                    (ColourIndex::ALL[best], w)
                })
            })
    }
}

/// Each Force source's intended colours: the [`FORCE_TOP_K`] colours its pixels are
/// most often nearest to, weighted by coverage (ties by index).
fn intended_colours(px: &[Px], tables: &MetricTables) -> Vec<(AttrSource, ColourSet)> {
    let mut hist: Vec<(AttrSource, [f32; 16])> = Vec::new();
    for q in px
        .iter()
        .filter(|q| q.fit == FitMode::Force && q.weight > 0.0)
    {
        let colours = tables.colours(q.metric.space());
        let near = (0..16)
            .min_by(|&a, &b| {
                q.metric
                    .distance(q.coords, colours[a])
                    .total_cmp(&q.metric.distance(q.coords, colours[b]))
            })
            .expect("16 colours");
        let i = match hist.iter().position(|(s, _)| *s == q.source) {
            Some(i) => i,
            None => {
                hist.push((q.source, [0.0; 16]));
                hist.len() - 1
            }
        };
        hist[i].1[near] += q.weight;
    }
    hist.into_iter()
        .map(|(source, h)| {
            let mut order: Vec<usize> = (0..16).filter(|&c| h[c] > 0.0).collect();
            order.sort_by(|&a, &b| h[b].total_cmp(&h[a]).then(a.cmp(&b)));
            let set = order
                .into_iter()
                .take(FORCE_TOP_K)
                .map(|c| ColourIndex::ALL[c])
                .collect();
            (source, set)
        })
        .collect()
}

/// Per cell, the `16 × P` table of weighted distances. A Force pixel counts
/// [`FORCE_WEIGHT`] times, and a colour outside its layer's intended colours costs it
/// [`FORCE_PENALTY`] more, so sets keeping the intended colours win and neighbours pay.
fn build_dist(
    px: &[Px],
    tables: &MetricTables,
    w: usize,
    (cw, ch): (usize, usize),
    (cx, cy): (usize, usize),
    intended: &[(AttrSource, ColourSet)],
) -> Vec<f32> {
    let p = cw * ch;
    let mut dist = vec![0.0f32; cx * cy * 16 * p];
    let mut cells: Vec<(usize, &mut [f32])> = dist.chunks_mut(16 * p).enumerate().collect();
    crate::par::for_each(&mut cells, |(ci, table)| {
        let (gx, gy) = (*ci % cx, *ci / cx);
        for (k, (x, y)) in cell_pixels(gx, gy, cw, ch).enumerate() {
            let q = &px[y * w + x];
            let colours = tables.colours(q.metric.space());
            let force = (q.fit == FitMode::Force)
                .then(|| {
                    intended
                        .iter()
                        .find(|(s, _)| *s == q.source)
                        .map(|(_, set)| *set)
                })
                .flatten();
            match force {
                None => {
                    for (c, coord) in colours.iter().enumerate() {
                        table[c * p + k] = q.weight * q.metric.distance(q.coords, *coord);
                    }
                }
                Some(want) => {
                    for (c, coord) in colours.iter().enumerate() {
                        let d = q.metric.distance(q.coords, *coord);
                        let extra = if want.contains(ColourIndex::ALL[c]) {
                            0.0
                        } else {
                            FORCE_PENALTY
                        };
                        table[c * p + k] = FORCE_WEIGHT * q.weight * (d + extra);
                    }
                }
            }
        }
    });
    dist
}

fn cell_pixels(gx: usize, gy: usize, cw: usize, ch: usize) -> impl Iterator<Item = (usize, usize)> {
    (0..ch).flat_map(move |dy| (0..cw).map(move |dx| (gx * cw + dx, gy * ch + dy)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::colour::ColourIndex as C;
    use crate::composite::Compositor;
    use crate::layer::{Layer, SourceImage};
    use crate::mode::Mode;
    use crate::palette::Palette;
    use std::sync::Arc;

    /// A frame whose every multicolor cell uses exactly four palette colours.
    fn four_colour_frame(p: &Palette) -> TargetFrame {
        let cols = [C::BLUE, C::YELLOW, C::LIGHT_RED, C::BLACK];
        let px: Vec<[f32; 4]> = (0..320 * 200)
            .map(|i| {
                // Multicolor pixel x = (i % 320) / 2; stripes of 1 MC pixel.
                let c = cols[((i % 320) / 2) % 4];
                let l = p.linear(c);
                [l.r, l.g, l.b, 1.0]
            })
            .collect();
        let src = Arc::new(SourceImage::from_linear(320, 200, px).unwrap());
        let mut c = Compositor::new(SuperGrid::new(Mode::MulticolorBitmap, 2).unwrap());
        c.update(&[Layer::new("stripes", src)], &[]);
        c.frame().clone()
    }

    #[test]
    fn finds_the_four_colours_of_a_real_cell() {
        let p = Palette::pepto_pal();
        let t = MetricTables::new(&p);
        let input = Input::new(&four_colour_frame(&p), &t, InputOptions::default());
        assert_eq!(input.k(), 3);
        assert!(input.has_background());
        let cell = CellPos::new(5, 7);
        let best = input.best_set(cell, Some(C::BLACK), &Constraints::FREE, 0.0);
        let want: ColourSet = [C::BLUE, C::YELLOW, C::LIGHT_RED].into_iter().collect();
        assert_eq!(best.set, want);
        assert!(best.score < 1e-6, "{}", best.score);
        let wanted = input.wanted_colours(cell);
        assert_eq!(wanted.len(), 4);
        assert!(wanted.iter().all(|(_, w)| (*w - 8.0).abs() < 1e-4));
        // With a different background one colour must be dropped.
        let other = input.best_set(cell, Some(C::WHITE), &Constraints::FREE, 0.0);
        assert!(other.score > 0.0);
    }
}
