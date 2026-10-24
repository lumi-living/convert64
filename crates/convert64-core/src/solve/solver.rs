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

/// Whether the multicolor background is pinned (multicolor only).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Background {
    /// Not pinned: the solver chooses. The reference (png2c64's port) and the native
    /// solver search all 16; the png2amiga solver follows
    /// [`SolveOptions::background_rule`] (by default [`BackgroundRule::Black`],
    /// png2amiga's colour 0).
    #[default]
    Auto,
    /// Use this one, on every solver.
    Fixed(ColourIndex),
}

/// How the png2amiga solver chooses an unpinned multicolor background
/// ([`Background::Auto`]; #273, experimental, ADR-014 §5). Hi-res has no background, and
/// the reference (png2c64's port) and the native solver always search all 16, so this
/// acts only on the png2amiga solver in multicolor.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BackgroundRule {
    /// png2amiga's: always colour 0 (`c64.cpp:459`, `res.bg_color = 0`). The default,
    /// and what png2amiga's parity tests run with.
    #[default]
    Black,
    /// png2c64's outer loop over all 16 backgrounds, ported onto png2amiga's encoder
    /// ([`super::png2amiga_ref::c64::search_background`]): per background, every free
    /// cell's least `mse` error over the 455 triples of the other 15 colours (each
    /// pixel's squared OKLab distance to the nearest of the four, summed over the cell),
    /// summed over the cells; the lowest total wins, ties to the lower index.
    Search,
}

impl BackgroundRule {
    /// Whether this is the default (a project file writes it only when not).
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// Which solver solves a bitmap frame (#181, ADR-010, ADR-014).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SolverKind {
    /// The reference: png2c64's pipeline, ported (`png2c64_ref`; ADR-010, kept by
    /// ADR-014; the default). Frames it cannot solve fall back to the native solver
    /// ([`Solution::fallback`] says why). `png2c64` is accepted as another name for it.
    #[default]
    #[serde(alias = "png2c64")]
    Reference,
    /// png2amiga's C64 path, ported (`png2amiga_ref`, #261): a comparison solver
    /// (ADR-014 §2), with png2amiga's own front end ([`Self::pipeline`]). A frame it
    /// cannot solve goes to the reference if that can solve it, else to the native
    /// solver.
    Png2amiga,
    /// Convert64's own solver (E5, #157): background search, dither and refinement.
    Native,
}

