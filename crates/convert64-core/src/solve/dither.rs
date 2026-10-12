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
use crate::palette::luma;

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

/// "False" Floyd–Steinberg: 3/8 right, 3/8 down, 2/8 down-right.
const FALSE_FS: Kernel = Kernel {
    taps: &[(1, 0, 3), (0, 1, 3), (1, 1, 2)],
    divisor: 8.0,
};

/// png2c64's Floyd–Steinberg for 2:1 pixels: weight moved from the wide horizontal
/// neighbour to the one below.
const FS_WIDE: Kernel = Kernel {
    taps: &[(1, 0, 5), (-1, 1, 3), (0, 1, 6), (1, 1, 2)],
    divisor: 16.0,
};

/// png2c64's line-biased Floyd–Steinberg: nothing to the right, so the pixels of a row
/// dither independently and rows stay coherent.
const LINE_FS: Kernel = Kernel {
    taps: &[(-1, 1, 2), (0, 1, 3), (1, 1, 2), (0, 2, 1)],
    divisor: 8.0,
};

/// Stucki (1981).
const STUCKI: Kernel = Kernel {
    taps: &[
        (1, 0, 8),
        (2, 0, 4),
        (-2, 1, 2),
        (-1, 1, 4),
        (0, 1, 8),
        (1, 1, 4),
        (2, 1, 2),
        (-2, 2, 1),
        (-1, 2, 2),
        (0, 2, 4),
        (1, 2, 2),
        (2, 2, 1),
    ],
    divisor: 42.0,
};

/// Burkes (1988): Stucki's first two rows.
const BURKES: Kernel = Kernel {
    taps: &[
        (1, 0, 8),
        (2, 0, 4),
        (-2, 1, 2),
        (-1, 1, 4),
        (0, 1, 8),
        (1, 1, 4),
        (2, 1, 2),
    ],
    divisor: 32.0,
};

/// Two-row Sierra.
const SIERRA2: Kernel = Kernel {
    taps: &[
        (1, 0, 4),
        (2, 0, 3),
        (-2, 1, 1),
        (-1, 1, 2),
        (0, 1, 3),
        (1, 1, 2),
        (2, 1, 1),
    ],
    divisor: 16.0,
};

/// Sierra Lite.
const SIERRA_LITE: Kernel = Kernel {
    taps: &[(1, 0, 2), (-1, 1, 1), (0, 1, 1)],
    divisor: 4.0,
};

/// Stevenson–Arce (1985), weights for a hexagonal grid laid on the square one.
const STEVENSON_ARCE: Kernel = Kernel {
    taps: &[
        (2, 0, 32),
        (-3, 1, 12),
        (-1, 1, 26),
        (1, 1, 30),
        (3, 1, 16),
        (-2, 2, 12),
        (0, 2, 26),
        (2, 2, 12),
        (-3, 3, 5),
        (-1, 3, 12),
        (1, 3, 12),
        (3, 3, 5),
    ],
    divisor: 200.0,
};

/// Shiau–Fan (1996), four taps.
const SHIAU_FAN1: Kernel = Kernel {
    taps: &[(1, 0, 4), (-2, 1, 1), (-1, 1, 1), (0, 1, 2)],
    divisor: 8.0,
};

/// Shiau–Fan (1996), five taps.
const SHIAU_FAN2: Kernel = Kernel {
    taps: &[(1, 0, 8), (-3, 1, 1), (-2, 1, 1), (-1, 1, 2), (0, 1, 4)],
    divisor: 16.0,
};

