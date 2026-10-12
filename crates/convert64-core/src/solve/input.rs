//! The solver's view of a composite: per target pixel its colour in its own metric's
//! coordinates, its coverage weight and its recipe; per cell a table of weighted
//! distances to all 16 colours, built once and shared by every background tried.

use super::cell::{CellChoice, Constraints, search};
use super::metric::{Metric, MetricTables};
use crate::colour::{ColourIndex, ColourSet};
use crate::composite::{AttrSource, TargetFrame};
use crate::geom::SuperGrid;
use crate::layer::{
    CellMetric, CustomMatrix, DitherKind, FitMode, OrderedSemantics, Orientation, PatternPhase,
    Recipe,
};
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
    /// Drop diffused error at cell borders (the recipe's `cell_clamp`).
    pub cell_clamp: bool,
    /// Dither mixes stay within this many luma levels (8 = unrestricted).
    pub max_luma_diff: u8,
    /// `1 − adaptive × detail`: scales this pixel's dither (1 when `adaptive` is 0).
    pub adapt: f32,
    /// Received diffusion error is clamped to `±error_clamp` per channel (0: off).
    pub error_clamp: f32,
    /// How an ordered pixel turns its threshold into a colour.
    pub semantics: OrderedSemantics,
    /// The chroma guard on ordered mixes.
    pub chroma_guard: bool,
    /// The colour in OKLab (for the chroma guard and lightness bracketing).
    pub oklab: [f32; 3],
    /// Mix-ratio levels of an ordered or noise pixel (0 = continuous).
    pub mix_levels: u8,
    /// The colour in linear RGB (pre-smoothed like `coords`): mix ratios are taken in
    /// linear light, where the eye averages a dither pattern.
    pub lin: [f32; 3],
    /// [`DitherKind::Custom`]'s matrix: an index into `Input::customs` plus one, or 0.
    pub custom: u8,
    /// What the cell-set search scores this pixel as (its metric's coordinates): its
    /// colour, with a threshold pixel's lightness nudged by its pattern threshold so the
    /// search picks colours the pattern can mix ([`SEARCH_BIAS_L`]), and a diffusion
    /// pixel's replaced by what an unconstrained diffusion of the frame makes it.
    pub search: [f32; 3],
}