impl SolverKind {
    /// Whose front end the compositor runs for this solver (ADR-014): png2amiga's for
    /// the png2amiga solver, png2c64's for the others.
    pub const fn pipeline(self) -> crate::composite::Pipeline {
        match self {
            Self::Png2amiga => crate::composite::Pipeline::Png2amiga,
            Self::Reference | Self::Native => crate::composite::Pipeline::Png2c64,
        }
    }
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
    /// Which solver (the reference by default).
    pub solver: SolverKind,
    /// The reference solver's named extras, all off by default (#181 step 7). The
    /// png2amiga solver has none: a frame asking for one goes to the reference.
    pub divergences: super::png2c64_ref::Divergences,
    /// png2amiga's `--cell-refine` after a png2amiga solve (#261; experimental, ADR-014
    /// §5): keeps the result only if its full-image SSIMULACRA2 improves. Off by
    /// default; the png2amiga solver only (the others ignore it), and only on a full
    /// solve with no locked cells.
    pub refine: bool,
    /// Full search or the interactive fast path.
    pub mode: SolveMode,
    /// Background choice.
    pub background: Background,
    /// How the png2amiga solver chooses an unpinned multicolor background (Black by
    /// default); the other solvers always search.
    pub background_rule: BackgroundRule,
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
            solver: SolverKind::Reference,
            divergences: super::png2c64_ref::Divergences::default(),
            refine: false,
            mode: SolveMode::Full,
            background: Background::Auto,
            background_rule: BackgroundRule::Black,
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
    /// The solver that made it.
    pub solver: SolverKind,
    /// Why the native solver made it although the reference was asked for.
    pub fallback: Option<String>,
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
        let reference = |why: Option<String>| match super::reference::unsupported(frame, opts) {
            None => Ok(Solution {
                fallback: why.clone(),
                ..super::reference::solve(frame, self.palette(), steering, previous, opts)
            }),
            Some(own) => Err(match why {
                Some(w) => format!("{w}; reference (png2c64): {own}"),
                None => own,
            }),
        };
        let fallback = match opts.solver {
            SolverKind::Native => None,
            SolverKind::Reference => match reference(None) {
                Ok(sol) => return sol,
                Err(why) => Some(why),
            },
            SolverKind::Png2amiga => match super::reference_png2amiga::unsupported(frame, opts) {
                None => {
                    return super::reference_png2amiga::solve(
                        frame,
                        self.palette(),
                        steering,
                        previous,
                        opts,
                    );
                }
                Some(why) => match reference(Some(format!("png2amiga: {why}"))) {
                    Ok(sol) => return sol,
                    Err(why) => Some(why),
                },
            },
        };
        // The fast path needs tables only for the cells it re-solves.
        let only = match (opts.mode, previous) {
            (SolveMode::Interactive { dirty }, Some(_)) => Some(dirty),
            _ => None,
        };
        let mut input = Input::new_for(frame, &self.tables, opts.input, only);
        input.pair_bias = opts.luma_pair_bias;
        Solution {
            fallback,
            ..self.solve_input(&input, steering, previous, opts)
        }
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
                    // A cell that received no error re-picks what it already has
                    // (and the rendered and blur/ssim re-picks are not cheap).
                    let still = input
                        .pixels_of(i)
                        .all(|(x, y)| r.received[y * input.size.0 + x] == [0.0; 3]);
                    if still && ctx.locked_set(cell).is_none() {
                        return sets[i];
                    }
                    ctx.locked_set(cell).unwrap_or_else(|| {
                        let pick = moved.pick_set(
                            t,
                            cell,
                            cand.bg,
                            &ctx.steering.constraints(cell),
                            opts.luma_pair_bias,
                        );
                        match opts.hysteresis {
                            Some(ratio) if pick.set != sets[i] => {
                                let current =
                                    moved.set_score(t, i, cand.bg, sets[i], opts.luma_pair_bias);
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
        // Cells scored by `blur` or `ssim`, and cells of png2c64-ordered pixels (by what
        // they render, #156), re-pick their sets for the finalists only (the
        // background search stays on squared distances, #120).
        // Not on the interactive fast path: the release's full solve does it.
        if interactive.is_none()
            && input
                .cell_metric
                .iter()
                .zip(&input.rendered)
                .any(|(m, r)| *m != crate::layer::CellMetric::Mse || *r)
        {
            for cand in &mut found {
                let n = cand.sets.len();
                let idx: Vec<usize> = (0..n).collect();
                let repicked = crate::par::map(&idx, |&i| {
                    let cell = input.cell_at(i);
                    if (input.cell_metric[i] == crate::layer::CellMetric::Mse && !input.rendered[i])
                        || ctx.locked_set(cell).is_some()
                    {
                        cand.sets[i]
                    } else {
                        input
                            .pick_set(
                                &self.tables,
                                cell,
                                cand.bg,
                                &ctx.steering.constraints(cell),
                                opts.luma_pair_bias,
                            )
                            .set
                    }
                });
                cand.sets = repicked;
            }
        }
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
            solver: SolverKind::Native,
            fallback: None,
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

    /// The reference path on a frame is png2c64's port run on the frame's colours, bit
    /// for bit, in both modes and for ordered, diffusion and `none` recipes. The
    /// compositor's frames have been through png2c64's preprocessing already (ADR-012),
    /// so the solver takes their colours as they are.
    #[test]
    fn the_reference_path_is_the_port_on_the_frame() {
        use super::super::png2c64_ref::dither::{PxSettings, Settings};
        use super::super::png2c64_ref::quantize::Mode as RefMode;
        use super::super::png2c64_ref::{Image, colour, convert};
        let s = Solver::new(&Palette::pepto_pal());
        for (mode, rm) in [
            (Mode::MulticolorBitmap, RefMode::Multicolor),
            (Mode::HiresBitmap, RefMode::Hires),
        ] {
            for kind in [
                DitherKind::None,
                DitherKind::Bayer4,
                DitherKind::FloydSteinberg,
                DitherKind::Ostromoukhov,
            ] {
                let f = frame(mode, kind);
                let sol = s.solve(&f, &Steering::new(), None, &SolveOptions::default());
                assert_eq!(sol.solver, SolverKind::Reference, "{mode} {kind:?}");
                assert_eq!(sol.fallback, None);
                let (w, h) = rm.screen();
                assert!(f.prepared());
                let im = Image::new(w, h, f.colour().to_vec());
                let settings = Settings {
                    method: super::super::reference::ref_method(kind).unwrap(),
                    strength: f.attributes().pixels()[0].strength,
                    ..Settings::default()
                };
                assert_eq!(
                    PxSettings::from(&settings).strength,
                    f.attributes().pixels()[w * h / 2].strength
                );
                let port = convert(
                    &im,
                    &colour::palette_of(&Palette::pepto_pal()),
                    rm,
                    &settings,
                );
                let ours: Vec<u8> = sol
                    .image
                    .to_indexed()
                    .pixels()
                    .iter()
                    .map(|c| c.get())
                    .collect();
                assert_eq!(ours, port.indices(), "{mode} {kind:?}");
                assert_eq!(validate(&sol.image.to_indexed()), Ok(()));
            }
        }
    }

    /// #275: png2amiga's 32 methods png2c64 lacks are the reference's, on png2c64's
    /// pipeline: a frame of each is `png2c64_ref::amiga::convert` (the hook #280's sheets
    /// were judged on) run on the frame's colours, bit for bit, in both modes, with no
    /// fallback; the output is legal.
    #[test]
    fn png2amiga_s_methods_are_the_reference_s_on_png2c64_s_pipeline() {
        use super::super::png2c64_ref::quantize::Mode as RefMode;
        use super::super::png2c64_ref::{Image, amiga, colour};
        let s = Solver::new(&Palette::pepto_pal());
        let pal = colour::palette_of(&Palette::pepto_pal());
        let kinds: Vec<DitherKind> = DitherKind::ALL
            .into_iter()
            .filter(|k| k.is_png2amiga_only())
            .collect();
        assert_eq!(kinds.len(), 32);
        for (mode, rm) in [
            (Mode::MulticolorBitmap, RefMode::Multicolor),
            (Mode::HiresBitmap, RefMode::Hires),
        ] {
            for &kind in &kinds {
                let f = frame(mode, kind);
                let sol = s.solve(&f, &Steering::new(), None, &SolveOptions::default());
                assert_eq!(sol.solver, SolverKind::Reference, "{mode} {kind:?}");
                assert_eq!(sol.fallback, None, "{mode} {kind:?}");
                let (w, h) = rm.screen();
                let im = Image::new(w, h, f.colour().to_vec());
                let port = amiga::convert(
                    &im,
                    &pal,
                    rm,
                    super::super::reference::amiga_method(kind).unwrap(),
                    f.attributes().pixels()[0].strength,
                    Recipe::default().error_clamp(),
                );
                let ours: Vec<u8> = indices(&sol).iter().map(|c| c.get()).collect();
                assert_eq!(ours, port.indices(), "{mode} {kind:?}");
                assert_eq!(validate(&sol.image.to_indexed()), Ok(()));
            }
        }
    }

    /// #275: png2amiga's methods mix with png2c64's in one frame and keep the reference's
    /// hooks: a locked cell is copied through, and the interactive fast path re-solves
    /// only the dirty cells.
    #[test]
    fn png2amiga_s_methods_keep_the_reference_s_hooks() {
        use crate::geom::CellSet;
        use crate::model::CellPos;
        let s = Solver::new(&Palette::pepto_pal());
        for kind in [
            DitherKind::Bayer5,
            DitherKind::StructureFs,
            DitherKind::TriTone,
        ] {
            let f = frame(Mode::MulticolorBitmap, DitherKind::Checker);
            let first = s.solve(&f, &Steering::new(), None, &SolveOptions::default());
            let g = frame(Mode::MulticolorBitmap, kind);
            let mut steering = Steering::new();
            let cell = CellPos::new(3, 4);
            steering.set_lock(cell, true);
            let locked = s.solve(&g, &steering, Some(&first), &SolveOptions::default());
            assert_eq!(locked.solver, SolverKind::Reference);
            let (a, b) = (first.image.to_indexed(), locked.image.to_indexed());
            for y in 32..40 {
                for x in 12..16 {
                    let i = y * 160 + x;
                    assert_eq!(a.pixels()[i], b.pixels()[i], "{kind:?} ({x}, {y})");
                }
            }
            assert_eq!(validate(&b), Ok(()));
            let mut dirty = CellSet::EMPTY;
            dirty.insert_rect(10, 5, 14, 9);
            let fast = s.solve(
                &g,
                &Steering::new(),
                Some(&first),
                &SolveOptions {
                    mode: SolveMode::Interactive { dirty },
                    ..SolveOptions::default()
                },
            );
            assert_eq!(fast.solver, SolverKind::Reference);
            assert_eq!(fast.background, first.background);
            for y in 0..25 {
                for x in 0..40 {
                    let c = CellPos::new(x, y);
                    if !dirty.contains(c) {
                        assert_eq!(
                            cell_bytes(&fast.image, c),
                            cell_bytes(&first.image, c),
                            "{kind:?} {c}"
                        );
                    }
                }
            }
            assert_eq!(validate(&fast.image.to_indexed()), Ok(()));
        }
    }

    #[test]
    fn frames_the_reference_cannot_solve_fall_back_to_native() {
        let s = Solver::new(&Palette::pepto_pal());
        let f = frame(Mode::MulticolorBitmap, DitherKind::Yliluoma1);
        let sol = s.solve(&f, &Steering::new(), None, &SolveOptions::default());
        assert_eq!(sol.solver, SolverKind::Native);
        assert!(
            sol.fallback
                .as_deref()
                .is_some_and(|w| w.contains("png2c64"))
        );
        let knob = frame_with(
            Mode::MulticolorBitmap,
            Recipe::with_method(DitherKind::Bayer4, 1.0)
                .unwrap()
                .with_max_luma_diff(0)
                .unwrap(),
            false,
        );
        let sol = s.solve(&knob, &Steering::new(), None, &SolveOptions::default());
        assert_eq!(sol.solver, SolverKind::Native);
        assert!(sol.fallback.unwrap().contains("max_luma_diff"));
        let f = frame(Mode::MulticolorBitmap, DitherKind::Bayer4);
        let biased = SolveOptions {
            luma_pair_bias: 0.01,
            ..SolveOptions::default()
        };
        assert_eq!(
            s.solve(&f, &Steering::new(), None, &biased).solver,
            SolverKind::Native
        );
    }

    /// ADR-014 §2: `--solver png2amiga` is png2amiga's port run on the frame's colours,
    /// bit for bit, in both modes and for ordered, diffusion, palette-aware and `none`
    /// recipes; the frame has been through a front end already, so the solver takes its
    /// colours as they are.
    #[test]
    fn the_png2amiga_solver_is_png2amiga_s_port_on_the_frame() {
        use super::super::png2amiga_ref::c64::Metric as RefMetric;
        use super::super::png2amiga_ref::{Image, Mode as RefMode, Settings, colour, convert};
        let s = Solver::new(&Palette::pepto_pal());
        let recipe = Recipe::default();
        let opts = SolveOptions {
            solver: SolverKind::Png2amiga,
            ..SolveOptions::default()
        };
        for (mode, rm) in [
            (Mode::MulticolorBitmap, RefMode::Multicolor),
            (Mode::HiresBitmap, RefMode::Hires),
        ] {
            for kind in [
                DitherKind::None,
                DitherKind::Bayer4,
                DitherKind::FloydSteinberg,
                DitherKind::Yliluoma1,
                DitherKind::OptChecker,
            ] {
                let f = frame(mode, kind);
                let sol = s.solve(&f, &Steering::new(), None, &opts);
                assert_eq!(sol.solver, SolverKind::Png2amiga, "{mode} {kind:?}");
                assert_eq!(sol.fallback, None);
                let (w, h) = rm.screen();
                assert!(f.prepared());
                let im = Image::new(w, h, f.colour().to_vec());
                let strength = f.attributes().pixels()[0].strength;
                let settings = Settings {
                    method: super::super::reference_png2amiga::ref_method(kind).unwrap(),
                    strength,
                    error_clamp: recipe.error_clamp(),
                    serpentine: true,
                    metric: RefMetric::Mse,
                };
                let port = convert(
                    &im,
                    &colour::palette_of(&Palette::pepto_pal()),
                    rm,
                    &settings,
                );
                let ours: Vec<u8> = indices(&sol).iter().map(|c| c.get()).collect();
                assert_eq!(ours, port.indices(), "{mode} {kind:?}");
                assert_eq!(validate(&sol.image.to_indexed()), Ok(()));
            }
        }
    }

    /// ADR-014: a frame the png2amiga solver cannot solve goes to the reference
    /// (png2c64's port), then to native; the reference falls back to native only, and
    /// `png2c64` names the reference.
    #[test]
    fn the_png2amiga_solver_falls_back_to_the_reference_then_native() {
        let s = Solver::new(&Palette::pepto_pal());
        let p2a = SolveOptions {
            solver: SolverKind::Png2amiga,
            ..SolveOptions::default()
        };
        // png2c64 has Ostromoukhov, png2amiga has not.
        let f = frame(Mode::MulticolorBitmap, DitherKind::Ostromoukhov);
        let sol = s.solve(&f, &Steering::new(), None, &p2a);
        assert_eq!(sol.solver, SolverKind::Reference);
        assert!(
            sol.fallback
                .as_deref()
                .is_some_and(|w| w.contains("png2amiga"))
        );
        // png2amiga has the optimal checker, png2c64 has not: the reference runs it on
        // png2c64's pipeline (#275), the png2amiga solver as png2amiga does.
        let f = frame(Mode::MulticolorBitmap, DitherKind::OptChecker);
        let sol = s.solve(&f, &Steering::new(), None, &SolveOptions::default());
        assert_eq!(sol.solver, SolverKind::Reference);
        assert_eq!(sol.fallback, None);
        let sol = s.solve(&f, &Steering::new(), None, &p2a);
        assert_eq!(sol.solver, SolverKind::Png2amiga);
        // Neither has three-row Sierra: native, both reasons given.
        let f = frame(Mode::MulticolorBitmap, DitherKind::Sierra);
        let sol = s.solve(&f, &Steering::new(), None, &p2a);
        assert_eq!(sol.solver, SolverKind::Native);
        assert!(
            sol.fallback
                .as_deref()
                .is_some_and(|w| w.contains("png2amiga") && w.contains("png2c64"))
        );
        use serde::de::{IntoDeserializer, value::Error};
        let named = SolverKind::deserialize("png2c64".into_deserializer()).map_err(|e: Error| e);
        assert_eq!(named, Ok(SolverKind::Reference));
    }

    /// The reference path's output as palette indices.
    fn indices(sol: &Solution) -> Vec<ColourIndex> {
        sol.image.to_indexed().pixels().to_vec()
    }

    /// #181 step 6: steering that constrains nothing (allow everything, deny nothing),
    /// a region with the layer's own recipe and a full mask leave the reference's output
    /// png2c64's.
    #[test]
    fn neutral_hooks_leave_the_reference_untouched() {
        let s = Solver::new(&Palette::pepto_pal());
        for mode in [Mode::MulticolorBitmap, Mode::HiresBitmap] {
            for kind in [
                DitherKind::Bayer4,
                DitherKind::FloydSteinberg,
                DitherKind::None,
            ] {
                let f = frame(mode, kind);
                let plain = s.solve(&f, &Steering::new(), None, &SolveOptions::default());
                let mut neutral = Steering::new();
                for x in 0..40 {
                    for y in 0..25 {
                        let cell = crate::model::CellPos::new(x, y);
                        neutral.set_allow(cell, ColourSet::ALL);
                        neutral.set_deny(cell, ColourSet::EMPTY);
                    }
                }
                let steered = s.solve(&f, &neutral, None, &SolveOptions::default());
                assert_eq!(steered.solver, SolverKind::Reference);
                assert_eq!(
                    indices(&steered),
                    indices(&plain),
                    "{mode} {kind:?} steering"
                );

                // The same picture under a region with the layer's recipe, and with a
                // mask of all ones.
                let px: Vec<[f32; 4]> = (0..320 * 200)
                    .map(|i| {
                        let (x, y) = ((i % 320) as f32 / 320.0, (i / 320) as f32 / 200.0);
                        [x * x, y, (1.0 - x) * 0.6 + 0.2 * y, 1.0]
                    })
                    .collect();
                let recipe = Recipe::with_method(kind, 0.8).unwrap();
                let mut l = Layer::new(
                    "g",
                    Arc::new(SourceImage::from_linear(320, 200, px).unwrap()),
                );
                l.set_recipe(recipe);
                l.set_mask(Some(crate::mask::Mask::new(
                    crate::mask::MaskRaster::filled(320, 200, 1.0).unwrap(),
                )))
                .unwrap();
                // A region over the whole canvas: at a region's edge the compositor
                // feathers the strength (its mask is resampled), which is upstream.
                let whole = vec![1.0f32; 320 * 200];
                let region = crate::region::Region::new(
                    "same",
                    crate::mask::MaskRaster::from_values(320, 200, whole).unwrap(),
                    recipe,
                )
                .unwrap();
                let mut c = Compositor::new(SuperGrid::new(mode, 2).unwrap());
                c.update(&[l], &[region]);
                let hooked = s.solve(c.frame(), &Steering::new(), None, &SolveOptions::default());
                assert_eq!(hooked.solver, SolverKind::Reference);
                assert_eq!(
                    indices(&hooked),
                    indices(&plain),
                    "{mode} {kind:?} mask + region"
                );
            }
        }
    }

    #[test]
    fn pins_and_allow_deny_constrain_reference_cells() {
        use crate::model::CellPos;
        let s = Solver::new(&Palette::pepto_pal());
        for mode in [Mode::MulticolorBitmap, Mode::HiresBitmap] {
            for kind in [DitherKind::Bayer4, DitherKind::FloydSteinberg] {
                let f = frame(mode, kind);
                let mut st = Steering::new();
                let (full, part, deny) =
                    (CellPos::new(5, 5), CellPos::new(6, 5), CellPos::new(7, 5));
                let pair: ColourSet = [ColourIndex::RED, ColourIndex::BLUE].into_iter().collect();
                st.set_pin(
                    full,
                    super::super::steering::Pin {
                        colours: pair,
                        full: true,
                    },
                );
                st.set_pin(
                    part,
                    super::super::steering::Pin {
                        colours: ColourSet::EMPTY.with(ColourIndex::GREEN),
                        full: false,
                    },
                );
                let plain = s.solve(&f, &Steering::new(), None, &SolveOptions::default());
                let banned = plain.sets[7 + 5 * 40];
                st.set_deny(deny, banned);
                let sol = s.solve(&f, &st, None, &SolveOptions::default());
                assert_eq!(sol.solver, SolverKind::Reference);
                assert_eq!(validate(&sol.image.to_indexed()), Ok(()));
                let own = |c: CellPos| sol.sets[usize::from(c.y) * 40 + usize::from(c.x)];
                assert_eq!(own(full), pair, "{mode} {kind:?} full pin");
                assert!(
                    own(part).contains(ColourIndex::GREEN),
                    "{mode} {kind:?} pin"
                );
                assert!(
                    own(deny).intersection(banned).is_empty(),
                    "{mode} {kind:?} deny"
                );
            }
        }
    }

    #[test]
    fn locked_and_clean_cells_are_copied_through() {
        use crate::model::CellPos;
        let s = Solver::new(&Palette::pepto_pal());
        for mode in [Mode::MulticolorBitmap, Mode::HiresBitmap] {
            for kind in [DitherKind::Bayer4, DitherKind::FloydSteinberg] {
                let before = frame(mode, kind);
                let prev = s.solve(&before, &Steering::new(), None, &SolveOptions::default());
                // A different picture: the locked cells keep the previous result.
                let after = frame_with(mode, Recipe::with_method(kind, 1.0).unwrap(), true);
                let mut st = Steering::new();
                let locked: Vec<CellPos> = (0..10).map(|x| CellPos::new(x, 3)).collect();
                for &c in &locked {
                    st.set_lock(c, true);
                }
                let sol = s.solve(&after, &st, Some(&prev), &SolveOptions::default());
                assert_eq!(sol.solver, SolverKind::Reference);
                assert_eq!(sol.background, prev.background);
                assert_eq!(validate(&sol.image.to_indexed()), Ok(()));
                let (a, b) = (indices(&sol), indices(&prev));
                let (w, cw, ch) = if mode == Mode::MulticolorBitmap {
                    (160, 4, 8)
                } else {
                    (320, 8, 8)
                };
                for c in &locked {
                    for y in usize::from(c.y) * ch..(usize::from(c.y) + 1) * ch {
                        for x in usize::from(c.x) * cw..(usize::from(c.x) + 1) * cw {
                            assert_eq!(a[y * w + x], b[y * w + x], "{mode} {kind:?} {c}");
                        }
                    }
                }
                // Interactive: only the dirty cells are re-solved, on the reference path.
                let dirty: crate::geom::CellSet = [CellPos::new(20, 12), CellPos::new(21, 12)]
                    .into_iter()
                    .collect();
                let quick = s.solve(
                    &after,
                    &Steering::new(),
                    Some(&prev),
                    &SolveOptions {
                        mode: SolveMode::Interactive { dirty },
                        ..SolveOptions::default()
                    },
                );
                assert_eq!(quick.solver, SolverKind::Reference);
                assert_eq!(quick.background, prev.background);
                let q = indices(&quick);
                let mut changed_outside = 0;
                for (i, (x, y)) in (0..q.len()).map(|i| (i % w, i / w)).enumerate() {
                    let cell = CellPos::new((x / cw) as u16, (y / ch) as u16);
                    if !dirty.contains(cell) && q[i] != b[i] {
                        changed_outside += 1;
                    }
                }
                assert_eq!(changed_outside, 0, "{mode} {kind:?}");
                assert_ne!(
                    q, b,
                    "{mode} {kind:?}: the dirty cells follow the new picture"
                );
            }
        }
    }

    /// #181 step 7: each named divergence, on its own, changes the reference's output
    /// and keeps it legal; all off is png2c64's.
    #[test]
    fn every_divergence_changes_the_output_and_stays_legal() {
        use super::super::png2c64_ref::Divergences;
        let s = Solver::new(&Palette::pepto_pal());
        let cases: [(&str, DitherKind, Divergences); 6] = [
            (
                "chroma_guard",
                DitherKind::Bayer4,
                Divergences {
                    chroma_guard: true,
                    ..Divergences::default()
                },
            ),
            (
                "max_luma_diff",
                DitherKind::FloydSteinberg,
                Divergences {
                    max_luma_diff: Some(1),
                    ..Divergences::default()
                },
            ),
            (
                "pair_bias",
                DitherKind::Bayer4,
                Divergences {
                    pair_bias: 0.05,
                    ..Divergences::default()
                },
            ),
            (
                "cell_clamp",
                DitherKind::FloydSteinberg,
                Divergences {
                    cell_clamp: true,
                    ..Divergences::default()
                },
            ),
            (
                "mix_levels",
                DitherKind::Bayer8,
                Divergences {
                    mix_levels: Some(2),
                    ..Divergences::default()
                },
            ),
            (
                "bridging",
                DitherKind::Bayer4,
                Divergences {
                    bridging: true,
                    ..Divergences::default()
                },
            ),
        ];
        for mode in [Mode::MulticolorBitmap, Mode::HiresBitmap] {
            for (name, kind, div) in cases {
                let f = frame(mode, kind);
                let off = s.solve(&f, &Steering::new(), None, &SolveOptions::default());
                let on = s.solve(
                    &f,
                    &Steering::new(),
                    None,
                    &SolveOptions {
                        divergences: div,
                        ..SolveOptions::default()
                    },
                );
                assert_eq!(on.solver, SolverKind::Reference, "{mode} {name}");
                assert_eq!(validate(&on.image.to_indexed()), Ok(()), "{mode} {name}");
                assert_ne!(indices(&on), indices(&off), "{mode} {name} changes nothing");
            }
        }
    }

    #[test]
    fn the_chroma_guard_keeps_a_grey_ramp_grey() {
        use super::super::png2c64_ref::Divergences;
        let greys = [0u8, 1, 11, 12, 15];
        let px: Vec<[f32; 4]> = (0..320 * 200)
            .map(|i| {
                let v = (i % 320) as f32 / 319.0;
                [v, v, v, 1.0]
            })
            .collect();
        let mut l = Layer::new(
            "ramp",
            Arc::new(SourceImage::from_linear(320, 200, px).unwrap()),
        );
        l.set_recipe(Recipe::with_method(DitherKind::Bayer4, 1.0).unwrap());
        let mut c = Compositor::new(SuperGrid::new(Mode::MulticolorBitmap, 2).unwrap());
        c.update(&[l], &[]);
        let guarded = Solver::new(&Palette::pepto_pal()).solve(
            c.frame(),
            &Steering::new(),
            None,
            &SolveOptions {
                divergences: Divergences {
                    chroma_guard: true,
                    ..Divergences::default()
                },
                ..SolveOptions::default()
            },
        );
        let chromatic = indices(&guarded)
            .iter()
            .filter(|c| !greys.contains(&c.get()))
            .count();
        assert_eq!(chromatic, 0, "{chromatic} chromatic pixels in a grey ramp");
    }

    #[test]
    fn max_luma_diff_zero_keeps_mixes_to_one_luma_level() {
        use super::super::png2c64_ref::Divergences;
        use crate::palette::luma::level_diff;
        let s = Solver::new(&Palette::pepto_pal());
        for kind in [DitherKind::FloydSteinberg, DitherKind::Bayer4] {
            let f = frame(Mode::MulticolorBitmap, kind);
            let solve = |div| {
                s.solve(
                    &f,
                    &Steering::new(),
                    None,
                    &SolveOptions {
                        divergences: div,
                        ..SolveOptions::default()
                    },
                )
            };
            // Horizontal neighbours in a cell more than one luma level apart.
            let jumps = |sol: &Solution| {
                let px = indices(sol);
                (0..px.len())
                    .filter(|&i| i % 4 != 3 && level_diff(px[i], px[i + 1]) > 1)
                    .count()
            };
            let free = jumps(&solve(Divergences::default()));
            let safe = jumps(&solve(Divergences {
                max_luma_diff: Some(0),
                ..Divergences::default()
            }));
            assert!(
                safe < free,
                "{kind:?}: {safe} jumps with the limit, {free} without"
            );
        }
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
        // Native: refinement keeps the best final error. The reference picks the
        // background by its search error, as png2c64 does, so "no worse" is not its
        // promise.
        let s = Solver::new(&Palette::pepto_pal());
        let f = frame(Mode::MulticolorBitmap, DitherKind::None);
        let native = SolveOptions {
            solver: SolverKind::Native,
            ..SolveOptions::default()
        };
        let fixed = s.solve(
            &f,
            &Steering::new(),
            None,
            &SolveOptions {
                background: Background::Fixed(ColourIndex::YELLOW),
                ..native.clone()
            },
        );
        assert_eq!(fixed.background, Some(ColourIndex::YELLOW));
        let auto = s.solve(&f, &Steering::new(), None, &native);
        assert!(
            auto.error <= fixed.error,
            "{} > {}",
            auto.error,
            fixed.error
        );
        let reference = s.solve(
            &f,
            &Steering::new(),
            None,
            &SolveOptions {
                background: Background::Fixed(ColourIndex::YELLOW),
                ..SolveOptions::default()
            },
        );
        assert_eq!(reference.solver, SolverKind::Reference);
        assert_eq!(reference.background, Some(ColourIndex::YELLOW));
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
            (Mode::MulticolorBitmap, DitherKind::Yliluoma2),
            (Mode::MulticolorBitmap, DitherKind::Pair),
            (Mode::HiresBitmap, DitherKind::Riemersma),
        ] {
            let mut f = frame(mode, kind);
            if kind == DitherKind::Yliluoma2 {
                // The blur cell metric too.
                let r = Recipe::with_method(kind, 0.8)
                    .unwrap()
                    .with_cell_metric(crate::layer::CellMetric::Blur);
                f = frame_with(mode, r, true);
            }
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

    /// The native solver: what the recipe knobs below (`max_luma_diff`, `cell_clamp`)
    /// act on; the reference path has its own, opt-in versions (#181).
    fn solve_native(f: &TargetFrame) -> Solution {
        Solver::new(&Palette::pepto_pal()).solve(
            f,
            &Steering::new(),
            None,
            &SolveOptions {
                solver: SolverKind::Native,
                ..SolveOptions::default()
            },
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
        .unwrap()
        // These test E5's mix levels, which only the legacy semantics have (#156).
        .with_ordered_semantics(crate::layer::OrderedSemantics::Legacy);
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
    fn a_two_level_pattern_does_not_turn_dark_olive_black() {
        // #154: portrait's background after gamma 1.5, with checker at 70 %, used to
        // win with a black background and render flat black.
        let olive = [0.0105f32, 0.0125, 0.0055, 1.0];
        let mut l = Layer::new(
            "olive",
            Arc::new(SourceImage::from_linear(320, 200, vec![olive; 320 * 200]).unwrap()),
        );
        l.set_recipe(Recipe::with_method(DitherKind::Checker, 0.7).unwrap());
        let mut c = Compositor::new(SuperGrid::new(Mode::MulticolorBitmap, 2).unwrap());
        c.update(&[l], &[]);
        let sol = Solver::new(&Palette::pepto_pal()).solve(
            c.frame(),
            &Steering::new(),
            None,
            &SolveOptions::default(),
        );
        let px = sol.image.to_indexed();
        let black = px
            .pixels()
            .iter()
            .filter(|&&p| p == ColourIndex::BLACK)
            .count();
        assert!(black * 10 < px.pixels().len() * 9, "{black} black pixels");
        assert!(
            px.pixels().contains(&ColourIndex::BROWN),
            "no brown: {:?}",
            sol.background
        );
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
                let (a, b) = (solve_native(&ff), solve_native(&fs));
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
                solve_native(&frame_with(mode, on, false)),
                solve_native(&frame_with(mode, off, false)),
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
        // Pixel-level noise between a dark and a mid grey: every pixel's mean OKLab
        // |ΔL| to its neighbours is past png2c64's 0.15, so detail is 1 everywhere.
        let detail = |r: Recipe| {
            let mut st = 5u32;
            let px: Vec<[f32; 4]> = (0..320 * 200)
                .map(|_| {
                    st = st.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                    let v = if (st >> 16) & 1 == 0 { 0.03 } else { 0.35 };
                    [v, v * 0.9, v * 0.8, 1.0]
                })
                .collect();
            let mut l = Layer::new(
                "noise",
                Arc::new(SourceImage::from_linear(320, 200, px).unwrap()),
            );
            l.set_recipe(r);
            let mut c = Compositor::new(SuperGrid::new(Mode::HiresBitmap, 2).unwrap());
            c.update(&[l], &[]);
            c.frame().clone()
        };
        let (fa, fb) = (detail(fs), detail(fs.with_adaptive(1.0).unwrap()));
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
        let free = solve_native(&f);
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
            kind in 0usize..DitherKind::ALL.len(),
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
