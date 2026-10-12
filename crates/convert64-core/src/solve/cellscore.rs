//! The `blur` and `ssim` cell metrics (`docs/reference/dither-methods.md` §6, #120).
//!
//! Scoring every one of the 455 multicolor sets with a blur or SSIM is far slower than
//! the min-and-sum the squared-distance search runs on, so these metrics re-pick among a
//! short list: the squared-distance winner's colours and the cell's five most wanted
//! colours (its pixels' nearest, by weight), plus the required ones. With at most eight
//! colours that is at most 56 multicolor sets or 28 hi-res pairs per cell. The search
//! over all 16 backgrounds stays on squared distances; only the finalists' cells are
//! re-picked, and refinement re-picks with the same metric.

use super::cell::{CellChoice, Constraints};
use super::input::Input;
use super::metric::{MetricTables, luma_pair_bias};
use crate::colour::{ColourIndex, ColourSet};
use crate::layer::CellMetric;

/// How many of a cell's most wanted colours join the short list.
pub const WANTED: usize = 5;

/// Weight of SSIM's structural term against the squared distances, per pixel: about a
/// tenth of a typical squared OKLab error per unit of dissimilarity.
pub const SSIM_WEIGHT: f32 = 0.05;

/// SSIM's stabilising constants for lightness in `0..=1` (Wang et al.'s `K₁ = 0.01`,
/// `K₂ = 0.03`).
const C1: f32 = 0.0001;
const C2: f32 = 0.0009;

/// The 3 × 3 binomial blur weights by `|dx| + |dy|` (centre 4, edge 2, corner 1).
const BLUR: [f32; 3] = [4.0, 2.0, 1.0];

/// The target blurred with [`BLUR`] (each pixel over its in-frame neighbours of the same
/// metric), in metric coordinates.
pub(crate) fn blurred_target(input: &Input) -> Vec<[f32; 3]> {
    let (w, h) = input.size;
    (0..w * h)
        .map(|i| {
            let (x, y) = ((i % w) as i64, (i / w) as i64);
            let q = &input.px[i];
            let mut acc = [0.0f32; 3];
            let mut total = 0.0f32;
            for dy in -1i64..=1 {
                for dx in -1i64..=1 {
                    let (nx, ny) = (x + dx, y + dy);
                    if nx < 0 || ny < 0 || nx >= w as i64 || ny >= h as i64 {
                        continue;
                    }
                    let r = &input.px[ny as usize * w + nx as usize];
                    if r.metric != q.metric {
                        continue;
                    }
                    let wt = BLUR[(dx.abs() + dy.abs()) as usize];
                    for (a, v) in acc.iter_mut().zip(r.coords) {
                        *a += wt * v;
                    }
                    total += wt;
                }
            }
            acc.map(|v| v / total)
        })
        .collect()
}

