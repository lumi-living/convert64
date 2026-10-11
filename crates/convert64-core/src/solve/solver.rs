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

/// Options of a solve.
#[derive(Clone, Debug, PartialEq)]
pub struct SolveOptions {
    /// Background choice.
    pub background: Background,
    /// Same-luma pair bias per pair in a cell's set (positive penalises, negative
    /// rewards; see [`super::metric::luma_pair_bias`]).
    pub luma_pair_bias: f32,
    /// Refinement rounds, 0–2.
    pub refinements: u8,
    /// Backgrounds carried from the search into dithering (at least 1).
    pub finalists: u8,
    /// Layer-locked pattern anchors, by layer index (see [`super::dither::layer_origins`]).
    pub layer_origins: Vec<(i64, i64)>,
    /// Per-pixel input options.
    pub input: InputOptions,
}

impl Default for SolveOptions {
    fn default() -> Self {
        Self {
            background: Background::Auto,
            luma_pair_bias: 0.0,
            refinements: 2,
            finalists: 3,
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
}

/// The solver for one palette. Its metric tables belong to that palette: the preview
/// and the quantiser use the same palette object (TAD §3.4), and changing it means a new
/// solver (or [`Self::set_palette`]) and a re-solve.
#[derive(Clone, Debug, PartialEq)]
pub struct Solver {
    tables: MetricTables,
}

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
        let input = Input::new(frame, &self.tables, opts.input);
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
    ) -> (PassResult, Vec<ColourSet>, u8) {
        let t = &self.tables;
        let locked = ctx.locked();
        let mut sets = cand.sets.clone();
        let mut r = pass(input, t, &sets, cand.bg, &opts.layer_origins, locked);
        let mut best = (r.clone(), sets.clone(), 0u8);
        for round in 1..=opts.refinements.min(2) {
            let moved = input.retarget(&r.received, t);
            let next: Vec<ColourSet> = (0..sets.len())
                .map(|i| {
                    let cell = input.cell_at(i);
                    ctx.locked_set(cell).unwrap_or_else(|| {
                        moved
                            .best_set(
                                cell,
                                cand.bg,
                                &ctx.steering.constraints(cell),
                                opts.luma_pair_bias,
                            )
                            .set
                    })
                })
                .collect();
            if next == sets {
                break;
            }
            sets = next;
            r = pass(input, t, &sets, cand.bg, &opts.layer_origins, locked);
            if r.error < best.0.error {
                best = (r.clone(), sets.clone(), round);
            }
        }
        best
    }

    fn solve_input(
        &self,
        input: &Input,
        steering: &Steering,
        previous: Option<&Solution>,
        opts: &SolveOptions,
    ) -> Solution {
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
        let (winner, (result, sets, refined)) = found
            .iter()
            .zip(dithered)
            .min_by(|(ca, ra), (cb, rb)| {
                ra.0.error
                    .total_cmp(&rb.0.error)
                    .then(bg_key(ca.bg).cmp(&bg_key(cb.bg)))
            })
            .expect("at least one candidate");
        let indexed = IndexedBitmap::new(input.grid.target(), result.out, winner.bg)
            .expect("the output has the target's size");
        let image = BitmapImage::encode(&indexed).expect("every cell keeps to its set");
        Solution {
            image,
            background: winner.bg,
            sets,
            error: result.error,
            refined,
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
        ] {
            let f = frame(mode, kind);
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
    fn switching_palette_rebuilds_the_tables() {
        let mut s = Solver::new(&Palette::pepto_pal());
        let t = s.tables().clone();
        s.set_palette(&Palette::colodore(50.0, 100.0, 50.0));
        assert_ne!(&t, s.tables());
        assert_eq!(s.palette().name(), "Colodore");
    }
}
