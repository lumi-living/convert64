//! The solve scheduler (#194; `docs/TAD.md` §3.1, §7, UX §2, §11): the app hands it
//! every edit and draws the newest result, and no frame waits for a solve.
//!
//! - [`Scheduler::submit`] takes a [`Snapshot`] (layers, regions, steering, grid,
//!   palette, solve options and the previous solution) and a [`SolveMode`]: the
//!   interactive fast path while dragging, the full search on release. A newer
//!   submission supersedes every older one; a superseded submission's results are never
//!   delivered, even when already computed.
//! - [`Scheduler::cancel`] drops the current submission.
//! - [`Scheduler::poll`], once per frame, returns the newest undelivered
//!   [`SolveResult`]: the solution, which solver made it ([`SolveResult::solver`],
//!   [`SolveResult::fallback`]) and how long compositing and solving took.
//!
//! The scheduler owns a [`Compositor`](convert64_core::Compositor) and a
//! [`Solver`](convert64_core::solve::Solver), so the compositor's caches survive between
//! submissions and each submission recomposites only what changed. For the fast path it
//! widens the submitted dirty cells by every cell whose input changed since `previous`
//! was solved: the compositor's changed cells, steering changes it finds by comparing
//! steering, and every cell after a grid, palette or option change. That stays right
//! when submissions in between were superseded. A `previous` the scheduler did not make
//! counts as stale everywhere.
//!
//! **Drivers.** Natively ([`Worker`]) one worker thread runs the work and the core's
//! `parallel` feature is on. In wasm ([`Stepper`]) there are no threads: the work is cut
//! at stage boundaries (composite, then solve) and [`Scheduler::tick`] runs it a stage
//! at a time inside a frame budget ([`Config::frame_budget_ms`], 10 ms by default, so a
//! 60 Hz frame keeps about 6 ms for the user interface). A full solve then first
//! delivers the interactive fast path as a provisional result ([`Config::preview_full`],
//! on in wasm), so the preview follows a release within a frame or two while the full
//! search runs. A stage is one call into the core and cannot be split, so a full solve
//! (about 25–75 ms in wasm) is one long frame; it comes after the preview is on screen.
//! The core is unchanged: every solution is exactly `Solver::solve` on the compositor's
//! frame.

mod clock;
mod engine;
mod stepper;
#[cfg(test)]
mod tests;
#[cfg(not(target_arch = "wasm32"))]
mod worker;

pub use stepper::Stepper;
#[cfg(not(target_arch = "wasm32"))]
pub use worker::Worker;

use convert64_core::layer::Layer;
use convert64_core::region::Region;
use convert64_core::solve::{Solution, SolveMode, SolveOptions, SolverKind, Steering};
use convert64_core::{Palette, SuperGrid};
use std::sync::Arc;

/// Called when a result is ready; natively from the worker thread.
pub type Waker = Arc<dyn Fn() + Send + Sync>;

/// Everything a solve reads, copied from the project at submission time.
#[derive(Clone, Debug)]
pub struct Snapshot {
    /// Layers, bottom first.
    pub layers: Vec<Layer>,
    /// Region overrides, in order.
    pub regions: Vec<Region>,
    /// Pins, locks and allow/deny lists.
    pub steering: Steering,
    /// Target mode and supersample factor.
    pub grid: SuperGrid,
    /// The palette solved with (and previewed in: TAD §3.4).
    pub palette: Palette,
    /// Solve options. `mode` is replaced by the submission's, and `layer_origins` is
    /// derived from `layers` and `grid` as the CLI does.
    pub options: SolveOptions,
    /// The solution on screen: the fast path copies its clean cells, and locked cells
    /// come from it. Pass a result's [`SolveResult::solution`] back unchanged.
    pub previous: Option<Arc<Solution>>,
}

/// Wall-clock milliseconds a result took.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Timing {
    /// Bringing the compositor (and the solver's palette tables, when the palette
    /// changed) up to date with the snapshot.
    pub composite_ms: f64,
    /// The solve.
    pub solve_ms: f64,
}

/// A solved picture from a submission.
#[derive(Clone, Debug)]
pub struct SolveResult {
    /// The submission's ticket, from [`Scheduler::submit`].
    pub ticket: u64,
    /// The picture.
    pub solution: Arc<Solution>,
    /// The solve that ran: for the fast path, with every cell it re-solved.
    pub mode: SolveMode,
    /// A full submission's fast-path preview; the full result follows.
    pub provisional: bool,
    /// How long it took.
    pub timing: Timing,
}

