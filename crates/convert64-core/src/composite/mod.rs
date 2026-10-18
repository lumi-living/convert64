//! The compositor (`docs/TAD.md` §2, ADR-002, ADR-012): every layer resampled with
//! png2c64's Mitchell bicubic onto the super grid, 1× the target by default.
//!
//! Every visible layer is resampled onto the [`SuperGrid`] (`n ×` the target grid, the
//! 2:1 multicolor aspect included) and blended bottom to top into an f32 linear
//! **premultiplied** RGBA [`Canvas`]. Effective alpha is PNG alpha × mask × opacity.
//! An axis-aligned layer is resampled separably (`resample`); a rotated one is
//! inverse-mapped through its affine transform (`affine`, #198), its rectangle the
//! bounding box of the rotated quad.
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
//!
//! The single-threaded cold path (#139), again without changing the output:
//! - The vertical pass splits each resampled colour row into the planes as it writes it,
//!   with the coverage and the clamps in the same loop; where both coverages are 1 the
//!   scale is skipped (`v × 1 = v`). Bilinear outputs take their two taps from a table.
//! - A feather blurred at half resolution with no snap reduces the resampled mask rows
//!   to half resolution as they are made, so the full-resolution mask is not stored
//!   before the feather writes it.
//! - The box blur keeps its running sums in registers, and a group of rows is transposed
//!   in 4 × 4 tiles. Gradients and mask extents are computed a row at a time in loops
//!   that vectorise, and identity adjustments premultiply the source without copying it.

mod affine;
mod attr;
mod blend;
mod frame;
pub(crate) mod mip;
mod resample;

pub use attr::{AttrSource, AttributeMap, TargetAttr, debug_colour};
pub use frame::TargetFrame;

use crate::geom::{CellSet, PxRect, SuperGrid, Vec2};
use crate::layer::{Adjustments, Blend, Downscale, Layer, Recipe, SourceImage, Transform};
use crate::mask::{Mask, Modifiers, SnapToCells};
use crate::region::Region;
use crate::scratch;
use affine::AffineMap;
use attr::AttrPlanes;
use blend::RgbaRow;
use mip::{MipCache, MipLevel};
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
///
/// Tests 16 values at a time without an early exit, so the test vectorises; a masked
/// layer's rows can start and end with long zero runs (#139).
fn extent_of<T>(row: &[T], on: impl Fn(&T) -> bool) -> (u32, u32) {
    const C: usize = 16;
    let (chunks, tail) = row.as_chunks::<C>();
    let any = |c: &[T; C]| c.iter().fold(false, |a, x| a | on(x));
    let in_chunk = |i: usize| i * C;
    let first = chunks
        .iter()
        .position(any)
        .map(|i| in_chunk(i) + chunks[i].iter().position(&on).expect("found above"))
        .or_else(|| {
            tail.iter()
                .position(&on)
                .map(|p| in_chunk(chunks.len()) + p)
        });
    let last = tail
        .iter()
        .rposition(&on)
        .map(|p| in_chunk(chunks.len()) + p)
        .or_else(|| {
            let i = chunks.iter().rposition(any)?;
            Some(in_chunk(i) + chunks[i].iter().rposition(&on).expect("found above"))
        });
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
    downscale: Downscale,
    mask_generation: Option<u64>,
    grid: SuperGrid,
}

impl RasterKey {
    fn of(layer: &Layer, grid: SuperGrid) -> Self {
        Self {
            source: Arc::clone(layer.source()),
            adjustments: layer.adjustments().clone(),
            transform: layer.transform(),
            downscale: layer.downscale(),
            mask_generation: layer.mask().map(crate::mask::Mask::generation),
            grid,
        }
    }