/// Cell `i` rendered with `full` (each pixel its nearest colour), scored by `metric`,
/// plus the same-luma bias.
pub(crate) fn score(
    input: &Input,
    tables: &MetricTables,
    i: usize,
    full: ColourSet,
    metric: CellMetric,
    bias: f32,
) -> f32 {
    let w = input.size.0;
    let colours: Vec<ColourIndex> = full.iter().collect();
    let cells: Vec<(usize, usize)> = input.pixels_of(i).collect();
    let render: Vec<[f32; 3]> = cells
        .iter()
        .map(|&(x, y)| {
            let q = &input.px[y * w + x];
            let space = q.metric.space();
            let c = super::ordered::nearest(q.metric, q.coords, &colours, tables.colours(space));
            tables.colour(space, c)
        })
        .collect();
    let mut total = 0.0f32;
    match metric {
        CellMetric::Mse => {
            for (&(x, y), r) in cells.iter().zip(&render) {
                let q = &input.px[y * w + x];
                total += q.weight * q.metric.distance(q.coords, *r);
            }
        }
        CellMetric::Blur => {
            let (cw, _) = input.cell;
            let (x0, y0) = cells[0];
            for &(x, y) in &cells {
                let q = &input.px[y * w + x];
                if q.weight <= 0.0 {
                    continue;
                }
                let mut acc = [0.0f32; 3];
                let mut wsum = 0.0f32;
                for dy in -1i64..=1 {
                    for dx in -1i64..=1 {
                        let (nx, ny) = (x as i64 + dx, y as i64 + dy);
                        if nx < 0 || ny < 0 || nx >= w as i64 || ny >= input.size.1 as i64 {
                            continue;
                        }
                        let (nx, ny) = (nx as usize, ny as usize);
                        let r = &input.px[ny * w + nx];
                        if r.metric != q.metric {
                            continue;
                        }
                        // Inside the cell the rendering, outside it the target.
                        let inside = nx >= x0 && nx < x0 + cw && ny >= y0 && ny < y0 + input.cell.1;
                        let v = if inside {
                            render[(ny - y0) * cw + (nx - x0)]
                        } else {
                            r.coords
                        };
                        let wt = BLUR[(dx.abs() + dy.abs()) as usize];
                        for (a, c) in acc.iter_mut().zip(v) {
                            *a += wt * c;
                        }
                        wsum += wt;
                    }
                }
                let b = acc.map(|v| v / wsum);
                total += q.weight * q.metric.distance(b, input.blurred[y * w + x]);
            }
        }
        CellMetric::Ssim => {
            let (mut n, mut mx, mut my) = (0.0f32, 0.0f32, 0.0f32);
            for (&(x, y), r) in cells.iter().zip(&render) {
                let q = &input.px[y * w + x];
                total += q.weight * q.metric.distance(q.coords, *r);
                n += q.weight;
                mx += q.weight * q.coords[0];
                my += q.weight * r[0];
            }
            if n > 0.0 {
                let (mx, my) = (mx / n, my / n);
                let (mut vx, mut vy, mut cxy) = (0.0f32, 0.0f32, 0.0f32);
                for (&(x, y), r) in cells.iter().zip(&render) {
                    let q = &input.px[y * w + x];
                    let (dx, dy) = (q.coords[0] - mx, r[0] - my);
                    vx += q.weight * dx * dx;
                    vy += q.weight * dy * dy;
                    cxy += q.weight * dx * dy;
                }
                let (vx, vy, cxy) = (vx / n, vy / n, cxy / n);
                let ssim = ((2.0 * mx * my + C1) * (2.0 * cxy + C2))
                    / ((mx * mx + my * my + C1) * (vx + vy + C2));
                total += SSIM_WEIGHT * n * (1.0 - ssim);
            }
        }
    }
    total + luma_pair_bias(full, bias)
}

