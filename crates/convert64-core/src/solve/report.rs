//! The clash report (UX §7, §11): per cell, which colours the picture wanted, which the
//! solver chose and what the dropped ones cost, so "why is this cell like that" has an
//! answer as data.

use super::input::Input;
use super::metric::MetricTables;
use crate::colour::{ColourIndex, ColourSet};
use crate::model::CellPos;
use serde::{Deserialize, Serialize};

/// A cell counts as a clash when its dropped colours cost more than this (Σ weight ×
/// squared metric distance; about one pixel off by a clearly visible ΔE of 0.045).
pub const CLASH_THRESHOLD: f32 = 0.002;

/// One dropped colour.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Dropped {
    /// The colour.
    pub colour: ColourIndex,
    /// How many pixels (by coverage) wanted it.
    pub weight: f32,
    /// The error its absence costs: Σ over those pixels of weight × (distance to the
    /// colour they got − distance to this one).
    pub cost: f32,
}

/// One cell of the report.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CellReport {
    /// The cell.
    pub cell: CellPos,
    /// Colours the cell's pixels are nearest to with no constraint, with their weights,
    /// heaviest first.
    pub wanted: Vec<(ColourIndex, f32)>,
    /// The colours the cell shows (the background included in multicolor).
    pub chosen: ColourSet,
    /// Wanted colours that are not in `chosen`, costliest first.
    pub dropped: Vec<Dropped>,
    /// The cell's error: Σ weight × distance(composite colour, output colour).
    pub error: f32,
    /// The dither crosses more luma levels than a pixel's `max_luma_diff` allows: two
    /// neighbouring pixels that want the same colour came out more than that many
    /// levels apart. Only checked where a recipe asked for a limit (PAL-blend safe).
    #[serde(default)]
    pub luma_cross: bool,
}

impl CellReport {
    /// Total cost of the dropped colours.
    pub fn dropped_cost(&self) -> f32 {
        self.dropped.iter().map(|d| d.cost).sum()
    }

    /// Whether the cell clashes ([`CLASH_THRESHOLD`]).
    pub fn is_clash(&self) -> bool {
        self.dropped_cost() > CLASH_THRESHOLD
    }
}

/// Status-bar totals.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Summary {
    /// Cells that clash.
    pub clashes: usize,
    /// Σ cell error, the solve's total error.
    pub total_error: f32,
    /// Σ dropped cost over all cells.
    pub dropped_cost: f32,
    /// The worst cell, if any clashes.
    pub worst: Option<CellPos>,
    /// Cells flagged [`CellReport::luma_cross`].
    #[serde(default)]
    pub luma_crossings: usize,
}

/// The report for a whole solve.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClashReport {
    /// Every cell, row-major.
    pub cells: Vec<CellReport>,
}

/// Whether cell `i`'s dither crosses more luma levels than its pixels allow: a right or
/// lower neighbour in the cell wanting the same colour of `set` (its nearest) came out
/// more than `max_luma_diff` levels away. Unrestricted pixels are never checked.
fn crosses_luma(
    input: &Input,
    tables: &MetricTables,
    out: &[ColourIndex],
    i: usize,
    set: ColourSet,
) -> bool {
    let (w, _) = input.size;
    let (cw, ch) = input.cell;
    let (gx, gy) = (i % input.cells.0, i / input.cells.0);
    let colours: Vec<ColourIndex> = set.iter().collect();
    if colours.is_empty() {
        return false;
    }
    let nearest = |x: usize, y: usize| {
        let q = &input.px[y * w + x];
        super::ordered::nearest(
            q.metric,
            q.coords,
            &colours,
            tables.colours(q.metric.space()),
        )
    };
    for y in gy * ch..(gy + 1) * ch {
        for x in gx * cw..(gx + 1) * cw {
            let q = &input.px[y * w + x];
            if q.max_luma_diff >= crate::layer::recipe::MAX_LUMA_DIFF_OFF || q.weight <= 0.0 {
                continue;
            }
            for (nx, ny) in [(x + 1, y), (x, y + 1)] {
                if nx >= (gx + 1) * cw || ny >= (gy + 1) * ch {
                    continue;
                }
                let limit = q.max_luma_diff.min(input.px[ny * w + nx].max_luma_diff);
                if nearest(x, y) == nearest(nx, ny)
                    && crate::palette::luma::level_diff(out[y * w + x], out[ny * w + nx]) > limit
                {
                    return true;
                }
            }
        }
    }
    false
}