    fn matches(&self, layer: &Layer, grid: SuperGrid) -> bool {
        Arc::ptr_eq(&self.source, layer.source())
            && self.adjustments == *layer.adjustments()
            && self.transform == layer.transform()
            && self.downscale == layer.downscale()
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

/// A source's pixels premultiplied, with the source they came from.
type Premultiplied = (Arc<SourceImage>, Vec<[f32; 4]>);

/// The mip levels of a mask's layer-space values (#235), with what they were made from:
/// they change only with the mask's content or the source, never with the transform.
#[derive(Debug)]
struct MaskMips {
    generation: u64,
    source: Arc<SourceImage>,
    cache: MipCache<f32>,
}

/// Where a layer lands on the super grid: two axis maps for an axis-aligned layer (the
/// separable fast path), or the inverse affine map of a rotated one (#198).
enum Placement {
    Axis(AxisMap, AxisMap),
    Affine(AffineMap),
}

impl Placement {
    /// The mip level it samples along each layer axis; `(0, 0)` is the source itself.
    fn level(&self) -> (u32, u32) {
        match self {
            Self::Axis(mx, my) => (mx.level, my.level),
            Self::Affine(map) => map.level,
        }
    }
}

/// Where a layer's mask is resampled: its placement, and for a grid coarser than
/// [`MASK_N`] the rectangle it touches on the finer grid and how much finer that is.
struct MaskPlace {
    place: Placement,
    fine: Option<(PxRect, u32)>,
}

/// What a layer's refresh did, for the compositor's counters.
#[derive(Clone, Copy, Debug, Default)]
struct Refreshed {
    resampled: bool,
    evaluated: bool,
    /// Mip reductions made (#235), colour and mask.
    mip_levels: u32,
    /// Milliseconds they took, by the compositor's clock (0 without one).
    mip_ms: f64,
}

/// A monotonic clock in milliseconds, supplied by the front end: the core has no
/// platform clock (CLAUDE.md non-negotiable 2).
pub type Clock = fn() -> f64;

/// Runs `f`, which builds `n` mip reductions, and adds them and their time to `stats`.
fn timed<T>(stats: &mut Refreshed, clock: Option<Clock>, f: impl FnOnce() -> (T, u32)) -> T {
    let t0 = clock.map(|c| c());
    let (v, n) = f();
    if n > 0 {
        stats.mip_levels += n;
        if let (Some(c), Some(t0)) = (clock, t0) {
            stats.mip_ms += c() - t0;
        }
    }
    v
}

#[derive(Debug, Default)]
struct LayerCache {
    /// Whether every source pixel is opaque (its resampled alpha is then its coverage,
    /// exactly), keyed by the source's identity.
    opaque: Option<(Arc<SourceImage>, bool)>,
    /// The source pixels premultiplied, keyed by the source's identity (adjustments
    /// apply after resampling, #190); made only when the layer samples the source itself,
    /// not one of its mip levels (#235), which the source keeps.
    premul: Option<Premultiplied>,
    /// The mask's layer-space values when it has sources.
    values: Option<LayerValues>,
    /// The mip levels of the mask's layer-space values (#235).
    mask_mips: Option<MaskMips>,
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
    /// it resampled the layer, whether it evaluated the mask, and the mip levels it built.
    fn refresh(
        &mut self,
        layer: &Layer,
        grid: SuperGrid,
        full_res: bool,
        clock: Option<Clock>,
    ) -> Refreshed {
        let mut stats = Refreshed::default();
        let fresh = match &self.raster {
            Some((key, _)) => !key.matches(layer, grid),
            None => true,
        };
        if fresh {
            let (raster, values) = self.build_raster(layer, grid, full_res, clock, &mut stats);
            let raster = Arc::new(raster);
            let key = RasterKey::of(layer, grid);
            if let Some((_, old)) = self.raster.replace((key, Arc::clone(&raster))) {
                self.retired.0 = Some(old);
            }
            stats.resampled = true;
            match (layer.mask(), values) {
                (Some(m), Some(values)) => {
                    self.set_mask(m.modifiers(), raster, values);
                    stats.evaluated = true;
                }
                _ => self.mask = None,
            }
            return stats;
        }
        let (_, raster) = self.raster.as_ref().expect("not fresh");
        let raster = Arc::clone(raster);
        let Some(m) = layer.mask().filter(|_| !raster.rect.is_empty()) else {
            self.mask = None;
            return stats;
        };
        let modifiers = m.modifiers();
        let stale = self
            .mask
            .as_ref()
            .is_none_or(|e| e.modifiers != modifiers || !Arc::ptr_eq(&e.from, &raster));
        if !stale {
            return stats;
        }
        // A modifier changed: resample the mask again (the raster's colour stays) and
        // evaluate it.
        let mp = mask_placement(layer, grid);
        let mut values = std::mem::take(&mut self.spare.values);
        self.update_mask_source(m, layer.source());
        let level = timed(&mut stats, clock, || {
            self.mask_level(m, layer.source(), mp.place.level())
        });
        let (src, size) = match &level {
            Some(l) => (&l.px[..], l.size),
            None => (self.mask_source(m), layer.source().size()),
        };
        evaluate_mask(
            src,
            size,
            &mp,
            raster.rect,
            grid,
            &modifiers,
            full_res,
            &mut values,
        );
        self.set_mask(modifiers, raster, values);
        stats.evaluated = true;
        stats
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

    /// Mip `level` of the mask's layer-space values (#235), after
    /// [`Self::update_mask_source`], and the reductions building it took; `None` at level
    /// `(0, 0)`, where the values themselves are sampled. The levels are kept until the
    /// mask's content or the source changes.
    fn mask_level(
        &mut self,
        m: &Mask,
        src: &Arc<SourceImage>,
        level: (u32, u32),
    ) -> (Option<Arc<MipLevel<f32>>>, u32) {
        if level == (0, 0) {
            return (None, 0);
        }
        let fresh = matches!(&self.mask_mips, Some(x)
            if x.generation == m.generation() && Arc::ptr_eq(&x.source, src));
        if !fresh {
            self.mask_mips = Some(MaskMips {
                generation: m.generation(),
                source: Arc::clone(src),
                cache: MipCache::default(),
            });
        }
        let values = self.mask_source(m);
        let cache = &self.mask_mips.as_ref().expect("set above").cache;
        let (l, n) = cache.level(src.size(), |i| values[i], level);
        (Some(l), n)
    }

    /// Resamples the layer (adjusted, premultiplied) onto the grid and, alongside, its
    /// mask, which it evaluates (modifiers applied) when the layer has one.
    fn build_raster(
        &mut self,
        layer: &Layer,
        grid: SuperGrid,
        full_res: bool,
        clock: Option<Clock>,
        stats: &mut Refreshed,
    ) -> (LayerRaster, Option<Vec<f32>>) {
        let src = layer.source();
        let (sw, sh) = src.size();
        let (place, rect) = placement(layer, grid, colour_levels(layer, grid));
        if rect.is_empty() {
            let raster = LayerRaster {
                rect: PxRect::EMPTY,
                rgba: Planes::default(),
                extent: Arc::default(),
            };
            return (raster, None);
        }
        let adj = layer.adjustments();
        if !matches!(&self.opaque, Some((s, _)) if Arc::ptr_eq(s, src)) {
            let opaque = src.pixels().iter().all(|p| p[3] == 1.0);
            self.opaque = Some((Arc::clone(src), opaque));
        }
        let opaque = self.opaque.as_ref().expect("filled above").1;
        // Shrunk far enough, the colour samples a mip level the source keeps (#235);
        // else the source itself, premultiplied here. Adjustments apply after
        // resampling, in png2c64's order (#190).
        let level = place.level();
        let colour_level =
            (level != (0, 0)).then(|| timed(stats, clock, || src.colour_level(level)));
        if colour_level.is_none() && !matches!(&self.premul, Some((s, _)) if Arc::ptr_eq(s, src)) {
            let premul = src
                .pixels()
                .iter()
                .map(|p| [p[0] * p[3], p[1] * p[3], p[2] * p[3], p[3]])
                .collect();
            self.premul = Some((Arc::clone(src), premul));
        }
        let mut rgba = std::mem::take(&mut self.spare.rgba);
        let mut values = std::mem::take(&mut self.spare.values);
        let mask = layer.mask();
        let mp = mask.map(|_| mask_placement(layer, grid));
        let mask_level = match (mask, &mp) {
            (Some(m), Some(mp)) => {
                self.update_mask_source(m, src);
                timed(stats, clock, || self.mask_level(m, src, mp.place.level()))
            }
            _ => None,
        };
        let mask_src = mask.map(|m| match &mask_level {
            Some(l) => (&l.px[..], l.size),
            None => (self.mask_source(m), (sw, sh)),
        });
        let colour: (&[[f32; 4]], (u32, u32)) = match &colour_level {
            Some(l) => (&l.px, l.size),
            None => (&self.premul.as_ref().expect("filled above").1, (sw, sh)),
        };
        debug_assert_eq!(colour.1.0 as usize * colour.1.1 as usize, colour.0.len());
        // The colour and the mask are independent: resample them side by side.
        let (extent, values) = crate::par::join(
            || {
                let mut extent = Vec::new();
                match &place {
                    Placement::Axis(mx, my) => {
                        resample_planes(
                            colour.0,
                            colour.1.0,
                            mx,
                            my,
                            &mut rgba,
                            &mut extent,
                            opaque,
                        );
                    }
                    Placement::Affine(map) => {
                        affine::resample_planes_affine(
                            colour.0,
                            colour.1,
                            map,
                            &mut rgba,
                            &mut extent,
                        );
                    }
                }
                extent
            },
            || {
                let (m, (mask_src, size)) = mask.zip(mask_src)?;
                let modifiers = m.modifiers();
                evaluate_mask(
                    mask_src,
                    size,
                    mp.as_ref().expect("placed with the mask"),
                    rect,
                    grid,
                    &modifiers,
                    full_res,
                    &mut values,
                );
                Some(values)
            },
        );
        adjust_planes(&mut rgba, rect.width() as usize, adj, grid.n());
        let raster = LayerRaster {
            rect,
            rgba,
            extent: Arc::new(extent),
        };
        (raster, values)
    }
}

/// A layer's front end on its resampled, premultiplied planes (`width` wide): png2c64's
/// preprocessing, then the layer's own adjustments, in png2c64's order (#190, ADR-012).
///
/// 1. png2c64's preprocessing with the layer's gamma and sharpen and its other settings
///    neutral: the gamma (an exact `powf`), the sharpen (#228; png2c64's 3 × 3 at the
///    target grid, its taps `n` super pixels apart on an `n ×` grid, or a Gaussian, the
///    blur weighted by alpha so a transparent surround pulls in no halo), then the OKLab
///    round trip and clamp png2c64 puts every pixel through.
/// 2. The rest of the layer's adjustments (levels, brightness and contrast, hue,
///    saturation and lightness, curves), which png2c64 does not have.
/// 3. Range matching, png2c64's `match_range_to` over the layer's covered pixels.
///
/// A layer fitted to the screen therefore leaves this step as png2c64's prepared image
/// (`png2c64_ref::prepare`), and every frame counts as prepared: the reference solver
/// does not repeat the round trip. Fully transparent pixels stay transparent, so the
/// planes' extents still hold; the sharpen stays inside the layer's rectangle, so the
/// cells an adjustment can change are the rectangle's.
fn adjust_planes(planes: &mut Planes, width: usize, adj: &Adjustments, n: u8) {
    use crate::solve::png2c64_ref::{Image, preprocess};
    let len = planes.a.len();
    if len == 0 {
        return;
    }
    let straight = |i: usize| {
        let a = planes.a[i];
        if a > 0.0 {
            [planes.r[i] / a, planes.g[i] / a, planes.b[i] / a]
        } else {
            [0.0; 3]
        }
    };
    let height = len / width;
    let mut im = Image::new(width, height, (0..len).map(straight).collect());
    // png2c64's order: gamma, sharpen, then the round trip (`preprocess::apply` with
    // gamma 1 skips the power and, with no sharpen, does the rest).
    preprocess::apply_gamma(&mut im, adj.gamma.get());
    adj.sharpen
        .apply_rgb(&mut im.px, &planes.a, (width, height), u32::from(n));
    preprocess::apply(
        &mut im,
        &preprocess::Settings {
            gamma: 1.0,
            ..preprocess::Settings::default()
        },
    );
    let rest = Adjustments {
        gamma: crate::layer::Gamma::IDENTITY,
        sharpen: crate::layer::Sharpen::default(),
        match_range: None,
        ..adj.clone()
    }
    .plan();
    if !rest.is_identity() {
        crate::par::for_each(&mut im.px, |p| {
            let mut q = [p[0], p[1], p[2], 1.0];
            rest.apply(&mut q);
            *p = [q[0], q[1], q[2]];
        });
    }
    if let Some(m) = &adj.match_range {
        // Over the covered pixels only: a transparent surround has no colour to match.
        let covered: Vec<usize> = (0..len).filter(|&i| planes.a[i] > 0.0).collect();
        let mut cov = Image::new(
            covered.len(),
            usize::from(!covered.is_empty()),
            covered.iter().map(|&i| im.px[i]).collect(),
        );
        preprocess::match_range_to(&mut cov, m.percentile(), m.targets());
        for (&i, p) in covered.iter().zip(&cov.px) {
            im.px[i] = *p;
        }
    }
    for (i, p) in im.px.iter().enumerate() {
        let a = planes.a[i];
        if a > 0.0 {
            planes.r[i] = (p[0] * a).max(0.0);
            planes.g[i] = (p[1] * a).max(0.0);
            planes.b[i] = (p[2] * a).max(0.0);
        }
    }
}

/// The one layer a plain project consists of, if it is plain (#190): a bitmap mode, no
/// regions, one visible layer that is fully opaque 8-bit sRGB, filling the screen
/// exactly (no offset, no flip, no rotation: #198), at full opacity, Normal blend, with no mask and no
/// adjustment but gamma, png2c64's sharpen (its kernel, no threshold: #228) and range
/// matching, and sampling the source itself (no mip level, #235). Such a project gets
/// png2c64's front end.
fn plain_layer<'a>(layers: &'a [Layer], regions: &[Region], grid: SuperGrid) -> Option<&'a Layer> {
    use crate::mode::Mode;
    if !matches!(grid.target(), Mode::MulticolorBitmap | Mode::HiresBitmap) || !regions.is_empty() {
        return None;
    }
    let mut visible = layers.iter().filter(|l| l.visible());
    let layer = visible.next()?;
    if visible.next().is_some() {
        return None;
    }
    let src = layer.source();
    let (w, h) = src.size();
    let t = layer.transform();
    let adj = layer.adjustments();
    let tone_only = Adjustments {
        gamma: crate::layer::Gamma::IDENTITY,
        sharpen: crate::layer::Sharpen::default(),
        match_range: None,
        ..adj.clone()
    };
    (src.srgb8().is_some()
        && layer.opacity() == 1.0
        && layer.blend() == Blend::Normal
        && layer.mask().is_none()
        && t.translate() == Vec2::ZERO
        && t.scale() == Vec2::new(320.0 / w as f32, 200.0 / h as f32)
        && t.rotate() == 0.0
        && !t.flip_h()
        && !t.flip_v()
        && (adj.sharpen.is_off() || adj.sharpen.is_png2c64())
        && tone_only.is_identity()
        // Shrunk past 4× under Auto it samples a mip level, which png2c64 has not (#235).
        && colour_levels(layer, grid) == (0, 0))
        .then_some(layer)
}

/// png2c64's front end for a plain layer: its 8-bit source scaled straight to the
/// target with its Mitchell bicubic, then its preprocessing with the layer's gamma and
/// sharpen, then range matching onto the layer's targets (`solve::png2c64_ref::prepare`).
fn png2c64_front_end(layer: &Layer, grid: SuperGrid) -> Vec<[f32; 3]> {
    use crate::solve::png2c64_ref::{prepare, preprocess, quantize::Mode as RefMode};
    let src = layer.source();
    let (w, h) = src.size();
    let mode = if grid.target() == crate::mode::Mode::HiresBitmap {
        RefMode::Hires
    } else {
        RefMode::Multicolor
    };
    let adj = layer.adjustments();
    let mut im = prepare(
        src.srgb8().expect("a plain layer has its bytes"),
        (w as usize, h as usize),
        mode,
        &preprocess::Settings {
            gamma: adj.gamma.get(),
            sharpen: adj.sharpen.amount(),
            ..preprocess::Settings::default()
        },
        None,
    );
    if let Some(m) = &adj.match_range {
        preprocess::match_range_to(&mut im, m.percentile(), m.targets());
    }
    im.px
}

/// Where `layer` lands on `grid` and the super rectangle it touches: the separable axis
/// maps when it is axis-aligned (`rotate == 0`, unchanged by #198), else the inverse
/// affine map, whose rectangle is the rotated quad's bounding box on the grid. Either
/// samples the source's mip `level` along each layer axis ((0, 0): the source itself,
/// #235); the rectangle and the coverage do not depend on it.
fn placement(layer: &Layer, grid: SuperGrid, level: (u32, u32)) -> (Placement, PxRect) {
    let t = layer.transform();
    if t.is_axis_aligned() {
        let (mx, my, rect) = axis_maps(layer, grid, level);
        (Placement::Axis(mx, my), rect)
    } else {
        let map = AffineMap::new(&t, layer.source().size(), grid, level);
        let rect = map.rect;
        (Placement::Affine(map), rect)
    }
}

/// A layer's per-axis resampling maps onto `grid` and the super rectangle it touches.
fn axis_maps(layer: &Layer, grid: SuperGrid, level: (u32, u32)) -> (AxisMap, AxisMap, PxRect) {
    let (sw, sh) = layer.source().size();
    let t = layer.transform();
    let step = grid.super_px_in_canvas();
    let (gw, gh) = grid.size();
    let mx = AxisMap::new(
        sw,
        t.translate().x,
        t.scale().x,
        t.flip_h(),
        step.x,
        gw,
        level.0,
    );
    let my = AxisMap::new(
        sh,
        t.translate().y,
        t.scale().y,
        t.flip_v(),
        step.y,
        gh,
        level.1,
    );
    let rect = PxRect::new(mx.out0, my.out0, mx.out0 + mx.len(), my.out0 + my.len());
    (mx, my, rect)
}

/// The mip level `layer` samples along each layer axis when one pixel of a grid spans
/// `step` canvas pixels: from the source pixels that pixel spans along the axis (the
/// shrink; `step / scale` unrotated), by [`mip::level_for`]. `(0, 0)` under
/// [`Downscale::Exact`].
fn levels_at(layer: &Layer, step: Vec2) -> (u32, u32) {
    if layer.downscale() == Downscale::Exact {
        return (0, 0);
    }
    let size = layer.source().size();
    let (fu, fv) = AffineMap::footprint(&layer.transform(), size, step);
    (mip::level_for(fu, size.0), mip::level_for(fv, size.1))
}

/// The mip level `layer`'s colour samples on `grid` (#235): the shrink is measured on
/// the hi-res target grid, 320 · n pixels across in either mode (a canvas pixel is a
/// hi-res pixel, and a supersampled grid is n times finer), so a source up to 1280 px
/// wide fitted to the screen samples the original.
fn colour_levels(layer: &Layer, grid: SuperGrid) -> (u32, u32) {
    let s = 1.0 / f32::from(grid.n());
    levels_at(layer, Vec2::new(s, s))
}

/// The supersample factor a mask's edge is resampled at, at least (ADR-012): measured
/// against 8×, a hard raster edge on the 1× grid is off by up to 0.3 in coverage, at 4×
/// by at most 0.014.
const MASK_N: u8 = 4;

/// The grid a layer's mask is resampled on for `grid`, and how many times finer it is:
/// `grid` itself from [`MASK_N`] up, else the grid `k` times finer.
fn mask_grid(grid: SuperGrid) -> (SuperGrid, Option<u32>) {
    let n = grid.n();
    if n >= MASK_N {
        return (grid, None);
    }
    let k = MASK_N.div_ceil(n);
    let fine = SuperGrid::new(grid.target(), n * k).expect("a factor of at most 8");
    (fine, Some(u32::from(k)))
}

/// Where a layer's mask is resampled (ADR-012 §5): on `grid` from [`MASK_N`] up, else
/// on the grid `k` times finer, with the rectangle it touches there. It samples the
/// mask's mip level for the shrink measured on that grid (#235): source pixels per mask
/// pixel, so a mask on the 4× grid switches to a level only past a 4× shrink there.
fn mask_placement(layer: &Layer, grid: SuperGrid) -> MaskPlace {
    let (mgrid, k) = mask_grid(grid);
    let level = levels_at(layer, mgrid.super_px_in_canvas());
    // Rotated, the fine grid is sampled bilinearly (`affine::resample_mask_affine`):
    // Mitchell's sixteen taps per fine pixel cost a rotated layer 60–130 ms more than
    // bilinear's four, and after the box reduction the edge is still within 0.014 of
    // an 8× composite (0.006 with Mitchell).
    let (place, rect) = placement(layer, mgrid, level);
    MaskPlace {
        place,
        fine: k.map(|k| (rect, k)),
    }
}

/// Box-reduces `fine` (a mask on `frect` of the grid `k` times finer) onto `rect`: each
/// pixel the mean of its `k × k` fine pixels that the layer reaches (`frect`), or 0.
fn reduce_mask(fine: &[f32], frect: PxRect, k: u32, rect: PxRect, out: &mut Vec<f32>) {
    let (w, h) = (rect.width() as usize, rect.height() as usize);
    resample::sized(out, w * h, 0.0);
    let fw = frect.width() as usize;
    let mut rows: Vec<_> = out.chunks_mut(w.max(1)).enumerate().collect();
    crate::par::for_each(&mut rows, |(j, row)| {
        let y = rect.y0 + *j as u32;
        let (fy0, fy1) = ((y * k).max(frect.y0), ((y + 1) * k).min(frect.y1));
        for (i, v) in row.iter_mut().enumerate() {
            let x = rect.x0 + i as u32;
            let (fx0, fx1) = ((x * k).max(frect.x0), ((x + 1) * k).min(frect.x1));
            if fx0 >= fx1 || fy0 >= fy1 {
                continue;
            }
            let mut sum = 0.0f32;
            for fy in fy0..fy1 {
                let r = (fy - frect.y0) as usize * fw;
                for fx in fx0..fx1 {
                    sum += fine[r + (fx - frect.x0) as usize];
                }
            }
            *v = sum / ((fx1 - fx0) * (fy1 - fy0)) as f32;
        }
    });
}

/// Resamples a mask's layer-space values (`size`: the source's, or the mip level's that
/// `mp` samples, #235) onto `rect` of `grid` through `mp` into `out` and applies
/// `modifiers`: a rotated layer's mask goes through the same affine map as its colour
/// (ADR-008), then its modifiers. With a finer placement ([`mask_placement`]) the mask
/// is resampled there, inverted and levelled, box-reduced to `grid`, then snapped and
/// feathered on `grid` (σ in target px either way). On the separable path, a
/// feather blurred at half resolution with no snap before it reduces the resampled rows
/// to half resolution as they are made ([`resample::resample_half`]), so the
/// full-resolution mask is never stored before the feather writes it (#139); the values
/// are the same either way.
#[expect(
    clippy::too_many_arguments,
    reason = "the resampling maps and the modifiers"
)]
fn evaluate_mask(
    values: &[f32],
    size: (u32, u32),
    mp: &MaskPlace,
    rect: PxRect,
    grid: SuperGrid,
    modifiers: &Modifiers,
    full_res: bool,
    out: &mut Vec<f32>,
) {
    use crate::mask::eval;
    let place = &mp.place;
    if let Some((frect, k)) = &mp.fine {
        let mut fv = Vec::new();
        match place {
            Placement::Axis(mx, my) => resample_mask(values, size.0, mx, my, &mut fv),
            Placement::Affine(map) => {
                affine::resample_mask_affine(values, size, map, true, &mut fv);
            }
        }
        eval::point_ops(&mut fv, modifiers);
        reduce_mask(&fv, *frect, *k, rect, out);
        eval::spatial_ops(out, rect, grid, modifiers, full_res);
        return;
    }
    let (mx, my) = match place {
        Placement::Axis(mx, my) => (mx, my),
        Placement::Affine(map) => {
            if let Some(sigma) = eval::half_res_sigma(modifiers, grid, full_res) {
                // Sampled straight at half resolution, then the same point operations,
                // blur and upsample as the separable path's.
                let (w, h) = (rect.width() as usize, rect.height() as usize);
                resample::sized(out, w * h, 0.0);
                scratch::with(w.div_ceil(2) * h.div_ceil(2), |half| {
                    affine::resample_mask_affine_half(values, size, map, half);
                    eval::point_ops(half, modifiers);
                    eval::blur_and_up(out, w, h, sigma, half);
                });
            } else {
                affine::resample_mask_affine(values, size, map, false, out);
                eval::apply_modifiers_with(out, rect, grid, modifiers, full_res);
            }
            return;
        }
    };
    let sw = size.0;
    let Some(sigma) = eval::half_res_sigma(modifiers, grid, full_res) else {
        resample_mask(values, sw, mx, my, out);
        eval::apply_modifiers_with(out, rect, grid, modifiers, full_res);
        return;
    };
    let (w, h) = (rect.width() as usize, rect.height() as usize);
    resample::sized(out, w * h, 0.0);
    scratch::with(w.div_ceil(2) * h.div_ceil(2), |half| {
        resample::resample_half(
            values,
            sw,
            mx,
            my,
            half,
            |row| {
                for x in &mut *row {
                    *x = x.clamp(0.0, 1.0);
                }
                eval::point_ops(row, modifiers);
            },
            eval::down_row,
        );
        eval::blur_and_up(out, w, h, sigma, half);
    });
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
    /// Mip reductions built for this compositor (#235), and the milliseconds they took.
    mip_builds: u64,
    mip_ms: f64,
    /// The clock that times them ([`Self::set_clock`]).
    clock: Option<Clock>,
    /// png2c64's prepared image of the last plain layer (#190), with what it was made
    /// from.
    front: Option<FrontEnd>,
    /// Whether a plain layer takes png2c64's front end ([`Self::set_png2c64_front_end`]).
    front_end: bool,
}