/// The kernel of a diffusion method; Riemersma has none (it walks a Hilbert curve).
pub(crate) const fn kernel(kind: DitherKind) -> Option<&'static Kernel> {
    match kind {
        DitherKind::FloydSteinberg | DitherKind::Ostromoukhov | DitherKind::ZhouFang => {
            Some(&FLOYD_STEINBERG)
        }
        DitherKind::Atkinson => Some(&ATKINSON),
        DitherKind::JarvisJudiceNinke => Some(&JARVIS_JUDICE_NINKE),
        DitherKind::Sierra => Some(&SIERRA),
        DitherKind::FalseFs => Some(&FALSE_FS),
        DitherKind::FsWide => Some(&FS_WIDE),
        DitherKind::LineFs => Some(&LINE_FS),
        DitherKind::Stucki => Some(&STUCKI),
        DitherKind::Burkes => Some(&BURKES),
        DitherKind::Sierra2 => Some(&SIERRA2),
        DitherKind::SierraLite => Some(&SIERRA_LITE),
        DitherKind::StevensonArce => Some(&STEVENSON_ARCE),
        DitherKind::ShiauFan1 => Some(&SHIAU_FAN1),
        DitherKind::ShiauFan2 => Some(&SHIAU_FAN2),
        _ => None,
    }
}

/// How uncertain a pixel's choice is: `√d₁ / (√d₁ + √d₂)` for its distances to the
/// nearest and second-nearest colour, 0 (certain) to ½ (a tie). png2c64's Ostromoukhov
/// scales the kernel by `0.6 + 0.8 ×` this, so uncertain pixels diffuse more.
fn uncertainty(d1: f32, d2: f32) -> f32 {
    let (a, b) = (d1.sqrt(), d2.sqrt());
    if a + b > 1e-6 { a / (a + b) } else { 0.0 }
}

/// Zhou–Fang threshold modulation: the choice between the two nearest colours moves
/// from ½ by `(blue noise − ½) ×` this.
pub const ZHOU_FANG_MODULATION: f32 = 0.5;

/// The 16 history weights of Riemersma's method (`16^(i/15)` rounded, oldest first),
/// applied over the largest. The history holds each pixel's error against its own
/// colour (not against colour plus received error), so the sum is a weighted moving
/// sum that keeps the walk's recent error near zero, as in Riemersma's reference code.
const RIEMERSMA_WEIGHTS: [f32; 16] = [
    1.0, 1.0, 1.0, 2.0, 2.0, 3.0, 3.0, 4.0, 4.0, 5.0, 6.0, 8.0, 9.0, 11.0, 13.0, 16.0,
];

