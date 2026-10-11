//! The supersampled compositor (`docs/TAD.md` §2, ADR-002).
//!
//! Every visible layer is resampled onto the [`SuperGrid`] (`n ×` the target grid, the
//! 2:1 multicolor aspect included) and blended bottom to top into an f32 linear
//! **premultiplied** RGBA [`Canvas`]. Effective alpha is PNG alpha × mask × opacity.
//!
//! Each layer's resampled raster is cached and rebuilt only when its source,
//! adjustments, transform, mask or the grid change; a recomposite with unchanged layers
//! only re-blends. [`Compositor::update`] also keeps the fold of the layers below the
//! lowest changed one, so a nudge or a slider re-blends only that layer and the layers
//! above it (#98).
//!
//! Performance notes (#98), none of which changes the output (a canvas zero may change
//! sign, which compares equal and leaves the frame alone):
//! - Rasters are stored one plane per channel, and the fold blends a target row's `n`
//!   super rows layer by layer, with the attribute claim fused into the blend (each
//!   visible layer overwrites, bottom-up), so the loops vectorise across pixels. On a
//!   clear band the bottom layer is stored, not blended.
//! - Per row, a layer only touches the columns where it can contribute: a source pixel
//!   that is all zero, or under a zero mask, leaves the backdrop unchanged.
//! - Layers are resampled in parallel, colour beside mask; resampling works in chunks of
//!   rows whose horizontal pass stays in cache. A rebuilt layer reuses the buffers it
//!   replaced, and parallel tasks take their scratch from per-thread pools
//!   (`crate::scratch`), so a drag does not page in fresh memory.

mod attr;
mod blend;
mod frame;
mod resample;

pub use attr::{AttrSource, AttributeMap, TargetAttr, debug_colour};
pub use frame::TargetFrame;

use crate::geom::{CellSet, PxRect, SuperGrid, Vec2};
use crate::layer::{AdjustCache, Adjustments, Blend, Layer, SourceImage, Transform};
use crate::mask::{Mask, Modifiers, SnapToCells};
use crate::region::Region;
use crate::scratch;
use attr::AttrPlanes;
use blend::RgbaRow;
use resample::{AxisMap, Planes, resample_into, resample_planes};
use std::sync::Arc;

/// The composite on the super grid: premultiplied linear RGBA, row-major.
#[derive(Clone, Debug, PartialEq)]
pub struct Canvas {
    grid: SuperGrid,
    px: Vec<[f32; 4]>,
}

impl Canvas {
    /// A transparent canvas for `grid`.
    pub fn new(grid: SuperGrid) -> Self {
        let (w, h) = grid.size();
        Self {
            grid,
            px: vec![[0.0; 4]; w as usize * h as usize],
        }
    }

    /// The grid this canvas covers.
    pub const fn grid(&self) -> SuperGrid {
        self.grid
    }

    /// Row-major premultiplied pixels.
    pub fn pixels(&self) -> &[[f32; 4]] {
        &self.px
    }

    /// The premultiplied pixel at super `(x, y)`; panics outside the grid.
    pub fn pixel(&self, x: u32, y: u32) -> [f32; 4] {
        let (w, h) = self.grid.size();
        assert!(x < w && y < h, "({x}, {y}) is outside the super grid");
        self.px[y as usize * w as usize + x as usize]
    }

    /// The canvas box-filtered to the target grid: each target pixel is the mean of its
    /// `n × n` super pixels, still premultiplied.
    pub fn downsample(&self) -> Vec<[f32; 4]> {
        let (tw, th) = self.grid.target_size();
        let (w, _) = self.grid.size();
        let n = u32::from(self.grid.n());
        let inv = 1.0 / (n * n) as f32;
        let mut out = vec![[0.0; 4]; tw as usize * th as usize];
        let mut rows: Vec<(usize, &mut [[f32; 4]])> =
            out.chunks_mut(tw as usize).enumerate().collect();
        crate::par::for_each(&mut rows, |(ty, row)| {
            for (tx, o) in row.iter_mut().enumerate() {
                let mut acc = [0.0f32; 4];
                for sy in 0..n {
                    let base = (*ty as u32 * n + sy) as usize * w as usize + tx * n as usize;
                    for p in &self.px[base..base + n as usize] {
                        for i in 0..4 {
                            acc[i] += p[i];
                        }
                    }
                }
                *o = acc.map(|v| v * inv);
            }
        });
        out
    }
}

/// A layer resampled onto the super grid.
#[derive(Debug)]
struct LayerRaster {
    /// The super pixels the layer touches.
    rect: PxRect,
    /// Premultiplied colour and PNG alpha, coverage applied; `rect`-sized.
    rgba: Planes,
    /// Where `rgba` is not all zero.
    extent: Extents,
}

/// Per row of a layer's rectangle, the columns (relative to it) outside which the layer
/// contributes nothing, so the fold skips them. A source pixel that is all zero, or
/// under a zero mask, blends to the backdrop unchanged in every mode (up to the sign of
/// a zero) and claims no attribute.
type Extents = Arc<Vec<(u32, u32)>>;

/// The first and one past the last index in `row` where `on` holds, or `(0, 0)`.
fn extent_of<T>(row: &[T], on: impl Fn(&T) -> bool) -> (u32, u32) {
    let first = row.iter().position(&on);
    let last = row.iter().rposition(&on);
    match (first, last) {
        (Some(a), Some(b)) => (
            u32::try_from(a).expect("row fits u32"),
            u32::try_from(b + 1).expect("row fits u32"),
        ),
        _ => (0, 0),
    }
}

/// What a cached raster was built from.
#[derive(Debug)]
struct RasterKey {
    source: Arc<SourceImage>,
    adjustments: Adjustments,
    transform: Transform,
    mask_generation: Option<u64>,
    grid: SuperGrid,
}

impl RasterKey {
    fn of(layer: &Layer, grid: SuperGrid) -> Self {
        Self {
            source: Arc::clone(layer.source()),
            adjustments: layer.adjustments().clone(),
            transform: layer.transform(),
            mask_generation: layer.mask().map(crate::mask::Mask::generation),
            grid,
        }
    }

    fn matches(&self, layer: &Layer, grid: SuperGrid) -> bool {
        Arc::ptr_eq(&self.source, layer.source())
            && self.adjustments == *layer.adjustments()
            && self.transform == layer.transform()
            && self.mask_generation == layer.mask().map(crate::mask::Mask::generation)
            && self.grid == grid
    }
}

/// A mask after its modifiers, with what it was built from.
#[derive(Debug)]
struct EvaluatedMask {
    modifiers: Modifiers,
    /// The raster it was evaluated from (pointer identity).
    from: Arc<LayerRaster>,
    values: Arc<Vec<f32>>,
    /// Where the raster is not all zero and the mask is above zero.
    extent: Extents,
}

/// A mask's raster combined with its sources in layer space, with what it was built
/// from: it depends on neither the transform nor the grid, so a nudge reuses it.
#[derive(Debug)]
struct LayerValues {
    generation: u64,
    source: Arc<SourceImage>,
    values: Vec<f32>,
}

/// Buffers of a layer's replaced raster and evaluated mask, reused by its next rebuild:
/// a drag rebuilds the same layer every frame, and fresh buffers would page in fresh
/// memory each time.
#[derive(Debug, Default)]
struct Spare {
    rgba: Planes,
    values: Vec<f32>,
}

/// The adjusted pixels premultiplied, with the adjusted buffer they came from.
type Premultiplied = (Arc<[[f32; 4]]>, Vec<[f32; 4]>);

#[derive(Debug, Default)]
struct LayerCache {
    adjust: AdjustCache,
    /// The adjusted pixels premultiplied, keyed by the adjusted buffer's identity.
    premul: Option<Premultiplied>,
    /// The mask's layer-space values when it has sources.
    values: Option<LayerValues>,
    raster: Option<(RasterKey, Arc<LayerRaster>)>,
    /// The feather cache: rebuilt only when the modifiers or the raster change.
    mask: Option<EvaluatedMask>,
    spare: Spare,
    /// What the last refresh replaced, recycled into `spare` by [`Self::recycle`] once
    /// the previous composite no longer holds it.
    retired: (Option<Arc<LayerRaster>>, Option<Arc<Vec<f32>>>),
}

impl LayerCache {
    /// Brings the raster and the evaluated mask up to date with `layer`; returns whether
    /// it resampled the layer and whether it evaluated the mask.
    fn refresh(&mut self, layer: &Layer, grid: SuperGrid, full_res: bool) -> (bool, bool) {
        let fresh = match &self.raster {
            Some((key, _)) => !key.matches(layer, grid),
            None => true,
        };
        if fresh {
            let (raster, values) = self.build_raster(layer, grid, full_res);
            let raster = Arc::new(raster);
            let key = RasterKey::of(layer, grid);
            if let Some((_, old)) = self.raster.replace((key, Arc::clone(&raster))) {
                self.retired.0 = Some(old);
            }
            return match (layer.mask(), values) {
                (Some(m), Some(values)) => {
                    self.set_mask(m.modifiers(), raster, values);
                    (true, true)
                }
                _ => {
                    self.mask = None;
                    (true, false)
                }
            };
        }
        let (_, raster) = self.raster.as_ref().expect("not fresh");
        let raster = Arc::clone(raster);
        let Some(m) = layer.mask().filter(|_| !raster.rect.is_empty()) else {
            self.mask = None;
            return (false, false);
        };
        let modifiers = m.modifiers();
        let stale = self
            .mask
            .as_ref()
            .is_none_or(|e| e.modifiers != modifiers || !Arc::ptr_eq(&e.from, &raster));
        if !stale {
            return (false, false);
        }
        // A modifier changed: resample the mask again (the raster's colour stays) and
        // evaluate it.
        let (mx, my, _) = axis_maps(layer, grid);
        let mut values = std::mem::take(&mut self.spare.values);
        self.update_mask_source(m, layer.source());
        let src = self.mask_source(m);
        resample_mask(src, layer.source().size().0, &mx, &my, &mut values);
        let rect = raster.rect;
        crate::mask::eval::apply_modifiers_with(&mut values, rect, grid, &modifiers, full_res);
        self.set_mask(modifiers, raster, values);
        (false, true)
    }

