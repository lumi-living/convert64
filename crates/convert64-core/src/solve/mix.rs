//! Stable mix levels for ordered and noise dithers (#142).
//!
//! E5 compared each pixel's own position between its two nearest colours (in the
//! metric's space) with the pattern's threshold, so in a near-flat area the pattern
//! followed the source's micro-texture and switched on and off in blotches, and mixes
//! with dark colours came out too bright (the eye averages a pattern in linear light,
//! not in OKLab). With `mix_levels = N` an ordered pixel instead works like this:
//!
//! 1. It finds its nearest colour of its cell's set (in its metric) and the partner
//!    whose linear-light mix with it reproduces the pixel best (projection onto the
//!    segment between them in linear RGB, scored in the metric plus
//!    [`SPREAD_PENALTY`]). The pair is ordered by palette index (`lo`, `hi`); the
//!    pixel's *ratio* is the share of `hi`. If no mix beats the nearest colour alone,
//!    the pixel shows that colour.
//! 2. Per cell and pair, pixels within half a level step of the pair's median ratio are
//!    *flat*; their mean is the cell's ratio. Other pixels (edges, texture, highlights)
//!    keep their own continuous ratio, so detail is dithered as in E5.
//! 3. Hysteresis across neighbouring cells: what the eight surrounding cells show of the
//!    same two colours (a neighbour pixel showing only `lo` counts 0, only `hi` 1, a
//!    `lo`/`hi` mix its share) is averaged. If the cell's ratio is within half a level
//!    step of that, the cell takes the neighbours' ratio. The result is quantised to one of
//!    `N + 1` evenly spaced levels and every flat pixel of the pair takes it, so a flat
//!    area gets one calm pattern, the way pixel artists pick one pattern per area.
//!
//! A pixel takes `hi` where its level exceeds the pattern threshold, pulled towards ½
//! by strength as in E5 (strength 0 is nearest-in-set). Everything depends
//! only on the input and the cell sets, cell by cell, so the plan is deterministic and
//! parallel.

use super::input::{Input, Px};
use super::metric::MetricTables;
use super::ordered;
use crate::colour::ColourIndex;
use crate::layer::recipe::MAX_LUMA_DIFF_OFF;
use crate::palette::luma;

/// What an ordered pixel mixes: its two colours (by palette index) and its level, the
/// share of `hi` in `0..=1`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mix {
    /// The lower-index colour.
    pub lo: ColourIndex,
    /// The higher-index colour.
    pub hi: ColourIndex,
    /// Share of `hi`: the cell's quantised level for flat pixels, the pixel's own ratio
    /// otherwise.
    pub level: f32,
    /// The pixel's distance (its metric) to the linear-light mix of `lo` and `hi` at
    /// `level`: its error as the eye sees the pattern.
    pub error: f32,
}

impl Mix {
    /// The colour a pixel with pattern threshold `thr` (`0 < thr < 1`) takes.
    pub fn pick(self, thr: f32) -> ColourIndex {
        if self.level > thr { self.hi } else { self.lo }
    }
}

/// The colours a pixel may mix: its cell's set, restricted by `max_luma_diff` to those
/// within N luma levels of its nearest colour.
pub(crate) fn mix_colours(
    q: &Px,
    set: &[ColourIndex],
    colours: &[[f32; 3]; 16],
) -> ([ColourIndex; 16], usize) {
    let mut out = [ColourIndex::BLACK; 16];
    let mut n = 0;
    let c0 = (q.max_luma_diff < MAX_LUMA_DIFF_OFF)
        .then(|| ordered::nearest(q.metric, q.coords, set, colours));
    for &c in set {
        if c0.is_none_or(|c0| luma::level_diff(c, c0) <= q.max_luma_diff) {
            out[n] = c;
            n += 1;
        }
    }
    (out, n)
}

/// What mixing far-apart colours costs, per unit of `t(1 − t) × distance(lo, hi)`
/// (Yliluoma's psychovisual penalty): a pixel mixes only if the mix, plus this, is
/// closer than its nearest colour. Without it a pale sky next to a dark background
/// colour gets dark dots that match it on average but read as dirt. 0.25 is the value
/// at which all six golden fixtures beat png2c64's blurred error (#142).
pub const SPREAD_PENALTY: f32 = 0.25;

/// Quantises `t` to `n` steps.
fn quantise(t: f32, n: u8) -> f32 {
    let n = f32::from(n);
    (t * n).round() / n
}

/// Half a level step: how far from its pair's median a pixel may be and still be flat.
fn half_step(n: u8) -> f32 {
    0.5 / f32::from(n)
}

/// One pixel after step 1.
#[derive(Clone, Copy, Debug)]
enum Seen {
    /// Not an ordered pixel with levels (or uncovered): ignored.
    Skip,
    /// Shows a single colour whatever the pattern.
    Solid(ColourIndex),
    /// Mixes `lo` and `hi` at ratio `t`, quantised to `n` levels where flat.
    Pair {
        lo: ColourIndex,
        hi: ColourIndex,
        t: f32,
        n: u8,
    },
}

