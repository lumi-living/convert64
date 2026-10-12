//! Background search and refinement (`docs/TAD.md` §3.1).
//!
//! 1. **Search.** For every background candidate (all 16, or the pinned one; hi-res has
//!    none), every cell's best set is found by brute force. This is cheap: the
//!    per-cell distance tables are shared by all backgrounds.
//! 2. **Dither and refine.** The best few backgrounds by search error (the finalists)
//!    are dithered; then up to two refinement rounds re-pick each cell's set from its
//!    colours plus the error the dither pushed into them, and re-dither. A round that
//!    changes no set stops early, and the best-scoring round is kept, so refinement can
//!    neither oscillate nor make the result worse.
//! 3. **Pick.** The finalist with the least error wins; ties go to the lower background
//!    index, so the result does not depend on the thread count.

use super::dither::{Locked, PassResult, pass};
use super::input::{Input, InputOptions};
use super::metric::MetricTables;
use super::report::ClashReport;
use super::steering::Steering;
use crate::colour::{ColourIndex, ColourSet};
use crate::composite::TargetFrame;
use crate::model::{BitmapImage, IndexedBitmap};
use crate::palette::Palette;
use serde::{Deserialize, Serialize};

/// How the background is chosen (multicolor only).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Background {
    /// Try all 16.
    #[default]
    Auto,
    /// Use this one.
    Fixed(ColourIndex),
}

/// How much work a solve does (UX §7).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SolveMode {
    /// Background search, dithering of the finalists and refinement: on release.
    #[default]
    Full,
    /// The fast path during a drag: the previous solution's background, one dithering
    /// pass, no refinement, and only `dirty` cells re-solved (every other cell is copied
    /// from the previous solution as if locked). Without a previous solution it is a
    /// single-pass solve of the whole picture with one background.
    Interactive {
        /// The cells to re-solve (from `Compositor::update` or a steering command).
        dirty: crate::geom::CellSet,
    },
}

/// Options of a solve.
#[derive(Clone, Debug, PartialEq)]
pub struct SolveOptions {
    /// Full search or the interactive fast path.
    pub mode: SolveMode,
    /// Background choice.
    pub background: Background,
    /// Same-luma pair bias per pair in a cell's set (positive penalises, negative
    /// rewards; see [`super::metric::luma_pair_bias`]).
    pub luma_pair_bias: f32,
    /// Refinement rounds, 0–2.
    pub refinements: u8,
    /// Backgrounds carried from the search into dithering (at least 1).
    pub finalists: u8,
    /// Refinement hysteresis: a re-picked cell set replaces the current one only if its
    /// error is below `hysteresis ×` the current set's (Dithertron's sub-palette ratio
    /// is 0.97), so sets cannot flip-flop between rounds. `None` (the default) accepts
    /// every re-pick, as E5 shipped.
    pub hysteresis: Option<f32>,
    /// Layer-locked pattern anchors, by layer index (see [`super::dither::layer_origins`]).
    pub layer_origins: Vec<(i64, i64)>,
    /// Per-pixel input options.
    pub input: InputOptions,
}

impl Default for SolveOptions {
    fn default() -> Self {
        Self {
            mode: SolveMode::Full,
            background: Background::Auto,
            luma_pair_bias: 0.0,
            refinements: 2,
            finalists: 3,
            hysteresis: None,
            layer_origins: Vec::new(),
            input: InputOptions::default(),
        }
    }
}

/// A solved picture.
#[derive(Clone, Debug, PartialEq)]
pub struct Solution {
    /// The encoded picture.
    pub image: BitmapImage,
    /// The background (multicolor), or `None` (hi-res).
    pub background: Option<ColourIndex>,
    /// Each cell's own colours (the background excluded), row-major.
    pub sets: Vec<ColourSet>,
    /// Σ weight × distance(composite colour, output colour) over all pixels.
    pub error: f32,
    /// Refinement rounds that changed the result.
    pub refined: u8,
    /// Each cell's set before refinement and after every refinement round that changed
    /// any set, for diagnostics (convergence, flip-flop).
    pub rounds: Vec<Vec<ColourSet>>,
    /// Wanted vs chosen vs dropped colours per cell.
    pub report: ClashReport,
}

/// The solver for one palette. Its metric tables belong to that palette: the preview
/// and the quantiser use the same palette object (TAD §3.4), and changing it means a new
/// solver (or [`Self::set_palette`]) and a re-solve.
#[derive(Clone, Debug, PartialEq)]
pub struct Solver {
    tables: MetricTables,
}

/// A refined finalist: the best pass, its sets, the round that produced it, and the
/// sets of every round.
type Refined = (PassResult, Vec<ColourSet>, u8, Vec<Vec<ColourSet>>);

/// One background's search result.
struct Candidate {
    bg: Option<ColourIndex>,
    sets: Vec<ColourSet>,
    score: f32,
}

impl Solver {
    /// A solver for `palette`.
    pub fn new(palette: &Palette) -> Self {
        Self {
            tables: MetricTables::new(palette),
        }
    }