/// The best set for cell `i` under `metric` among the short list (see the module
/// docs), starting from the squared-distance winner `mse`. Ties keep the earlier set
/// in index order, the winner first.
#[expect(
    clippy::too_many_arguments,
    reason = "the search's inputs, as `best_set` takes them"
)]
pub(crate) fn repick(
    input: &Input,
    tables: &MetricTables,
    i: usize,
    bg: Option<ColourIndex>,
    c: &Constraints,
    bias: f32,
    metric: CellMetric,
    mse: CellChoice,
) -> CellChoice {
    let bg_set = bg.map_or(ColourSet::EMPTY, |b| ColourSet::EMPTY.with(b));
    let k = input.k();
    let mut short = mse.set;
    for (col, _) in input
        .wanted_colours(input.cell_at(i))
        .into_iter()
        .filter(|(col, _)| c.candidates.contains(*col) && !bg_set.contains(*col))
        .take(WANTED)
    {
        short = short.with(col);
    }
    let required = c.required.difference(bg_set);
    let free: Vec<ColourIndex> = short
        .difference(required)
        .difference(bg_set)
        .iter()
        .collect();
    let picks = k.saturating_sub(required.len() as usize).min(free.len());
    let full_of = |set: ColourSet| bg.map_or(set, |b| set.with(b));
    let mut best = CellChoice {
        set: mse.set,
        score: score(input, tables, i, full_of(mse.set), metric, bias),
    };
    let mut idx: Vec<usize> = (0..picks).collect();
    loop {
        let set = idx.iter().fold(required, |s, &j| s.with(free[j]));
        let sc = score(input, tables, i, full_of(set), metric, bias);
        if sc < best.score {
            best = CellChoice { set, score: sc };
        }
        // Next combination of `picks` out of `free`.
        let Some(pos) = (0..picks).rev().find(|&p| idx[p] < free.len() - picks + p) else {
            break;
        };
        idx[pos] += 1;
        for q in pos + 1..picks {
            idx[q] = idx[q - 1] + 1;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::{Layer, Recipe, SourceImage};
    use crate::mode::Mode;
    use crate::palette::Palette;
    use crate::solve::input::InputOptions;
    use crate::{Compositor, SuperGrid};
    use std::sync::Arc;

    /// A noisy multicolor frame, dithered with `kind`, whose recipe scores cells with
    /// `metric`.
    fn noisy(kind: crate::layer::DitherKind, metric: CellMetric) -> (Input, MetricTables) {
        let mut st = 3u32;
        let px: Vec<[f32; 4]> = (0..320 * 200)
            .map(|i| {
                st = st.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                let n = (st >> 16) as f32 / 65_536.0;
                let x = (i % 320) as f32 / 320.0;
                [x * 0.8 + n * 0.2, 0.3 + n * 0.3, (1.0 - x) * 0.5, 1.0]
            })
            .collect();
        let mut l = Layer::new(
            "n",
            Arc::new(SourceImage::from_linear(320, 200, px).unwrap()),
        );
        l.set_recipe(
            Recipe::with_method(kind, 0.7)
                .unwrap()
                .with_cell_metric(metric),
        );
        let mut c = Compositor::new(SuperGrid::new(Mode::MulticolorBitmap, 2).unwrap());
        c.update(&[l], &[]);
        let t = MetricTables::new(&Palette::pepto_pal());
        (Input::new(c.frame(), &t, InputOptions::default()), t)
    }

    #[test]
    fn mse_scores_match_the_search_tables() {
        // No dither: the search tables score each pixel's own colour.
        let (input, t) = noisy(crate::layer::DitherKind::None, CellMetric::Mse);
        let set: ColourSet = [ColourIndex::BLACK, ColourIndex::WHITE, ColourIndex::RED]
            .into_iter()
            .collect();
        for i in [0, 17, 999] {
            let a = score(&input, &t, i, set, CellMetric::Mse, 0.0);
            let b = super::super::cell::score_set(input.dist(i), input.per_cell(), None, set, 0.0);
            assert!((a - b).abs() <= 1e-4 * b.max(1.0), "{i}: {a} vs {b}");
        }
    }

    #[test]
    fn a_repick_is_never_worse_than_the_squared_distance_winner() {
        for metric in [CellMetric::Blur, CellMetric::Ssim] {
            let (input, t) = noisy(crate::layer::DitherKind::Bayer2, metric);
            assert_eq!(input.blurred.is_empty(), metric != CellMetric::Blur);
            for i in (0..1000).step_by(37) {
                let cell = input.cell_at(i);
                let bg = Some(ColourIndex::BLACK);
                let mse = input.best_set(cell, bg, &Constraints::FREE, 0.0);
                let picked = input.pick_set(&t, cell, bg, &Constraints::FREE, 0.0);
                let full = |s: ColourSet| s.with(ColourIndex::BLACK);
                assert!(
                    picked.score <= score(&input, &t, i, full(mse.set), metric, 0.0),
                    "{metric:?} cell {i}"
                );
                assert!(picked.set.len() <= 3);
            }
        }
    }
}
