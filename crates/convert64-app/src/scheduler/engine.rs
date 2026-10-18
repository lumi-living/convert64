//! What both drivers share: the compositor and solver a scheduler keeps between
//! submissions ([`Engine`]), and one submission's work cut into stages ([`Job`]).
//!
//! A job runs [`Stage::Composite`], then, for a full solve with the preview on and a
//! previous solution, [`Stage::Preview`] (the interactive fast path, delivered as
//! provisional), then [`Stage::Solve`]. Each stage is one call into the core, so the
//! output of every solve is exactly `Solver::solve` on the compositor's frame. The web
//! worker driver (#265) runs the composite stage here and the solve stages in the worker
//! ([`super::remote`]), through the same [`solve`].

use super::{Snapshot, SolveResult, Timing, clock};
use convert64_core::solve::dither::layer_origins;
use convert64_core::solve::{Solution, SolveMode, SolveOptions, Solver, Steering};
use convert64_core::{CellPos, CellSet, Compositor, Palette, TargetFrame};
use std::collections::VecDeque;
use std::sync::{Arc, Weak};

/// Composites remembered for widening a fast path's dirty cells (one per job that got
/// past its composite stage).
const LOG_LEN: usize = 64;
/// Solutions remembered as possible bases of a fast path.
const PRODUCED_LEN: usize = 16;

/// One stage of a [`Job`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Stage {
    /// Bring the compositor up to date with the snapshot.
    Composite,
    /// A full solve's interactive preview.
    Preview,
    /// The submitted solve.
    Solve,
    /// Nothing left.
    Done,
}

/// Which kind of work a stage is, for the stepping driver's time estimates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Work {
    /// The composite stage.
    Composite = 0,
    /// An interactive solve (a preview or an interactive submission).
    Interactive = 1,
    /// A full solve.
    Full = 2,
}

/// The solver for `palette` in `slot`: made, or switched to `palette` (new metric
/// tables) when it changed.
pub(crate) fn solver_for<'a>(slot: &'a mut Option<Solver>, palette: &Palette) -> &'a Solver {
    match slot {
        Some(s) => s.set_palette(palette),
        None => *slot = Some(Solver::new(palette)),
    }
    slot.as_ref().expect("set above")
}

/// One solve, as every driver runs it: `Solver::solve` on `frame` in `mode` with
/// `options`' other settings. Returns the solution and how long it took, ms.
pub(crate) fn solve(
    solver: &Solver,
    frame: &TargetFrame,
    steering: &Steering,
    previous: Option<&Solution>,
    options: &SolveOptions,
    mode: SolveMode,
) -> (Solution, f64) {
    let start = clock::now_ms();
    let opts = SolveOptions {
        mode,
        ..options.clone()
    };
    let solution = solver.solve(frame, steering, previous, &opts);
    (solution, clock::now_ms() - start)
}

/// The state kept between submissions: the compositor's caches, the solver's metric
/// tables, and enough history to know which cells changed since a solution was made.
#[derive(Debug, Default)]
pub(crate) struct Engine {
    compositor: Option<Compositor>,
    /// The last composited palette.
    palette: Option<Palette>,
    /// The solver of the in-thread drivers (made on the first solve).
    solver: Option<Solver>,
    /// The last composited options with `mode` and `layer_origins` cleared.
    options: Option<SolveOptions>,
    /// The last composited steering.
    steering: Option<Steering>,
    /// Counts composites.
    epoch: u64,
    /// Per composite: the cells whose input changed (composite, steering, palette, grid
    /// or options), oldest first.
    log: VecDeque<(u64, CellSet)>,
    /// Solutions this engine made and the composite each was solved from.
    produced: VecDeque<(Weak<Solution>, u64)>,
}

impl Engine {
    /// The cells whose input changed since `previous` was solved: every cell if this
    /// engine did not make `previous` (or no longer remembers it).
    fn dirty_since(&self, previous: Option<&Arc<Solution>>) -> CellSet {
        let made = previous.and_then(|p| {
            self.produced
                .iter()
                .find(|(w, _)| w.upgrade().is_some_and(|s| Arc::ptr_eq(&s, p)))
                .map(|(_, epoch)| *epoch)
        });
        let Some(made) = made else {
            return CellSet::all();
        };
        if self.log.front().is_some_and(|(first, _)| *first > made + 1) {
            return CellSet::all();
        }
        self.log
            .iter()
            .filter(|(epoch, _)| *epoch > made)
            .fold(CellSet::EMPTY, |acc, (_, cells)| acc.union(*cells))
    }