    /// Caches `values` as the evaluated mask of `raster`, with where it is above zero.
    fn set_mask(&mut self, modifiers: Modifiers, raster: Arc<LayerRaster>, values: Vec<f32>) {
        let w = raster.rect.width() as usize;
        let rows: Vec<(&[f32], (u32, u32))> = values
            .chunks(w.max(1))
            .zip(raster.extent.iter().copied())
            .collect();
        let extent = crate::par::map(&rows, |&(row, (a, b))| {
            let (i, j) = extent_of(&row[a as usize..b as usize], |&m| m > 0.0);
            if i == j { (0, 0) } else { (a + i, a + j) }
        });
        let new = EvaluatedMask {
            modifiers,
            from: raster,
            values: Arc::new(values),
            extent: Arc::new(extent),
        };
        if let Some(old) = self.mask.replace(new) {
            self.retired.1 = Some(old.values);
        }
    }

    /// Moves what the last refresh replaced into the spare buffers, if nothing else
    /// holds it any more.
    fn recycle(&mut self) {
        if let Some(old) = self.retired.0.take()
            && let Ok(old) = Arc::try_unwrap(old)
        {
            self.spare.rgba = old.rgba;
        }
        if let Some(old) = self.retired.1.take()
            && let Ok(old) = Arc::try_unwrap(old)
        {
            self.spare.values = old;
        }
    }

    /// Brings the cache of `m`'s layer-space values (its raster combined with its
    /// sources) up to date; it changes only with the mask or the source.
    fn update_mask_source(&mut self, m: &Mask, src: &Arc<SourceImage>) {
        let stale = !m.sources().is_empty()
            && !matches!(&self.values, Some(v)
                if v.generation == m.generation() && Arc::ptr_eq(&v.source, src));
        if stale {
            self.values = Some(LayerValues {
                generation: m.generation(),
                source: Arc::clone(src),
                values: m.layer_values(src).into_owned(),
            });
        }
    }

    /// The layer-space values `m` resamples, after [`Self::update_mask_source`].
    fn mask_source<'a>(&'a self, m: &'a Mask) -> &'a [f32] {
        if m.sources().is_empty() {
            m.raster().values()
        } else {
            &self.values.as_ref().expect("updated by the caller").values
        }
    }

    /// Resamples the layer (adjusted, premultiplied) onto the grid and, alongside, its
    /// mask, which it evaluates (modifiers applied) when the layer has one.
    fn build_raster(
        &mut self,
        layer: &Layer,
        grid: SuperGrid,
        full_res: bool,
    ) -> (LayerRaster, Option<Vec<f32>>) {
        let src = layer.source();
        let (sw, sh) = src.size();
        let (mx, my, rect) = axis_maps(layer, grid);
        if rect.is_empty() {
            let raster = LayerRaster {
                rect: PxRect::EMPTY,
                rgba: Planes::default(),
                extent: Arc::default(),
            };
            return (raster, None);
        }
        let adjusted = self.adjust.get(src, layer.adjustments());
        if !matches!(&self.premul, Some((a, _)) if Arc::ptr_eq(a, &adjusted)) {
            let premul = adjusted
                .iter()
                .map(|p| [p[0] * p[3], p[1] * p[3], p[2] * p[3], p[3]])
                .collect();
            self.premul = Some((Arc::clone(&adjusted), premul));
        }
        let mut rgba = std::mem::take(&mut self.spare.rgba);
        let mut values = std::mem::take(&mut self.spare.values);
        let mask = layer.mask();
        if let Some(m) = mask {
            self.update_mask_source(m, src);
        }
        let mask_src = mask.map(|m| self.mask_source(m));
        let (_, premul) = self.premul.as_ref().expect("filled above");
        debug_assert_eq!(sh as usize * sw as usize, premul.len());
        // The colour and the mask are independent: resample them side by side.
        let (extent, values) = crate::par::join(
            || {
                let mut extent = Vec::new();
                resample_planes(premul, sw, &mx, &my, &mut rgba, &mut extent, |y, row| {
                    let cy = my.cov[y];
                    for (p, cx) in row.iter_mut().zip(&mx.cov) {
                        let k = cx * cy;
                        // Lanczos can ring slightly negative; alpha stays in 0..=1 and
                        // colour ≥ 0.
                        p[3] = (p[3] * k).clamp(0.0, 1.0);
                        for c in &mut p[..3] {
                            *c = (*c * k).max(0.0);
                        }
                    }
                    extent_of(row, |p| *p != [0.0; 4])
                });
                extent
            },
            || {
                let (m, mask_src) = mask.zip(mask_src)?;
                resample_mask(mask_src, sw, &mx, &my, &mut values);
                let modifiers = m.modifiers();
                crate::mask::eval::apply_modifiers_with(
                    &mut values,
                    rect,
                    grid,
                    &modifiers,
                    full_res,
                );
                Some(values)
            },
        );
        let raster = LayerRaster {
            rect,
            rgba,
            extent: Arc::new(extent),
        };
        (raster, values)
    }
}

/// A layer's per-axis resampling maps onto `grid` and the super rectangle it touches.
fn axis_maps(layer: &Layer, grid: SuperGrid) -> (AxisMap, AxisMap, PxRect) {
    let (sw, sh) = layer.source().size();
    let t = layer.transform();
    let step = grid.super_px_in_canvas();
    let (gw, gh) = grid.size();
    let mx = AxisMap::new(sw, t.translate().x, t.scale().x, t.flip_h(), step.x, gw);
    let my = AxisMap::new(sh, t.translate().y, t.scale().y, t.flip_v(), step.y, gh);
    let rect = PxRect::new(mx.out0, my.out0, mx.out0 + mx.len(), my.out0 + my.len());
    (mx, my, rect)
}

/// Resamples a mask's layer-space values (`sw` wide) into `out`, clamped to `0..=1`.
fn resample_mask(values: &[f32], sw: u32, mx: &AxisMap, my: &AxisMap, out: &mut Vec<f32>) {
    resample_into(values, sw, mx, my, out, |_, row| {
        for x in row {
            *x = x.clamp(0.0, 1.0);
        }
    });
}

/// One layer ready to blend.
#[derive(Clone)]
struct Blendable {
    /// Attribute-map code: the layer's index in the stack.
    code: u16,
    /// The recipe's dither strength.
    strength: f32,
    blend: Blend,
    opacity: f32,
    raster: Arc<LayerRaster>,
    mask: Option<Arc<Vec<f32>>>,
    /// Where the layer contributes, per row of its rectangle.
    extent: Extents,
}

impl Blendable {
    /// Whether blending `other` instead of `self` gives the same pixels and attributes.
    fn same(&self, other: &Self) -> bool {
        self.code == other.code
            && self.strength == other.strength
            && self.blend == other.blend
            && self.opacity == other.opacity
            && Arc::ptr_eq(&self.raster, &other.raster)
            && match (&self.mask, &other.mask) {
                (None, None) => true,
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                _ => false,
            }
    }
}

/// Whether two stack slots (`None`: hidden) blend the same.
fn same_slot(a: Option<&Blendable>, b: Option<&Blendable>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => a.same(b),
        _ => false,
    }
}

/// The bottom slots of the stack folded over the whole grid: colour and attributes
/// before any layer above them and before regions. An edit to a higher layer starts its
/// rows from here and re-blends only the layers from the edited one up; the fold is the
/// same operations in the same order, so the result is bit-identical.
#[derive(Debug)]
struct Below {
    /// The slots folded, `state[..key.len()]` of the stack it was built from.
    key: Vec<Option<Blendable>>,
    rgba: Planes,
    attrs: AttrPlanes,
}

/// Composites layers onto a [`SuperGrid`], caching each layer's resampled raster, and
/// keeps the [`TargetFrame`] the solver reads.
///
/// [`Self::composite_with`] recomposites everything; [`Self::update`] recomposites only
/// the cells an edit can have touched and reports the cells whose output changed, so
/// the solver can re-solve locally (ADR-002, ADR-008).
#[derive(Debug)]
pub struct Compositor {
    grid: SuperGrid,
    caches: Vec<LayerCache>,
    canvas: Canvas,
    /// The last composite on the target grid.
    frame: TargetFrame,
    /// What was blended last time, per layer index (`None`: hidden), and the regions.
    last: Option<(Vec<Option<Blendable>>, Vec<Region>)>,
    /// The fold of the layers below the last edited one, for [`Self::update`].
    below: Option<Below>,
    /// Blur every feather at full resolution ([`Self::set_full_res_feather`]).
    full_res_feather: bool,
    resamples: u64,
    mask_evals: u64,
}

impl std::fmt::Debug for Blendable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Blendable")
            .field("code", &self.code)
            .field("rect", &self.raster.rect)
            .finish_non_exhaustive()
    }
}