/// How far (OKLab L, at full strength, per unit of threshold offset from ½) a threshold
/// pixel's lightness is nudged for the cell-set search: png2c64's dither-aware search
/// (`precompute_cell_dist`). Without it a cell in a smooth ramp keeps only the colour
/// its pixels are nearest to, and the pattern has nothing to mix with.
pub const SEARCH_BIAS_L: f32 = 0.15;

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
    /// The custom threshold matrices the pixels' recipes use (see `Px::custom`).
    pub(crate) customs: Vec<CustomMatrix>,
    /// The solve's signed same-luma pair bias (`pair` dithering reads it).
    pub(crate) pair_bias: f32,
    /// Per cell, the metric its set is scored by: the first covered pixel's recipe's
    /// non-default one, else squared distance.
    pub(crate) cell_metric: Vec<CellMetric>,
    /// The target blurred for the `blur` cell metric (empty when no cell uses it).
    pub(crate) blurred: Vec<[f32; 3]>,
    /// Per cell, whether it has png2c64-ordered pixels: its set is then re-picked by
    /// what it renders (#156).
    pub(crate) rendered: Vec<bool>,
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
        Self::new_for(frame, tables, opts, None)
    }

    /// Like [`Self::new`], but builds distance tables only for `only`'s cells (the
    /// interactive fast path re-solves just those; the others keep zero tables and are
    /// copied from the previous solution).
    pub(crate) fn new_for(
        frame: &TargetFrame,
        tables: &MetricTables,
        opts: InputOptions,
        only: Option<crate::geom::CellSet>,
    ) -> Self {
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
        // Ordered and noise pixels with `smooth` take their mix ratio from an
        // edge-aware blur of the composite (#142); colours are still chosen from the
        // unblurred one.
        let smooths = |i: usize| {
            let r = attrs
                .recipe(attrs.pixels()[i].source)
                .unwrap_or(&default_recipe);
            r.smooth() && r.method().kind.family().is_threshold()
        };
        let smoothed = (0..w * h)
            .any(smooths)
            .then(|| smooth_colours(frame.colour(), frame.coverage(), w, h));
        let mut customs: Vec<CustomMatrix> = Vec::new();
        let px: Vec<Px> = (0..w * h)
            .map(|i| {
                let a = attrs.pixels()[i];
                let recipe = attrs.recipe(a.source).unwrap_or(&default_recipe);
                let metric = Metric::from_recipe(recipe.metric(), opts.luma_weight);
                let method = recipe.method();
                let colour = frame.colour()[i];
                Px {
                    coords: metric.coords(colour),
                    metric,
                    weight: frame.coverage()[i],
                    kind: method.kind,
                    orientation: method.orientation,
                    strength: a.strength,
                    phase: recipe.phase(),
                    fit: recipe.fit_mode(),
                    source: a.source,
                    cell_clamp: recipe.cell_clamp(),
                    max_luma_diff: recipe.max_luma_diff(),
                    adapt: recipe.adaptive(),
                    error_clamp: recipe.error_clamp(),
                    semantics: recipe.ordered_semantics(),
                    chroma_guard: recipe.chroma_guard(),
                    oklab: super::metric::Space::OkLab.coords(colour),
                    mix_levels: recipe.mix_levels(),
                    lin: match &smoothed {
                        Some(s) if smooths(i) => s[i],
                        _ => colour,
                    },
                    custom: match method.custom {
                        Some(m) if method.kind == DitherKind::Custom => {
                            custom_index(&mut customs, m)
                        }
                        _ => 0,
                    },
                    search: [0.0; 3],
                }
            })
            .collect();
        let mut px = px;
        apply_adaptive(&mut px, frame.colour(), w, h);
        search_colours(&mut px, frame.colour(), &customs, w);
        // Diffusion pixels are searched as the colour an unconstrained diffusion gives
        // them, so a cell keeps the colours its dither mixes.
        let wanted = super::dither::unconstrained(&px, tables, w, h);
        for (q, c) in px.iter_mut().zip(wanted) {
            if let Some(c) = c {
                q.search = tables.colour(q.metric.space(), c);
            }
        }
        let intended = intended_colours(&px, tables);
        let dist = build_dist(&px, tables, w, (cw, ch), (cx, cy), &intended, only);
        let mut allowed = vec![ColourSet::ALL; cx * cy];
        let mut cell_metric = vec![CellMetric::Mse; cx * cy];
        for (ci, (set, metric)) in allowed.iter_mut().zip(&mut cell_metric).enumerate() {
            let (gx, gy) = (ci % cx, ci / cx);
            for (x, y) in cell_pixels(gx, gy, cw, ch) {
                let a = attrs.pixels()[y * w + x];
                if frame.coverage()[y * w + x] > 0.0
                    && let Some(r) = attrs.recipe(a.source)
                {
                    *set = set.intersection(r.allowed());
                    if *metric == CellMetric::Mse {
                        *metric = r.cell_metric();
                    }
                }
            }
        }
        let mut input = Self {
            grid,
            size: (w, h),
            cells: (cx, cy),
            cell: (cw, ch),
            px,
            dist,
            allowed,
            intended,
            customs,
            pair_bias: 0.0,
            cell_metric,
            blurred: Vec::new(),
            rendered: Vec::new(),
        };
        input.rendered = (0..cx * cy)
            .map(|ci| {
                input
                    .pixels_of(ci)
                    .any(|(x, y)| super::cellscore::png2c64_ordered(&input.px[y * w + x]))
            })
            .collect();
        if input.cell_metric.contains(&CellMetric::Blur) {
            input.blurred = super::cellscore::blurred_target(&input);
        }
        input
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
                search: [q.search[0] + e[0], q.search[1] + e[1], q.search[2] + e[2]],
                ..*q
            })
            .collect();
        // Only cells that received error need new distances.
        let (w, p) = (self.size.0, self.per_cell());
        let mut dist = self.dist.clone();
        let (cw, ch) = self.cell;
        let mut cells: Vec<(usize, &mut [f32])> = dist.chunks_mut(16 * p).enumerate().collect();
        crate::par::for_each(&mut cells, |(ci, table)| {
            let (gx, gy) = (*ci % self.cells.0, *ci / self.cells.0);
            let touched = cell_pixels(gx, gy, cw, ch).any(|(x, y)| received[y * w + x] != [0.0; 3]);
            if touched {
                fill_cell(table, &px, tables, w, (gx, gy), (cw, ch), &self.intended);
            }
        });
        Self {
            grid: self.grid,
            size: self.size,
            cells: self.cells,
            cell: self.cell,
            px,
            dist,
            allowed: self.allowed.clone(),
            intended: self.intended.clone(),
            customs: self.customs.clone(),
            pair_bias: self.pair_bias,
            cell_metric: self.cell_metric.clone(),
            blurred: self.blurred.clone(),
            rendered: self.rendered.clone(),
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

    /// The best set for `cell` under its cell metric: [`Self::best_set`], then for the
    /// `blur` and `ssim` metrics a re-pick among a short list ([`super::cellscore`]).
    pub(crate) fn pick_set(
        &self,
        tables: &MetricTables,
        cell: CellPos,
        bg: Option<ColourIndex>,
        extra: &Constraints,
        bias: f32,
    ) -> CellChoice {
        let mse = self.best_set(cell, bg, extra, bias);
        let i = self.index(cell);
        match self.cell_metric[i] {
            CellMetric::Mse if self.rendered[i] => {
                let c = Constraints {
                    candidates: extra.candidates.intersection(self.allowed[i]),
                    required: extra.required,
                };
                super::cellscore::rendered_repick(self, tables, i, bg, &c, bias, mse)
            }
            CellMetric::Mse => mse,
            metric => {
                let c = Constraints {
                    candidates: extra.candidates.intersection(self.allowed[i]),
                    required: extra.required,
                };
                super::cellscore::repick(self, tables, i, bg, &c, bias, metric, mse)
            }
        }
    }

    /// Cell `i`'s score with `set` under its cell metric (what [`Self::pick_set`]
    /// minimises).
    pub(crate) fn set_score(
        &self,
        tables: &MetricTables,
        i: usize,
        bg: Option<ColourIndex>,
        set: ColourSet,
        bias: f32,
    ) -> f32 {
        match self.cell_metric[i] {
            CellMetric::Mse if self.rendered[i] => super::cellscore::rendered_score(
                self,
                tables,
                i,
                bg.map_or(set, |b| set.with(b)),
                bias,
            ),
            CellMetric::Mse => super::cell::score_set(self.dist(i), self.per_cell(), bg, set, bias),
            metric => super::cellscore::score(
                self,
                tables,
                i,
                bg.map_or(set, |b| set.with(b)),
                metric,
                bias,
            ),
        }
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

    /// Cell `cell`'s weighted distances to the 16 colours (`16 × P`, colour-major): the
    /// statistics its set is chosen from.
    pub fn cell_distances(&self, cell: CellPos) -> &[f32] {
        self.dist(self.index(cell))
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

/// Fills every pixel's `search` colour: its coordinates, or for a threshold pixel with
/// strength the colour with OKLab L moved by `(threshold − ½) × strength ×`
/// [`SEARCH_BIAS_L`] (pattern position in screen pixels).
fn search_colours(px: &mut [Px], colour: &[[f32; 3]], customs: &[CustomMatrix], w: usize) {
    use super::metric::Space;
    use crate::palette::{LinearRgb, Oklab};
    let mut rows: Vec<(usize, &mut [Px])> = px.chunks_mut(w).enumerate().collect();
    crate::par::for_each(&mut rows, |(y, row)| {
        for (x, q) in row.iter_mut().enumerate() {
            let s = q.strength * q.adapt;
            let family = q.kind.family();
            let positional =
                family.is_threshold() || family == crate::layer::DitherFamily::PaletteAware;
            if !positional || s <= 0.0 || q.weight <= 0.0 {
                q.search = q.coords;
                continue;
            }
            let (xi, yi) = (x as i64, *y as i64);
            // Palette-aware methods are placed by Bayer 8.
            let kind = if family.is_threshold() {
                q.kind
            } else {
                DitherKind::Bayer8
            };
            let thr = match q.custom {
                0 => super::ordered::threshold(kind, q.orientation, xi, yi)
                    .expect("threshold methods have thresholds"),
                c => super::ordered::custom_threshold(
                    &customs[usize::from(c) - 1],
                    q.orientation,
                    xi,
                    yi,
                ),
            };
            let [l, a, b] = Space::OkLab.coords(colour[*y * w + x]);
            let lin = LinearRgb::from_oklab(Oklab {
                l: l + (thr - 0.5) * s.min(1.0) * SEARCH_BIAS_L,
                a,
                b,
            });
            q.search = q.metric.coords([lin.r, lin.g, lin.b].map(|v| v.max(0.0)));
        }
    });
}

/// The index (plus one) of `m` in `customs`, added if new.
#[expect(clippy::cast_possible_truncation, reason = "a frame has few recipes")]
fn custom_index(customs: &mut Vec<CustomMatrix>, m: CustomMatrix) -> u8 {
    match customs.iter().position(|c| *c == m) {
        Some(i) => i as u8 + 1,
        None => {
            customs.push(m);
            customs.len() as u8
        }
    }
}

/// Turns each pixel's `adaptive` (stored in `adapt`) into its dither scale
/// `1 − adaptive × detail`, with detail the mean OKLab |ΔL| to the 4 neighbours / 0.15,
/// capped at 1 (png2c64 `--adaptive`), measured on the target grid.
fn apply_adaptive(px: &mut [Px], colour: &[[f32; 3]], w: usize, h: usize) {
    if px.iter().all(|q| q.adapt == 0.0) {
        for q in px.iter_mut() {
            q.adapt = 1.0;
        }
        return;
    }
    let l: Vec<f32> = colour
        .iter()
        .map(|c| super::metric::Space::OkLab.coords(*c)[0])
        .collect();
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let q = &mut px[i];
            if q.adapt == 0.0 {
                q.adapt = 1.0;
                continue;
            }
            let mut sum = 0.0;
            let mut n = 0.0;
            for (dx, dy) in [(-1i64, 0i64), (1, 0), (0, -1), (0, 1)] {
                let (nx, ny) = (x as i64 + dx, y as i64 + dy);
                if nx >= 0 && ny >= 0 && (nx as usize) < w && (ny as usize) < h {
                    sum += (l[i] - l[ny as usize * w + nx as usize]).abs();
                    n += 1.0;
                }
            }
            let detail = (sum / n / 0.15).min(1.0);
            q.adapt = 1.0 - q.adapt * detail;
        }
    }
}

/// Spatial weights of the pre-smoothing blur: a Gaussian with σ = 1 target pixel at
/// distances 0, 1 and 2 (`exp(−d²/2)`), written out so every platform gets the same bits.
const SMOOTH_TAPS: [f32; 3] = [1.0, 0.606_530_66, 0.135_335_28];

/// Neighbours at least this far (OKLab ΔE) from a pixel are left out of its blur, so
/// edges stay sharp.
pub const SMOOTH_EDGE: f32 = 0.08;

/// The composite blurred for the mix ratio of ordered pixels with `smooth` (#142): a
/// 5 × 5 Gaussian (σ = 1 target pixel) whose taps are weighted by coverage and by
/// `1 − (ΔE / SMOOTH_EDGE)²` (OKLab), so micro-texture is averaged out but edges are not
/// blurred across. Linear RGB in, linear RGB out.
fn smooth_colours(colour: &[[f32; 3]], coverage: &[f32], w: usize, h: usize) -> Vec<[f32; 3]> {
    let lab: Vec<[f32; 3]> = colour
        .iter()
        .map(|c| super::metric::Space::OkLab.coords(*c))
        .collect();
    let mut out: Vec<(usize, [f32; 3])> = (0..w * h).map(|i| (i, [0.0; 3])).collect();
    crate::par::for_each(&mut out, |(i, o)| {
        let (x, y) = (*i % w, *i / w);
        let mut acc = [0.0f32; 3];
        let mut total = 0.0f32;
        for (dy, gy) in (-2i64..=2).zip([2usize, 1, 0, 1, 2]) {
            for (dx, gx) in (-2i64..=2).zip([2usize, 1, 0, 1, 2]) {
                let (nx, ny) = (x as i64 + dx, y as i64 + dy);
                if nx < 0 || ny < 0 || nx >= w as i64 || ny >= h as i64 {
                    continue;
                }
                let n = ny as usize * w + nx as usize;
                let (a, b) = (lab[*i], lab[n]);
                let d2 = ((a[0] - b[0]) * (a[0] - b[0])
                    + (a[1] - b[1]) * (a[1] - b[1])
                    + (a[2] - b[2]) * (a[2] - b[2]))
                    / (SMOOTH_EDGE * SMOOTH_EDGE);
                let wt = SMOOTH_TAPS[gx] * SMOOTH_TAPS[gy] * coverage[n] * (1.0 - d2);
                if wt <= 0.0 {
                    continue;
                }
                for (s, c) in acc.iter_mut().zip(colour[n]) {
                    *s += wt * c;
                }
                total += wt;
            }
        }
        *o = if total > 0.0 {
            acc.map(|v| v / total)
        } else {
            colour[*i]
        };
    });
    out.into_iter().map(|(_, c)| c).collect()
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
    only: Option<crate::geom::CellSet>,
) -> Vec<f32> {
    let p = cw * ch;
    let mut dist = vec![0.0f32; cx * cy * 16 * p];
    #[expect(clippy::cast_possible_truncation, reason = "cells are below 40 × 25")]
    let wanted = |ci: usize| {
        only.is_none_or(|s| s.contains(CellPos::new((ci % cx) as u16, (ci / cx) as u16)))
    };
    let mut cells: Vec<(usize, &mut [f32])> = dist
        .chunks_mut(16 * p)
        .enumerate()
        .filter(|(ci, _)| wanted(*ci))
        .collect();
    crate::par::for_each(&mut cells, |(ci, table)| {
        fill_cell(
            table,
            px,
            tables,
            w,
            (*ci % cx, *ci / cx),
            (cw, ch),
            intended,
        );
    });
    dist
}

/// One cell's `16 × P` table (see [`build_dist`]).
fn fill_cell(
    table: &mut [f32],
    px: &[Px],
    tables: &MetricTables,
    w: usize,
    (gx, gy): (usize, usize),
    (cw, ch): (usize, usize),
    intended: &[(AttrSource, ColourSet)],
) {
    let p = cw * ch;
    for (k, (x, y)) in cell_pixels(gx, gy, cw, ch).enumerate() {
        let q = &px[y * w + x];
        let d = tables.distances(q.metric, q.search);
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
                for (c, dc) in d.iter().enumerate() {
                    table[c * p + k] = q.weight * dc;
                }
            }
            Some(want) => {
                for (c, dc) in d.iter().enumerate() {
                    let extra = if want.contains(ColourIndex::ALL[c]) {
                        0.0
                    } else {
                        FORCE_PENALTY
                    };
                    table[c * p + k] = FORCE_WEIGHT * q.weight * (dc + extra);
                }
            }
        }
    }
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
    fn smoothing_averages_texture_and_keeps_edges() {
        // Left half black, right half a grey with ±0.02 texture.
        let (w, h) = (16, 8);
        let colour: Vec<[f32; 3]> = (0..w * h)
            .map(|i| {
                let x = i % w;
                if x < 8 {
                    [0.0; 3]
                } else {
                    let v = 0.4 + if (i / w + x) % 2 == 0 { 0.02 } else { -0.02 };
                    [v; 3]
                }
            })
            .collect();
        let out = smooth_colours(&colour, &vec![1.0; w * h], w, h);
        // The black side does not bleed into the grey edge column, nor grey into black.
        for y in 0..h {
            assert_eq!(out[y * w + 7], [0.0; 3]);
            assert!(out[y * w + 8][0] > 0.37, "{:?}", out[y * w + 8]);
        }
        // Inside the grey, the texture is mostly averaged out.
        for y in 2..h - 2 {
            let v = out[y * w + 12][0];
            assert!((v - 0.4).abs() < 0.01, "{v}");
        }
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