    fn record(&mut self, changed: CellSet) -> u64 {
        self.epoch += 1;
        if self.log.len() == LOG_LEN {
            self.log.pop_front();
        }
        self.log.push_back((self.epoch, changed));
        self.epoch
    }

    /// The composite the last composite stage made.
    pub(crate) fn frame(&self) -> &TargetFrame {
        self.compositor
            .as_ref()
            .expect("the composite stage runs first")
            .frame()
    }

    /// Notes that this engine made `solution` from composite `epoch`.
    pub(crate) fn produced(&mut self, solution: &Arc<Solution>, epoch: u64) {
        self.produced.retain(|(w, _)| w.strong_count() > 0);
        if self.produced.len() == PRODUCED_LEN {
            self.produced.pop_front();
        }
        self.produced.push_back((Arc::downgrade(solution), epoch));
    }
}

/// The cells whose pins, locks or allow/deny lists differ.
fn steering_changes(old: &Steering, new: &Steering) -> CellSet {
    if old == new {
        return CellSet::EMPTY;
    }
    let mut cells = CellSet::EMPTY;
    for y in 0..CellSet::ROWS {
        for x in 0..CellSet::COLS {
            let c = CellPos::new(x, y);
            if old.pin(c) != new.pin(c)
                || old.locks().contains(c) != new.locks().contains(c)
                || old.allow(c) != new.allow(c)
                || old.deny(c) != new.deny(c)
            {
                cells.insert(c);
            }
        }
    }
    cells
}

/// Options with the per-solve parts cleared, to tell a settings change from a drag.
fn settings_of(options: &SolveOptions) -> SolveOptions {
    SolveOptions {
        mode: SolveMode::Full,
        layer_origins: Vec::new(),
        ..options.clone()
    }
}

/// One submission's work.
#[derive(Debug)]
pub(crate) struct Job {
    ticket: u64,
    snapshot: Snapshot,
    mode: SolveMode,
    preview: bool,
    stage: Stage,
    /// The composite this job's solves read (set by the composite stage).
    epoch: u64,
    composite_ms: f64,
    /// The fast path's dirty cells (set by the composite stage).
    dirty: CellSet,
    /// The options solved with (set by the composite stage).
    options: SolveOptions,
}

impl Job {
    /// A job for submission `ticket`. With `preview`, a full solve that has a previous
    /// solution delivers the interactive fast path first.
    pub(crate) fn new(ticket: u64, snapshot: Snapshot, mode: SolveMode, preview: bool) -> Self {
        Self {
            ticket,
            snapshot,
            mode,
            preview,
            stage: Stage::Composite,
            epoch: 0,
            composite_ms: 0.0,
            dirty: CellSet::EMPTY,
            options: SolveOptions::default(),
        }
    }

    /// The submission this job runs.
    pub(crate) const fn ticket(&self) -> u64 {
        self.ticket
    }

    /// Whether every stage has run.
    pub(crate) fn is_done(&self) -> bool {
        self.stage == Stage::Done
    }

    /// The kind of work the next stage does (`None` when done).
    pub(crate) fn next_work(&self) -> Option<Work> {
        match (self.stage, self.mode) {
            (Stage::Composite, _) => Some(Work::Composite),
            (Stage::Preview, _) | (Stage::Solve, SolveMode::Interactive { .. }) => {
                Some(Work::Interactive)
            }
            (Stage::Solve, SolveMode::Full) => Some(Work::Full),
            (Stage::Done, _) => None,
        }
    }

    /// Runs the next stage on `engine`; returns the result it produced, if any.
    pub(crate) fn step(&mut self, engine: &mut Engine) -> Option<SolveResult> {
        match self.stage {
            Stage::Composite => {
                self.composite(engine);
                let preview = self.preview
                    && self.mode == SolveMode::Full
                    && self.snapshot.previous.is_some();
                self.stage = if preview {
                    Stage::Preview
                } else {
                    Stage::Solve
                };
                None
            }
            Stage::Preview => {
                self.stage = Stage::Solve;
                let mode = SolveMode::Interactive { dirty: self.dirty };
                Some(self.solve(engine, mode, true))
            }
            Stage::Solve => {
                self.stage = Stage::Done;
                Some(self.solve(engine, self.solve_mode(), false))
            }
            Stage::Done => None,
        }
    }

