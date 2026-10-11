//! The dithering pass: one serpentine walk over the target that gives every pixel a
//! colour from its cell's set (`docs/TAD.md` §3.1, ADR-002).
//!
//! Error diffusion is clamped at cell boundaries: error that would cross into a cell
//! whose set lacks the colour the error is pulling towards is dropped, not
//! redistributed. Error lives in the metric's coordinates, flows only between pixels
//! judged by the same metric, and is scaled by the sending pixel's strength. Every pixel
//! is chosen from its own cell's set, so a cell's output can never contain a colour
//! outside it.

use super::input::{Input, Px};
use super::metric::{Metric, MetricTables};
use super::ordered;
use crate::colour::{ColourIndex, ColourSet};
use crate::layer::{DitherFamily, DitherKind};

/// An error-diffusion kernel: `(dx, dy, weight)` for neighbours ahead of the pixel in
/// scan order, and the divisor of the weights.
pub(crate) struct Kernel {
    taps: &'static [(i8, i8, u8)],
    divisor: f32,
}

/// Floyd–Steinberg (1976).
const FLOYD_STEINBERG: Kernel = Kernel {
    taps: &[(1, 0, 7), (-1, 1, 3), (0, 1, 5), (1, 1, 1)],
    divisor: 16.0,
};

/// Atkinson: diffuses 6/8 of the error, keeping highlights and shadows clean.
const ATKINSON: Kernel = Kernel {
    taps: &[
        (1, 0, 1),
        (2, 0, 1),
        (-1, 1, 1),
        (0, 1, 1),
        (1, 1, 1),
        (0, 2, 1),
    ],
    divisor: 8.0,
};

/// Jarvis, Judice and Ninke (1976).
const JARVIS_JUDICE_NINKE: Kernel = Kernel {
    taps: &[
        (1, 0, 7),
        (2, 0, 5),
        (-2, 1, 3),
        (-1, 1, 5),
        (0, 1, 7),
        (1, 1, 5),
        (2, 1, 3),
        (-2, 2, 1),
        (-1, 2, 3),
        (0, 2, 5),
        (1, 2, 3),
        (2, 2, 1),
    ],
    divisor: 48.0,
};

/// Sierra (three-row, Frankie Sierra 1989).
const SIERRA: Kernel = Kernel {
    taps: &[
        (1, 0, 5),
        (2, 0, 3),
        (-2, 1, 2),
        (-1, 1, 4),
        (0, 1, 5),
        (1, 1, 4),
        (2, 1, 2),
        (-1, 2, 2),
        (0, 2, 3),
        (1, 2, 2),
    ],
    divisor: 32.0,
};

/// The kernel of a diffusion method.
pub(crate) const fn kernel(kind: DitherKind) -> Option<&'static Kernel> {
    match kind {
        DitherKind::FloydSteinberg => Some(&FLOYD_STEINBERG),
        DitherKind::Atkinson => Some(&ATKINSON),
        DitherKind::JarvisJudiceNinke => Some(&JARVIS_JUDICE_NINKE),
        DitherKind::Sierra => Some(&SIERRA),
        _ => None,
    }
}

/// What one pass produced.
#[derive(Clone, Debug, PartialEq)]
pub struct PassResult {
    /// Row-major colour of every target pixel.
    pub out: Vec<ColourIndex>,
    /// Error each pixel received from its neighbours (its own metric's coordinates):
    /// the refinement re-picks cell sets from colour + received error.
    pub received: Vec<[f32; 3]>,
    /// Σ weight × distance(colour, output) over all pixels, the solve's error.
    pub error: f32,
}

/// Cells whose output is copied from a previous solution (locks).
#[derive(Clone, Copy, Debug)]
pub struct Locked<'a> {
    /// The locked cells.
    pub cells: crate::geom::CellSet,
    /// The previous solution's row-major pixels.
    pub previous: &'a [ColourIndex],
}

