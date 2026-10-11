//! The supersampled compositor (`docs/TAD.md` §2, ADR-002).
//!
//! Every visible layer is resampled onto the [`SuperGrid`] (`n ×` the target grid, the
//! 2:1 multicolor aspect included) and blended bottom to top into an f32 linear
//! **premultiplied** RGBA [`Canvas`]. Effective alpha is PNG alpha × mask × opacity.
//!
//! Each layer's resampled raster is cached and rebuilt only when its source,
//! adjustments, transform, mask or the grid change; a recomposite with unchanged layers
//! only re-blends.

mod attr;
mod blend;
mod frame;
mod resample;

pub use attr::{AttrSource, AttributeMap, TargetAttr, debug_colour};
pub use frame::TargetFrame;

use crate::geom::{CellSet, PxRect, SuperGrid, Vec2};
use crate::layer::{AdjustCache, Adjustments, Blend, Layer, SourceImage, Transform};
use crate::mask::{HALF_RES_FEATHER, Modifiers, SnapToCells};
use crate::region::Region;
use attr::AttrPx;
use resample::{AxisMap, resample};
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
    rgba: Vec<[f32; 4]>,
    /// The resampled layer-space mask (raster ⊕ sources, before modifiers), `rect`-sized,
    /// or `None` for an unmasked layer.
    mask: Option<Vec<f32>>,
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
}