impl Compositor {
    /// A compositor for `grid` with empty caches.
    pub fn new(grid: SuperGrid) -> Self {
        Self {
            grid,
            caches: Vec::new(),
            canvas: Canvas::new(grid),
            frame: TargetFrame::empty(grid),
            last: None,
            below: None,
            full_res_feather: false,
            resamples: 0,
            mask_evals: 0,
        }
    }

    /// The grid.
    pub const fn grid(&self) -> SuperGrid {
        self.grid
    }

    /// Switches grid (target mode or supersample factor); every cache is rebuilt on the
    /// next composite.
    pub fn set_grid(&mut self, grid: SuperGrid) {
        if grid != self.grid {
            *self = Self {
                resamples: self.resamples,
                mask_evals: self.mask_evals,
                full_res_feather: self.full_res_feather,
                ..Self::new(grid)
            };
        }
    }

    /// Blurs every feather at full resolution instead of at half resolution from
    /// σ = [`HALF_RES_FEATHER`](crate::mask::HALF_RES_FEATHER) target px: the reference
    /// the half-resolution blur is measured against (#98). Masks are re-evaluated on the
    /// next composite.
    #[doc(hidden)]
    pub fn set_full_res_feather(&mut self, on: bool) {
        if on != self.full_res_feather {
            self.full_res_feather = on;
            for c in &mut self.caches {
                c.mask = None;
            }
        }
    }

    /// The last composite.
    pub const fn canvas(&self) -> &Canvas {
        &self.canvas
    }

    /// The last composite on the target grid: colour, coverage and attribute map.
    pub const fn frame(&self) -> &TargetFrame {
        &self.frame
    }

    /// How many layer rasters have been (re)built; for tests and diagnostics.
    pub const fn resamples(&self) -> u64 {
        self.resamples
    }

    /// How many masks have been (re)evaluated (modifiers, including feather).
    pub const fn mask_evals(&self) -> u64 {
        self.mask_evals
    }

    /// Layer `i`'s mask after its modifiers, on the super grid, from the last composite:
    /// the rectangle it covers and its values. `None` for an unmasked, hidden or unknown
    /// layer. For the quick-mask overlay and tests.
    pub fn evaluated_mask(&self, i: usize) -> Option<(PxRect, &[f32])> {
        let c = self.caches.get(i)?;
        let m = c.mask.as_ref()?;
        Some((m.from.rect, m.values.as_slice()))
    }

    /// The attribute map of the last composite, box-filtered to the target grid.
    pub fn attribute_map(&self) -> AttributeMap {
        self.frame.attributes().clone()
    }

    /// Composites `layers` (bottom first) with no regions and returns the canvas.
    pub fn composite(&mut self, layers: &[Layer]) -> &Canvas {
        self.composite_with(layers, &[])
    }

    /// Composites `layers` (bottom first), then applies `regions` (in order) to the
    /// attribute map, and returns the canvas. Everything is recomposited.
    pub fn composite_with(&mut self, layers: &[Layer], regions: &[Region]) -> &Canvas {
        let state = self.prepare(layers);
        self.recomposite(layers, regions, state, self.grid.bounds(), 0);
        &self.canvas
    }

    /// Brings the composite up to date with `layers` and `regions`, recompositing only
    /// what changed since the last call, and returns the target cells whose output
    /// (colour, coverage or attributes) changed. The first call composites everything.
    ///
    /// Only the layers from the lowest changed one up are re-blended: the ones below come
    /// from a cached fold of them, kept while they stay unchanged.
    pub fn update(&mut self, layers: &[Layer], regions: &[Region]) -> CellSet {
        let state = self.prepare(layers);
        let grid = self.grid;
        let (area, lowest) = match &self.last {
            None => (grid.bounds(), 0),
            Some((old, old_regions)) => {
                let mut r = PxRect::EMPTY;
                let mut lowest = state.len();
                for i in 0..old.len().max(state.len()) {
                    let (a, b) = (
                        old.get(i).and_then(Option::as_ref),
                        state.get(i).and_then(Option::as_ref),
                    );
                    if !same_slot(a, b) {
                        lowest = lowest.min(i);
                        for x in [a, b].into_iter().flatten() {
                            r = r.union(x.raster.rect);
                        }
                    }
                }
                if old_regions.as_slice() != regions {
                    for reg in old_regions.iter().chain(regions) {
                        r = r.union(region_rect(grid, reg));
                    }
                }
                (r, lowest)
            }
        };
        if area.is_empty() {
            self.last = Some((state, regions.to_vec()));
            self.recycle();
            return CellSet::EMPTY;
        }
        // Whole cells, so every target pixel in the area is re-filtered from fresh
        // super pixels and the diff below sees whole cells.
        let (cw, ch) = grid.cell_size_super();
        let area = PxRect::new(
            area.x0 / cw * cw,
            area.y0 / ch * ch,
            area.x1.div_ceil(cw) * cw,
            area.y1.div_ceil(ch) * ch,
        )
        .intersect(grid.bounds());
        let from = self.below_for(&state, lowest, area);
        let changed = self.recomposite(layers, regions, state, area, from);
        let cw = grid.cell_size().0;
        let mut cells = CellSet::EMPTY;
        for (ty, bits) in changed.into_iter().enumerate() {
            for col in (0..64).filter(|c| bits & (1 << c) != 0) {
                if let Some(c) = grid.target_px_to_cell(col * cw, ty as u32) {
                    cells.insert(c);
                }
            }
        }
        cells
    }

    /// How many bottom slots of `state` the cached fold can supply for an update that
    /// changed slot `lowest` and up and recomposites `area`; 0 for none. Rebuilds the
    /// cache when it does not match and the area is large enough to repay it (a quarter
    /// of the grid): a drag repeats the same update many times.
    fn below_for(&mut self, state: &[Option<Blendable>], lowest: usize, area: PxRect) -> usize {
        if lowest == 0 {
            return 0;
        }
        if let Some(b) = &self.below {
            let k = b.key.len();
            if k <= lowest
                && k <= state.len()
                && b.key
                    .iter()
                    .zip(state)
                    .all(|(a, s)| same_slot(a.as_ref(), s.as_ref()))
            {
                return k;
            }
        }
        let (w, h) = self.grid.size();
        if u64::from(area.width()) * u64::from(area.height()) * 4 < u64::from(w) * u64::from(h) {
            return 0;
        }
        let k = lowest.min(state.len());
        let mut b = self.below.take().unwrap_or_else(|| Below {
            key: Vec::new(),
            rgba: Planes::new(grid_len(self.grid)),
            attrs: AttrPlanes::new(grid_len(self.grid)),
        });
        b.key = state[..k].to_vec();
        let stack: Vec<Blendable> = b.key.iter().flatten().cloned().collect();
        fold_below(&mut b, self.grid, &stack);
        self.below = Some(b);
        k
    }

    /// Folds `state` into the canvas over `area` and re-filters that part of the frame.
    /// With `from > 0`, the first `from` slots come from the cached fold.
    fn recomposite(
        &mut self,
        layers: &[Layer],
        regions: &[Region],
        state: Vec<Option<Blendable>>,
        area: PxRect,
        from: usize,
    ) -> Vec<u64> {
        let stack: Vec<Blendable> = state[from..].iter().flatten().cloned().collect();
        self.frame.attributes_mut().set_recipes(
            layers.iter().map(|l| *l.recipe()).collect(),
            regions.iter().map(|r| *r.recipe()).collect(),
        );
        let base = if from > 0 { self.below.as_ref() } else { None };
        let changed = fold(
            &mut self.canvas,
            &mut self.frame,
            base,
            &stack,
            regions,
            area,
        );
        self.last = Some((state, regions.to_vec()));
        self.recycle();
        changed
    }

    /// Recycles the buffers the last refresh replaced, now that the previous composite's
    /// stack no longer holds them.
    fn recycle(&mut self) {
        for c in &mut self.caches {
            c.recycle();
        }
    }

    /// Brings every visible layer's raster and evaluated mask up to date, the layers in
    /// parallel (each is independent).
    fn prepare(&mut self, layers: &[Layer]) -> Vec<Option<Blendable>> {
        assert!(layers.len() < 0x8000, "at most 32767 layers");
        self.caches.resize_with(layers.len(), LayerCache::default);
        let (grid, full_res) = (self.grid, self.full_res_feather);
        let shown = |l: &Layer| l.visible() && l.opacity() > 0.0;
        let mut work: Vec<(&Layer, &mut LayerCache, (bool, bool))> = layers
            .iter()
            .zip(&mut self.caches)
            .filter(|(l, _)| shown(l))
            .map(|(l, c)| (l, c, (false, false)))
            .collect();
        crate::par::for_each(&mut work, |(layer, cache, done)| {
            *done = cache.refresh(layer, grid, full_res);
        });
        for (_, _, (resampled, evaluated)) in work {
            self.resamples += u64::from(resampled);
            self.mask_evals += u64::from(evaluated);
        }
        layers
            .iter()
            .zip(&self.caches)
            .enumerate()
            .map(|(index, (layer, cache))| {
                if !shown(layer) {
                    return None;
                }
                let (_, raster) = cache.raster.as_ref().expect("refreshed above");
                Some(Blendable {
                    code: u16::try_from(index).expect("checked above"),
                    strength: layer.recipe().strength(),
                    blend: layer.blend(),
                    opacity: layer.opacity(),
                    raster: Arc::clone(raster),
                    mask: cache.mask.as_ref().map(|e| Arc::clone(&e.values)),
                    extent: Arc::clone(cache.mask.as_ref().map_or(&raster.extent, |e| &e.extent)),
                })
            })
            .collect()
    }
}