/// One dithering pass over `input` with these cell sets (each cell's own colours; `bg`
/// is added to every cell in multicolor). `layer_origins[i]` anchors layer `i`'s
/// layer-locked patterns (whole target pixels). Locked cells keep their previous pixels
/// and neither send nor receive error.
pub fn pass(
    input: &Input,
    tables: &MetricTables,
    sets: &[ColourSet],
    bg: Option<ColourIndex>,
    layer_origins: &[(i64, i64)],
    locked: Option<Locked<'_>>,
) -> PassResult {
    let (w, h) = input.size;
    let (cw, ch) = input.cell;
    let cx = input.cells.0;
    let full: Vec<Vec<ColourIndex>> = sets
        .iter()
        .map(|s| {
            let s = bg.map_or(*s, |b| s.with(b));
            let v: Vec<ColourIndex> = s.iter().collect();
            if v.is_empty() {
                vec![ColourIndex::BLACK]
            } else {
                v
            }
        })
        .collect();
    let full_sets: Vec<ColourSet> = full.iter().map(|v| v.iter().copied().collect()).collect();
    let cell_of = |x: usize, y: usize| (y / ch) * cx + x / cw;
    let is_locked = |cell: usize| match locked {
        None => false,
        Some(l) => l.cells.contains(input.cell_at(cell)),
    };
    let mut out = vec![ColourIndex::BLACK; w * h];
    let mut received = vec![[0.0f32; 3]; w * h];
    let mut error = 0.0f32;
    for y in 0..h {
        let reverse = y % 2 == 1;
        for i in 0..w {
            let x = if reverse { w - 1 - i } else { i };
            let idx = y * w + x;
            let q: &Px = &input.px[idx];
            let cell = cell_of(x, y);
            let set = &full[cell];
            let colours = tables.colours(q.metric.space());
            let family = q.kind.family();
            let target = if family == DitherFamily::Diffusion {
                add(q.coords, received[idx])
            } else {
                q.coords
            };
            if is_locked(cell) {
                let prev = locked.expect("checked").previous[idx];
                out[idx] = prev;
                error += q.weight * q.metric.distance(q.coords, colours[prev.as_usize()]);
                continue;
            }
            let choice = if q.weight <= 0.0 {
                // Uncovered: the background where there is one, else the nearest.
                bg.filter(|b| set.contains(b))
                    .unwrap_or_else(|| ordered::nearest(q.metric, q.coords, set, colours))
            } else {
                match family {
                    DitherFamily::Ordered | DitherFamily::Noise => {
                        let origin = match q.source {
                            crate::composite::AttrSource::Layer(l) => {
                                layer_origins.get(usize::from(l)).copied().unwrap_or((0, 0))
                            }
                            _ => (0, 0),
                        };
                        let (px, py) =
                            ordered::pattern_position(x as i64, y as i64, q.phase, origin);
                        let thr = ordered::threshold(q.kind, q.orientation, px, py)
                            .expect("ordered and noise methods have thresholds");
                        ordered::ordered_pick(q.metric, q.coords, set, colours, thr, q.strength)
                    }
                    _ => ordered::nearest(q.metric, target, set, colours),
                }
            };
            out[idx] = choice;
            error += q.weight * q.metric.distance(q.coords, colours[choice.as_usize()]);
            // Diffuse.
            let Some(k) = kernel(q.kind) else { continue };
            if q.weight <= 0.0 || q.strength <= 0.0 {
                continue;
            }
            let c = colours[choice.as_usize()];
            let e = [target[0] - c[0], target[1] - c[1], target[2] - c[2]];
            let scale = q.strength / k.divisor;
            for &(dx, dy, wt) in k.taps {
                let dx = if reverse {
                    -i64::from(dx)
                } else {
                    i64::from(dx)
                };
                let (nx, ny) = (x as i64 + dx, y as i64 + i64::from(dy));
                if nx < 0 || ny < 0 || nx >= w as i64 || ny >= h as i64 {
                    continue;
                }
                let (nx, ny) = (nx as usize, ny as usize);
                let n = ny * w + nx;
                let r = &input.px[n];
                if r.metric != q.metric
                    || r.kind.family() != DitherFamily::Diffusion
                    || r.weight <= 0.0
                {
                    continue;
                }
                let part = e.map(|v| v * scale * f32::from(wt));
                let ncell = cell_of(nx, ny);
                if is_locked(ncell) {
                    continue;
                }
                if ncell != cell
                    && !accepts(
                        r.metric,
                        add(add(r.coords, received[n]), part),
                        full_sets[ncell],
                        tables,
                    )
                {
                    continue;
                }
                received[n] = add(received[n], part);
            }
        }
    }
    PassResult {
        out,
        received,
        error,
    }
}

/// Where each layer's layer-locked patterns are anchored: its top-left corner in whole
/// target pixels (rounded down), so a pattern moves with the layer one target pixel at a
/// time.
#[expect(
    clippy::cast_possible_truncation,
    reason = "layer positions are far inside i64 after flooring"
)]
pub fn layer_origins(
    grid: crate::geom::SuperGrid,
    layers: &[crate::layer::Layer],
) -> Vec<(i64, i64)> {
    layers
        .iter()
        .map(|l| {
            let t = grid.canvas_to_target(l.transform().translate());
            (t.x.floor() as i64, t.y.floor() as i64)
        })
        .collect()
}