#[derive(Debug, Default)]
struct LayerCache {
    adjust: AdjustCache,
    raster: Option<(RasterKey, Arc<LayerRaster>)>,
    /// The feather cache: rebuilt only when the modifiers or the raster change.
    mask: Option<EvaluatedMask>,
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
    /// The super-grid attribute map of the last composite.
    attrs: Vec<AttrPx>,
    /// The last composite on the target grid.
    frame: TargetFrame,
    /// What was blended last time, per layer index (`None`: hidden), and the regions.
    last: Option<(Vec<Option<Blendable>>, Vec<Region>)>,
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
            attrs: vec![AttrPx::EMPTY; grid_len(grid)],
            frame: TargetFrame::empty(grid),
            last: None,
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
                ..Self::new(grid)
            };
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
        self.recomposite(layers, regions, state, self.grid.bounds());
        &self.canvas
    }

    /// Brings the composite up to date with `layers` and `regions`, recompositing only
    /// what changed since the last call, and returns the target cells whose output
    /// (colour, coverage or attributes) changed. The first call composites everything.
    pub fn update(&mut self, layers: &[Layer], regions: &[Region]) -> CellSet {
        let state = self.prepare(layers);
        let grid = self.grid;
        let area = match &self.last {
            None => grid.bounds(),
            Some((old, old_regions)) => {
                let mut r = PxRect::EMPTY;
                for i in 0..old.len().max(state.len()) {
                    let (a, b) = (
                        old.get(i).and_then(Option::as_ref),
                        state.get(i).and_then(Option::as_ref),
                    );
                    let same = match (a, b) {
                        (None, None) => true,
                        (Some(a), Some(b)) => a.same(b),
                        _ => false,
                    };
                    if !same {
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
                r
            }
        };
        if area.is_empty() {
            self.last = Some((state, regions.to_vec()));
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
        let before = self.frame.clone();
        self.recomposite(layers, regions, state, area);
        let n = u32::from(grid.n());
        let target = PxRect::new(area.x0 / n, area.y0 / n, area.x1 / n, area.y1 / n);
        self.frame
            .differing(&before, target)
            .into_iter()
            .filter_map(|(x, y)| grid.target_px_to_cell(x, y))
            .collect()
    }

    /// Folds `state` into the canvas over `area` and re-filters that part of the frame.
    fn recomposite(
        &mut self,
        layers: &[Layer],
        regions: &[Region],
        state: Vec<Option<Blendable>>,
        area: PxRect,
    ) {
        let stack: Vec<Blendable> = state.iter().flatten().cloned().collect();
        self.frame.attributes_mut().set_recipes(
            layers.iter().map(|l| *l.recipe()).collect(),
            regions.iter().map(|r| *r.recipe()).collect(),
        );
        fold(
            &mut self.canvas,
            &mut self.attrs,
            &mut self.frame,
            &stack,
            regions,
            area,
        );
        self.last = Some((state, regions.to_vec()));
    }

    /// Brings every visible layer's raster and evaluated mask up to date.
    fn prepare(&mut self, layers: &[Layer]) -> Vec<Option<Blendable>> {
        assert!(layers.len() < 0x8000, "at most 32767 layers");
        self.caches.resize_with(layers.len(), LayerCache::default);
        let grid = self.grid;
        let mut stack = Vec::with_capacity(layers.len());
        for (index, (layer, cache)) in layers.iter().zip(&mut self.caches).enumerate() {
            if !layer.visible() || layer.opacity() <= 0.0 {
                stack.push(None);
                continue;
            }
            let fresh = match &cache.raster {
                Some((key, _)) => !key.matches(layer, grid),
                None => true,
            };
            if fresh {
                let raster = Arc::new(build_raster(layer, &mut cache.adjust, grid));
                cache.raster = Some((RasterKey::of(layer, grid), raster));
                self.resamples += 1;
            }
            let (_, raster) = cache.raster.as_ref().expect("filled above");
            let mask = match (layer.mask(), &raster.mask) {
                (Some(m), Some(raw)) => {
                    let modifiers = m.modifiers();
                    let stale = cache
                        .mask
                        .as_ref()
                        .is_none_or(|e| e.modifiers != modifiers || !Arc::ptr_eq(&e.from, raster));
                    if stale {
                        let mut values = raw.clone();
                        crate::mask::eval::apply_modifiers(
                            &mut values,
                            raster.rect,
                            grid,
                            &modifiers,
                        );
                        cache.mask = Some(EvaluatedMask {
                            modifiers,
                            from: Arc::clone(raster),
                            values: Arc::new(values),
                        });
                        self.mask_evals += 1;
                    }
                    cache.mask.as_ref().map(|e| Arc::clone(&e.values))
                }
                _ => {
                    cache.mask = None;
                    None
                }
            };
            stack.push(Some(Blendable {
                code: u16::try_from(index).expect("checked above"),
                strength: layer.recipe().strength(),
                blend: layer.blend(),
                opacity: layer.opacity(),
                raster: Arc::clone(raster),
                mask,
            }));
        }
        stack
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

/// Resamples one layer (adjusted, premultiplied) onto the grid.
fn build_raster(layer: &Layer, adjust: &mut AdjustCache, grid: SuperGrid) -> LayerRaster {
    let src = layer.source();
    let (sw, sh) = src.size();
    let t = layer.transform();
    let step = grid.super_px_in_canvas();
    let (gw, gh) = grid.size();
    let mx = AxisMap::new(sw, t.translate().x, t.scale().x, t.flip_h(), step.x, gw);
    let my = AxisMap::new(sh, t.translate().y, t.scale().y, t.flip_v(), step.y, gh);
    let rect = PxRect::new(mx.out0, my.out0, mx.out0 + mx.len(), my.out0 + my.len());
    if rect.is_empty() {
        return LayerRaster {
            rect: PxRect::EMPTY,
            rgba: Vec::new(),
            mask: None,
        };
    }
    let adjusted = adjust.get(src, layer.adjustments());
    let premul: Vec<[f32; 4]> = adjusted
        .iter()
        .map(|p| [p[0] * p[3], p[1] * p[3], p[2] * p[3], p[3]])
        .collect();
    let mut rgba = resample(&premul, sw, &mx, &my);
    let w = mx.len() as usize;
    let mut rows: Vec<(usize, &mut [[f32; 4]])> = rgba.chunks_mut(w).enumerate().collect();
    crate::par::for_each(&mut rows, |(y, row)| {
        let cy = my.cov[*y];
        for (p, cx) in row.iter_mut().zip(&mx.cov) {
            let k = cx * cy;
            // Lanczos can ring slightly negative; alpha stays in 0..=1 and colour ≥ 0.
            p[3] = (p[3] * k).clamp(0.0, 1.0);
            for c in &mut p[..3] {
                *c = (*c * k).max(0.0);
            }
        }
    });
    let mask = layer.mask().map(|m| {
        let mut v = resample(&m.layer_values(src), sw, &mx, &my);
        for x in &mut v {
            *x = x.clamp(0.0, 1.0);
        }
        v
    });
    debug_assert_eq!(sh as usize * sw as usize, premul.len());
    LayerRaster { rect, rgba, mask }
}

/// A layer's part of one super-grid row: the canvas columns, its pixels and its mask.
type RowSlices<'a> = (std::ops::Range<usize>, &'a [[f32; 4]], Option<&'a [f32]>);

/// Where layer `b` meets row `y` of `area`, or `None`.
fn row_slices(b: &Blendable, area: PxRect, y: u32) -> Option<RowSlices<'_>> {
    let r = &b.raster;
    let span = r.rect.intersect(PxRect::new(area.x0, y, area.x1, y + 1));
    if span.is_empty() {
        return None;
    }
    let i0 = (y - r.rect.y0) as usize * r.rect.width() as usize + (span.x0 - r.rect.x0) as usize;
    let n = span.width() as usize;
    Some((
        span.x0 as usize..span.x1 as usize,
        &r.rgba[i0..i0 + n],
        b.mask.as_ref().map(|m| &m[i0..i0 + n]),
    ))
}

/// Blends `stack` (bottom first) into `canvas` over `area` (aligned to whole target
/// pixels), clearing it first, rebuilds the attribute map there, and box-filters the
/// result into `frame`. Works one target row at a time, so the `n` super rows it filters
/// are still in cache.
///
/// Colour blends bottom-up. Attributes are claimed top-down: the top-most layer whose
/// effective alpha (PNG alpha × mask) is above zero wins, strength × that alpha, and a
/// row stops once every pixel is claimed; then each region in order wins where its
/// mask is above zero, strength × mask.
fn fold(
    canvas: &mut Canvas,
    attrs: &mut [AttrPx],
    frame: &mut TargetFrame,
    stack: &[Blendable],
    regions: &[Region],
    area: PxRect,
) {
    let grid = canvas.grid;
    let (w, _) = grid.size();
    let (tw, _) = grid.target_size();
    let n = u32::from(grid.n());
    debug_assert!(
        [area.x0, area.y0, area.x1, area.y1]
            .iter()
            .all(|v| v.is_multiple_of(n)),
        "area must cover whole target pixels: {area:?}"
    );
    let (w, tw, band) = (w as usize, tw as usize, n as usize * w as usize);
    let (colour, coverage, target_attrs) = frame.parts_mut();
    let mut bands: Vec<FoldBand<'_>> = canvas
        .px
        .chunks_mut(band)
        .zip(attrs.chunks_mut(band))
        .zip(colour.chunks_mut(tw).zip(coverage.chunks_mut(tw)))
        .zip(target_attrs.chunks_mut(tw))
        .enumerate()
        .filter(|(ty, _)| ((area.y0 / n) as usize..(area.y1 / n) as usize).contains(ty))
        .map(|(ty, (((px, at), (c, cov)), ta))| (ty, px, at, c, cov, ta))
        .collect();
    crate::par::for_each(&mut bands, |(ty, px, at, colour, coverage, target_attr)| {
        let (x0, x1) = (area.x0 as usize, area.x1 as usize);
        for sy in 0..n as usize {
            let y = (*ty * n as usize + sy) as u32;
            let row = &mut px[sy * w..(sy + 1) * w];
            let attr = &mut at[sy * w..(sy + 1) * w];
            row[x0..x1].fill([0.0; 4]);
            attr[x0..x1].fill(AttrPx::EMPTY);
            for b in stack {
                if let Some((xs, src, mask)) = row_slices(b, area, y) {
                    blend::span(b.blend, &mut row[xs], src, mask, b.opacity);
                }
            }
            let mut unclaimed = x1 - x0;
            for b in stack.iter().rev() {
                if unclaimed == 0 {
                    break;
                }
                if let Some((xs, src, mask)) = row_slices(b, area, y) {
                    unclaimed -= blend::claim(&mut attr[xs], src, mask, b.code, b.strength);
                }
            }
            for (j, region) in regions.iter().enumerate() {
                let code = attr::AttrSource::Region(u16::try_from(j).expect("few regions")).code();
                let strength = region.recipe().strength();
                for x in area.x0..area.x1 {
                    let m = region.sample(grid.super_px_centre_in_canvas(x, y));
                    if m > 0.0 {
                        attr[x as usize] = AttrPx {
                            code,
                            strength: strength * m,
                        };
                    }
                }
            }
        }
        let mut seen = Vec::with_capacity(4);
        for tx in x0 / n as usize..x1 / n as usize {
            (colour[tx], coverage[tx]) = frame::reduce_px(px, w, n, tx);
            target_attr[tx] = attr::reduce_px(at, w, n, tx, &mut seen);
        }
    });
}

/// One target row's work: its index, its `n` canvas and attribute super rows, and its
/// colour, coverage and attribute target rows.
type FoldBand<'a> = (
    usize,
    &'a mut [[f32; 4]],
    &'a mut [AttrPx],
    &'a mut [[f32; 3]],
    &'a mut [f32],
    &'a mut [TargetAttr],
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
        if m.feather() > HALF_RES_FEATHER {
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
