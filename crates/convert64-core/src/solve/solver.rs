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

use super::cell::Constraints;
use super::dither::{PassResult, pass};
use super::input::{Input, InputOptions};
use super::metric::MetricTables;
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

    /// Solves `frame` (the composite on the target grid with its attribute map).
    pub fn solve(&self, frame: &TargetFrame, opts: &SolveOptions) -> Solution {
        let input = Input::new(frame, &self.tables, opts.input);
        self.solve_input(&input, opts)
    }

    /// The best sets for one background.
    fn search(&self, input: &Input, bg: Option<ColourIndex>, bias: f32) -> Candidate {
        let n = input.cells.0 * input.cells.1;
        let mut score = 0.0;
        let sets = (0..n)
            .map(|i| {
                let c = input.best_set(input.cell_at(i), bg, &Constraints::FREE, bias);
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
        opts: &SolveOptions,
    ) -> (PassResult, Vec<ColourSet>, u8) {
        let t = &self.tables;
        let mut sets = cand.sets.clone();
        let mut r = pass(input, t, &sets, cand.bg, &opts.layer_origins);
        let mut best = (r.clone(), sets.clone(), 0u8);
        for round in 1..=opts.refinements.min(2) {
            let moved = input.retarget(&r.received, t);
            let n = sets.len();
            let next: Vec<ColourSet> = (0..n)
                .map(|i| {
                    moved
                        .best_set(
                            input.cell_at(i),
                            cand.bg,
                            &Constraints::FREE,
                            opts.luma_pair_bias,
                        )
                        .set
                })
                .collect();
            if next == sets {
                break;
            }
            sets = next;
            r = pass(input, t, &sets, cand.bg, &opts.layer_origins);
            if r.error < best.0.error {
                best = (r.clone(), sets.clone(), round);
            }
        }
        best
    }

    fn solve_input(&self, input: &Input, opts: &SolveOptions) -> Solution {
        let candidates: Vec<Option<ColourIndex>> = if !input.has_background() {
            vec![None]
        } else {
            match opts.background {
                Background::Fixed(b) => vec![Some(b)],
                Background::Auto => ColourIndex::ALL.into_iter().map(Some).collect(),
            }
        };
        let mut found = crate::par::map(&candidates, |&bg| {
            self.search(input, bg, opts.luma_pair_bias)
        });
        // Lowest search error first; the lower background index breaks ties.
        found.sort_by(|a, b| {
            a.score
                .total_cmp(&b.score)
                .then(bg_key(a.bg).cmp(&bg_key(b.bg)))
        });
        found.truncate(usize::from(opts.finalists.max(1)));
        let dithered = crate::par::map(&found, |c| self.refine(input, c, opts));
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
                let sol = s.solve(&frame(mode, kind), &SolveOptions::default());
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
            &SolveOptions {
                background: Background::Fixed(ColourIndex::YELLOW),
                ..SolveOptions::default()
            },
        );
        assert_eq!(fixed.background, Some(ColourIndex::YELLOW));
        let auto = s.solve(&f, &SolveOptions::default());
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
            &SolveOptions {
                refinements: 0,
                ..SolveOptions::default()
            },
        );
        let two = s.solve(&f, &SolveOptions::default());
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
                .install(|| s.solve(&f, &SolveOptions::default()));
            let many = rayon::ThreadPoolBuilder::new()
                .num_threads(8)
                .build()
                .unwrap()
                .install(|| s.solve(&f, &SolveOptions::default()));
            assert_eq!(one, many, "{mode} {kind:?}");
        }
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