impl SolveResult {
    /// The solver that made the picture.
    pub fn solver(&self) -> SolverKind {
        self.solution.solver
    }

    /// Why the native solver made it although the reference was asked for.
    pub fn fallback(&self) -> Option<&str> {
        self.solution.fallback.as_deref()
    }
}

/// Scheduler settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Config {
    /// A full submission with a previous solution first delivers the fast path as a
    /// provisional result. Default: on in wasm, off natively (where a full solve takes
    /// a few milliseconds with `parallel`).
    pub preview_full: bool,
    /// The stepping driver's work per [`Scheduler::tick`], ms: at least one stage, then
    /// more while the next stage's last measured duration fits.
    pub frame_budget_ms: f64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            preview_full: cfg!(target_arch = "wasm32"),
            frame_budget_ms: 10.0,
        }
    }
}

/// The driver behind a [`Scheduler`].
#[derive(Debug)]
enum Driver {
    #[cfg(not(target_arch = "wasm32"))]
    Thread(Worker),
    Steps(Box<Stepper>),
}

/// The app's solve scheduler: a worker thread natively, stepping in wasm.
#[derive(Debug)]
pub struct Scheduler {
    driver: Driver,
}

impl Scheduler {
    /// The platform's driver: a worker thread natively, stepping in wasm.
    pub fn new(config: Config) -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        let driver = Driver::Thread(Worker::new(config));
        #[cfg(target_arch = "wasm32")]
        let driver = Driver::Steps(Box::new(Stepper::new(config)));
        Self { driver }
    }

    /// The stepping driver on any platform (what wasm uses).
    pub fn stepping(config: Config) -> Self {
        Self {
            driver: Driver::Steps(Box::new(Stepper::new(config))),
        }
    }

    /// Queues `snapshot` for solving in `mode`, superseding every earlier submission.
    /// For [`SolveMode::Interactive`], `dirty` names cells to re-solve besides those the
    /// scheduler finds changed. Returns the submission's ticket (increasing from 1).
    pub fn submit(&mut self, snapshot: Snapshot, mode: SolveMode) -> u64 {
        match &mut self.driver {
            #[cfg(not(target_arch = "wasm32"))]
            Driver::Thread(w) => w.submit(snapshot, mode),
            Driver::Steps(s) => s.submit(snapshot, mode),
        }
    }

    /// Drops the current submission; nothing it produces is delivered.
    pub fn cancel(&mut self) {
        match &mut self.driver {
            #[cfg(not(target_arch = "wasm32"))]
            Driver::Thread(w) => w.cancel(),
            Driver::Steps(s) => s.cancel(),
        }
    }

    /// The newest result not yet taken, if any. Call once per frame, after
    /// [`Self::tick`].
    pub fn poll(&mut self) -> Option<SolveResult> {
        match &mut self.driver {
            #[cfg(not(target_arch = "wasm32"))]
            Driver::Thread(w) => w.poll(),
            Driver::Steps(s) => s.poll(),
        }
    }

    /// Does this frame's share of the work. Returns whether to request another frame
    /// for it: stepping work remains. Natively the worker needs no frames (`false`); the
    /// waker ([`Self::set_waker`]) asks for a repaint when a result is ready.
    pub fn tick(&mut self) -> bool {
        match &mut self.driver {
            #[cfg(not(target_arch = "wasm32"))]
            Driver::Thread(_) => false,
            Driver::Steps(s) => s.tick(),
        }
    }

    /// Whether a submission is queued or running.
    pub fn is_busy(&self) -> bool {
        match &self.driver {
            #[cfg(not(target_arch = "wasm32"))]
            Driver::Thread(w) => w.is_busy(),
            Driver::Steps(s) => s.is_busy(),
        }
    }

    /// Calls `waker` whenever a result is ready (natively from the worker thread), e.g.
    /// `egui::Context::request_repaint`.
    pub fn set_waker(&mut self, waker: Waker) {
        match &mut self.driver {
            #[cfg(not(target_arch = "wasm32"))]
            Driver::Thread(w) => w.set_waker(waker),
            Driver::Steps(s) => s.set_waker(waker),
        }
    }
}

impl Default for Scheduler {
    fn default() -> Self {
        Self::new(Config::default())
    }
}