    /// The palette.
    pub const fn palette(&self) -> &Palette {
        self.tables.palette()
    }

    /// The metric tables of the palette.
    pub const fn tables(&self) -> &MetricTables {
        &self.tables
    }

    /// Switches palette; the next solve uses the new tables.
    pub fn set_palette(&mut self, palette: &Palette) {
        if palette != self.palette() {
            self.tables = MetricTables::new(palette);
        }
    }

    /// Solves `frame` (the composite on the target grid with its attribute map), steered
    /// by `steering`. Locked cells are copied from `previous`; while any cell is locked
    /// (and `previous` is given) a multicolor solve keeps `previous`'s background, since
    /// a locked cell's pixels may use it.
    pub fn solve(
        &self,
        frame: &TargetFrame,
        steering: &Steering,
        previous: Option<&Solution>,
        opts: &SolveOptions,
    ) -> Solution {
        // The fast path needs tables only for the cells it re-solves.
        let only = match (opts.mode, previous) {
            (SolveMode::Interactive { dirty }, Some(_)) => Some(dirty),
            _ => None,
        };
        let input = Input::new_for(frame, &self.tables, opts.input, only);
        self.solve_input(&input, steering, previous, opts)
    }

    /// The best sets for one background. Locked cells keep their previous colours.
    fn search(
        &self,
        input: &Input,
        bg: Option<ColourIndex>,
        ctx: &Context<'_>,
        bias: f32,
    ) -> Candidate {
        let n = input.cells.0 * input.cells.1;
        let mut score = 0.0;
        let sets = (0..n)
            .map(|i| {
                let cell = input.cell_at(i);
                if let Some(prev) = ctx.locked_set(cell) {
                    score +=
                        super::cell::score_set(input.dist(i), input.per_cell(), bg, prev, bias);
                    return prev;
                }
                let c = input.best_set(cell, bg, &ctx.steering.constraints(cell), bias);
                score += c.score;
                c.set
            })
            .collect();
        Candidate { bg, sets, score }
    }

    /// Dithers one finalist and refines its sets.
    fn refine(
        &self,
        input: &Input,
        cand: &Candidate,
        ctx: &Context<'_>,
        opts: &SolveOptions,
    ) -> Refined {
        let t = &self.tables;
        let locked = ctx.locked();
        let mut sets = cand.sets.clone();
        let mut r = pass(input, t, &sets, cand.bg, &opts.layer_origins, locked);
        let mut best = (r.clone(), sets.clone(), 0u8);
        let mut rounds = vec![sets.clone()];
        for round in 1..=opts.refinements.min(2) {
            let moved = input.retarget(&r.received, t);
            let next: Vec<ColourSet> = (0..sets.len())
                .map(|i| {
                    let cell = input.cell_at(i);
                    ctx.locked_set(cell).unwrap_or_else(|| {
                        let pick = moved.best_set(
                            cell,
                            cand.bg,
                            &ctx.steering.constraints(cell),
                            opts.luma_pair_bias,
                        );
                        match opts.hysteresis {
                            Some(ratio) if pick.set != sets[i] => {
                                let current = super::cell::score_set(
                                    moved.dist(i),
                                    moved.per_cell(),
                                    cand.bg,
                                    sets[i],
                                    opts.luma_pair_bias,
                                );
                                if pick.score < ratio * current {
                                    pick.set
                                } else {
                                    sets[i]
                                }
                            }
                            _ => pick.set,
                        }
                    })
                })
                .collect();
            if next == sets {
                break;
            }
            sets = next;
            rounds.push(sets.clone());
            r = pass(input, t, &sets, cand.bg, &opts.layer_origins, locked);
            if r.seen < best.0.seen {
                best = (r.clone(), sets.clone(), round);
            }
        }
        (best.0, best.1, best.2, rounds)
    }