/// Whether a cell with colours `set` has the colour a pixel at `target` wants: the
/// nearest of all 16.
fn accepts(metric: Metric, target: [f32; 3], set: ColourSet, tables: &MetricTables) -> bool {
    // The nearest of all 16 (ties to the lower index) is in `set`.
    let d = tables.distances(metric, target);
    let bits = set.bits();
    let mut best = (f32::INFINITY, 16usize);
    for (i, &v) in d.iter().enumerate() {
        if v < best.0 {
            best = (v, i);
        }
    }
    bits & (1 << best.1) != 0
}

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::composite::Compositor;
    use crate::geom::SuperGrid;
    use crate::layer::{Layer, Recipe, SourceImage};
    use crate::mode::Mode;
    use crate::palette::Palette;
    use crate::solve::input::InputOptions;
    use proptest::prelude::*;
    use std::sync::Arc;

    /// A smooth diagonal gradient through every hue, solved with `kind`.
    fn gradient_input(
        mode: Mode,
        kind: DitherKind,
        strength: f32,
        seed: u32,
    ) -> (Input, MetricTables) {
        let px: Vec<[f32; 4]> = (0..320 * 200)
            .map(|i| {
                let (x, y) = ((i % 320) as f32 / 320.0, (i / 320) as f32 / 200.0);
                let s = (seed % 7) as f32 / 7.0;
                [x, (y + s) % 1.0, (1.0 - x) * y, 1.0]
            })
            .collect();
        let mut layer = Layer::new(
            "g",
            Arc::new(SourceImage::from_linear(320, 200, px).unwrap()),
        );
        layer.set_recipe(Recipe::with_method(kind, strength).unwrap());
        let mut c = Compositor::new(SuperGrid::new(mode, 1).unwrap());
        c.update(&[layer], &[]);
        let t = MetricTables::new(&Palette::pepto_pal());
        (Input::new(c.frame(), &t, InputOptions::default()), t)
    }

    /// Arbitrary but legal cell sets: three colours derived from the cell index.
    fn sets(input: &Input, salt: usize) -> Vec<ColourSet> {
        (0..input.cells.0 * input.cells.1)
            .map(|i| {
                (0..input.k())
                    .map(|j| ColourIndex::ALL[(i * 7 + j * 5 + salt) % 15 + 1])
                    .collect()
            })
            .collect()
    }

    const KERNELS: [DitherKind; 4] = [
        DitherKind::FloydSteinberg,
        DitherKind::Atkinson,
        DitherKind::JarvisJudiceNinke,
        DitherKind::Sierra,
    ];

    #[test]
    fn kernel_weights_sum_to_their_divisor_or_less() {
        for kind in KERNELS {
            let k = kernel(kind).unwrap();
            let sum: u32 = k.taps.iter().map(|t| u32::from(t.2)).sum();
            assert!(sum as f32 <= k.divisor, "{kind:?}");
            assert!(
                k.taps.iter().all(|&(dx, dy, _)| dy > 0 || dx > 0),
                "taps only ahead"
            );
        }
        assert_eq!(kernel(DitherKind::Bayer4).map(|k| k.taps.len()), None);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(12))]
        /// Clamped diffusion never leaks: every cell's output stays within its set.
        #[test]
        fn clamped_diffusion_never_leaks(k in 0usize..4, salt in 0usize..15, mc in any::<bool>(), seed in 0u32..50) {
            let mode = if mc { Mode::MulticolorBitmap } else { Mode::HiresBitmap };
            let (input, t) = gradient_input(mode, KERNELS[k], 1.0, seed);
            let s = sets(&input, salt);
            let bg = mc.then_some(ColourIndex::BLACK);
            let r = pass(&input, &t, &s, bg, &[], None);
            let (cw, ch) = input.cell;
            for (i, c) in r.out.iter().enumerate() {
                let (x, y) = (i % input.size.0, i / input.size.0);
                let cell = (y / ch) * input.cells.0 + x / cw;
                let allowed = bg.map_or(s[cell], |b| s[cell].with(b));
                prop_assert!(allowed.contains(*c), "pixel ({x}, {y}) took {c} outside {allowed}");
            }
        }
    }

    #[test]
    fn diffusion_tracks_the_mean_better_than_nearest() {
        // A flat mid grey with only black and white available: nearest gives one colour,
        // diffusion approaches the grey on average.
        let px = vec![[0.18f32, 0.18, 0.18, 1.0]; 320 * 200];
        for kind in KERNELS {
            let mut layer = Layer::new(
                "g",
                Arc::new(SourceImage::from_linear(320, 200, px.clone()).unwrap()),
            );
            layer.set_recipe(Recipe::with_method(kind, 1.0).unwrap());
            let mut c = Compositor::new(SuperGrid::new(Mode::HiresBitmap, 1).unwrap());
            c.update(&[layer], &[]);
            let t = MetricTables::new(&Palette::pepto_pal());
            let input = Input::new(c.frame(), &t, InputOptions::default());
            let bw: ColourSet = [ColourIndex::BLACK, ColourIndex::WHITE]
                .into_iter()
                .collect();
            let r = pass(&input, &t, &vec![bw; 1000], None, &[], None);
            let whites = r.out.iter().filter(|&&c| c == ColourIndex::WHITE).count() as f32;
            let frac = whites / r.out.len() as f32;
            // OKLab L of linear 0.18 is ~0.565: roughly half the pixels white.
            assert!((0.35..0.65).contains(&frac), "{kind:?}: {frac}");
        }
    }

    #[test]
    fn strength_zero_diffuses_nothing() {
        let (input, t) = gradient_input(Mode::MulticolorBitmap, DitherKind::FloydSteinberg, 0.0, 3);
        let s = sets(&input, 2);
        let r = pass(&input, &t, &s, Some(ColourIndex::BLACK), &[], None);
        assert!(r.received.iter().all(|e| *e == [0.0; 3]));
    }

    /// Left half ordered (Bayer 4), right half diffused (Floyd–Steinberg), in one pass.
    fn halves(
        left: DitherKind,
        right: DitherKind,
        mode: Mode,
    ) -> (Input, MetricTables, Vec<Layer>) {
        let ramp = |x0: u32| -> Vec<[f32; 4]> {
            (0..160 * 200)
                .map(|i| {
                    let (x, y) = ((i % 160 + x0) as f32 / 320.0, (i / 160) as f32 / 200.0);
                    [x, y, 0.5 * (x + y), 1.0]
                })
                .collect()
        };
        let mut a = Layer::new(
            "left",
            Arc::new(SourceImage::from_linear(160, 200, ramp(0)).unwrap()),
        );
        a.set_recipe(Recipe::with_method(left, 1.0).unwrap());
        let mut b = Layer::new(
            "right",
            Arc::new(SourceImage::from_linear(160, 200, ramp(160)).unwrap()),
        );
        b.set_recipe(Recipe::with_method(right, 1.0).unwrap());
        b.set_transform(
            crate::layer::Transform::translated(crate::geom::Vec2::new(160.0, 0.0)).unwrap(),
        );
        let grid = SuperGrid::new(mode, 1).unwrap();
        let mut c = Compositor::new(grid);
        let layers = vec![a, b];
        c.update(&layers, &[]);
        let t = MetricTables::new(&Palette::pepto_pal());
        (
            Input::new(c.frame(), &t, InputOptions::default()),
            t,
            layers,
        )
    }

    #[test]
    fn ordered_and_diffused_pixels_coexist_in_one_row() {
        for mode in [Mode::HiresBitmap, Mode::MulticolorBitmap] {
            let (mixed, t, layers) = halves(DitherKind::Bayer4, DitherKind::FloydSteinberg, mode);
            let (left_only, _, _) = halves(DitherKind::Bayer4, DitherKind::None, mode);
            let (right_only, _, _) = halves(DitherKind::None, DitherKind::FloydSteinberg, mode);
            let s = sets(&mixed, 3);
            let bg = (mode == Mode::MulticolorBitmap).then_some(ColourIndex::BLACK);
            let origins = layer_origins(mixed.grid(), &layers);
            let m = pass(&mixed, &t, &s, bg, &origins, None);
            let l = pass(&left_only, &t, &s, bg, &origins, None);
            let r = pass(&right_only, &t, &s, bg, &origins, None);
            let w = mixed.size.0;
            for (i, c) in m.out.iter().enumerate() {
                let want = if i % w < w / 2 { l.out[i] } else { r.out[i] };
                assert_eq!(
                    *c, want,
                    "{mode} pixel {i}: each half dithers by its own method"
                );
            }
            // And both families really are present in a single row.
            let row = &m.out[100 * w..101 * w];
            assert_ne!(row[..w / 2], row[w / 2..]);
        }
    }

    #[test]
    fn the_pass_is_deterministic() {
        let (input, t) = gradient_input(Mode::HiresBitmap, DitherKind::Sierra, 0.8, 1);
        let s = sets(&input, 4);
        assert_eq!(
            pass(&input, &t, &s, None, &[], None),
            pass(&input, &t, &s, None, &[], None)
        );
    }
}