/// png2c64's front end for one plain layer: its source and adjustments, and the
/// prepared colour on the target grid.
#[derive(Debug)]
struct FrontEnd {
    source: Arc<SourceImage>,
    adjustments: Adjustments,
    colour: Vec<[f32; 3]>,
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
            mip_builds: 0,
            mip_ms: 0.0,
            clock: None,
            front: None,
            front_end: true,
        }
    }

    /// Times the mip levels this compositor builds with `clock` (milliseconds from any
    /// fixed origin), for [`Self::mip_build_ms`]. The core has no clock of its own; the
    /// app passes the browser's `performance.now()` in wasm.
    pub fn set_clock(&mut self, clock: Option<Clock>) {
        self.clock = clock;
    }

    /// How many mip reductions (#235) composites have built through this compositor:
    /// levels of a source's colour, which the source keeps for every compositor, and of
    /// a mask. A scale, rotation or nudge drag of a layer shrunk past 4× builds none
    /// after its first composite.
    pub const fn mip_builds(&self) -> u64 {
        self.mip_builds
    }

    /// The milliseconds [`Self::mip_builds`] took, by the clock of [`Self::set_clock`]
    /// (0 without one).
    pub const fn mip_build_ms(&self) -> f64 {
        self.mip_ms
    }

    /// Whether a plain project (one opaque 8-bit layer filling the screen, no mask, no
    /// regions, no adjustment but gamma, png2c64's sharpen and range matching) takes
    /// png2c64's front end
    /// (#190; on by default) or the supersampled composite like any other. The next
    /// update recomposites everything when this changes.
    pub fn set_png2c64_front_end(&mut self, on: bool) {
        if on != self.front_end {
            self.front_end = on;
            self.front = None;
            self.last = None;
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
                mip_builds: self.mip_builds,
                mip_ms: self.mip_ms,
                clock: self.clock,
                full_res_feather: self.full_res_feather,
                front_end: self.front_end,
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
        // Every layer went through png2c64's preprocessing (ADR-012).
        self.frame.set_prepared(None);
        &self.canvas
    }

    /// Brings the composite up to date with `layers` and `regions`, recompositing only
    /// what changed since the last call, and returns the target cells whose output
    /// (colour, coverage or attributes) changed. The first call composites everything.
    ///
    /// Only the layers from the lowest changed one up are re-blended: the ones below come
    /// from a cached fold of them, kept while they stay unchanged.
    pub fn update(&mut self, layers: &[Layer], regions: &[Region]) -> CellSet {
        let plain = plain_layer(layers, regions, self.grid).filter(|_| self.front_end);
        if plain.is_none() && self.front.take().is_some() {
            // The frame held png2c64's image: composite everything afresh.
            self.last = None;
        }
        let first = self.last.is_none();
        let old = self.frame.attributes().recipes();
        let mut cells = self.update_composite(layers, regions);
        // A recipe change that blends the same (the method, the metric: anything but
        // the strength) recomposites nothing, but the frame must carry the new recipes
        // and its pixels' cells are dirty for the solver (#260).
        let new = (
            layers.iter().map(|l| *l.recipe()).collect::<Vec<_>>(),
            regions.iter().map(|r| *r.recipe()).collect::<Vec<_>>(),
        );
        if !first && (old.0 != new.0 || old.1 != new.1) {
            let changed = |i: usize, old: &[Recipe], new: &[Recipe]| old.get(i) != new.get(i);
            let (w, _) = self.grid.target_size();
            let (cw, ch) = self.grid.cell_size();
            for (i, a) in self.frame.attributes().pixels().iter().enumerate() {
                let differs = match a.source {
                    AttrSource::None => false,
                    AttrSource::Layer(l) => changed(usize::from(l), &old.0, &new.0),
                    AttrSource::Region(r) => changed(usize::from(r), &old.1, &new.1),
                };
                if differs {
                    let (x, y) = ((i as u32) % w, (i as u32) / w);
                    if let Some(cell) = self.grid.target_px_to_cell(x / cw * cw, y / ch * ch) {
                        cells.insert(cell);
                    }
                }
            }
        }
        self.frame.attributes_mut().set_recipes(new.0, new.1);
        let Some(layer) = plain else {
            self.frame.set_prepared(None);
            return cells;
        };
        let src = layer.source();
        let adj = layer.adjustments();
        let fresh =
            !matches!(&self.front, Some(f) if Arc::ptr_eq(&f.source, src) && f.adjustments == *adj);
        if fresh {
            let colour = png2c64_front_end(layer, self.grid);
            let (w, _) = self.grid.target_size();
            let (cw, ch) = self.grid.cell_size();
            for (i, c) in colour.iter().enumerate() {
                let changed = self.front.as_ref().is_none_or(|f| f.colour[i] != *c);
                if changed {
                    let (x, y) = ((i as u32) % w, (i as u32) / w);
                    if let Some(cell) = self.grid.target_px_to_cell(x / cw * cw, y / ch * ch) {
                        cells.insert(cell);
                    }
                }
            }
            self.front = Some(FrontEnd {
                source: Arc::clone(src),
                adjustments: adj.clone(),
                colour,
            });
        }
        let colour = &self.front.as_ref().expect("filled above").colour;
        self.frame.set_prepared(Some(colour));
        cells
    }

    /// [`Self::update`]'s compositing proper: blends the changed layers and returns the
    /// cells whose target pixels changed.
    fn update_composite(&mut self, layers: &[Layer], regions: &[Region]) -> CellSet {
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
        let (grid, full_res, clock) = (self.grid, self.full_res_feather, self.clock);
        let shown = |l: &Layer| l.visible() && l.opacity() > 0.0;
        let mut work: Vec<(&Layer, &mut LayerCache, Refreshed)> = layers
            .iter()
            .zip(&mut self.caches)
            .filter(|(l, _)| shown(l))
            .map(|(l, c)| (l, c, Refreshed::default()))
            .collect();
        crate::par::for_each(&mut work, |(layer, cache, done)| {
            *done = cache.refresh(layer, grid, full_res, clock);
        });
        for (_, _, done) in work {
            self.resamples += u64::from(done.resampled);
            self.mask_evals += u64::from(done.evaluated);
            self.mip_builds += u64::from(done.mip_levels);
            self.mip_ms += done.mip_ms;
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
    let corner = |x: u32, y: u32| {
        grid.canvas_to_super(t.layer_to_canvas(size, Vec2::new(x as f32, y as f32)))
    };
    let step = grid.super_px_in_canvas();
    // The mask's mip level on the grid it is resampled on (#235).
    let level = levels_at(layer, mask_grid(grid).0.super_px_in_canvas());
    let (a, b, reach_x, reach_y) = if t.is_axis_aligned() {
        let (a, b) = (
            corner(layer_rect.x0, layer_rect.y0),
            corner(layer_rect.x1, layer_rect.y1),
        );
        // Mitchell reaches two source pixels each way (ADR-012), on a mip level two of
        // its pixels and the one a source pixel was averaged into; three super pixels
        // at least, for the edge.
        let src = |l: u32| if l == 0 { 2.0 } else { (3u32 << l) as f32 };
        let reach_x = (src(level.0) * t.scale().x / step.x).max(3.0).ceil() as u32 + 1;
        let reach_y = (src(level.1) * t.scale().y / step.y).max(3.0).ceil() as u32 + 1;
        (a, b, reach_x, reach_y)
    } else {
        // Rotated (#198): the bounding box of the rect's four corners, grown by what a
        // source pixel reaches through the kernel, plus the antialiased edge.
        let q = [
            corner(layer_rect.x0, layer_rect.y0),
            corner(layer_rect.x1, layer_rect.y0),
            corner(layer_rect.x1, layer_rect.y1),
            corner(layer_rect.x0, layer_rect.y1),
        ];
        let lo = q
            .iter()
            .fold(q[0], |m, p| Vec2::new(m.x.min(p.x), m.y.min(p.y)));
        let hi = q
            .iter()
            .fold(q[0], |m, p| Vec2::new(m.x.max(p.x), m.y.max(p.y)));
        let map = AffineMap::new(&t, size, grid, level);
        let canvas = map.reach_src() as f32 * t.scale().x.max(t.scale().y);
        let reach = (canvas / step.x.min(step.y)).max(3.0).ceil() as u32 + 2;
        (lo, hi, reach, reach)
    };
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

/// The target cells a change inside `canvas_rect` (canvas pixels, 320 × 200) can affect,
/// as a region command reports it ([`Dirty::CanvasRect`](crate::command::Dirty)): the
/// rectangle grown by one canvas pixel, since regions are sampled bilinearly, and mapped
/// onto the cells.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "coordinates are clamped to the super grid before casting"
)]
pub fn canvas_change_cells(grid: SuperGrid, canvas_rect: PxRect) -> CellSet {
    if canvas_rect.is_empty() {
        return CellSet::EMPTY;
    }
    let a = grid.canvas_to_super(Vec2::new(
        canvas_rect.x0 as f32 - 1.0,
        canvas_rect.y0 as f32 - 1.0,
    ));
    let b = grid.canvas_to_super(Vec2::new(
        canvas_rect.x1 as f32 + 1.0,
        canvas_rect.y1 as f32 + 1.0,
    ));
    let (gw, gh) = grid.size();
    let r = PxRect::new(
        a.x.max(0.0).floor() as u32,
        a.y.max(0.0).floor() as u32,
        (b.x.ceil().max(0.0) as u32).min(gw),
        (b.y.ceil().max(0.0) as u32).min(gh),
    );
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

    /// A 200 × 125 opaque 8-bit picture filling the screen, gamma 1.5: a plain layer.
    fn plain() -> Layer {
        let (w, h) = (200u32, 125u32);
        let rgba: Vec<u8> = (0..w * h)
            .flat_map(|i| {
                let (x, y) = (i % w, i / w);
                [
                    (x * 255 / w) as u8,
                    (y * 255 / h) as u8,
                    ((x + y) % 256) as u8,
                    255,
                ]
            })
            .collect();
        let mut l = Layer::new(
            "p",
            Arc::new(SourceImage::from_srgba8(w, h, &rgba).unwrap()),
        );
        l.set_transform(
            Transform::new(
                Vec2::ZERO,
                Vec2::new(320.0 / w as f32, 200.0 / h as f32),
                false,
                false,
            )
            .unwrap(),
        );
        l.set_adjustments(Adjustments {
            gamma: crate::layer::Gamma::PNG2C64,
            ..Adjustments::default()
        });
        l
    }

    /// ADR-012's invariant: a plain layer through the general path (the compatibility
    /// branch off) is png2c64's prepared image, bit for bit, at the default 1× grid, with
    /// and without gamma and range matching.
    #[test]
    fn the_general_path_is_png2c64_s_front_end_for_a_plain_layer() {
        use crate::layer::{Gamma, RangeMatch, Sharpen};
        use crate::solve::png2c64_ref::{prepare, preprocess, quantize::Mode as RefMode};
        for (mode, rm) in [
            (Mode::MulticolorBitmap, RefMode::Multicolor),
            (Mode::HiresBitmap, RefMode::Hires),
        ] {
            // With #228's sharpen too: png2c64's kernel, both ways.
            for (gamma, range, sharpen) in [
                (1.0, false, 0.0),
                (1.5, false, 0.0),
                (1.5, true, 0.0),
                (1.5, false, 0.5),
                (1.5, false, -0.5),
                (1.0, true, 0.5),
            ] {
                let mut layer = plain();
                let mut adj = layer.adjustments().clone();
                adj.gamma = Gamma::new(gamma).unwrap();
                adj.sharpen = Sharpen::png2c64(sharpen).unwrap();
                let targets = [(0.1, 0.9), (-0.2, 0.2), (-0.25, 0.25)];
                adj.match_range = range.then(|| {
                    RangeMatch::new(0.01, [0.1, 0.9], [-0.2, 0.2], [-0.25, 0.25]).unwrap()
                });
                layer.set_adjustments(adj);
                let mut c = Compositor::new(SuperGrid::with_default_factor(mode).unwrap());
                c.set_png2c64_front_end(false);
                c.update(std::slice::from_ref(&layer), &[]);
                assert!(c.frame().png2c64_prepared());
                let src = layer.source();
                let mut want = prepare(
                    src.srgb8().unwrap(),
                    (200, 125),
                    rm,
                    &preprocess::Settings {
                        gamma,
                        sharpen,
                        ..preprocess::Settings::default()
                    },
                    None,
                );
                if range {
                    preprocess::match_range_to(&mut want, 0.01, targets);
                }
                let got = c.frame().colour();
                let same = got.iter().zip(&want.px).filter(|(a, b)| a == b).count();
                assert_eq!(
                    same,
                    want.px.len(),
                    "{mode} gamma {gamma} range {range} sharpen {sharpen}"
                );
            }
        }
    }

    /// #190: a plain layer's frame is png2c64's prepared image; a nudge leaves the
    /// compatibility path (and recomposites everything), a nudge back returns to it.
    #[test]
    fn a_plain_layer_takes_png2c64_s_front_end() {
        use crate::solve::png2c64_ref::{prepare, preprocess, quantize::Mode as RefMode};
        for (mode, rm) in [
            (Mode::MulticolorBitmap, RefMode::Multicolor),
            (Mode::HiresBitmap, RefMode::Hires),
        ] {
            let layer = plain();
            let mut c = Compositor::new(grid(mode, 4));
            let dirty = c.update(std::slice::from_ref(&layer), &[]);
            assert_eq!(
                dirty.len(),
                1000,
                "{mode}: the first update dirties everything"
            );
            assert!(c.frame().png2c64_prepared());
            let src = layer.source();
            let want = prepare(
                src.srgb8().unwrap(),
                (200, 125),
                rm,
                &preprocess::Settings::default(),
                None,
            );
            assert_eq!(c.frame().colour(), &want.px[..], "{mode}");

            let mut nudged = layer.clone();
            nudged.set_transform(
                Transform::new(
                    Vec2::new(0.25, 0.0),
                    layer.transform().scale(),
                    false,
                    false,
                )
                .unwrap(),
            );
            let dirty = c.update(std::slice::from_ref(&nudged), &[]);
            assert!(c.front.is_none(), "{mode}: a nudged layer is composited");
            assert_eq!(dirty.len(), 1000);
            let mut fresh = Compositor::new(grid(mode, 4));
            fresh.update(std::slice::from_ref(&nudged), &[]);
            assert_eq!(c.frame(), fresh.frame(), "{mode}: as a cold composite");

            let dirty = c.update(std::slice::from_ref(&layer), &[]);
            assert!(c.frame().png2c64_prepared());
            assert!(!dirty.is_empty());
            assert_eq!(c.frame().colour(), &want.px[..]);
            assert!(
                c.update(std::slice::from_ref(&layer), &[]).is_empty(),
                "no change, no dirt"
            );

            // Switched off, the same layer is composited like any other (at n = 1 to the
            // same frame: `the_general_path_is_png2c64_s_front_end_for_a_plain_layer`).
            c.set_png2c64_front_end(false);
            c.update(std::slice::from_ref(&layer), &[]);
            assert!(c.front.is_none());
        }
    }

    /// A translucent or 8-bit-less source, a mask, a second layer or a region keep the
    /// supersampled composite.
    #[test]
    fn anything_beyond_a_plain_layer_is_composited() {
        let layer = plain();
        let g = grid(Mode::MulticolorBitmap, 4);
        let half = Arc::new(SourceImage::from_srgba8(1, 1, &[10, 20, 30, 128]).unwrap());
        let mut translucent = Layer::new("t", half);
        translucent.set_transform(
            Transform::new(Vec2::ZERO, Vec2::new(320.0, 200.0), false, false).unwrap(),
        );
        let mut masked = layer.clone();
        masked
            .set_mask(Some(Mask::new(MaskRaster::filled(200, 125, 1.0).unwrap())))
            .unwrap();
        let region = Region::new(
            "r",
            MaskRaster::filled(320, 200, 1.0).unwrap(),
            *layer.recipe(),
        )
        .unwrap();
        for (what, layers, regions) in [
            ("translucent", vec![translucent], vec![]),
            ("masked", vec![masked], vec![]),
            ("two layers", vec![layer.clone(), layer.clone()], vec![]),
            ("region", vec![layer.clone()], vec![region]),
        ] {
            let mut c = Compositor::new(g);
            c.update(&layers, &regions);
            assert!(c.front.is_none(), "{what}");
        }
    }

    /// #228: a plain layer sharpened with png2c64's kernel and no threshold keeps
    /// png2c64's front end, with the amount in its preprocessing; a Gaussian radius or a
    /// threshold, which png2c64 does not have, is composited.
    #[test]
    fn png2c64_sharpen_keeps_the_front_end() {
        use crate::layer::{Radius, Sharpen};
        use crate::solve::png2c64_ref::{prepare, preprocess, quantize::Mode as RefMode};
        let base = plain();
        let with = |s: Sharpen| {
            let mut l = base.clone();
            l.set_adjustments(Adjustments {
                sharpen: s,
                ..base.adjustments().clone()
            });
            l
        };
        let g = grid(Mode::MulticolorBitmap, 4);
        for amount in [0.5, -0.5] {
            let layer = with(Sharpen::png2c64(amount).unwrap());
            let mut c = Compositor::new(g);
            c.update(std::slice::from_ref(&layer), &[]);
            assert!(c.front.is_some(), "{amount}");
            let want = prepare(
                layer.source().srgb8().unwrap(),
                (200, 125),
                RefMode::Multicolor,
                &preprocess::Settings {
                    sharpen: amount,
                    ..preprocess::Settings::default()
                },
                None,
            );
            assert_eq!(c.frame().colour(), &want.px[..], "{amount}");
            // The sharpen changed the prepared image.
            let mut plain_c = Compositor::new(g);
            plain_c.update(std::slice::from_ref(&base), &[]);
            assert_ne!(plain_c.frame().colour(), c.frame().colour(), "{amount}");
        }
        for s in [
            Sharpen::new(0.5, Radius::gaussian(0.8).unwrap(), 0.0).unwrap(),
            Sharpen::new(0.5, Radius::Png2c64, 0.01).unwrap(),
        ] {
            let mut c = Compositor::new(g);
            c.update(&[with(s)], &[]);
            assert!(c.front.is_none(), "{s:?}");
        }
    }

    /// #228: sharpening edits, a nudge of a sharpened layer and turning the sharpen
    /// off update exactly the cells whose output changed, as a full recomposite, and
    /// stay inside the layer's footprint: the sharpen never spreads past the layer's
    /// rectangle, so its dirty region is that rectangle's cells.
    #[test]
    fn sharpen_updates_equal_a_full_recomposite() {
        use crate::layer::{Radius, Sharpen};
        for mode in [Mode::HiresBitmap, Mode::MulticolorBitmap] {
            let g = grid(mode, 2);
            let (w, h) = (40, 30);
            let bg = Layer::new("bg", solid(320, 200, [0.1, 0.2, 0.3, 1.0]));
            let top = layer_of(
                pattern(w, h),
                w,
                h,
                Transform::translated(Vec2::new(64.25, 40.5)).unwrap(),
            );
            let mut layers = vec![bg, top];
            let mut c = Compositor::new(g);
            c.update(&layers, &[]);
            let set = |l: &mut Vec<Layer>, s: Sharpen| {
                let a = Adjustments {
                    sharpen: s,
                    ..l[1].adjustments().clone()
                };
                l[1].set_adjustments(a);
            };
            type Edit = Box<dyn Fn(&mut Vec<Layer>)>;
            let edits: [(&str, Edit); 5] = [
                (
                    "png2c64",
                    Box::new(move |l| set(l, Sharpen::png2c64(1.0).unwrap())),
                ),
                (
                    "gaussian",
                    Box::new(move |l| {
                        set(
                            l,
                            Sharpen::new(1.0, Radius::gaussian(0.8).unwrap(), 0.0).unwrap(),
                        );
                    }),
                ),
                (
                    "nudge",
                    Box::new(|l| {
                        let t = l[1].transform().nudged(Vec2::new(0.25, 0.0)).unwrap();
                        l[1].set_transform(t);
                    }),
                ),
                (
                    "soften",
                    Box::new(move |l| set(l, Sharpen::png2c64(-1.0).unwrap())),
                ),
                ("off", Box::new(move |l| set(l, Sharpen::default()))),
            ];
            for (name, edit) in edits {
                let (_, before) = full(g, &layers, &[]);
                let footprint = |l: &[Layer]| {
                    let (_, rect) = placement(&l[1], g, colour_levels(&l[1], g));
                    CellSet::from_super_rect(g, rect)
                };
                let mut allowed = footprint(&layers);
                edit(&mut layers);
                allowed = allowed.union(footprint(&layers));
                let reported = c.update(&layers, &[]);
                let (canvas, frame) = full(g, &layers, &[]);
                assert!(c.canvas() == &canvas, "{mode} {name}: canvas");
                assert!(c.frame() == &frame, "{mode} {name}: frame");
                let (tw, th) = g.target_size();
                let changed: CellSet = before
                    .differing(&frame, PxRect::of_size(tw, th))
                    .into_iter()
                    .filter_map(|(x, y)| g.target_px_to_cell(x, y))
                    .collect();
                assert_eq!(reported, changed, "{mode} {name}");
                assert!(!reported.is_empty(), "{mode} {name}: changed nothing?");
                assert_eq!(
                    reported.union(allowed),
                    allowed,
                    "{mode} {name}: outside the layer"
                );
            }
        }
    }

    fn at(layer: Layer, x: f32, y: f32) -> Layer {
        let mut l = layer;
        l.set_transform(Transform::translated(Vec2::new(x, y)).unwrap());
        l
    }

    #[test]
    fn extent_of_matches_a_plain_scan() {
        for len in [0, 1, 15, 16, 17, 33, 64, 100] {
            for (a, b) in [
                (None, None),
                (Some(0), Some(0)),
                (Some(3), Some(40)),
                (Some(31), Some(32)),
                (Some(99), Some(99)),
            ] {
                let mut row = vec![0.0f32; len];
                for i in [a, b].into_iter().flatten().filter(|&i| i < len) {
                    row[i] = 0.5;
                }
                let on = |m: &f32| *m > 0.0;
                let want = match (row.iter().position(on), row.iter().rposition(on)) {
                    (Some(i), Some(j)) => (i as u32, j as u32 + 1),
                    _ => (0, 0),
                };
                assert_eq!(extent_of(&row, on), want, "{len} {a:?} {b:?}");
            }
        }
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
        // A 2000-wide source scaled to 320: a downscaled flat field stays flat.
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
        // The last three are rotated (#198), one scaled down far enough to sample a mip.
        for (mode, snap, feather, rotate, scale) in [
            (Mode::HiresBitmap, SnapToCells::Off, 0.0, 0.0, 1.3),
            (Mode::HiresBitmap, SnapToCells::Expand, 3.0, 0.0, 1.3),
            (Mode::MulticolorBitmap, SnapToCells::Nearest, 12.0, 0.0, 1.3),
            (Mode::HiresBitmap, SnapToCells::Off, 0.0, 33.0, 1.3),
            (Mode::MulticolorBitmap, SnapToCells::Expand, 3.0, -71.5, 1.3),
            (Mode::HiresBitmap, SnapToCells::Off, 1.0, 12.0, 0.3),
        ] {
            let g = grid(mode, 2);
            let m = Modifiers::new(false, crate::layer::Levels::IDENTITY, snap, feather).unwrap();
            let mut l = masked(
                solid(200, 150, [0.8, 0.4, 0.2, 1.0]),
                vec![0.0; 200 * 150],
                m,
            );
            l.set_transform(
                Transform::new(Vec2::new(30.5, 20.25), Vec2::new(scale, 0.9), false, false)
                    .unwrap()
                    .with_rotation(rotate)
                    .unwrap(),
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

    /// Region commands report a canvas rectangle (#216); its cells cover every cell the
    /// compositor finds changed, undo included.
    #[test]
    fn region_commands_report_the_cells_they_change() {
        use crate::command::{
            Command, Dirty, Editor, RecipeChange, RegionAdd, RegionClear, RegionFill,
            RegionGradient, RegionReorder, RegionSetRecipe, RegionStroke,
        };
        use crate::mask::{Combine, Gradient, GradientKind, PaintMode};
        for mode in [Mode::HiresBitmap, Mode::MulticolorBitmap] {
            let g = grid(mode, 2);
            let mut e = Editor::new(crate::Project {
                layers: edit_scene(),
                regions: vec![sky_region()],
                ..crate::Project::default()
            });
            let mut c = Compositor::new(g);
            c.update(&e.project().layers, &e.project().regions);
            let square = vec![
                Vec2::new(100.0, 60.0),
                Vec2::new(180.0, 60.0),
                Vec2::new(180.0, 120.0),
                Vec2::new(100.0, 120.0),
            ];
            let commands = [
                Command::RegionAdd(Box::new(RegionAdd {
                    index: 1,
                    name: "dither".into(),
                    recipe: crate::layer::Recipe::with_method(
                        crate::layer::DitherKind::Atkinson,
                        1.0,
                    )
                    .unwrap(),
                })),
                Command::RegionStroke(RegionStroke {
                    region: 1,
                    points: vec![Vec2::new(150.0, 40.0), Vec2::new(250.0, 150.0)],
                    radius: 9.0,
                    hardness: 0.4,
                    flow: 0.8,
                    mode: PaintMode::Add,
                }),
                Command::RegionFill(RegionFill {
                    region: 1,
                    polygon: square,
                    value: 1.0,
                    mode: PaintMode::Add,
                }),
                Command::RegionGradient(RegionGradient {
                    region: 1,
                    gradient: Gradient::new(
                        GradientKind::Radial,
                        Vec2::new(40.0, 40.0),
                        Vec2::new(90.0, 40.0),
                        0.5,
                    )
                    .unwrap(),
                    combine: Combine::Max,
                }),
                Command::RegionReorder(RegionReorder { region: 1, to: 0 }),
                Command::RegionSetRecipe(Box::new(RegionSetRecipe {
                    region: 1,
                    recipe: RecipeChange {
                        strength: Some(0.9),
                        ..RecipeChange::default()
                    },
                })),
                Command::RegionClear(RegionClear {
                    region: 0,
                    value: 0.0,
                }),
            ];
            let check = |c: &mut Compositor, e: &Editor, dirty: Dirty, what: &str| {
                let Dirty::CanvasRect(rect) = dirty else {
                    panic!("{what}: {dirty:?}")
                };
                let reported = c.update(&e.project().layers, &e.project().regions);
                let covered = canvas_change_cells(g, rect);
                assert_eq!(
                    reported.intersection(covered),
                    reported,
                    "{mode} {what}: changed cells outside {rect:?}"
                );
            };
            for cmd in commands {
                let name = cmd.name();
                let dirty = e.execute(cmd).unwrap();
                check(&mut c, &e, dirty, name);
            }
            while let Some(dirty) = e.undo() {
                check(&mut c, &e, dirty, "undo");
            }
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

    /// A deterministic test pattern with partial alpha.
    fn pattern(w: u32, h: u32) -> Vec<[f32; 4]> {
        let mut s = 0x2545_f491_u32;
        (0..w * h)
            .map(|_| {
                let mut next = || {
                    s ^= s << 13;
                    s ^= s >> 17;
                    s ^= s << 5;
                    (s >> 8) as f32 / (1u32 << 24) as f32
                };
                [next(), next(), next(), 0.5 + 0.5 * next()]
            })
            .collect()
    }

    /// A smooth pattern: a few slow waves.
    fn smooth(w: u32, h: u32) -> Vec<[f32; 4]> {
        (0..w * h)
            .map(|i| {
                let (x, y) = ((i % w) as f32, (i / w) as f32);
                [
                    0.5 + 0.4 * (x * 0.21).sin(),
                    0.5 + 0.4 * (y * 0.17).cos(),
                    0.5 + 0.3 * ((x + y) * 0.11).sin(),
                    1.0,
                ]
            })
            .collect()
    }

    /// `px` (`w × h`) turned 90° clockwise: `h × w`.
    fn turned<T: Copy>(px: &[T], w: u32, h: u32) -> Vec<T> {
        let mut out = Vec::with_capacity(px.len());
        for y in 0..w {
            for x in 0..h {
                out.push(px[((h - 1 - x) * w + y) as usize]);
            }
        }
        out
    }

    fn layer_of(px: Vec<[f32; 4]>, w: u32, h: u32, t: Transform) -> Layer {
        let mut l = Layer::new("p", Arc::new(SourceImage::from_linear(w, h, px).unwrap()));
        l.set_transform(t);
        l
    }

    fn max_diff(a: &[[f32; 4]], b: &[[f32; 4]]) -> f32 {
        assert_eq!(a.len(), b.len());
        a.iter()
            .zip(b)
            .flat_map(|(p, q)| p.iter().zip(q).map(|(x, y)| (x - y).abs()))
            .fold(0.0, f32::max)
    }

    /// #198: with `rotate == 0` the anchor changes nothing, and `-0.0` is no rotation.
    #[test]
    fn an_unrotated_layer_ignores_its_anchor() {
        let (w, h) = (37, 23);
        let t = Transform::new(Vec2::new(10.3, 7.6), Vec2::new(1.7, 0.6), true, false).unwrap();
        for mode in [Mode::HiresBitmap, Mode::MulticolorBitmap] {
            let g = grid(mode, 4);
            let a = Compositor::new(g)
                .composite(&[layer_of(pattern(w, h), w, h, t)])
                .clone();
            for t2 in [
                t.with_anchor(Vec2::new(0.0, 1.0)).unwrap(),
                t.with_rotation(-0.0).unwrap(),
            ] {
                let b = Compositor::new(g)
                    .composite(&[layer_of(pattern(w, h), w, h, t2)])
                    .clone();
                assert!(
                    a.pixels()
                        .iter()
                        .zip(b.pixels())
                        .all(|(p, q)| p.map(f32::to_bits) == q.map(f32::to_bits)),
                    "{mode}"
                );
            }
        }
    }

    /// #198: a full turn (360°) takes the rotated path and reproduces the unrotated
    /// layer: exactly on a 1× hi-res grid (every sample on a pixel centre, the coverage
    /// exact), and within 0.006 per channel (1.5 8-bit steps) of the target frame on a
    /// smooth source, where the rotated path's inverse-mapped Mitchell meets the
    /// separable path's (ADR-012).
    #[test]
    fn a_full_turn_is_the_identity() {
        let (w, h) = (40, 30);
        for (mode, n, src, tol) in [
            (Mode::HiresBitmap, 1, pattern(w, h), 1e-6),
            (Mode::HiresBitmap, 4, smooth(w, h), 6e-3),
            (Mode::MulticolorBitmap, 4, smooth(w, h), 6e-3),
            (Mode::MulticolorBitmap, 1, smooth(w, h), 6e-3),
        ] {
            let g = grid(mode, n);
            let t = Transform::new(Vec2::new(20.0, 16.0), Vec2::ONE, false, false).unwrap();
            let a = layer_of(src.clone(), w, h, t);
            let b = layer_of(src.clone(), w, h, t.with_rotation(360.0).unwrap());
            let mut ca = Compositor::new(g);
            let mut cb = Compositor::new(g);
            ca.composite(&[a]);
            cb.composite(&[b]);
            let d = max_diff(&ca.canvas().downsample(), &cb.canvas().downsample());
            assert!(d <= tol, "{mode} {n}×: {d}");
        }
        // A sub-pixel placement: the edge coverage agrees too.
        let g = grid(Mode::HiresBitmap, 4);
        let t = Transform::new(Vec2::new(20.3, 16.6), Vec2::ONE, false, false).unwrap();
        let src = vec![[0.4, 0.5, 0.6, 1.0]; (w * h) as usize];
        let mut ca = Compositor::new(g);
        let mut cb = Compositor::new(g);
        ca.composite(&[layer_of(src.clone(), w, h, t)]);
        cb.composite(&[layer_of(src, w, h, t.with_rotation(-360.0).unwrap())]);
        let d = max_diff(ca.canvas().pixels(), cb.canvas().pixels());
        assert!(d < 1e-5, "coverage: {d}");
    }

    /// #198: turning a layer 90° about its centre composites what the source's pixels,
    /// turned and placed axis-aligned, do: to 1e-6 on a 1× grid, mask, feather
    /// and attribute map included (masks transform with their layer, ADR-008), and within
    /// 0.006 per channel of the target frame at 4× on a smooth source (both Mitchell,
    /// ADR-012, inverse-mapped against separable).
    #[test]
    fn a_quarter_turn_matches_the_pixels_turned() {
        let (w, h) = (40u32, 24u32);
        // The turned box (h × w) about the same centre: its corner moves by
        // ((w − h) / 2, (h − w) / 2) = (8, −8).
        let t = Transform::new(Vec2::new(50.0, 60.0), Vec2::ONE, false, false).unwrap();
        let rt = Transform::translated(Vec2::new(58.0, 52.0)).unwrap();
        let mask: Vec<f32> = (0..w * h)
            .map(|i| if i % w < 15 || i / w < 5 { 1.0 } else { 0.3 })
            .collect();
        // At 4× the mask is smooth too, so the two kernels agree.
        let soft: Vec<f32> = smooth(w, h).iter().map(|p| p[0]).collect();
        for (n, src, mask, tol) in [
            (1, pattern(w, h), mask, 1e-6),
            (4, smooth(w, h), soft, 6e-3),
        ] {
            for feather in [0.0, 1.5] {
                let m = Modifiers::feathered(feather).unwrap();
                let mut rotated = masked(
                    Arc::new(SourceImage::from_linear(w, h, src.clone()).unwrap()),
                    mask.clone(),
                    m,
                );
                rotated.set_transform(t.with_rotation(90.0).unwrap());
                let mut reference = masked(
                    Arc::new(SourceImage::from_linear(h, w, turned(&src, w, h)).unwrap()),
                    turned(&mask, w, h),
                    m,
                );
                reference.set_transform(rt);
                let g = grid(Mode::HiresBitmap, n);
                let mut ca = Compositor::new(g);
                let mut cb = Compositor::new(g);
                ca.composite(&[rotated]);
                cb.composite(&[reference]);
                let (ra, ma) = ca.evaluated_mask(0).unwrap();
                let (rb, mb) = cb.evaluated_mask(0).unwrap();
                assert_eq!(ra, rb, "{n}× feather {feather}: the same rectangle");
                if n == 1 {
                    let d = max_diff(ca.canvas().pixels(), cb.canvas().pixels());
                    assert!(d <= tol, "{n}× feather {feather}: colour {d}");
                    let dm = ma
                        .iter()
                        .zip(mb)
                        .map(|(a, b)| (a - b).abs())
                        .fold(0.0, f32::max);
                    assert!(dm <= 1e-6, "{n}× feather {feather}: mask {dm}");
                    // The mask is resampled at 4× (ADR-012), where the two kernels
                    // agree to rounding: the same sources, strengths to 1e-6.
                    let (am, bm) = (ca.attribute_map(), cb.attribute_map());
                    let (tw, th) = g.target_size();
                    for (x, y) in (0..th).flat_map(|y| (0..tw).map(move |x| (x, y))) {
                        let (a, b) = (am.get(x, y), bm.get(x, y));
                        assert_eq!(a.source, b.source, "{n}× feather {feather}: ({x}, {y})");
                        assert!((a.strength - b.strength).abs() <= 1e-6);
                    }
                } else {
                    let d = max_diff(&ca.canvas().downsample(), &cb.canvas().downsample());
                    assert!(d <= tol, "{n}× feather {feather}: colour {d}");
                }
            }
        }
    }

    /// #198: a rotated, scaled-down layer samples a mip and still keeps a flat field
    /// flat inside its quad, with the coverage summing to the quad's area.
    #[test]
    fn a_rotated_downscale_keeps_flat_fields() {
        let (w, h) = (400u32, 300u32);
        let g = grid(Mode::MulticolorBitmap, 4);
        let t = Transform::new(Vec2::new(100.0, 50.0), Vec2::new(0.2, 0.25), false, false)
            .unwrap()
            .with_rotation(27.0)
            .unwrap();
        let mut c = Compositor::new(g);
        c.composite(&[layer_of(
            vec![[0.3, 0.6, 0.9, 1.0]; (w * h) as usize],
            w,
            h,
            t,
        )]);
        let step = g.super_px_in_canvas();
        let mut area = 0.0f64;
        for p in c.canvas().pixels() {
            area += f64::from(p[3]);
            if p[3] > 0.0 {
                let straight = [p[0] / p[3], p[1] / p[3], p[2] / p[3]];
                for (a, b) in straight.iter().zip([0.3, 0.6, 0.9]) {
                    assert!((a - b).abs() < 1e-4, "{straight:?}");
                }
            }
        }
        let area = area * f64::from(step.x * step.y);
        let want = 400.0 * 0.2 * 300.0 * 0.25;
        assert!((area - want).abs() < 1e-3 * want, "{area} {want}");
    }

    /// #198: under a feather blurred at half resolution, a rotated mask is sampled
    /// straight at half resolution (one sample per 2 × 2 block). Against the reference
    /// (sampled and blurred at full resolution, `set_full_res_feather`) on a hard edge
    /// at 4×, it was measured within 0.0052 at σ = 4 target px, 0.0034 at 6 and 0.0023
    /// at 9.5; the separable path's half-resolution feather on the same layer unrotated:
    /// 0.0020, 0.0015 and 0.0010. The bound is 0.006 from σ = 4 and one 8-bit step from
    /// σ = 6.
    #[test]
    fn a_rotated_feathered_mask_matches_the_full_resolution_reference() {
        let (w, h) = (120u32, 90u32);
        for (mode, feather, bound) in [
            (Mode::HiresBitmap, 4.0, 0.006),
            (Mode::MulticolorBitmap, 4.0, 0.006),
            (Mode::MulticolorBitmap, 6.0, 1.0 / 255.0),
            (Mode::HiresBitmap, 9.5, 1.0 / 255.0),
        ] {
            let mut l = masked(
                solid(w, h, [0.6, 0.3, 0.1, 1.0]),
                half_mask(w, h, 50),
                Modifiers::feathered(feather).unwrap(),
            );
            l.set_transform(
                Transform::new(Vec2::new(80.3, 40.6), Vec2::new(1.3, 0.8), true, false)
                    .unwrap()
                    .with_rotation(23.0)
                    .unwrap(),
            );
            let g = grid(mode, 4);
            let mut half = Compositor::new(g);
            let mut full = Compositor::new(g);
            full.set_full_res_feather(true);
            half.composite(std::slice::from_ref(&l));
            full.composite(std::slice::from_ref(&l));
            let (ra, a) = half.evaluated_mask(0).unwrap();
            let (rb, b) = full.evaluated_mask(0).unwrap();
            assert_eq!(ra, rb);
            let d = a
                .iter()
                .zip(b)
                .map(|(x, y)| (x - y).abs())
                .fold(0.0, f32::max);
            assert!(d < bound, "{mode} feather {feather}: {d}");
        }
    }

    /// #198: rotating a layer recomposites, and reports, the cells whose output changed,
    /// all inside the bounding boxes of the old and new quads; the result equals a full
    /// recomposite.
    #[test]
    fn rotation_updates_cover_the_old_and_new_quads() {
        for mode in [Mode::HiresBitmap, Mode::MulticolorBitmap] {
            let g = grid(mode, 2);
            let mut layers = edit_scene();
            let mut c = Compositor::new(g);
            c.update(&layers, &[]);
            type Edit = fn(Transform) -> Transform;
            let edits: [(&str, Edit); 5] = [
                ("rotate", |t| t.with_rotation(30.0).unwrap()),
                ("a quarter degree more", |t| t.with_rotation(30.25).unwrap()),
                ("move the anchor", |t| {
                    t.with_anchor(Vec2::new(0.0, 0.0)).unwrap()
                }),
                ("turn past a right angle", |t| {
                    t.with_rotation(-120.0).unwrap()
                }),
                ("back to axis-aligned", |t| t.with_rotation(0.0).unwrap()),
            ];
            for (name, edit) in edits {
                let (_, before) = full(g, &layers, &[]);
                let old = c.last.as_ref().unwrap().0[1].as_ref().unwrap().raster.rect;
                let t = edit(layers[1].transform());
                layers[1].set_transform(t);
                let reported = c.update(&layers, &[]);
                let new = c.last.as_ref().unwrap().0[1].as_ref().unwrap().raster.rect;
                let (canvas, frame) = full(g, &layers, &[]);
                assert!(c.canvas() == &canvas, "{mode} {name}: canvas");
                assert!(c.frame() == &frame, "{mode} {name}: frame");
                let (tw, th) = g.target_size();
                let changed: CellSet = before
                    .differing(&frame, PxRect::of_size(tw, th))
                    .into_iter()
                    .filter_map(|(x, y)| g.target_px_to_cell(x, y))
                    .collect();
                assert!(!changed.is_empty(), "{mode} {name}");
                assert_eq!(reported, changed, "{mode} {name}");
                // The quad's bounding box on the grid is the layer's rectangle.
                let size = layers[1].source().size();
                let (lo, hi) = t.canvas_bounds(size);
                let (a, b) = (g.canvas_to_super(lo), g.canvas_to_super(hi));
                assert!(new.x0 as f32 <= a.x.max(0.0) && new.y0 as f32 <= a.y.max(0.0));
                assert!(new.x1 as f32 >= b.x.min(g.size().0 as f32) - 1e-3);
                assert!(new.y1 as f32 >= b.y.min(g.size().1 as f32) - 1e-3);
                let footprint = CellSet::from_super_rect(g, old.union(new));
                assert_eq!(reported.intersection(footprint), reported, "{mode} {name}");
            }
        }
    }

    /// An opaque 8-bit `w × h` source with detail everywhere, fitted to the screen.
    fn fitted(w: u32, h: u32, downscale: Downscale) -> Layer {
        let rgba: Vec<u8> = (0..w * h)
            .flat_map(|i| {
                let (x, y) = (i % w, i / w);
                [
                    (x * 7 + y * 3) as u8,
                    (x ^ y) as u8,
                    ((x * x + y) / 3) as u8,
                    255,
                ]
            })
            .collect();
        let src = Arc::new(SourceImage::from_srgba8(w, h, &rgba).unwrap());
        let mut l = layer_of_src(src, (320.0 / w as f32, 200.0 / h as f32));
        l.set_downscale(downscale);
        l
    }

    fn layer_of_src(src: Arc<SourceImage>, (sx, sy): (f32, f32)) -> Layer {
        let mut l = Layer::new("s", src);
        l.set_transform(Transform::new(Vec2::ZERO, Vec2::new(sx, sy), false, false).unwrap());
        l.set_adjustments(Adjustments {
            gamma: crate::layer::Gamma::PNG2C64,
            ..Adjustments::default()
        });
        l
    }

    /// #235: up to a 4× shrink on the 320 grid (every source up to 1280 px wide fitted to
    /// the screen) Auto samples the original: the frame and the canvas are Exact's bit
    /// for bit, in both modes, through png2c64's front end and through the composite, and
    /// no mip level is built.
    #[test]
    fn auto_is_exact_up_to_a_4x_shrink() {
        for (w, h) in [
            (200, 125),
            (640, 400),
            (1024, 637),
            (1280, 720),
            (1280, 800),
        ] {
            for mode in [Mode::MulticolorBitmap, Mode::HiresBitmap] {
                for front_end in [true, false] {
                    let run = |d: Downscale| {
                        let mut c = Compositor::new(grid(mode, 1));
                        c.set_png2c64_front_end(front_end);
                        c.update(&[fitted(w, h, d)], &[]);
                        assert_eq!(c.mip_builds(), 0, "{w}×{h} {mode} {d:?}");
                        (c.frame().colour().to_vec(), c.canvas().clone())
                    };
                    let (auto, exact) = (run(Downscale::Auto), run(Downscale::Exact));
                    let same = auto
                        .0
                        .iter()
                        .zip(&exact.0)
                        .all(|(a, b)| a.map(f32::to_bits) == b.map(f32::to_bits));
                    assert!(same, "{w}×{h} {mode} front end {front_end}");
                    assert_eq!(auto.1, exact.1, "{w}×{h} {mode} front end {front_end}");
                }
            }
        }
    }

    /// #235: past a 4× shrink a plain picture leaves png2c64's front end under Auto
    /// (png2c64 has no mip levels), and keeps it under Exact.
    #[test]
    fn a_large_shrink_leaves_the_compatibility_path_under_auto() {
        let g = SuperGrid::with_default_factor(Mode::MulticolorBitmap).unwrap();
        let auto = fitted(1600, 1000, Downscale::Auto);
        let exact = fitted(1600, 1000, Downscale::Exact);
        assert!(plain_layer(std::slice::from_ref(&auto), &[], g).is_none());
        assert!(plain_layer(std::slice::from_ref(&exact), &[], g).is_some());
        assert_eq!(colour_levels(&auto, g), (2, 2));
        // The Source view's 4× grid is four times finer: the same source is no shrink
        // past 4× there.
        let fine = SuperGrid::new(Mode::MulticolorBitmap, 4).unwrap();
        assert_eq!(colour_levels(&auto, fine), (0, 0));
    }

    /// #235: scale, rotation and nudge drags of a layer shrunk past 4× reuse its mip
    /// levels (colour and mask); a level is built once, when first needed, and never
    /// again while the source and mask are unchanged.
    #[test]
    fn drags_reuse_the_mip_levels() {
        let (w, h) = (1024, 640);
        let src = Arc::new(SourceImage::from_linear(w, h, pattern(w, h)).unwrap());
        // 5 %: a 20× shrink, colour level 4; the mask's 4× grid, a 5× shrink, level 2.
        let mut layer = masked(
            Arc::clone(&src),
            half_mask(w, h, 400),
            Modifiers::feathered(1.5).unwrap(),
        );
        let place = |layer: &mut Layer, t: Vec2, s: f32, deg: f32| {
            let t = Transform::new(t, Vec2::new(s, s), false, false)
                .and_then(|t| t.with_rotation(deg))
                .unwrap();
            layer.set_transform(t);
        };
        place(&mut layer, Vec2::new(40.3, 30.7), 0.05, 0.0);
        let g = grid(Mode::HiresBitmap, 1);
        assert_eq!(colour_levels(&layer, g), (4, 4));
        assert_eq!(mask_placement(&layer, g).place.level(), (2, 2));
        let mut c = Compositor::new(g);
        c.update(std::slice::from_ref(&layer), &[]);
        let built = c.mip_builds();
        assert!(built > 0);
        let resampled = c.resamples();
        // Nudges, scales within the level, and rotations (a turned layer's footprint
        // spans more source pixels, still within the level).
        for (t, s, deg) in [
            (Vec2::new(40.55, 30.7), 0.05, 0.0),
            (Vec2::new(41.0, 31.2), 0.06, 0.0),
            (Vec2::new(41.0, 31.2), 0.045, 0.0),
            (Vec2::new(41.0, 31.2), 0.05, 10.0),
            (Vec2::new(41.0, 31.2), 0.05, 30.0),
            (Vec2::new(39.0, 30.0), 0.05, 0.0),
        ] {
            place(&mut layer, t, s, deg);
            c.update(std::slice::from_ref(&layer), &[]);
        }
        assert_eq!(c.resamples(), resampled + 6, "every drag step resampled");
        assert_eq!(c.mip_builds(), built, "and built no mip level");
        // Another compositor shares the source's colour levels; only the mask's (kept
        // per layer) are new to it.
        let mut other = Compositor::new(g);
        other.update(std::slice::from_ref(&layer), &[]);
        assert!(other.mip_builds() < built);
        // A shrink past the next level builds it, once.
        place(&mut layer, Vec2::new(40.0, 30.0), 0.02, 0.0);
        c.update(std::slice::from_ref(&layer), &[]);
        let more = c.mip_builds();
        assert!(more > built);
        for s in [0.05, 0.02, 0.05] {
            place(&mut layer, Vec2::new(40.0, 30.0), s, 0.0);
            c.update(std::slice::from_ref(&layer), &[]);
        }
        assert_eq!(c.mip_builds(), more);
        // Exact samples the original, whatever the shrink.
        layer.set_downscale(Downscale::Exact);
        c.update(std::slice::from_ref(&layer), &[]);
        assert_eq!(c.mip_builds(), more);
    }

    /// #235: a zone plate (rings up to the source's Nyquist frequency at its corners)
    /// shrunk 6.4×: beyond its centre the correct picture is flat grey. Exact (png2c64's
    /// four taps) fills it with aliasing rings; Auto (the 4× level, the rest by Mitchell)
    /// leaves it nearly flat. The same holds rotated.
    #[test]
    fn a_zone_plate_shrunk_past_4x_is_grey_beyond_its_centre() {
        let (w, h) = (2048u32, 1280u32);
        let (cx, cy) = (f64::from(w) / 2.0, f64::from(h) / 2.0);
        let rmax = cx.hypot(cy);
        let k = std::f64::consts::PI / (2.0 * rmax);
        let px: Vec<[f32; 4]> = (0..w * h)
            .map(|i| {
                let (x, y) = (f64::from(i % w) + 0.5 - cx, f64::from(i / w) + 0.5 - cy);
                let v = (0.5 + 0.5 * (k * (x * x + y * y)).cos()) as f32;
                [v, v, v, 1.0]
            })
            .collect();
        let src = Arc::new(SourceImage::from_linear(w, h, px).unwrap());
        for deg in [0.0f32, 2.0] {
            let spread = |d: Downscale| {
                let mut l = layer_of_src(Arc::clone(&src), (320.0 / w as f32, 200.0 / h as f32));
                l.set_adjustments(Adjustments::default());
                l.set_transform(l.transform().with_rotation(deg).unwrap());
                l.set_downscale(d);
                let g = grid(Mode::HiresBitmap, 1);
                let mut c = Compositor::new(g);
                c.update(&[l], &[]);
                // Target pixels more than half the way out, away from the edges.
                let colour = c.frame().colour();
                let vals: Vec<f64> = (0..320 * 200)
                    .filter(|i| {
                        let (x, y) = (f64::from(i % 320) - 159.5, f64::from(i / 320) - 99.5);
                        let r = (x * 6.4).hypot(y * 6.4);
                        r > 0.5 * rmax && x.abs() < 150.0 && y.abs() < 90.0
                    })
                    .map(|i| f64::from(colour[i as usize][1]))
                    .collect();
                let mean = vals.iter().sum::<f64>() / vals.len() as f64;
                let var = vals.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / vals.len() as f64;
                (mean, var.sqrt())
            };
            // Measured: Exact's standard deviation 0.262 around 0.498, Auto's 0.027
            // around 0.500, at both angles.
            let (exact, auto) = (spread(Downscale::Exact), spread(Downscale::Auto));
            assert!(exact.1 > 0.2, "{deg}°: exact aliases: {exact:?}");
            assert!(
                auto.1 < 0.05 && (auto.0 - 0.5).abs() < 0.01,
                "{deg}°: auto is flat grey: {auto:?} {exact:?}"
            );
        }
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