impl ClashReport {
    /// Builds the report from a solve's input and output (row-major target pixels).
    /// `reuse`: cells outside the set keep their entry from the previous report (the
    /// interactive fast path, where only those cells were re-solved).
    pub(crate) fn build(
        input: &Input,
        tables: &MetricTables,
        out: &[ColourIndex],
        background: Option<ColourIndex>,
        sets: &[ColourSet],
        reuse: Option<(crate::geom::CellSet, &Self)>,
    ) -> Self {
        let (w, _) = input.size;
        let n = input.cells.0 * input.cells.1;
        let cells = (0..n)
            .map(|i| {
                let cell = input.cell_at(i);
                if let Some((fresh, prev)) = reuse
                    && !fresh.contains(cell)
                    && let Some(old) = prev.cells.get(i)
                {
                    return old.clone();
                }
                let mut wanted = [0.0f32; 16];
                let mut cost = [0.0f32; 16];
                let mut chosen = background.map_or(ColourSet::EMPTY, |b| ColourSet::EMPTY.with(b));
                let mut error = 0.0f32;
                for (x, y) in input.pixels_of(i) {
                    let q = &input.px[y * w + x];
                    let got = out[y * w + x];
                    chosen.insert(got);
                    let d = tables.distances(q.metric, q.coords);
                    let d_got = d[got.as_usize()];
                    error += q.weight * d_got;
                    if q.weight <= 0.0 {
                        continue;
                    }
                    let mut near = 0;
                    for c in 1..16 {
                        if d[c] < d[near] {
                            near = c;
                        }
                    }
                    wanted[near] += q.weight;
                    cost[near] += q.weight * (d_got - d[near]);
                }
                let mut want: Vec<(ColourIndex, f32)> = ColourIndex::ALL
                    .into_iter()
                    .zip(wanted)
                    .filter(|(_, w)| *w > 0.0)
                    .collect();
                want.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
                let mut dropped: Vec<Dropped> = want
                    .iter()
                    .filter(|(c, _)| !chosen.contains(*c))
                    .map(|&(c, weight)| Dropped {
                        colour: c,
                        weight,
                        cost: cost[c.as_usize()],
                    })
                    .collect();
                dropped.sort_by(|a, b| b.cost.total_cmp(&a.cost).then(a.colour.cmp(&b.colour)));
                let set = background.map_or(sets[i], |b| sets[i].with(b));
                CellReport {
                    cell,
                    wanted: want,
                    chosen,
                    dropped,
                    error,
                    luma_cross: crosses_luma(input, tables, out, i, set),
                }
            })
            .collect();
        Self { cells }
    }

    /// The report of one cell.
    pub fn cell(&self, cell: CellPos) -> Option<&CellReport> {
        self.cells.iter().find(|c| c.cell == cell)
    }

    /// Status-bar totals.
    pub fn summary(&self) -> Summary {
        let order = self.worst_first();
        Summary {
            clashes: self.cells.iter().filter(|c| c.is_clash()).count(),
            total_error: self.cells.iter().map(|c| f64::from(c.error)).sum::<f64>() as f32,
            dropped_cost: self.cells.iter().map(|c| c.dropped_cost()).sum(),
            worst: order
                .first()
                .copied()
                .filter(|c| self.cell(*c).is_some_and(CellReport::is_clash)),
            luma_crossings: self.cells.iter().filter(|c| c.luma_cross).count(),
        }
    }

    /// Cells from worst to best: by dropped cost, then error, then position. `Shift Cmd ]`
    /// steps through this list (UX §7).
    pub fn worst_first(&self) -> Vec<CellPos> {
        let mut v: Vec<&CellReport> = self.cells.iter().collect();
        v.sort_by(|a, b| {
            b.dropped_cost()
                .total_cmp(&a.dropped_cost())
                .then(b.error.total_cmp(&a.error))
                .then(a.cell.cmp(&b.cell))
        });
        v.into_iter().map(|c| c.cell).collect()
    }

    /// The next-worst cell after `cell` in [`Self::worst_first`] order (wrapping), for
    /// `Shift Cmd ]`; `[` walks the other way with `forward = false`.
    pub fn step(&self, cell: CellPos, forward: bool) -> Option<CellPos> {
        let order = self.worst_first();
        let i = order.iter().position(|c| *c == cell)?;
        let n = order.len();
        Some(
            order[if forward {
                (i + 1) % n
            } else {
                (i + n - 1) % n
            }],
        )
    }
}