    fn solve_input(
        &self,
        input: &Input,
        steering: &Steering,
        previous: Option<&Solution>,
        opts: &SolveOptions,
    ) -> Solution {
        let interactive = match opts.mode {
            SolveMode::Interactive { dirty } => Some(dirty),
            SolveMode::Full => None,
        };
        // Interactive: everything but the dirty cells is copied from the previous result.
        let frozen;
        let steering = match (interactive, previous) {
            (Some(dirty), Some(_)) => {
                let mut s = steering.clone();
                for cell in crate::geom::CellSet::all().iter() {
                    if !dirty.contains(cell) {
                        s.set_lock(cell, true);
                    }
                }
                frozen = s;
                &frozen
            }
            _ => steering,
        };
        let opts = &SolveOptions {
            refinements: if interactive.is_some() {
                0
            } else {
                opts.refinements
            },
            finalists: if interactive.is_some() {
                1
            } else {
                opts.finalists
            },
            background: match (interactive, previous.and_then(|p| p.background)) {
                (Some(_), Some(b)) => Background::Fixed(b),
                _ => opts.background,
            },
            ..opts.clone()
        };
        let ctx = Context::new(steering, previous);
        let keep_bg = ctx.locked().and(previous.and_then(|p| p.background));
        let candidates: Vec<Option<ColourIndex>> = if !input.has_background() {
            vec![None]
        } else if let Some(b) = keep_bg {
            vec![Some(b)]
        } else {
            match opts.background {
                Background::Fixed(b) => vec![Some(b)],
                Background::Auto => ColourIndex::ALL.into_iter().map(Some).collect(),
            }
        };
        let mut found = crate::par::map(&candidates, |&bg| {
            self.search(input, bg, &ctx, opts.luma_pair_bias)
        });
        // Lowest search error first; the lower background index breaks ties.
        found.sort_by(|a, b| {
            a.score
                .total_cmp(&b.score)
                .then(bg_key(a.bg).cmp(&bg_key(b.bg)))
        });
        found.truncate(usize::from(opts.finalists.max(1)));
        let dithered = crate::par::map(&found, |c| self.refine(input, c, &ctx, opts));
        let (winner, (result, sets, refined, rounds)) = found
            .iter()
            .zip(dithered)
            .min_by(|(ca, ra), (cb, rb)| {
                ra.0.seen
                    .total_cmp(&rb.0.seen)
                    .then(bg_key(ca.bg).cmp(&bg_key(cb.bg)))
            })
            .expect("at least one candidate");
        let reuse = match (interactive, previous) {
            (Some(dirty), Some(p)) if p.background == winner.bg => Some((dirty, &p.report)),
            _ => None,
        };
        let report = ClashReport::build(input, &self.tables, &result.out, winner.bg, &sets, reuse);
        let indexed = IndexedBitmap::new(input.grid.target(), result.out, winner.bg)
            .expect("the output has the target's size");
        let image = BitmapImage::encode(&indexed).expect("every cell keeps to its set");
        Solution {
            image,
            background: winner.bg,
            sets,
            error: result.error,
            refined,
            rounds,
            report,
        }
    }
}

/// Steering and the previous solution, as the search and the pass need them.
struct Context<'a> {
    steering: &'a Steering,
    /// The previous solution's pixels, when any cell is locked.
    previous: Option<(Vec<ColourIndex>, &'a Solution)>,
}

impl<'a> Context<'a> {
    fn new(steering: &'a Steering, previous: Option<&'a Solution>) -> Self {
        let previous = previous
            .filter(|_| !steering.locks().is_empty())
            .map(|p| (p.image.to_indexed().pixels().to_vec(), p));
        Self { steering, previous }
    }

    fn locked(&self) -> Option<Locked<'_>> {
        self.previous.as_ref().map(|(px, _)| Locked {
            cells: self.steering.locks(),
            previous: px,
        })
    }

    /// A locked cell's previous own colours.
    fn locked_set(&self, cell: crate::model::CellPos) -> Option<ColourSet> {
        let (_, prev) = self.previous.as_ref()?;
        if !self.steering.locks().contains(cell) {
            return None;
        }
        let i = usize::from(cell.y) * 40 + usize::from(cell.x);
        Some(prev.sets[i])
    }
}