/// The super pixels a region's mask can affect: the bounding box of its non-zero
/// values, grown by one canvas pixel for bilinear sampling.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "coordinates are clamped to the super grid"
)]
fn region_rect(grid: SuperGrid, region: &Region) -> PxRect {
    let m = region.mask();
    let (w, h) = m.size();
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for y in 0..h {
        for x in 0..w {
            if m.get(x, y) > 0.0 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    if x1 <= x0 {
        return PxRect::EMPTY;
    }
    let a = grid.canvas_to_super(Vec2::new(x0 as f32 - 1.0, y0 as f32 - 1.0));
    let b = grid.canvas_to_super(Vec2::new(x1 as f32 + 1.0, y1 as f32 + 1.0));
    let (gw, gh) = grid.size();
    PxRect::new(
        a.x.max(0.0).floor() as u32,
        a.y.max(0.0).floor() as u32,
        (b.x.ceil().max(0.0) as u32).min(gw),
        (b.y.ceil().max(0.0) as u32).min(gh),
    )
}

fn grid_len(grid: SuperGrid) -> usize {
    let (w, h) = grid.size();
    w as usize * h as usize
}

/// A layer's part of one super-grid row: the canvas columns, its pixels and its mask.
type RowSlices<'a> = (std::ops::Range<usize>, [&'a [f32]; 4], Option<&'a [f32]>);

/// Where layer `b` contributes to row `y` of `area`, or `None`.
fn row_slices(b: &Blendable, area: PxRect, y: u32) -> Option<RowSlices<'_>> {
    let r = &b.raster;
    if !(r.rect.y0..r.rect.y1).contains(&y) {
        return None;
    }
    let (e0, e1) = b.extent[(y - r.rect.y0) as usize];
    let span = PxRect::new(r.rect.x0 + e0, y, r.rect.x0 + e1, y + 1).intersect(PxRect::new(
        area.x0,
        y,
        area.x1,
        y + 1,
    ));
    if span.is_empty() {
        return None;
    }
    let i0 = (y - r.rect.y0) as usize * r.rect.width() as usize + (span.x0 - r.rect.x0) as usize;
    let n = span.width() as usize;
    let px = i0..i0 + n;
    let rgba = &r.rgba;
    Some((
        span.x0 as usize..span.x1 as usize,
        [
            &rgba.r[px.clone()],
            &rgba.g[px.clone()],
            &rgba.b[px.clone()],
            &rgba.a[px.clone()],
        ],
        b.mask.as_ref().map(|m| &m[px]),
    ))
}

/// Blends `stack` (bottom first) into `rows` super rows from `y0`, over `area`'s
/// columns: row `i` of `dst`, `code` and `strength` is `i × width .. (i + 1) × width`
/// (`width` = the area's). Layer by layer, so each layer's rows are read in one run.
///
/// The rows already hold what lies below, or, with `clear`, nothing: then whatever they
/// hold is overwritten, the first layer stored straight in (blending onto a transparent
/// backdrop gives the source itself in every mode) and the rest cleared.
///
/// Colour blends bottom-up. Attributes are overwritten bottom-up too, so the top-most
/// layer whose effective alpha (PNG alpha × mask) is above zero governs, with strength ×
/// opacity × that alpha.
#[expect(
    clippy::too_many_arguments,
    reason = "one band's planes and where it lies"
)]
fn compose_band(
    mut dst: RgbaRow<'_>,
    code: &mut [u16],
    strength: &mut [f32],
    stack: &[Blendable],
    area: PxRect,
    y0: u32,
    rows: u32,
    clear: bool,
) {
    let (x0, width) = (area.x0 as usize, area.width() as usize);
    let claim = |b: &Blendable| blend::Claim {
        code: b.code,
        scale: b.strength * b.opacity,
    };
    let mut stack = stack;
    if clear {
        let first = stack.first();
        for i in 0..rows {
            let o = i as usize * width;
            let cols = match first.and_then(|b| row_slices(b, area, y0 + i)) {
                Some((xs, src, mask)) => {
                    let b = first.expect("has a row");
                    let cols = o + xs.start - x0..o + xs.end - x0;
                    let attr = blend::AttrRow {
                        code: &mut code[cols.clone()],
                        strength: &mut strength[cols.clone()],
                    };
                    blend::store(
                        dst.slice(cols.clone()),
                        attr,
                        src,
                        mask,
                        b.opacity,
                        claim(b),
                    );
                    cols
                }
                None => o..o,
            };
            for gap in [o..cols.start, cols.end..o + width] {
                let d = dst.slice(gap.clone());
                for p in [&mut *d.r, &mut *d.g, &mut *d.b, &mut *d.a] {
                    p.fill(0.0);
                }
                code[gap.clone()].fill(attr::UNCLAIMED);
                strength[gap].fill(0.0);
            }
        }
        stack = stack.get(1..).unwrap_or_default();
    }
    for b in stack {
        let claim = claim(b);
        for i in 0..rows {
            if let Some((xs, src, mask)) = row_slices(b, area, y0 + i) {
                let o = i as usize * width;
                let cols = o + xs.start - x0..o + xs.end - x0;
                let attr = blend::AttrRow {
                    code: &mut code[cols.clone()],
                    strength: &mut strength[cols.clone()],
                };
                blend::span(b.blend, dst.slice(cols), attr, src, mask, b.opacity, claim);
            }
        }
    }
}

/// Super rows per task in [`fold_below`].
const BELOW_ROWS: usize = 4;

/// Folds `stack` over the whole grid into `below`'s planes.
fn fold_below(below: &mut Below, grid: SuperGrid, stack: &[Blendable]) {
    let w = grid.size().0 as usize;
    let bounds = grid.bounds();
    let Below { rgba, attrs, .. } = below;
    let n = w * BELOW_ROWS;
    let mut bands: Vec<_> = rgba
        .rows_mut(n)
        .zip(attrs.code.chunks_mut(n).zip(attrs.strength.chunks_mut(n)))
        .enumerate()
        .collect();
    crate::par::for_each(&mut bands, |(i, (dst, (code, strength)))| {
        let y0 = u32::try_from(*i * BELOW_ROWS).expect("super rows fit u32");
        let rows = u32::try_from(code.len() / w).expect("super rows fit u32");
        let len = code.len();
        let dst = dst.slice(0..len);
        compose_band(dst, code, strength, stack, bounds, y0, rows, true);
    });
}

/// Blends `stack` (bottom first) into `canvas` over `area` (aligned to whole target
/// pixels), starting from `base` or a cleared canvas, rebuilds the attribute map there
/// ([`compose_band`]; then each region in order governs where its mask is above zero,
/// strength × mask), and box-filters the result into `frame`. Works one target row (`n`
/// super rows) at a time, so the rows it filters are still in cache. Returns, per target
/// row, the cell columns (one bit each) where the frame's output changed.
fn fold(
    canvas: &mut Canvas,
    frame: &mut TargetFrame,
    base: Option<&Below>,
    stack: &[Blendable],
    regions: &[Region],
    area: PxRect,
) -> Vec<u64> {
    let grid = canvas.grid;
    let (w, _) = grid.size();
    let (tw, th) = grid.target_size();
    let cell_w = grid.cell_size().0 as usize;
    let n = u32::from(grid.n());
    debug_assert!(
        [area.x0, area.y0, area.x1, area.y1]
            .iter()
            .all(|v| v.is_multiple_of(n)),
        "area must cover whole target pixels: {area:?}"
    );
    let (w, tw, band) = (w as usize, tw as usize, n as usize * w as usize);
    let (colour, coverage, target_attrs) = frame.parts_mut();
    let mut changed = vec![0u64; th as usize];
    let mut bands: Vec<FoldBand<'_>> = canvas
        .px
        .chunks_mut(band)
        .zip(colour.chunks_mut(tw).zip(coverage.chunks_mut(tw)))
        .zip(target_attrs.chunks_mut(tw).zip(&mut changed))
        .enumerate()
        .filter(|(ty, _)| ((area.y0 / n) as usize..(area.y1 / n) as usize).contains(ty))
        .map(|(ty, ((px, (c, cov)), (ta, ch)))| (ty, px, c, cov, ta, ch))
        .collect();
    let (x0, x1) = (area.x0 as usize, area.x1 as usize);
    let width = x1 - x0;
    let len = n as usize * width;
    let mut groups: Vec<&mut [FoldBand<'_>]> = bands.chunks_mut(BANDS_PER_TASK).collect();
    crate::par::for_each(&mut groups, |group| {
        // The band being composed, its `n` super rows over the area: colour planes, then
        // the strength plane, and the code plane.
        scratch::with(5 * len, |planes| {
            scratch::with(len, |code: &mut [u16]| {
                let (rgba, strength) = planes.split_at_mut(4 * len);
                let (r, rest) = rgba.split_at_mut(len);
                let (g, rest) = rest.split_at_mut(len);
                let (b, a) = rest.split_at_mut(len);
                let mut rgba = RgbaRow { r, g, b, a };
                let mut seen = Vec::with_capacity(4);
                for (ty, px, colour, coverage, target_attr, changed) in group.iter_mut() {
                    let y0 = *ty as u32 * n;
                    // With no base, the band starts clear (`compose_band` overwrites it).
                    match base {
                        None => {}
                        Some(b) => {
                            for sy in 0..n as usize {
                                let o = (y0 as usize + sy) * w;
                                let (from, to) = (o + x0..o + x1, sy * width..(sy + 1) * width);
                                rgba.r[to.clone()].copy_from_slice(&b.rgba.r[from.clone()]);
                                rgba.g[to.clone()].copy_from_slice(&b.rgba.g[from.clone()]);
                                rgba.b[to.clone()].copy_from_slice(&b.rgba.b[from.clone()]);
                                rgba.a[to.clone()].copy_from_slice(&b.rgba.a[from.clone()]);
                                code[to.clone()].copy_from_slice(&b.attrs.code[from.clone()]);
                                strength[to].copy_from_slice(&b.attrs.strength[from]);
                            }
                        }
                    }
                    compose_band(
                        rgba.slice(0..len),
                        code,
                        strength,
                        stack,
                        area,
                        y0,
                        n,
                        base.is_none(),
                    );
                    for (j, region) in regions.iter().enumerate() {
                        let rc =
                            attr::AttrSource::Region(u16::try_from(j).expect("few regions")).code();
                        let s = region.recipe().strength();
                        for sy in 0..n {
                            let row = sy as usize * width..(sy as usize + 1) * width;
                            let attrs = code[row.clone()].iter_mut().zip(&mut strength[row]);
                            for (x, (c, st)) in (area.x0..area.x1).zip(attrs) {
                                let m = region.sample(grid.super_px_centre_in_canvas(x, y0 + sy));
                                if m > 0.0 {
                                    *c = rc;
                                    *st = s * m;
                                }
                            }
                        }
                    }
                    for sy in 0..n as usize {
                        let from = sy * width..(sy + 1) * width;
                        let (r, g, b, a) = (
                            &rgba.r[from.clone()],
                            &rgba.g[from.clone()],
                            &rgba.b[from.clone()],
                            &rgba.a[from],
                        );
                        for (i, o) in px[sy * w + x0..sy * w + x1].iter_mut().enumerate() {
                            *o = [r[i], g[i], b[i], a[i]];
                        }
                    }
                    // Box-filter into the frame, noting the cell columns whose output
                    // changed.
                    let txs = x0 / n as usize..x1 / n as usize;
                    let mut bits = 0u64;
                    frame::reduce_row(px, w, n, txs.clone(), |tx, c, v| {
                        if colour[tx] != c || coverage[tx] != v {
                            bits |= 1 << (tx / cell_w);
                        }
                        (colour[tx], coverage[tx]) = (c, v);
                    });
                    let t0 = txs.start;
                    attr::reduce_row(code, strength, width, n, 0..txs.len(), &mut seen, |t, a| {
                        let tx = t0 + t;
                        if target_attr[tx] != a {
                            bits |= 1 << (tx / cell_w);
                        }
                        target_attr[tx] = a;
                    });
                    **changed = bits;
                }
            });
        });
    });
    changed
}

/// Target rows per task in [`fold`], which share their scratch buffers.
const BANDS_PER_TASK: usize = 4;

impl Planes {
    /// Chunks of `n` pixels.
    fn rows_mut(&mut self, n: usize) -> impl Iterator<Item = RgbaRow<'_>> {
        self.r
            .chunks_mut(n)
            .zip(self.g.chunks_mut(n))
            .zip(self.b.chunks_mut(n).zip(self.a.chunks_mut(n)))
            .map(|((r, g), (b, a))| RgbaRow { r, g, b, a })
    }
}