    fn composite(&mut self, engine: &mut Engine) {
        let start = clock::now_ms();
        let snap = &self.snapshot;
        let mut changed = match self.mode {
            SolveMode::Interactive { dirty } => dirty,
            SolveMode::Full => CellSet::EMPTY,
        };
        let mut everything = false;
        match &mut engine.compositor {
            Some(c) if c.grid() == snap.grid => {}
            Some(c) => {
                c.set_grid(snap.grid);
                everything = true;
            }
            None => {
                let mut c = Compositor::new(snap.grid);
                c.set_clock(Some(clock::now_ms));
                engine.compositor = Some(c);
                everything = true;
            }
        }
        if engine.palette.as_ref() != Some(&snap.palette) {
            engine.palette = Some(snap.palette.clone());
            everything = true;
        }
        let settings = settings_of(&snap.options);
        if engine.options.as_ref() != Some(&settings) {
            engine.options = Some(settings);
            everything = true;
        }
        match &engine.steering {
            Some(old) => changed = changed.union(steering_changes(old, &snap.steering)),
            None => everything = true,
        }
        if engine.steering.as_ref() != Some(&snap.steering) {
            engine.steering = Some(snap.steering.clone());
        }
        let compositor = engine.compositor.as_mut().expect("set above");
        let (builds, ms) = (compositor.mip_builds(), compositor.mip_build_ms());
        changed = changed.union(compositor.update(&snap.layers, &snap.regions));
        // A source's mip levels are usually built by the Source view's composite first
        // (`Document::source_rgba`) and shared; any built here are logged the same way.
        if compositor.mip_builds() > builds {
            clock::log(&format!(
                "convert64: built {} mip level(s) in {:.1} ms (#235, solve)",
                compositor.mip_builds() - builds,
                compositor.mip_build_ms() - ms
            ));
        }
        if everything {
            changed = CellSet::all();
        }
        self.epoch = engine.record(changed);
        self.dirty = engine.dirty_since(snap.previous.as_ref());
        self.options = SolveOptions {
            layer_origins: layer_origins(snap.grid, &snap.layers),
            ..snap.options.clone()
        };
        self.composite_ms = clock::now_ms() - start;
    }

    fn solve(&self, engine: &mut Engine, mode: SolveMode, provisional: bool) -> SolveResult {
        let Some(compositor) = &engine.compositor else {
            unreachable!("the composite stage runs first");
        };
        let solver = solver_for(&mut engine.solver, &self.snapshot.palette);
        let (solution, solve_ms) = solve(
            solver,
            compositor.frame(),
            &self.snapshot.steering,
            self.snapshot.previous.as_deref(),
            &self.options,
            mode,
        );
        self.result(engine, Arc::new(solution), mode, provisional, solve_ms)
    }

    /// The result of a solve of this job's composite, noted in `engine` as made by it.
    pub(crate) fn result(
        &self,
        engine: &mut Engine,
        solution: Arc<Solution>,
        mode: SolveMode,
        provisional: bool,
        solve_ms: f64,
    ) -> SolveResult {
        engine.produced(&solution, self.epoch);
        SolveResult {
            ticket: self.ticket,
            solution,
            mode,
            provisional,
            timing: Timing {
                composite_ms: self.composite_ms,
                solve_ms,
            },
        }
    }

    /// After the composite stage: the dirty cells of the fast-path preview, if this job
    /// runs one.
    pub(crate) fn preview(&self) -> Option<CellSet> {
        (self.stage == Stage::Preview).then_some(self.dirty)
    }

    /// After the composite stage: the mode of the submitted solve, with the fast path's
    /// dirty cells.
    pub(crate) fn solve_mode(&self) -> SolveMode {
        match self.mode {
            SolveMode::Interactive { .. } => SolveMode::Interactive { dirty: self.dirty },
            SolveMode::Full => SolveMode::Full,
        }
    }

    /// The mode submitted.
    pub(crate) const fn submitted_mode(&self) -> SolveMode {
        self.mode
    }

    /// The snapshot submitted.
    pub(crate) const fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }

    /// After the composite stage: the options solved with (`layer_origins` filled in).
    pub(crate) const fn options(&self) -> &SolveOptions {
        &self.options
    }

    /// Ends the job: the solve stages ran elsewhere.
    pub(crate) fn finish(&mut self) {
        self.stage = Stage::Done;
    }
}