const fn bg_key(bg: Option<ColourIndex>) -> u8 {
    match bg {
        Some(c) => c.get(),
        None => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::composite::Compositor;
    use crate::geom::SuperGrid;
    use crate::layer::{DitherKind, Layer, Recipe, SourceImage};
    use crate::mode::Mode;
    use crate::solve::input::InputOptions;
    use crate::validate;
    use std::sync::Arc;

    fn frame(mode: Mode, kind: DitherKind) -> TargetFrame {
        let px: Vec<[f32; 4]> = (0..320 * 200)
            .map(|i| {
                let (x, y) = ((i % 320) as f32 / 320.0, (i / 320) as f32 / 200.0);
                [x * x, y, (1.0 - x) * 0.6 + 0.2 * y, 1.0]
            })
            .collect();
        let mut l = Layer::new(
            "g",
            Arc::new(SourceImage::from_linear(320, 200, px).unwrap()),
        );
        l.set_recipe(Recipe::with_method(kind, 0.8).unwrap());
        let mut c = Compositor::new(SuperGrid::new(mode, 2).unwrap());
        c.update(&[l], &[]);
        c.frame().clone()
    }

    #[test]
    fn solutions_validate_in_both_modes() {
        let s = Solver::new(&Palette::pepto_pal());
        for mode in [Mode::MulticolorBitmap, Mode::HiresBitmap] {
            for kind in [
                DitherKind::None,
                DitherKind::Bayer4,
                DitherKind::FloydSteinberg,
            ] {
                let sol = s.solve(
                    &frame(mode, kind),
                    &Steering::new(),
                    None,
                    &SolveOptions::default(),
                );
                assert_eq!(validate(&sol.image.to_indexed()), Ok(()), "{mode} {kind:?}");
                assert_eq!(sol.background.is_some(), mode == Mode::MulticolorBitmap);
                assert!(sol.error.is_finite() && sol.error > 0.0);
            }
        }
    }

    #[test]
    fn a_fixed_background_is_honoured_and_auto_is_no_worse() {
        let s = Solver::new(&Palette::pepto_pal());
        let f = frame(Mode::MulticolorBitmap, DitherKind::None);
        let fixed = s.solve(
            &f,
            &Steering::new(),
            None,
            &SolveOptions {
                background: Background::Fixed(ColourIndex::YELLOW),
                ..SolveOptions::default()
            },
        );
        assert_eq!(fixed.background, Some(ColourIndex::YELLOW));
        let auto = s.solve(&f, &Steering::new(), None, &SolveOptions::default());
        assert!(
            auto.error <= fixed.error,
            "{} > {}",
            auto.error,
            fixed.error
        );
    }

    #[test]
    fn refinement_never_makes_it_worse() {
        let s = Solver::new(&Palette::pepto_pal());
        let f = frame(Mode::MulticolorBitmap, DitherKind::FloydSteinberg);
        let none = s.solve(
            &f,
            &Steering::new(),
            None,
            &SolveOptions {
                refinements: 0,
                ..SolveOptions::default()
            },
        );
        let two = s.solve(&f, &Steering::new(), None, &SolveOptions::default());
        assert!(two.error <= none.error, "{} > {}", two.error, none.error);
        assert!(two.refined <= 2);
    }

    /// Same input, 1 thread or many: byte-identical output.
    #[cfg(feature = "parallel")]
    #[test]
    fn deterministic_regardless_of_thread_count() {
        let s = Solver::new(&Palette::pepto_pal());
        for (mode, kind) in [
            (Mode::MulticolorBitmap, DitherKind::FloydSteinberg),
            (Mode::MulticolorBitmap, DitherKind::Bayer4),
            (Mode::HiresBitmap, DitherKind::Atkinson),
            (Mode::HiresBitmap, DitherKind::BlueNoise),
        ] {
            let mut f = frame(mode, kind);
            if kind == DitherKind::BlueNoise {
                // Pre-smoothing too.
                let r = Recipe::with_method(kind, 0.8).unwrap().with_smooth(true);
                f = frame_with(mode, r, true);
            }
            let one = rayon::ThreadPoolBuilder::new()
                .num_threads(1)
                .build()
                .unwrap()
                .install(|| s.solve(&f, &Steering::new(), None, &SolveOptions::default()));
            let many = rayon::ThreadPoolBuilder::new()
                .num_threads(8)
                .build()
                .unwrap()
                .install(|| s.solve(&f, &Steering::new(), None, &SolveOptions::default()));
            assert_eq!(one, many, "{mode} {kind:?}");
        }
    }

    fn cell_bytes(img: &BitmapImage, cell: crate::model::CellPos) -> (Vec<u8>, u8, u8) {
        let i = usize::from(cell.y) * 40 + usize::from(cell.x);
        let b = usize::from(cell.y) * 320 + usize::from(cell.x) * 8;
        (
            img.bitmap()[b..b + 8].to_vec(),
            img.screen()[i].to_byte(),
            img.colour().get(i).map_or(0, |c| c.get()),
        )
    }

    #[test]
    fn pins_are_honoured() {
        use super::super::steering::Pin;
        use crate::model::CellPos;
        let s = Solver::new(&Palette::pepto_pal());
        let f = frame(Mode::MulticolorBitmap, DitherKind::Bayer4);
        let mut st = Steering::new();
        let pinned: ColourSet = [ColourIndex::RED, ColourIndex::CYAN, ColourIndex::WHITE]
            .into_iter()
            .collect();
        st.set_pin(
            CellPos::new(3, 4),
            Pin {
                colours: pinned,
                full: true,
            },
        );
        st.set_pin(
            CellPos::new(10, 10),
            Pin {
                colours: ColourSet::EMPTY.with(ColourIndex::PURPLE),
                full: false,
            },
        );
        let opts = SolveOptions {
            background: Background::Fixed(ColourIndex::BLACK),
            ..SolveOptions::default()
        };
        let sol = s.solve(&f, &st, None, &opts);
        assert_eq!(sol.sets[4 * 40 + 3], pinned);
        assert!(sol.sets[10 * 40 + 10].contains(ColourIndex::PURPLE));
        let used = sol.image.to_indexed().cell_colours(CellPos::new(3, 4));
        assert_eq!(
            used.difference(pinned)
                .difference(ColourSet::EMPTY.with(ColourIndex::BLACK)),
            ColourSet::EMPTY
        );
    }

    #[test]
    fn a_locked_cell_is_byte_identical_through_a_solve() {
        use crate::model::CellPos;
        let s = Solver::new(&Palette::pepto_pal());
        for mode in [Mode::MulticolorBitmap, Mode::HiresBitmap] {
            let first = s.solve(
                &frame(mode, DitherKind::FloydSteinberg),
                &Steering::new(),
                None,
                &SolveOptions::default(),
            );
            // A different picture: same gradient, other recipe and a flat patch.
            let other = frame(mode, DitherKind::Bayer8);
            let mut st = Steering::new();
            let locked = [
                CellPos::new(2, 2),
                CellPos::new(20, 12),
                CellPos::new(39, 24),
            ];
            for c in locked {
                st.set_lock(c, true);
            }
            let second = s.solve(&other, &st, Some(&first), &SolveOptions::default());
            assert_eq!(
                second.background, first.background,
                "{mode}: locks keep the background"
            );
            for c in locked {
                assert_eq!(
                    cell_bytes(&second.image, c),
                    cell_bytes(&first.image, c),
                    "{mode} {c}"
                );
            }
            assert_eq!(validate(&second.image.to_indexed()), Ok(()));
        }
    }

    #[test]
    fn force_pulls_a_cell_towards_its_layer_colours() {
        use crate::geom::Vec2;
        use crate::layer::{FitMode, Method, Metric, PatternPhase, Transform};
        use crate::model::CellPos;
        // Hi-res cell (0,0): 56 pixels of a blue/white backdrop, 8 pixels (one row) of a
        // small red Force layer. Adaptive keeps blue and white; Force keeps red.
        let p = Palette::pepto_pal();
        let col = |c: ColourIndex| {
            let l = p.linear(c);
            [l.r, l.g, l.b, 1.0]
        };
        let back: Vec<[f32; 4]> = (0..320 * 200)
            .map(|i| {
                if i % 2 == 0 {
                    col(ColourIndex::BLUE)
                } else {
                    col(ColourIndex::WHITE)
                }
            })
            .collect();
        let bg_layer = Layer::new(
            "back",
            Arc::new(SourceImage::from_linear(320, 200, back).unwrap()),
        );
        let solve = |fit: FitMode| {
            let mut red = Layer::new(
                "red",
                Arc::new(SourceImage::from_linear(8, 1, vec![col(ColourIndex::RED); 8]).unwrap()),
            );
            red.set_transform(Transform::translated(Vec2::new(0.0, 3.0)).unwrap());
            red.set_recipe(
                Recipe::new(
                    Method::new(DitherKind::None),
                    0.0,
                    PatternPhase::Screen,
                    Metric::Oklab,
                    ColourSet::ALL,
                    fit,
                )
                .unwrap(),
            );
            let mut c = Compositor::new(SuperGrid::new(Mode::HiresBitmap, 1).unwrap());
            c.update(&[bg_layer.clone(), red], &[]);
            Solver::new(&p).solve(c.frame(), &Steering::new(), None, &SolveOptions::default())
        };
        let adaptive = solve(FitMode::Adaptive);
        let force = solve(FitMode::Force);
        let cell = |s: &Solution| s.image.to_indexed().cell_colours(CellPos::new(0, 0));
        assert!(
            !cell(&adaptive).contains(ColourIndex::RED),
            "{}",
            cell(&adaptive)
        );
        assert!(cell(&force).contains(ColourIndex::RED), "{}", cell(&force));
        assert!(force.error > adaptive.error, "neighbours pay");
    }

    #[test]
    fn interactive_resolves_only_dirty_cells() {
        use crate::geom::CellSet;
        use crate::model::CellPos;
        let s = Solver::new(&Palette::pepto_pal());
        for mode in [Mode::MulticolorBitmap, Mode::HiresBitmap] {
            let full = s.solve(
                &frame(mode, DitherKind::FloydSteinberg),
                &Steering::new(),
                None,
                &SolveOptions::default(),
            );
            let changed = frame(mode, DitherKind::Bayer4);
            let mut dirty = CellSet::EMPTY;
            dirty.insert_rect(10, 5, 14, 9);
            let fast = s.solve(
                &changed,
                &Steering::new(),
                Some(&full),
                &SolveOptions {
                    mode: SolveMode::Interactive { dirty },
                    ..SolveOptions::default()
                },
            );
            assert_eq!(fast.background, full.background);
            for y in 0..25 {
                for x in 0..40 {
                    let c = CellPos::new(x, y);
                    if !dirty.contains(c) {
                        assert_eq!(
                            cell_bytes(&fast.image, c),
                            cell_bytes(&full.image, c),
                            "{mode} {c}"
                        );
                    }
                }
            }
            assert_eq!(validate(&fast.image.to_indexed()), Ok(()));
            // Without a previous solution it still solves the whole picture.
            let alone = s.solve(
                &changed,
                &Steering::new(),
                None,
                &SolveOptions {
                    mode: SolveMode::Interactive { dirty },
                    ..SolveOptions::default()
                },
            );
            assert_eq!(validate(&alone.image.to_indexed()), Ok(()));
        }
    }

    #[test]
    fn clash_report_sums_match_the_solve() {
        let s = Solver::new(&Palette::pepto_pal());
        for mode in [Mode::MulticolorBitmap, Mode::HiresBitmap] {
            for kind in [DitherKind::FloydSteinberg, DitherKind::Bayer8] {
                let sol = s.solve(
                    &frame(mode, kind),
                    &Steering::new(),
                    None,
                    &SolveOptions::default(),
                );
                let sum = sol.report.summary();
                let rel = (sum.total_error - sol.error).abs() / sol.error;
                assert!(
                    rel < 1e-4,
                    "{mode} {kind:?}: {} vs {}",
                    sum.total_error,
                    sol.error
                );
                assert_eq!(sol.report.cells.len(), 1000);
                for c in &sol.report.cells {
                    // Chosen colours are exactly what the cell shows.
                    assert_eq!(
                        c.chosen,
                        sol.image.to_indexed().cell_colours(c.cell).union(
                            sol.background
                                .map_or(ColourSet::EMPTY, |b| ColourSet::EMPTY.with(b))
                        )
                    );
                    assert!(
                        c.dropped
                            .iter()
                            .all(|d| !c.chosen.contains(d.colour) && d.cost >= -1e-6)
                    );
                }
                // Worst-first is a permutation, and stepping wraps.
                let order = sol.report.worst_first();
                assert_eq!(order.len(), 1000);
                assert_eq!(sol.report.step(order[999], true), Some(order[0]));
                assert_eq!(sol.report.step(order[0], false), Some(order[999]));
            }
        }
    }

    /// A frame whose single full-screen layer uses `recipe`.
    fn frame_with(mode: Mode, recipe: Recipe, noise: bool) -> TargetFrame {
        let mut st = 7u32;
        let px: Vec<[f32; 4]> = (0..320 * 200)
            .map(|i| {
                let (x, y) = ((i % 320) as f32 / 320.0, (i / 320) as f32 / 200.0);
                if noise {
                    st = st.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                    let n = (st >> 16) as f32 / 65_536.0 * 0.08;
                    [0.25 + n, 0.22 + n, 0.3 - n, 1.0]
                } else {
                    [x * x, y, (1.0 - x) * 0.6 + 0.2 * y, 1.0]
                }
            })
            .collect();
        let mut l = Layer::new(
            "g",
            Arc::new(SourceImage::from_linear(320, 200, px).unwrap()),
        );
        l.set_recipe(recipe);
        let mut c = Compositor::new(SuperGrid::new(mode, 2).unwrap());
        c.update(&[l], &[]);
        c.frame().clone()
    }

    fn solve_default(f: &TargetFrame) -> Solution {
        Solver::new(&Palette::pepto_pal()).solve(
            f,
            &Steering::new(),
            None,
            &SolveOptions::default(),
        )
    }

    /// Adjacent pixel pairs that want the same colour of their cell but came out more
    /// than `limit` luma levels apart: cross-luma speckle.
    fn speckle(sol: &Solution, f: &TargetFrame, limit: u8) -> usize {
        let input = Input::new(
            f,
            Solver::new(&Palette::pepto_pal()).tables(),
            InputOptions::default(),
        );
        let img = sol.image.to_indexed();
        let (w, h) = (usize::from(img.width()), usize::from(img.height()));
        let (cw, ch) = input.cell;
        let t = Solver::new(&Palette::pepto_pal());
        let nearest = |x: usize, y: usize| {
            let cell = (y / ch) * input.cells.0 + x / cw;
            let set: Vec<ColourIndex> = sol
                .background
                .map_or(sol.sets[cell], |b| sol.sets[cell].with(b))
                .iter()
                .collect();
            let q = &input.px[y * w + x];
            super::super::ordered::nearest(
                q.metric,
                q.coords,
                &set,
                t.tables().colours(q.metric.space()),
            )
        };
        let mut n = 0;
        for y in 0..h {
            for x in 0..w {
                for (nx, ny) in [(x + 1, y), (x, y + 1)] {
                    if nx < w
                        && ny < h
                        && nx / cw == x / cw
                        && ny / ch == y / ch
                        && nearest(x, y) == nearest(nx, ny)
                        && crate::palette::luma::level_diff(
                            img.pixel(x as u16, y as u16),
                            img.pixel(nx as u16, ny as u16),
                        ) > limit
                    {
                        n += 1;
                    }
                }
            }
        }
        n
    }

    /// A hi-res frame of one grey (linear `level`, ± `noise` texture) that may use only
    /// black and white, dithered with `kind` at `strength` and `mix_levels`.
    fn black_and_white(
        level: f32,
        noise: f32,
        kind: DitherKind,
        strength: f32,
        mix_levels: u8,
    ) -> TargetFrame {
        let r = Recipe::new(
            crate::layer::Method::new(kind),
            strength,
            crate::layer::PatternPhase::Screen,
            crate::layer::Metric::Oklab,
            [ColourIndex::BLACK, ColourIndex::WHITE]
                .into_iter()
                .collect(),
            crate::layer::FitMode::Adaptive,
        )
        .and_then(|r| r.with_mix_levels(mix_levels))
        .unwrap();
        let mut st = 11u32;
        let px: Vec<[f32; 4]> = (0..320 * 200)
            .map(|_| {
                st = st.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                let n = ((st >> 16) as f32 / 65_536.0 - 0.5) * 2.0 * noise;
                [level + n, level + n, level + n, 1.0]
            })
            .collect();
        let mut l = Layer::new(
            "grey",
            Arc::new(SourceImage::from_linear(320, 200, px).unwrap()),
        );
        l.set_recipe(r);
        let mut c = Compositor::new(SuperGrid::new(Mode::HiresBitmap, 2).unwrap());
        c.update(&[l], &[]);
        c.frame().clone()
    }

    /// White pixels per hi-res cell → how many cells have that count.
    fn white_counts(sol: &Solution) -> std::collections::BTreeMap<usize, usize> {
        let px = sol.image.to_indexed();
        let mut counts = std::collections::BTreeMap::new();
        for cy in 0..25u16 {
            for cx in 0..40u16 {
                let n = (cy * 8..cy * 8 + 8)
                    .flat_map(|y| (cx * 8..cx * 8 + 8).map(move |x| (x, y)))
                    .filter(|&(x, y)| px.pixel(x, y) == ColourIndex::WHITE)
                    .count();
                *counts.entry(n).or_insert(0) += 1;
            }
        }
        counts
    }

    #[test]
    fn mix_levels_give_a_flat_area_one_pattern() {
        // A flat grey with ±0.03 texture: E5's per-pixel ratio follows the texture and
        // the share of white wanders from cell to cell; mix levels give every cell the
        // same pattern.
        let s = Solver::new(&Palette::pepto_pal());
        let solve = |levels| {
            let f = black_and_white(0.2, 0.03, DitherKind::Bayer2, 0.7, levels);
            s.solve(&f, &Steering::new(), None, &SolveOptions::default())
        };
        let calm = white_counts(&solve(4));
        let top = calm.values().max().copied().unwrap_or(0);
        assert!(top * 100 >= 1000 * 98, "{calm:?}");
        let busy = white_counts(&solve(0));
        assert!(busy.len() >= 5, "{busy:?}");
    }

    #[test]
    fn mixes_are_taken_in_linear_light() {
        // Linear 0.25 grey from black and white: a quarter of the pixels white (a ratio
        // taken in OKLab, as E5 did, makes it about 63 %).
        let s = Solver::new(&Palette::pepto_pal());
        let f = black_and_white(0.25, 0.0, DitherKind::Bayer8, 1.0, 16);
        let sol = s.solve(&f, &Steering::new(), None, &SolveOptions::default());
        let white: usize = white_counts(&sol).iter().map(|(n, c)| n * c).sum();
        let share = white as f32 / 64_000.0;
        assert!((share - 0.25).abs() < 0.02, "{share}");
    }

    #[test]
    fn knobs_at_their_defaults_change_nothing() {
        for mode in [Mode::MulticolorBitmap, Mode::HiresBitmap] {
            let base = Recipe::with_method(DitherKind::FloydSteinberg, 0.9).unwrap();
            let explicit = base
                .with_cell_clamp(true)
                .with_max_luma_diff(crate::layer::recipe::MAX_LUMA_DIFF_OFF)
                .unwrap()
                .with_adaptive(0.0)
                .unwrap();
            assert_eq!(
                solve_default(&frame_with(mode, base, false)),
                solve_default(&frame_with(mode, explicit, false)),
                "{mode}"
            );
        }
    }

    #[test]
    fn max_luma_diff_zero_leaves_no_cross_luma_speckle() {
        for mode in [Mode::MulticolorBitmap, Mode::HiresBitmap] {
            for kind in [DitherKind::FloydSteinberg, DitherKind::Bayer4] {
                let free = Recipe::with_method(kind, 1.0).unwrap();
                let safe = free.with_max_luma_diff(0).unwrap();
                let (ff, fs) = (frame_with(mode, free, false), frame_with(mode, safe, false));
                let (a, b) = (solve_default(&ff), solve_default(&fs));
                assert!(
                    speckle(&a, &ff, 0) > 0,
                    "{mode} {kind:?}: the free dither speckles"
                );
                assert_eq!(speckle(&b, &fs, 0), 0, "{mode} {kind:?}");
                assert_eq!(b.report.summary().luma_crossings, 0);
                assert_eq!(validate(&b.image.to_indexed()), Ok(()));
            }
        }
    }

    #[test]
    fn cell_clamp_off_lets_error_cross_cells() {
        for mode in [Mode::MulticolorBitmap, Mode::HiresBitmap] {
            let on = Recipe::with_method(DitherKind::FloydSteinberg, 1.0).unwrap();
            let off = on.with_cell_clamp(false);
            let (a, b) = (
                solve_default(&frame_with(mode, on, false)),
                solve_default(&frame_with(mode, off, false)),
            );
            assert_ne!(a.image, b.image, "{mode}");
            assert_eq!(validate(&b.image.to_indexed()), Ok(()), "{mode}");
        }
    }

    #[test]
    fn adaptive_dithers_less_where_there_is_detail() {
        // Fine noise is all detail: adaptive 1 removes most of the dither.
        let fs = Recipe::with_method(DitherKind::FloydSteinberg, 1.0).unwrap();
        let dithered = |sol: &Solution, f: &TargetFrame| {
            // Pixels whose output differs from their nearest in-set colour.
            let input = Input::new(
                f,
                Solver::new(&Palette::pepto_pal()).tables(),
                InputOptions::default(),
            );
            let img = sol.image.to_indexed();
            let t = Solver::new(&Palette::pepto_pal());
            let w = usize::from(img.width());
            let (cw, ch) = input.cell;
            (0..img.pixels().len())
                .filter(|&i| {
                    let (x, y) = (i % w, i / w);
                    let cell = (y / ch) * input.cells.0 + x / cw;
                    let set: Vec<ColourIndex> = sol
                        .background
                        .map_or(sol.sets[cell], |b| sol.sets[cell].with(b))
                        .iter()
                        .collect();
                    let q = &input.px[i];
                    img.pixels()[i]
                        != super::super::ordered::nearest(
                            q.metric,
                            q.coords,
                            &set,
                            t.tables().colours(q.metric.space()),
                        )
                })
                .count()
        };
        let (fa, fb) = (
            frame_with(Mode::HiresBitmap, fs, true),
            frame_with(Mode::HiresBitmap, fs.with_adaptive(1.0).unwrap(), true),
        );
        let (a, b) = (solve_default(&fa), solve_default(&fb));
        assert!(
            dithered(&b, &fb) < dithered(&a, &fa) / 2,
            "{} vs {}",
            dithered(&b, &fb),
            dithered(&a, &fa)
        );
    }

    #[test]
    fn hysteresis_converges_without_flip_flop() {
        let f = frame_with(
            Mode::MulticolorBitmap,
            Recipe::with_method(DitherKind::FloydSteinberg, 1.0).unwrap(),
            true,
        );
        let sol = Solver::new(&Palette::pepto_pal()).solve(
            &f,
            &Steering::new(),
            None,
            &SolveOptions {
                hysteresis: Some(0.97),
                ..SolveOptions::default()
            },
        );
        assert!(sol.rounds.len() <= 3, "at most two refinement rounds");
        if sol.rounds.len() == 3 {
            let flips = (0..sol.rounds[0].len())
                .filter(|&i| {
                    sol.rounds[1][i] != sol.rounds[0][i] && sol.rounds[2][i] == sol.rounds[0][i]
                })
                .count();
            assert_eq!(flips, 0, "a cell went back to its first set");
        }
        // Hysteresis only ever keeps more cells unchanged than accepting every re-pick.
        let free = solve_default(&f);
        let changed = |r: &[Vec<ColourSet>]| {
            r.windows(2)
                .map(|w| w[0].iter().zip(&w[1]).filter(|(a, b)| a != b).count())
                .sum::<usize>()
        };
        assert!(changed(&sol.rounds) <= changed(&free.rounds));
    }

    #[test]
    fn switching_palette_rebuilds_the_tables() {
        let mut s = Solver::new(&Palette::pepto_pal());
        let t = s.tables().clone();
        s.set_palette(&Palette::colodore(50.0, 100.0, 50.0));
        assert_ne!(&t, s.tables());
        assert_eq!(s.palette().name(), "Colodore");
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(6))]
        /// Whatever the picture and recipe, the solver's output validates (E2's rules).
        #[test]
        fn any_solver_output_validates(
            seed in 0u32..10_000,
            kind in 0usize..9,
            strength in 0.0f32..=1.0,
            mc in proptest::prelude::any::<bool>(),
        ) {
            let mode = if mc { Mode::MulticolorBitmap } else { Mode::HiresBitmap };
            let mut st = seed;
            let px: Vec<[f32; 4]> = (0..320 * 200)
                .map(|i| {
                    if i % 37 == 0 {
                        st = st.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                    }
                    let v = (st >> 8) as f32 / 16_777_216.0;
                    let x = (i % 320) as f32 / 320.0;
                    [v, (v + x) % 1.0, (1.0 - v) * x, 1.0]
                })
                .collect();
            let mut l = Layer::new("r", Arc::new(SourceImage::from_linear(320, 200, px).unwrap()));
            l.set_recipe(Recipe::with_method(DitherKind::ALL[kind], strength).unwrap());
            let mut c = Compositor::new(SuperGrid::new(mode, 1).unwrap());
            c.update(&[l], &[]);
            let sol = Solver::new(&Palette::pepto_pal()).solve(
                c.frame(),
                &Steering::new(),
                None,
                &SolveOptions::default(),
            );
            proptest::prop_assert_eq!(validate(&sol.image.to_indexed()), Ok(()));
        }
    }
}