/// One target row's work: its index, its `n` canvas super rows, its colour, coverage
/// and attribute target rows, and the cell columns whose output changed (one bit each).
type FoldBand<'a> = (
    usize,
    &'a mut [[f32; 4]],
    &'a mut [[f32; 3]],
    &'a mut [f32],
    &'a mut [TargetAttr],
    &'a mut u64,
);

/// The target cells a change to `layer`'s mask inside `layer_rect` (layer pixels) can
/// affect: the rectangle mapped through the layer transform onto the super grid, grown
/// by the resampling support, snapped out to whole cells when the mask snaps, and grown
/// by the feather's reach. The solver re-solves only these cells (ADR-008).
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "coordinates are clamped to the super grid before casting"
)]
pub fn mask_change_cells(grid: SuperGrid, layer: &Layer, layer_rect: PxRect) -> CellSet {
    if layer_rect.is_empty() {
        return CellSet::EMPTY;
    }
    let size = layer.source().size();
    let t = layer.transform();
    let a = grid.canvas_to_super(
        t.layer_to_canvas(size, Vec2::new(layer_rect.x0 as f32, layer_rect.y0 as f32)),
    );
    let b = grid.canvas_to_super(
        t.layer_to_canvas(size, Vec2::new(layer_rect.x1 as f32, layer_rect.y1 as f32)),
    );
    let step = grid.super_px_in_canvas();
    // Resampling reaches one source pixel (bilinear) or three super pixels (Lanczos).
    let reach_x = (t.scale().x / step.x).max(3.0).ceil() as u32 + 1;
    let reach_y = (t.scale().y / step.y).max(3.0).ceil() as u32 + 1;
    let (gw, gh) = grid.size();
    let clampf = |v: f32, hi: u32| v.clamp(0.0, hi as f32);
    let mut r = PxRect::new(
        (clampf(a.x.min(b.x), gw).floor() as u32).saturating_sub(reach_x),
        (clampf(a.y.min(b.y), gh).floor() as u32).saturating_sub(reach_y),
        (clampf(a.x.max(b.x), gw).ceil() as u32 + reach_x).min(gw),
        (clampf(a.y.max(b.y), gh).ceil() as u32 + reach_y).min(gh),
    );
    let m = layer
        .mask()
        .map(crate::mask::Mask::modifiers)
        .unwrap_or_default();
    if m.snap() != SnapToCells::Off {
        let (cw, ch) = grid.cell_size_super();
        r = PxRect::new(
            r.x0 / cw * cw,
            r.y0 / ch * ch,
            r.x1.div_ceil(cw) * cw,
            r.y1.div_ceil(ch) * ch,
        )
        .intersect(grid.bounds());
    }
    if m.feather() > 0.0 {
        let mut reach = 3.0 * m.feather() * f32::from(grid.n()) + 3.0;
        if crate::mask::eval::half_res(m.feather()) {
            reach += 4.0;
        }
        r = r.grow(reach.ceil() as u32, grid.bounds());
    }
    CellSet::from_super_rect(grid, r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Vec2;
    use crate::mask::{Mask, MaskRaster, Modifiers, SnapToCells};
    use crate::mode::Mode;
    use crate::region::Region;
    use proptest::prelude::*;

    fn solid(w: u32, h: u32, rgba: [f32; 4]) -> Arc<SourceImage> {
        Arc::new(SourceImage::from_linear(w, h, vec![rgba; (w * h) as usize]).unwrap())
    }

    fn grid(mode: Mode, n: u8) -> SuperGrid {
        SuperGrid::new(mode, n).unwrap()
    }

    fn at(layer: Layer, x: f32, y: f32) -> Layer {
        let mut l = layer;
        l.set_transform(Transform::translated(Vec2::new(x, y)).unwrap());
        l
    }

    #[test]
    fn full_screen_layer_is_opaque_to_the_edges() {
        for mode in [Mode::HiresBitmap, Mode::MulticolorBitmap] {
            let mut c = Compositor::new(grid(mode, 4));
            let l = Layer::new("bg", solid(320, 200, [0.5, 0.25, 0.125, 1.0]));
            let canvas = c.composite(&[l]);
            for p in canvas.pixels() {
                assert!(
                    (p[3] - 1.0).abs() < 1e-6 && (p[0] - 0.5).abs() < 1e-6,
                    "{p:?}"
                );
            }
            let down = canvas.downsample();
            assert_eq!(
                down.len(),
                if mode == Mode::HiresBitmap {
                    64_000
                } else {
                    32_000
                }
            );
            assert!((down[0][1] - 0.25).abs() < 1e-6);
        }
    }

    /// The E4 exit criterion: a sub-pixel nudge changes target pixels continuously.
    #[test]
    fn sub_pixel_nudge_moves_target_pixels_continuously() {
        for mode in [Mode::HiresBitmap, Mode::MulticolorBitmap] {
            let g = grid(mode, 4);
            let mut c = Compositor::new(g);
            // A white 40×40 square on black: track the target pixel its left edge crosses.
            let bg = Layer::new("bg", solid(320, 200, [0.0, 0.0, 0.0, 1.0]));
            let sq = Layer::new("sq", solid(40, 40, [1.0, 1.0, 1.0, 1.0]));
            let (tx, ty) = (50u32, 60u32);
            let (tw, _) = g.target_size();
            let probe = |c: &mut Compositor, dx: f32| -> f32 {
                let layers = [bg.clone(), at(sq.clone(), 100.0 + dx, 40.0)];
                c.composite(&layers).downsample()[(ty * tw + tx) as usize][0]
            };
            // Canvas x of target pixel 50: 50 (hi-res) or 100 (multicolor). Sweep the
            // square's left edge across it in 0.05 px steps.
            let span = if mode == Mode::HiresBitmap {
                -50.0
            } else {
                0.0
            };
            let mut prev = probe(&mut c, span - 1.0);
            let mut max_step = 0.0f32;
            let mut steps = 0;
            for i in 1..=60 {
                let v = probe(&mut c, span - 1.0 + i as f32 * 0.05);
                assert!(
                    v <= prev + 1e-6,
                    "{mode}: not monotone at step {i}: {v} > {prev}"
                );
                max_step = max_step.max(prev - v);
                prev = v;
                steps += 1;
            }
            assert_eq!(steps, 60);
            // A 0.05 px move changes a pixel by at most 0.05 of its width in coverage.
            let width_px = if mode == Mode::HiresBitmap { 1.0 } else { 2.0 };
            assert!(
                max_step <= 0.05 / width_px + 1e-4,
                "{mode}: jump of {max_step}"
            );
            assert!(prev < 1e-6, "{mode}: ends fully covered by black? {prev}");
        }
    }

    #[test]
    fn quarter_pixel_nudge_changes_coverage_by_a_quarter() {
        let g = grid(Mode::HiresBitmap, 4);
        let mut c = Compositor::new(g);
        let sq = Layer::new("sq", solid(8, 8, [1.0, 1.0, 1.0, 1.0]));
        let a = c.composite(&[at(sq.clone(), 10.0, 10.0)]).downsample();
        let b = c.composite(&[at(sq, 10.25, 10.0)]).downsample();
        let i = 10 * 320 + 10;
        assert!((a[i][3] - 1.0).abs() < 1e-6);
        assert!((b[i][3] - 0.75).abs() < 1e-6, "{}", b[i][3]);
        assert!((b[i + 8][3] - 0.25).abs() < 1e-6, "{}", b[i + 8][3]);
    }

    #[test]
    fn caches_rebuild_only_changed_layers() {
        let mut c = Compositor::new(grid(Mode::HiresBitmap, 2));
        let mut layers = vec![
            Layer::new("a", solid(32, 32, [1.0, 0.0, 0.0, 1.0])),
            Layer::new("b", solid(32, 32, [0.0, 1.0, 0.0, 0.5])),
        ];
        c.composite(&layers);
        assert_eq!(c.resamples(), 2);
        c.composite(&layers);
        assert_eq!(c.resamples(), 2);
        layers[1].set_opacity(0.5).unwrap();
        layers[1].set_blend(Blend::Screen);
        c.composite(&layers);
        assert_eq!(c.resamples(), 2, "opacity and blend do not resample");
        layers[1].set_transform(Transform::translated(Vec2::new(0.25, 0.0)).unwrap());
        c.composite(&layers);
        assert_eq!(c.resamples(), 3);
        layers[0]
            .set_mask(Some(Mask::new(MaskRaster::filled(32, 32, 0.5).unwrap())))
            .unwrap();
        c.composite(&layers);
        assert_eq!(c.resamples(), 4);
    }

    #[test]
    fn mask_opacity_and_alpha_multiply() {
        let mut c = Compositor::new(grid(Mode::HiresBitmap, 1));
        let mut l = Layer::new("a", solid(4, 4, [1.0, 1.0, 1.0, 0.5]));
        l.set_mask(Some(Mask::new(MaskRaster::filled(4, 4, 0.5).unwrap())))
            .unwrap();
        l.set_opacity(0.5).unwrap();
        let p = c.composite(&[l]).pixel(1, 1);
        assert!(
            (p[3] - 0.125).abs() < 1e-6 && (p[0] - 0.125).abs() < 1e-6,
            "{p:?}"
        );
    }

    #[test]
    fn scaled_layer_fills_its_box() {
        // A 2000-wide source scaled to 320: Lanczos downscale of a flat field stays flat.
        let mut c = Compositor::new(grid(Mode::MulticolorBitmap, 4));
        let mut l = Layer::new("big", solid(2000, 1250, [0.3, 0.6, 0.9, 1.0]));
        l.set_transform(Transform::new(Vec2::ZERO, Vec2::new(0.16, 0.16), false, false).unwrap());
        for p in c.composite(&[l]).pixels() {
            assert!(
                (p[1] - 0.6).abs() < 1e-4 && (p[3] - 1.0).abs() < 1e-4,
                "{p:?}"
            );
        }
    }

    #[test]
    fn hidden_and_zero_opacity_layers_are_skipped() {
        let mut c = Compositor::new(grid(Mode::HiresBitmap, 1));
        let mut l = Layer::new("a", solid(4, 4, [1.0; 4]));
        l.set_visible(false);
        assert_eq!(c.composite(&[l.clone()]).pixel(0, 0), [0.0; 4]);
        l.set_visible(true);
        l.set_opacity(0.0).unwrap();
        assert_eq!(c.composite(&[l]).pixel(0, 0), [0.0; 4]);
        assert_eq!(c.resamples(), 0);
    }

    fn masked(src: Arc<SourceImage>, mask: Vec<f32>, m: Modifiers) -> Layer {
        let (w, h) = src.size();
        let mut l = Layer::new("m", src);
        let mut mk = Mask::new(MaskRaster::from_values(w, h, mask).unwrap());
        mk.set_modifiers(m);
        l.set_mask(Some(mk)).unwrap();
        l
    }

    /// A mask that is 1 left of source column `edge` and 0 from it on.
    fn half_mask(w: u32, h: u32, edge: u32) -> Vec<f32> {
        (0..w * h)
            .map(|i| if i % w < edge { 1.0 } else { 0.0 })
            .collect()
    }

    #[test]
    fn identity_modifiers_leave_the_mask_untouched() {
        let g = grid(Mode::HiresBitmap, 2);
        let vals: Vec<f32> = (0..64).map(|i| (i % 7) as f32 / 6.0).collect();
        let mut v = vals.clone();
        crate::mask::eval::apply_modifiers(
            &mut v,
            PxRect::new(3, 5, 11, 13),
            g,
            &Modifiers::default(),
        );
        assert_eq!(v, vals);
        let mut v = vals.clone();
        let m =
            Modifiers::new(true, crate::layer::Levels::IDENTITY, SnapToCells::Off, 0.0).unwrap();
        crate::mask::eval::apply_modifiers(&mut v, PxRect::new(3, 5, 11, 13), g, &m);
        crate::mask::eval::apply_modifiers(&mut v, PxRect::new(3, 5, 11, 13), g, &m);
        for (a, b) in v.iter().zip(&vals) {
            assert!((a - b).abs() < 1e-6, "invert twice");
        }
    }

    #[test]
    fn snap_never_leaves_a_partially_covered_cell() {
        for mode in [Mode::HiresBitmap, Mode::MulticolorBitmap] {
            let g = grid(mode, 4);
            for snap in [
                SnapToCells::Expand,
                SnapToCells::Contract,
                SnapToCells::Nearest,
            ] {
                let (w, h) = (90, 70);
                let vals: Vec<f32> = (0..w * h)
                    .map(|i| {
                        let (x, y) = ((i % w) as f32, (i / w) as f32);
                        if (x - 45.0).hypot(y - 35.0) < 27.3 {
                            1.0
                        } else {
                            0.0
                        }
                    })
                    .collect();
                let m = Modifiers::new(false, crate::layer::Levels::IDENTITY, snap, 0.0).unwrap();
                let l = at(masked(solid(w, h, [1.0; 4]), vals, m), 13.3, 9.7);
                let mut c = Compositor::new(g);
                c.composite(&[l]);
                let (rect, v) = c.evaluated_mask(0).unwrap();
                let (cw, ch) = g.cell_size_super();
                let mut cells = std::collections::HashMap::new();
                for y in rect.y0..rect.y1 {
                    for x in rect.x0..rect.x1 {
                        let val = v[((y - rect.y0) * rect.width() + x - rect.x0) as usize];
                        let prev = *cells.entry((x / cw, y / ch)).or_insert(val);
                        assert_eq!(prev, val, "{mode} {snap:?}: cell ({}, {})", x / cw, y / ch);
                    }
                }
            }
        }
    }

    /// Mean of the evaluated mask over each target pixel of super row band `ty`.
    fn target_row(c: &Compositor, g: SuperGrid, ty: u32) -> Vec<f32> {
        let (rect, v) = c.evaluated_mask(0).unwrap();
        let n = u32::from(g.n());
        let (tw, _) = g.target_size();
        (0..tw)
            .map(|tx| {
                let mut sum = 0.0;
                for sy in ty * n..(ty + 1) * n {
                    for sx in tx * n..(tx + 1) * n {
                        if rect.contains(sx, sy) {
                            sum += v[((sy - rect.y0) * rect.width() + sx - rect.x0) as usize];
                        }
                    }
                }
                sum / (n * n) as f32
            })
            .collect()
    }

    #[test]
    fn feather_width_is_in_target_pixels_whatever_the_source_resolution() {
        for mode in [Mode::HiresBitmap, Mode::MulticolorBitmap] {
            let g = grid(mode, 4);
            let m = Modifiers::feathered(4.0).unwrap();
            let small = masked(solid(320, 200, [1.0; 4]), half_mask(320, 200, 160), m);
            let mut big = masked(solid(2000, 1250, [1.0; 4]), half_mask(2000, 1250, 1000), m);
            big.set_transform(
                Transform::new(Vec2::ZERO, Vec2::new(0.16, 0.16), false, false).unwrap(),
            );
            let mut a = Compositor::new(g);
            a.composite(&[small]);
            let mut b = Compositor::new(g);
            b.composite(&[big]);
            let (ra, rb) = (target_row(&a, g, 100), target_row(&b, g, 100));
            let width = |r: &[f32]| r.iter().filter(|&&v| v > 0.1 && v < 0.9).count();
            assert!(width(&ra) > 4, "{mode}: the feather must actually fade");
            assert_eq!(width(&ra), width(&rb), "{mode}: {ra:?} vs {rb:?}");
            for (x, (p, q)) in ra.iter().zip(&rb).enumerate() {
                assert!((p - q).abs() < 0.02, "{mode} x {x}: {p} vs {q}");
            }
        }
    }

    fn hex(bytes: &[u8]) -> String {
        bytes
            .chunks(16)
            .map(|c| {
                c.iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// A feathered edge on the cell boundary at hi-res x = 40, as target-pixel mask values
    /// (quantised to 8 bits) across cells 3–6 of one row.
    #[test]
    fn feathered_edge_across_a_cell_boundary_snapshot() {
        let g = grid(Mode::HiresBitmap, 4);
        let l = masked(
            solid(320, 200, [1.0; 4]),
            half_mask(320, 200, 40),
            Modifiers::feathered(3.0).unwrap(),
        );
        let mut c = Compositor::new(g);
        c.composite(&[l]);
        let row = target_row(&c, g, 100);
        let bytes: Vec<u8> = row[24..56]
            .iter()
            .map(|v| (v * 255.0).round() as u8)
            .collect();
        insta::assert_snapshot!(hex(&bytes));
    }

    #[test]
    fn mask_changes_touch_only_the_reported_cells() {
        for (mode, snap, feather) in [
            (Mode::HiresBitmap, SnapToCells::Off, 0.0),
            (Mode::HiresBitmap, SnapToCells::Expand, 3.0),
            (Mode::MulticolorBitmap, SnapToCells::Nearest, 12.0),
        ] {
            let g = grid(mode, 2);
            let m = Modifiers::new(false, crate::layer::Levels::IDENTITY, snap, feather).unwrap();
            let mut l = masked(
                solid(200, 150, [0.8, 0.4, 0.2, 1.0]),
                vec![0.0; 200 * 150],
                m,
            );
            l.set_transform(
                Transform::new(Vec2::new(30.5, 20.25), Vec2::new(1.3, 0.9), false, false).unwrap(),
            );
            let mut c = Compositor::new(g);
            let before = c.composite(&[l.clone()]).downsample();
            let dirty = PxRect::new(100, 60, 130, 90);
            let mut vals = l.mask().unwrap().raster().values().to_vec();
            for y in dirty.y0..dirty.y1 {
                for x in dirty.x0..dirty.x1 {
                    vals[(y * 200 + x) as usize] = 1.0;
                }
            }
            l.mask_mut()
                .unwrap()
                .set_raster(MaskRaster::from_values(200, 150, vals).unwrap())
                .unwrap();
            let after = c.composite(&[l.clone()]).downsample();
            let reported = mask_change_cells(g, &l, dirty);
            let (tw, _) = g.target_size();
            let mut changed = CellSet::EMPTY;
            for (i, (p, q)) in before.iter().zip(&after).enumerate() {
                if p != q {
                    changed.insert(g.target_px_to_cell(i as u32 % tw, i as u32 / tw).unwrap());
                }
            }
            assert!(!changed.is_empty(), "{mode} {snap:?}");
            assert_eq!(
                changed.intersection(reported),
                changed,
                "{mode} {snap:?}: {changed:?} ⊄ {reported:?}"
            );
            assert!(
                reported.len() < 1000 / 2,
                "{mode} {snap:?}: {} cells",
                reported.len()
            );
        }
    }

    #[test]
    fn sources_combine_with_the_raster() {
        use crate::mask::{
            Combine, Gradient, GradientKind, Key, KeyChannel, MaskSource, SourceEntry,
        };
        let g = grid(Mode::HiresBitmap, 1);
        // Left half red, right half green; mask raster empty.
        let px: Vec<[f32; 4]> = (0..64 * 8)
            .map(|i| {
                if i % 64 < 32 {
                    [1.0, 0.0, 0.0, 1.0]
                } else {
                    [0.0, 1.0, 0.0, 1.0]
                }
            })
            .collect();
        let src = Arc::new(SourceImage::from_linear(64, 8, px).unwrap());
        let mut l = masked(Arc::clone(&src), vec![0.0; 64 * 8], Modifiers::default());
        let key = Key::new(KeyChannel::Colour, [0.0, 1.0, 0.0], 0.05, 0.0).unwrap();
        l.mask_mut().unwrap().push_source(SourceEntry {
            source: MaskSource::Key(key),
            combine: Combine::Max,
        });
        let mut c = Compositor::new(g);
        let canvas = c.composite(&[l.clone()]).clone();
        assert_eq!(canvas.pixel(10, 4)[3], 0.0, "red is not keyed");
        assert_eq!(canvas.pixel(50, 4)[3], 1.0, "green is keyed");
        // Multiply by a left-to-right gradient that is 0 from x = 48 on.
        let grad = Gradient::new(
            GradientKind::Linear,
            Vec2::new(32.0, 0.0),
            Vec2::new(48.0, 0.0),
            1.0,
        )
        .unwrap();
        l.mask_mut().unwrap().push_source(SourceEntry {
            source: MaskSource::Gradient(grad),
            combine: Combine::Multiply,
        });
        let canvas = c.composite(&[l]);
        assert_eq!(canvas.pixel(60, 4)[3], 0.0);
        assert!(canvas.pixel(40, 4)[3] > 0.4 && canvas.pixel(40, 4)[3] < 0.6);
    }

    #[test]
    fn feather_is_cached_until_modifiers_or_raster_change() {
        let g = grid(Mode::HiresBitmap, 2);
        let mut l = masked(
            solid(64, 64, [1.0; 4]),
            half_mask(64, 64, 32),
            Modifiers::feathered(2.0).unwrap(),
        );
        let mut c = Compositor::new(g);
        c.composite(&[l.clone()]);
        c.composite(&[l.clone()]);
        assert_eq!((c.resamples(), c.mask_evals()), (1, 1));
        l.mask_mut()
            .unwrap()
            .set_modifiers(Modifiers::feathered(4.0).unwrap());
        c.composite(&[l.clone()]);
        assert_eq!(
            (c.resamples(), c.mask_evals()),
            (1, 2),
            "a slider drag does not resample"
        );
        l.set_opacity(0.5).unwrap();
        c.composite(&[l.clone()]);
        assert_eq!((c.resamples(), c.mask_evals()), (1, 2));
        l.set_transform(Transform::translated(Vec2::new(0.5, 0.0)).unwrap());
        c.composite(&[l]);
        assert_eq!((c.resamples(), c.mask_evals()), (2, 3));
    }

    /// Bottom: full-screen Bayer 4×4 at 100 %. Top: full-screen Floyd–Steinberg at 80 %
    /// masked to the left of the hi-res cell boundary at x = 40, feathered σ = 3.
    fn two_layer_scene() -> Vec<Layer> {
        use crate::layer::DitherKind;
        let mut bottom = Layer::new("bottom", solid(320, 200, [0.2, 0.3, 0.4, 1.0]));
        bottom.set_recipe(crate::layer::Recipe::with_method(DitherKind::Bayer4, 1.0).unwrap());
        let mut top = masked(
            solid(320, 200, [0.9, 0.7, 0.1, 1.0]),
            half_mask(320, 200, 40),
            Modifiers::feathered(3.0).unwrap(),
        );
        top.set_recipe(crate::layer::Recipe::with_method(DitherKind::FloydSteinberg, 0.8).unwrap());
        vec![bottom, top]
    }

    /// The attribute map of the two-layer scene across cells 3–6 of target rows 99–100:
    /// per pixel the source (0 = bottom, 1 = top) and the strength quantised to 8 bits.
    #[test]
    fn attribute_map_across_a_feathered_cell_boundary_snapshot() {
        let g = grid(Mode::HiresBitmap, 4);
        let mut c = Compositor::new(g);
        c.composite(&two_layer_scene());
        let map = c.attribute_map();
        let mut bytes = Vec::new();
        for y in [99, 100] {
            for x in 24..56 {
                let a = map.get(x, y);
                let code = match a.source {
                    AttrSource::Layer(i) => i as u8,
                    AttrSource::Region(i) => 0x80 | i as u8,
                    AttrSource::None => 0xff,
                };
                bytes.extend([code, (a.strength * 255.0).round() as u8]);
            }
        }
        insta::assert_snapshot!(hex(&bytes));
    }

    #[test]
    fn strength_fades_across_the_feather_then_the_layer_below_takes_over() {
        let g = grid(Mode::MulticolorBitmap, 4);
        let mut c = Compositor::new(g);
        c.composite(&two_layer_scene());
        let map = c.attribute_map();
        // Edge at canvas x 40 = multicolor target x 20; σ 3 target px.
        assert_eq!(map.get(5, 50).source, AttrSource::Layer(1));
        assert!((map.get(5, 50).strength - 0.8).abs() < 1e-3);
        let mut prev = f32::INFINITY;
        let mut x = 10;
        while map.get(x, 50).source == AttrSource::Layer(1) {
            let s = map.get(x, 50).strength;
            assert!(
                s <= prev + 1e-6,
                "strength must fall across the feather at x {x}"
            );
            prev = s;
            x += 1;
        }
        assert!(
            prev < 0.05,
            "the fade reaches ~0 before the bottom layer wins: {prev}"
        );
        assert!((20..40).contains(&x), "switch at {x}");
        assert_eq!(map.get(x, 50).source, AttrSource::Layer(0));
        assert!((map.get(150, 50).strength - 1.0).abs() < 1e-6);
        assert_eq!(
            map.recipe_at(150, 50).unwrap().0.method().kind,
            crate::layer::DitherKind::Bayer4
        );
    }

    #[test]
    fn regions_override_only_under_their_mask() {
        use crate::layer::DitherKind;
        let g = grid(Mode::HiresBitmap, 2);
        let vals: Vec<f32> = (0..320 * 200)
            .map(|i| {
                let (x, y) = (i % 320, i / 320);
                if (100..140).contains(&x) && (50..90).contains(&y) {
                    1.0
                } else {
                    0.0
                }
            })
            .collect();
        let region = Region::new(
            "sky",
            MaskRaster::from_values(320, 200, vals).unwrap(),
            crate::layer::Recipe::with_method(DitherKind::None, 0.5).unwrap(),
        )
        .unwrap();
        let mut c = Compositor::new(g);
        let layers = two_layer_scene();
        let before = c.composite(&layers).clone();
        let after = c
            .composite_with(&layers, std::slice::from_ref(&region))
            .clone();
        assert_eq!(before, after, "regions change no colour");
        let map = c.attribute_map();
        assert_eq!(map.get(120, 70).source, AttrSource::Region(0));
        assert!((map.get(120, 70).strength - 0.5).abs() < 1e-6);
        assert_eq!(
            map.recipe_at(120, 70).unwrap().0.method().kind,
            DitherKind::None
        );
        for (x, y) in [(98, 70), (142, 70), (120, 48), (120, 92), (10, 10)] {
            assert!(
                matches!(map.get(x, y).source, AttrSource::Layer(_)),
                "({x}, {y})"
            );
        }
    }

    #[test]
    fn debug_render_marks_uncovered_pixels_black() {
        let g = grid(Mode::MulticolorBitmap, 2);
        let mut c = Compositor::new(g);
        c.composite(&[at(Layer::new("a", solid(40, 40, [1.0; 4])), 100.0, 100.0)]);
        let rgba = c.attribute_map().debug_rgba();
        assert_eq!(rgba.len(), 160 * 200);
        assert_eq!(rgba[0], [0, 0, 0, 255]);
        let inside = rgba[(120 * 160 + 60) as usize];
        assert_eq!(
            inside,
            debug_colour(crate::layer::DitherKind::FloydSteinberg, 1.0)
        );
    }

    /// A three-layer scene with a mask, adjustments and a non-Normal blend.
    fn edit_scene() -> Vec<Layer> {
        use crate::layer::DitherKind;
        let mut a = Layer::new("bg", solid(320, 200, [0.1, 0.2, 0.3, 1.0]));
        a.set_recipe(crate::layer::Recipe::with_method(DitherKind::Bayer8, 0.7).unwrap());
        let b = at(
            masked(
                solid(60, 40, [0.9, 0.5, 0.2, 0.8]),
                half_mask(60, 40, 30),
                Modifiers::feathered(1.5).unwrap(),
            ),
            40.25,
            30.5,
        );
        let mut c = at(
            Layer::new("c", solid(50, 50, [0.3, 0.9, 0.4, 1.0])),
            200.0,
            100.0,
        );
        c.set_blend(Blend::Screen);
        vec![a, b, c]
    }

    fn full(g: SuperGrid, layers: &[Layer], regions: &[Region]) -> (Canvas, TargetFrame) {
        let mut c = Compositor::new(g);
        let canvas = c.composite_with(layers, regions).clone();
        (canvas, c.frame().clone())
    }

    fn sky_region() -> Region {
        let vals: Vec<f32> = (0..320 * 200)
            .map(|i| {
                if (i % 320) < 64 && (i / 320) < 48 {
                    1.0
                } else {
                    0.0
                }
            })
            .collect();
        Region::new(
            "sky",
            MaskRaster::from_values(320, 200, vals).unwrap(),
            crate::layer::Recipe::with_method(crate::layer::DitherKind::None, 0.3).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn incremental_update_equals_a_full_recomposite() {
        for mode in [Mode::HiresBitmap, Mode::MulticolorBitmap] {
            let g = grid(mode, 2);
            let mut layers = edit_scene();
            let mut regions: Vec<Region> = Vec::new();
            let mut c = Compositor::new(g);
            assert_eq!(
                c.update(&layers, &regions),
                CellSet::all(),
                "{mode}: first update"
            );
            type Edit = fn(&mut Vec<Layer>, &mut Vec<Region>);
            let edits: [(&str, Edit); 9] = [
                ("nudge", |l, _| {
                    let t = l[1].transform().nudged(Vec2::new(0.25, -0.5)).unwrap();
                    l[1].set_transform(t);
                }),
                ("opacity", |l, _| l[2].set_opacity(0.4).unwrap()),
                ("blend", |l, _| l[2].set_blend(Blend::Overlay)),
                ("feather", |l, _| {
                    l[1].mask_mut()
                        .unwrap()
                        .set_modifiers(Modifiers::feathered(4.0).unwrap());
                }),
                ("hide", |l, _| l[2].set_visible(false)),
                ("recipe", |l, _| {
                    l[0].set_recipe(
                        crate::layer::Recipe::with_method(crate::layer::DitherKind::Atkinson, 0.2)
                            .unwrap(),
                    );
                }),
                ("region", |_, r| r.push(sky_region())),
                ("show", |l, _| l[2].set_visible(true)),
                ("remove region", |_, r| r.clear()),
            ];
            for (name, edit) in edits {
                let (_, before) = full(g, &layers, &regions);
                edit(&mut layers, &mut regions);
                let reported = c.update(&layers, &regions);
                let (canvas, frame) = full(g, &layers, &regions);
                assert!(
                    c.canvas() == &canvas,
                    "{mode} {name}: canvas differs from a full recomposite"
                );
                assert!(
                    c.frame() == &frame,
                    "{mode} {name}: frame differs from a full recomposite"
                );
                // Exactly the cells whose output changed.
                let (tw, th) = g.target_size();
                let changed: CellSet = before
                    .differing(&frame, PxRect::of_size(tw, th))
                    .into_iter()
                    .filter_map(|(x, y)| g.target_px_to_cell(x, y))
                    .collect();
                assert_eq!(reported, changed, "{mode} {name}");
                assert!(
                    !reported.is_empty(),
                    "{mode} {name}: the edit changed nothing?"
                );
            }
            assert!(
                c.update(&layers, &regions).is_empty(),
                "{mode}: no edit, no cells"
            );
        }
    }

    #[test]
    fn a_nudge_stays_inside_the_old_and_new_footprints() {
        let g = grid(Mode::MulticolorBitmap, 4);
        let mut layers = edit_scene();
        let mut c = Compositor::new(g);
        c.update(&layers, &[]);
        let old = c.last.as_ref().unwrap().0[1].as_ref().unwrap().raster.rect;
        let t = layers[1].transform().nudged(Vec2::new(0.25, 0.0)).unwrap();
        layers[1].set_transform(t);
        let cells = c.update(&layers, &[]);
        let new = c.last.as_ref().unwrap().0[1].as_ref().unwrap().raster.rect;
        let footprint = CellSet::from_super_rect(g, old.union(new));
        assert!(!cells.is_empty());
        assert_eq!(cells.intersection(footprint), cells);
        assert!(cells.len() < footprint.len() + 1);
    }

    /// #99: opacity scales the attribute map's strength, not which layer governs.
    #[test]
    fn opacity_scales_dither_strength() {
        use crate::layer::{DitherKind, Recipe};
        let g = grid(Mode::HiresBitmap, 2);
        let mut bottom = Layer::new("flat", solid(320, 200, [0.4, 0.4, 0.4, 1.0]));
        bottom.set_recipe(Recipe::with_method(DitherKind::Bayer4, 1.0).unwrap());
        let mut fog = Layer::new("fog", solid(320, 200, [0.9, 0.9, 0.9, 1.0]));
        fog.set_recipe(Recipe::with_method(DitherKind::FloydSteinberg, 0.8).unwrap());
        fog.set_opacity(0.5).unwrap();
        let mut c = Compositor::new(g);
        c.composite(&[bottom.clone(), fog.clone()]);
        let a = c.attribute_map().get(100, 100);
        assert_eq!(a.source, AttrSource::Layer(1));
        assert!(
            (a.strength - 0.4).abs() < 1e-6,
            "0.5 opacity × 0.8 strength: {}",
            a.strength
        );
        fog.set_opacity(0.0).unwrap();
        c.composite(&[bottom, fog]);
        let a = c.attribute_map().get(100, 100);
        assert_eq!(
            a.source,
            AttrSource::Layer(0),
            "an invisible layer governs nothing"
        );
        assert!((a.strength - 1.0).abs() < 1e-6);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(48))]
        /// Normal blend at opacity 1 is order-independent for layers that do not overlap.
        #[test]
        fn normal_blend_of_disjoint_layers_is_order_independent(
            specs in prop::collection::vec(
                (1u32..12, 1u32..12, 0.0f32..1.0, 0.0f32..1.0, 0.0f32..=1.0, 0u8..4),
                2..6,
            ),
            perm_seed in any::<u64>(),
            n in 1u8..=4,
            multicolor in any::<bool>(),
        ) {
            let mode = if multicolor { Mode::MulticolorBitmap } else { Mode::HiresBitmap };
            // Lay the layers out left to right in 40-px columns with fractional offsets,
            // so their boxes are at least 25 canvas px apart.
            let layers: Vec<Layer> = specs
                .iter()
                .enumerate()
                .map(|(i, &(w, h, fx, fy, alpha, q))| {
                    let l = Layer::new(format!("{i}"), solid(w, h, [0.2 * f32::from(q), 0.5, 0.9, alpha]));
                    at(l, 40.0 * i as f32 + fx, 30.0 + fy)
                })
                .collect();
            let mut shuffled = layers.clone();
            let mut s = perm_seed;
            for i in (1..shuffled.len()).rev() {
                s = s.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
                shuffled.swap(i, (s >> 33) as usize % (i + 1));
            }
            let g = grid(mode, n);
            let a = Compositor::new(g).composite(&layers).clone();
            let b = Compositor::new(g).composite(&shuffled).clone();
            prop_assert_eq!(a, b);
        }
    }
}