/// Step 1 for every pixel of cell `ci`; `linear` is the palette in linear RGB.
fn see_cell(
    input: &Input,
    tables: &MetricTables,
    linear: &[[f32; 3]; 16],
    ci: usize,
    set: &[ColourIndex],
    locked: Option<&[ColourIndex]>,
) -> Vec<Seen> {
    let w = input.size.0;
    input
        .pixels_of(ci)
        .map(|(x, y)| {
            let idx = y * w + x;
            if let Some(prev) = locked {
                return Seen::Solid(prev[idx]);
            }
            let q = &input.px[idx];
            if q.weight <= 0.0
                || q.mix_levels == 0
                || q.semantics != crate::layer::OrderedSemantics::Legacy
                || !q.kind.family().is_threshold()
            {
                return Seen::Skip;
            }
            let colours = tables.colours(q.metric.space());
            let (mix, k) = mix_colours(q, set, colours);
            let s = (q.strength * q.adapt).clamp(0.0, 1.0);
            let c1 = ordered::nearest(q.metric, q.coords, &mix[..k], colours);
            if k < 2 || s <= 0.0 {
                return Seen::Solid(c1);
            }
            // The partner: the colour whose linear-light mix with the nearest one best
            // reproduces the pixel. A mix far from the pixel (a hue the pair cannot
            // make) or of far-apart colours (speckle) must still beat the nearest colour
            // alone, or the pixel shows that.
            let alone = q.metric.distance(q.coords, colours[c1.as_usize()]);
            let mut best: Option<(f32, ColourIndex, f32)> = None;
            for &c2 in mix[..k].iter().filter(|&&c| c != c1) {
                let (a, b) = (linear[c1.as_usize()], linear[c2.as_usize()]);
                let seg = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
                let len2 = seg[0] * seg[0] + seg[1] * seg[1] + seg[2] * seg[2];
                if len2 <= 0.0 {
                    continue;
                }
                let d = [q.lin[0] - a[0], q.lin[1] - a[1], q.lin[2] - a[2]];
                let u = ((d[0] * seg[0] + d[1] * seg[1] + d[2] * seg[2]) / len2).clamp(0.0, 1.0);
                if u <= 0.0 {
                    continue;
                }
                let m = [0, 1, 2].map(|i| a[i] + u * seg[i]);
                let spread = q
                    .metric
                    .distance(colours[c1.as_usize()], colours[c2.as_usize()]);
                let score = q.metric.distance(q.coords, q.metric.coords(m))
                    + SPREAD_PENALTY * u * (1.0 - u) * spread;
                if score < alone && best.is_none_or(|b| score < b.0) {
                    best = Some((score, c2, u));
                }
            }
            let Some((_, c2, u)) = best else {
                return Seen::Solid(c1);
            };
            // `u` is the share of `c2`; the pair is stored by palette index.
            let (lo, hi, t) = if c1 < c2 {
                (c1, c2, u)
            } else {
                (c2, c1, 1.0 - u)
            };
            Seen::Pair {
                lo,
                hi,
                t,
                n: q.mix_levels,
            }
        })
        .collect()
}

/// A cell's pair after step 2.
#[derive(Clone, Copy, Debug)]
struct Pair {
    lo: ColourIndex,
    hi: ColourIndex,
    /// Median ratio.
    median: f32,
    /// Mean ratio of the flat pixels.
    mean: f32,
    /// Levels (the first flat pixel's `mix_levels`).
    n: u8,
}

impl Pair {
    fn is_flat(&self, t: f32) -> bool {
        (t - self.median).abs() <= half_step(self.n)
    }
}

/// Step 2 for one cell.
fn pairs_of(seen: &[Seen]) -> Vec<Pair> {
    let mut pairs: Vec<Pair> = Vec::new();
    for s in seen {
        if let Seen::Pair { lo, hi, n, .. } = *s
            && !pairs.iter().any(|p| p.lo == lo && p.hi == hi)
        {
            let mut ts: Vec<f32> = seen
                .iter()
                .filter_map(|s| match *s {
                    Seen::Pair {
                        lo: l, hi: h, t, ..
                    } if l == lo && h == hi => Some(t),
                    _ => None,
                })
                .collect();
            ts.sort_by(f32::total_cmp);
            let median = ts[ts.len() / 2];
            let flat: Vec<f32> = ts
                .into_iter()
                .filter(|t| (t - median).abs() <= half_step(n))
                .collect();
            let mean = flat.iter().sum::<f32>() / flat.len() as f32;
            pairs.push(Pair {
                lo,
                hi,
                median,
                mean,
                n,
            });
        }
    }
    pairs
}

