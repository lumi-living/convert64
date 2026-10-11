//! The supersampled compositor (`docs/TAD.md` §2, ADR-002).
//!
//! Every visible layer is resampled onto the [`SuperGrid`] (`n ×` the target grid, the
//! 2:1 multicolor aspect included) and blended bottom to top into an f32 linear
//! **premultiplied** RGBA [`Canvas`]. Effective alpha is PNG alpha × mask × opacity.
//!
//! Each layer's resampled raster is cached and rebuilt only when its source,
//! adjustments, transform, mask or the grid change; a recomposite with unchanged layers
//! only re-blends.

mod blend;
mod resample;

use crate::geom::{PxRect, SuperGrid};
use crate::layer::{AdjustCache, Adjustments, Blend, Layer, SourceImage, Transform};
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
    /// The resampled mask, `rect`-sized, or `None` for an unmasked layer.
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

#[derive(Debug, Default)]
struct LayerCache {
    adjust: AdjustCache,
    raster: Option<(RasterKey, Arc<LayerRaster>)>,
}

/// Composites layers onto a [`SuperGrid`], caching each layer's resampled raster.
#[derive(Debug)]
pub struct Compositor {
    grid: SuperGrid,
    caches: Vec<LayerCache>,
    canvas: Canvas,
    resamples: u64,
}

impl Compositor {
    /// A compositor for `grid` with empty caches.
    pub fn new(grid: SuperGrid) -> Self {
        Self {
            grid,
            caches: Vec::new(),
            canvas: Canvas::new(grid),
            resamples: 0,
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
            self.grid = grid;
            self.canvas = Canvas::new(grid);
        }
    }

    /// The last composite.
    pub const fn canvas(&self) -> &Canvas {
        &self.canvas
    }

    /// How many layer rasters have been (re)built; for tests and diagnostics.
    pub const fn resamples(&self) -> u64 {
        self.resamples
    }

    /// Composites `layers` (bottom first) and returns the canvas.
    pub fn composite(&mut self, layers: &[Layer]) -> &Canvas {
        self.caches.resize_with(layers.len(), LayerCache::default);
        let grid = self.grid;
        let mut stack = Vec::with_capacity(layers.len());
        for (layer, cache) in layers.iter().zip(&mut self.caches) {
            if !layer.visible() || layer.opacity() <= 0.0 {
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
            stack.push((layer.blend(), layer.opacity(), Arc::clone(raster)));
        }
        fold(&mut self.canvas, &stack, grid.bounds());
        &self.canvas
    }
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
        let mut v = resample(m.raster().values(), sw, &mx, &my);
        for x in &mut v {
            *x = x.clamp(0.0, 1.0);
        }
        v
    });
    debug_assert_eq!(sh as usize * sw as usize, premul.len());
    LayerRaster { rect, rgba, mask }
}

/// Blends `stack` (bottom first) into `canvas` over `area`, clearing it first.
fn fold(canvas: &mut Canvas, stack: &[(Blend, f32, Arc<LayerRaster>)], area: PxRect) {
    let (w, _) = canvas.grid.size();
    let w = w as usize;
    let mut rows: Vec<(usize, &mut [[f32; 4]])> = canvas
        .px
        .chunks_mut(w)
        .enumerate()
        .filter(|(y, _)| (area.y0 as usize..area.y1 as usize).contains(y))
        .collect();
    crate::par::for_each(&mut rows, |(y, row)| {
        let y = *y as u32;
        let (x0, x1) = (area.x0 as usize, area.x1 as usize);
        for p in &mut row[x0..x1] {
            *p = [0.0; 4];
        }
        for (mode, opacity, r) in stack {
            let span = r.rect.intersect(PxRect::new(area.x0, y, area.x1, y + 1));
            if span.is_empty() {
                continue;
            }
            let rw = r.rect.width() as usize;
            let base = (y - r.rect.y0) as usize * rw;
            for x in span.x0..span.x1 {
                let i = base + (x - r.rect.x0) as usize;
                let k = opacity * r.mask.as_ref().map_or(1.0, |m| m[i]);
                let s = r.rgba[i];
                blend::blend(*mode, &mut row[x as usize], s.map(|v| v * k));
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Vec2;
    use crate::mask::{Mask, MaskRaster};
    use crate::mode::Mode;
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