/// Point `d` of the Hilbert curve filling an `n × n` square (`n` a power of two).
fn hilbert(n: usize, d: usize) -> (usize, usize) {
    let (mut x, mut y, mut t) = (0, 0, d);
    let mut s = 1;
    while s < n {
        let rx = 1 & (t / 2);
        let ry = 1 & (t ^ rx);
        if ry == 0 {
            if rx == 1 {
                x = s - 1 - x;
                y = s - 1 - y;
            }
            std::mem::swap(&mut x, &mut y);
        }
        x += s * rx;
        y += s * ry;
        t /= 4;
        s *= 2;
    }
    (x, y)
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
    /// The same with each pixel dithered with mix levels measured against the
    /// linear-light mix it is part of ([`super::mix::Mix::error`]), which is what the
    /// eye sees of the pattern: finalists and refinement rounds are ranked by it (#142).
    pub seen: f32,
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
    // Stable mix levels of ordered pixels (#142), planned cell by cell up front.
    let mixes = super::mix::plan(input, tables, &full, locked);
    let linear: [[f32; 3]; 16] = ColourIndex::ALL.map(|c| {
        let l = tables.palette().linear(c);
        [l.r, l.g, l.b]
    });
    let mut out = vec![ColourIndex::BLACK; w * h];
    let mut received = vec![[0.0f32; 3]; w * h];
    let mut error = 0.0f32;
    let mut seen = 0.0f32;
    // Riemersma pixels walk a Hilbert curve first, carrying their own error history;
    // they neither send error to nor receive it from the raster walk.
    if input.px.iter().any(|q| q.kind == DitherKind::Riemersma) {
        let n = w.max(h).next_power_of_two();
        let mut history = [[0.0f32; 3]; 16];
        for d in 0..n * n {
            let (x, y) = hilbert(n, d);
            if x >= w || y >= h {
                continue;
            }
            let idx = y * w + x;
            let q: &Px = &input.px[idx];
            if q.kind != DitherKind::Riemersma {
                continue;
            }
            let cell = cell_of(x, y);
            let set = &full[cell];
            let colours = tables.colours(q.metric.space());
            if is_locked(cell) || q.weight <= 0.0 {
                let choice = if is_locked(cell) {
                    locked.expect("checked").previous[idx]
                } else {
                    bg.filter(|b| set.contains(b))
                        .unwrap_or_else(|| ordered::nearest(q.metric, q.coords, set, colours))
                };
                out[idx] = choice;
                let d = q.weight * q.metric.distance(q.coords, colours[choice.as_usize()]);
                error += d;
                seen += d;
                continue;
            }
            let mut e = [0.0f32; 3];
            for (h, wt) in history.iter().zip(RIEMERSMA_WEIGHTS) {
                for k in 0..3 {
                    e[k] += h[k] * wt / 16.0;
                }
            }
            let e = e.map(|v| v * q.adapt);
            let mut target = add(q.coords, e);
            if q.cell_clamp && !accepts(q.metric, target, full_sets[cell], tables) {
                target = q.coords;
            } else {
                received[idx] = e;
            }
            let (mix, k) = super::mix::mix_colours(q, set, colours);
            let choice = ordered::nearest(q.metric, target, &mix[..k], colours);
            out[idx] = choice;
            let c = colours[choice.as_usize()];
            let d = q.weight * q.metric.distance(q.coords, c);
            error += d;
            seen += d;
            history.rotate_left(1);
            history[15] = [0, 1, 2].map(|k| (q.coords[k] - c[k]) * q.strength);
        }
    }
    for y in 0..h {
        let reverse = y % 2 == 1;
        for i in 0..w {
            let x = if reverse { w - 1 - i } else { i };
            let idx = y * w + x;
            let q: &Px = &input.px[idx];
            if q.kind == DitherKind::Riemersma {
                continue;
            }
            let cell = cell_of(x, y);
            let set = &full[cell];
            let colours = tables.colours(q.metric.space());
            let family = q.kind.family();
            // `adaptive` scales the incoming error (1 when off, exactly).
            let target = if family == DitherFamily::Diffusion {
                let e = received[idx];
                add(q.coords, [e[0] * q.adapt, e[1] * q.adapt, e[2] * q.adapt])
            } else {
                q.coords
            };
            if is_locked(cell) {
                let prev = locked.expect("checked").previous[idx];
                out[idx] = prev;
                let d = q.weight * q.metric.distance(q.coords, colours[prev.as_usize()]);
                error += d;
                seen += d;
                continue;
            }
            // max_luma_diff: dither only between colours within N luma levels of the
            // pixel's own nearest colour (8 restricts nothing and skips this).
            let restricted;
            let mix: &[ColourIndex] = if q.max_luma_diff < crate::layer::recipe::MAX_LUMA_DIFF_OFF {
                let c0 = ordered::nearest(q.metric, q.coords, set, colours);
                restricted = set
                    .iter()
                    .copied()
                    .filter(|&c| luma::level_diff(c, c0) <= q.max_luma_diff)
                    .collect::<Vec<_>>();
                &restricted
            } else {
                set
            };
            let origin = match q.source {
                crate::composite::AttrSource::Layer(l) => {
                    layer_origins.get(usize::from(l)).copied().unwrap_or((0, 0))
                }
                _ => (0, 0),
            };
            let (px, py) = ordered::pattern_position(x as i64, y as i64, q.phase, origin);
            // What a palette-aware pixel's pattern averages to (linear RGB).
            let mut aware_mix = None;
            let choice = if q.weight <= 0.0 {
                // Uncovered: the background where there is one, else the nearest.
                bg.filter(|b| set.contains(b))
                    .unwrap_or_else(|| ordered::nearest(q.metric, q.coords, set, colours))
            } else {
                match family {
                    DitherFamily::PaletteAware => {
                        let thr = ordered::threshold(DitherKind::Bayer8, q.orientation, px, py)
                            .expect("Bayer 8 has thresholds");
                        let ctx = super::aware::Ctx {
                            set: mix,
                            colours,
                            linear: &linear,
                            oklab: tables.colours(super::metric::Space::OkLab),
                            thr: 0.5 + (thr - 0.5) * (q.strength * q.adapt).min(1.0),
                            x,
                            y,
                            wide: input.has_background(),
                            pair_bias: input.pair_bias,
                        };
                        let (c, m) = super::aware::pick(q, &ctx);
                        aware_mix = Some(m);
                        c
                    }
                    f if f.is_threshold() => {
                        let thr = match q.custom {
                            0 => ordered::threshold(q.kind, q.orientation, px, py)
                                .expect("threshold methods have thresholds"),
                            c => ordered::custom_threshold(
                                &input.customs[usize::from(c) - 1],
                                q.orientation,
                                px,
                                py,
                            ),
                        };
                        match mixes[idx] {
                            Some(m) => m.pick(0.5 + (thr - 0.5) * (q.strength * q.adapt).min(1.0)),
                            None => ordered::ordered_pick(
                                q.metric,
                                q.coords,
                                mix,
                                colours,
                                thr,
                                q.strength * q.adapt,
                            ),
                        }
                    }
                    DitherFamily::Diffusion if q.kind == DitherKind::ZhouFang => {
                        let (c1, c2) = ordered::nearest_two(q.metric, target, mix, colours);
                        let t = uncertainty(
                            q.metric.distance(target, colours[c1.as_usize()]),
                            q.metric.distance(target, colours[c2.as_usize()]),
                        );
                        let bn = ordered::threshold(
                            DitherKind::BlueNoise,
                            crate::layer::Orientation::Auto,
                            x as i64,
                            y as i64,
                        )
                        .expect("blue noise has thresholds");
                        if t > 0.5 + (bn - 0.5) * ZHOU_FANG_MODULATION {
                            c2
                        } else {
                            c1
                        }
                    }
                    DitherFamily::Diffusion => ordered::nearest(q.metric, target, mix, colours),
                    _ => ordered::nearest(q.metric, target, set, colours),
                }
            };
            out[idx] = choice;
            let d = q.weight * q.metric.distance(q.coords, colours[choice.as_usize()]);
            error += d;
            seen += match (mixes[idx], aware_mix) {
                (Some(m), _) => q.weight * m.error,
                (None, Some(m)) => {
                    q.weight
                        * q.metric
                            .distance(q.coords, q.metric.coords(m.map(|v| v.max(0.0))))
                }
                (None, None) => d,
            };
            // Diffuse.
            let Some(k) = kernel(q.kind) else { continue };
            if q.weight <= 0.0 || q.strength <= 0.0 {
                continue;
            }
            let c = colours[choice.as_usize()];
            let e = [target[0] - c[0], target[1] - c[1], target[2] - c[2]];
            // Ostromoukhov and Zhou–Fang diffuse more where the choice was uncertain.
            let variable = if matches!(q.kind, DitherKind::Ostromoukhov | DitherKind::ZhouFang) {
                let (c1, c2) = ordered::nearest_two(q.metric, target, mix, colours);
                0.6 + 0.8
                    * uncertainty(
                        q.metric.distance(target, colours[c1.as_usize()]),
                        q.metric.distance(target, colours[c2.as_usize()]),
                    )
            } else {
                1.0
            };
            let scale = q.strength * variable / k.divisor;
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
                    || r.kind == DitherKind::Riemersma
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
                    && q.cell_clamp
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
        seen,
    }
}

/// What every diffusion pixel would become in a serpentine diffusion of the whole frame
/// against all 16 colours, with no cells (`None` for other pixels): the colours its
/// dither wants, which the cell-set search keeps (png2c64's global-FS palette
/// coherence). Each pixel uses its own kernel (Riemersma uses Floyd–Steinberg here) and
/// strength; error flows only between diffusion pixels of the same metric.
pub(crate) fn unconstrained(
    px: &[Px],
    tables: &MetricTables,
    w: usize,
    h: usize,
) -> Vec<Option<ColourIndex>> {
    let mut out = vec![None; w * h];
    if !px
        .iter()
        .any(|q| q.kind.family() == DitherFamily::Diffusion)
    {
        return out;
    }
    let mut received = vec![[0.0f32; 3]; w * h];
    for y in 0..h {
        let reverse = y % 2 == 1;
        for i in 0..w {
            let x = if reverse { w - 1 - i } else { i };
            let idx = y * w + x;
            let q = &px[idx];
            if q.kind.family() != DitherFamily::Diffusion || q.weight <= 0.0 {
                continue;
            }
            let colours = tables.colours(q.metric.space());
            let e = received[idx];
            let target = add(q.coords, [e[0] * q.adapt, e[1] * q.adapt, e[2] * q.adapt]);
            let d = tables.distances(q.metric, target);
            let best = (0..16)
                .min_by(|&a, &b| d[a].total_cmp(&d[b]))
                .expect("16 colours");
            out[idx] = Some(ColourIndex::ALL[best]);
            let k = kernel(q.kind).unwrap_or(&FLOYD_STEINBERG);
            let c = colours[best];
            let err = [target[0] - c[0], target[1] - c[1], target[2] - c[2]];
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
                let n = ny as usize * w + nx as usize;
                let r = &px[n];
                if r.metric != q.metric || r.kind.family() != DitherFamily::Diffusion {
                    continue;
                }
                received[n] = add(received[n], err.map(|v| v * scale * f32::from(wt)));
            }
        }
    }
    out
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

    /// Every diffusion method with a kernel.
    const KERNELS: [DitherKind; 16] = [
        DitherKind::FloydSteinberg,
        DitherKind::Atkinson,
        DitherKind::JarvisJudiceNinke,
        DitherKind::Sierra,
        DitherKind::FalseFs,
        DitherKind::FsWide,
        DitherKind::LineFs,
        DitherKind::Stucki,
        DitherKind::Burkes,
        DitherKind::Sierra2,
        DitherKind::SierraLite,
        DitherKind::StevensonArce,
        DitherKind::ShiauFan1,
        DitherKind::ShiauFan2,
        DitherKind::Ostromoukhov,
        DitherKind::ZhouFang,
    ];

    /// Every diffusion method: the kernels and Riemersma.
    const DIFFUSERS: [DitherKind; 17] = [
        DitherKind::FloydSteinberg,
        DitherKind::Atkinson,
        DitherKind::JarvisJudiceNinke,
        DitherKind::Sierra,
        DitherKind::FalseFs,
        DitherKind::FsWide,
        DitherKind::LineFs,
        DitherKind::Stucki,
        DitherKind::Burkes,
        DitherKind::Sierra2,
        DitherKind::SierraLite,
        DitherKind::StevensonArce,
        DitherKind::ShiauFan1,
        DitherKind::ShiauFan2,
        DitherKind::Ostromoukhov,
        DitherKind::ZhouFang,
        DitherKind::Riemersma,
    ];

    #[test]
    fn kernel_weights_sum_to_their_divisor_or_less() {
        for kind in DitherKind::ALL {
            let has = kernel(kind).is_some();
            let diffusion = kind.family() == DitherFamily::Diffusion;
            assert_eq!(has, diffusion && kind != DitherKind::Riemersma, "{kind:?}");
            assert_eq!(KERNELS.contains(&kind), has, "{kind:?}");
            assert_eq!(DIFFUSERS.contains(&kind), diffusion, "{kind:?}");
        }
        for kind in KERNELS {
            let k = kernel(kind).unwrap();
            let sum: u32 = k.taps.iter().map(|t| u32::from(t.2)).sum();
            assert!(sum as f32 <= k.divisor, "{kind:?}");
            // Only Atkinson keeps some error back.
            assert_eq!(
                sum as f32 == k.divisor,
                kind != DitherKind::Atkinson,
                "{kind:?}"
            );
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
        fn clamped_diffusion_never_leaks(k in 0usize..17, salt in 0usize..15, mc in any::<bool>(), seed in 0u32..50) {
            let mode = if mc { Mode::MulticolorBitmap } else { Mode::HiresBitmap };
            let (input, t) = gradient_input(mode, DIFFUSERS[k], 1.0, seed);
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
        for kind in DIFFUSERS {
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
        for kind in [
            DitherKind::Sierra,
            DitherKind::ZhouFang,
            DitherKind::Riemersma,
        ] {
            let (input, t) = gradient_input(Mode::HiresBitmap, kind, 0.8, 1);
            let s = sets(&input, 4);
            assert_eq!(
                pass(&input, &t, &s, None, &[], None),
                pass(&input, &t, &s, None, &[], None),
                "{kind:?}"
            );
        }
    }

    #[test]
    fn the_hilbert_curve_visits_every_cell_once_in_steps_of_one() {
        for n in [1, 2, 8, 256] {
            let mut seen = vec![false; n * n];
            let mut last: Option<(usize, usize)> = None;
            for d in 0..n * n {
                let (x, y) = hilbert(n, d);
                assert!(!seen[y * n + x], "{n}: ({x}, {y}) twice");
                seen[y * n + x] = true;
                if let Some((lx, ly)) = last {
                    assert_eq!(lx.abs_diff(x) + ly.abs_diff(y), 1, "{n}: jump at {d}");
                }
                last = Some((x, y));
            }
        }
    }

    #[test]
    fn riemersma_and_raster_diffusion_do_not_trade_error() {
        // Left half Riemersma, right half Floyd–Steinberg, each against its own grey:
        // a half's output is the same as when the other half uses the other method.
        let solve = |left: DitherKind, right: DitherKind| {
            let px = vec![[0.18f32, 0.18, 0.18, 1.0]; 160 * 200];
            let mk = |kind| {
                let mut l = Layer::new(
                    "g",
                    Arc::new(SourceImage::from_linear(160, 200, px.clone()).unwrap()),
                );
                l.set_recipe(Recipe::with_method(kind, 1.0).unwrap());
                l
            };
            let mut a = mk(left);
            let mut b = mk(right);
            a.set_transform(crate::layer::Transform::default());
            b.set_transform(
                crate::layer::Transform::new(
                    crate::geom::Vec2::new(160.0, 0.0),
                    crate::geom::Vec2::new(1.0, 1.0),
                    false,
                    false,
                )
                .unwrap(),
            );
            let mut c = Compositor::new(SuperGrid::new(Mode::HiresBitmap, 1).unwrap());
            c.update(&[a, b], &[]);
            let t = MetricTables::new(&Palette::pepto_pal());
            let input = Input::new(c.frame(), &t, InputOptions::default());
            let bw: ColourSet = [ColourIndex::BLACK, ColourIndex::WHITE]
                .into_iter()
                .collect();
            pass(&input, &t, &vec![bw; 1000], None, &[], None).out
        };
        // Each half equals the same half next to a half that does not diffuse at all.
        let mixed = solve(DitherKind::Riemersma, DitherKind::FloydSteinberg);
        let r_alone = solve(DitherKind::Riemersma, DitherKind::None);
        let fs_alone = solve(DitherKind::None, DitherKind::FloydSteinberg);
        for y in 0..200 {
            for x in 0..320 {
                let i = y * 320 + x;
                let want = if x < 160 { r_alone[i] } else { fs_alone[i] };
                assert_eq!(mixed[i], want, "({x}, {y})");
            }
        }
    }
}