/// What pixel `s` of a neighbouring cell (whose pairs are `cell`) shows of `lo` and
/// `hi`: 0 (only `lo`) to 1 (only `hi`), or `None` if it shows another colour.
fn shows(s: Seen, lo: ColourIndex, hi: ColourIndex, cell: &[Pair]) -> Option<f32> {
    let solid = |c: ColourIndex| {
        if c == lo {
            Some(0.0)
        } else if c == hi {
            Some(1.0)
        } else {
            None
        }
    };
    match s {
        Seen::Skip => None,
        Seen::Solid(c) => solid(c),
        Seen::Pair {
            lo: l, hi: h, t, ..
        } => {
            let share = cell
                .iter()
                .find(|p| p.lo == l && p.hi == h && p.is_flat(t))
                .map_or(t, |p| quantise(p.mean, p.n));
            if share <= 0.0 {
                solid(l)
            } else if share >= 1.0 {
                solid(h)
            } else if l == lo && h == hi {
                Some(share)
            } else {
                None
            }
        }
    }
}

/// Every pixel's mix (row-major), `None` where the pixel is not an ordered pixel with
/// mix levels or shows a single colour (it then dithers as in E5). `sets` are the
/// cells' full colour lists (background included); `locked` cells show their previous
/// pixels.
pub(crate) fn plan(
    input: &Input,
    tables: &MetricTables,
    sets: &[Vec<ColourIndex>],
    locked: Option<super::dither::Locked<'_>>,
) -> Vec<Option<Mix>> {
    let (w, h) = input.size;
    let (cx, cy) = input.cells;
    let wants = input.px.iter().any(|q| {
        q.mix_levels > 0
            && q.semantics == crate::layer::OrderedSemantics::Legacy
            && q.kind.family().is_threshold()
    });
    if !wants {
        return vec![None; w * h];
    }
    let linear: [[f32; 3]; 16] = ColourIndex::ALL.map(|c| {
        let l = tables.palette().linear(c);
        [l.r, l.g, l.b]
    });
    let cells: Vec<usize> = (0..cx * cy).collect();
    let prev = |ci: usize| {
        locked
            .filter(|l| l.cells.contains(input.cell_at(ci)))
            .map(|l| l.previous)
    };
    let seen: Vec<Vec<Seen>> = crate::par::map(&cells, |&ci| {
        see_cell(input, tables, &linear, ci, &sets[ci], prev(ci))
    });
    let pairs: Vec<Vec<Pair>> = crate::par::map(&cells, |&ci| pairs_of(&seen[ci]));
    let mixes: Vec<Vec<Option<Mix>>> = crate::par::map(&cells, |&ci| {
        let (gx, gy) = ((ci % cx) as i64, (ci / cx) as i64);
        // Step 3: each pair's level, with hysteresis towards what the neighbours show.
        let levels: Vec<f32> = pairs[ci]
            .iter()
            .map(|p| {
                let (mut sum, mut count) = (0.0f32, 0usize);
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        let (nx, ny) = (gx + dx, gy + dy);
                        if (dx, dy) == (0, 0)
                            || nx < 0
                            || ny < 0
                            || nx >= cx as i64
                            || ny >= cy as i64
                        {
                            continue;
                        }
                        let nc = ny as usize * cx + nx as usize;
                        for s in &seen[nc] {
                            if let Some(v) = shows(*s, p.lo, p.hi, &pairs[nc]) {
                                sum += v;
                                count += 1;
                            }
                        }
                    }
                }

                let around = sum / count.max(1) as f32;
                let ratio = if count > 0 && (p.mean - around).abs() < half_step(p.n) {
                    around
                } else {
                    p.mean
                };
                quantise(ratio, p.n)
            })
            .collect();
        seen[ci]
            .iter()
            .zip(input.pixels_of(ci))
            .map(|(s, (x, y))| match *s {
                Seen::Pair { lo, hi, t, .. } => {
                    let (k, p) = pairs[ci]
                        .iter()
                        .enumerate()
                        .find(|(_, p)| p.lo == lo && p.hi == hi)
                        .expect("every pair was collected");
                    let level = if p.is_flat(t) { levels[k] } else { t };
                    let q = &input.px[y * w + x];
                    let shown = ordered::shown_share(level, q.kind, q.strength * q.adapt);
                    let (a, b) = (linear[lo.as_usize()], linear[hi.as_usize()]);
                    let m = [0, 1, 2].map(|i| a[i] + shown * (b[i] - a[i]));
                    let error = q.metric.distance(q.coords, q.metric.coords(m));
                    Some(Mix {
                        lo,
                        hi,
                        level,
                        error,
                    })
                }
                _ => None,
            })
            .collect()
    });
    let mut out = vec![None; w * h];
    for (ci, m) in mixes.into_iter().enumerate() {
        for (v, (x, y)) in m.into_iter().zip(input.pixels_of(ci)) {
            out[y * w + x] = v;
        }
    }
    out
}
