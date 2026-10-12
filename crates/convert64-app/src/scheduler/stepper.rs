//! The stepping driver: no threads; the caller runs the work a stage at a time, so every
//! frame yields (wasm, where there are no threads, and tests everywhere).

use super::engine::{Engine, Job, Work};
use super::{Config, Snapshot, SolveResult, Waker, clock};
use convert64_core::solve::SolveMode;

/// Runs submissions stage by stage on the caller's thread: [`Self::tick`] once per frame
/// (or [`Self::step`] for exactly one stage).
pub struct Stepper {
    engine: Engine,
    job: Option<Job>,
    result: Option<SolveResult>,
    last_ticket: u64,
    config: Config,
    /// The last measured duration of each kind of stage, ms.
    estimates: [Option<f64>; 3],
    waker: Option<Waker>,
}

impl std::fmt::Debug for Stepper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Stepper")
            .field("job", &self.job.as_ref().map(Job::ticket))
            .field("result", &self.result.as_ref().map(|r| r.ticket))
            .field("last_ticket", &self.last_ticket)
            .field("config", &self.config)
            .field("estimates", &self.estimates)
            .finish_non_exhaustive()
    }
}

impl Stepper {
    /// An idle stepper.
    pub fn new(config: Config) -> Self {
        Self {
            engine: Engine::default(),
            job: None,
            result: None,
            last_ticket: 0,
            config,
            estimates: [None; 3],
            waker: None,
        }
    }

    /// Queues `snapshot` for solving in `mode`, superseding any earlier submission: its
    /// remaining stages are dropped and its undelivered result is discarded. Returns the
    /// submission's ticket (increasing from 1).
    pub fn submit(&mut self, snapshot: Snapshot, mode: SolveMode) -> u64 {
        self.last_ticket += 1;
        self.job = Some(Job::new(
            self.last_ticket,
            snapshot,
            mode,
            self.config.preview_full,
        ));
        self.result = None;
        self.last_ticket
    }

    /// Drops the current submission and its undelivered result.
    pub fn cancel(&mut self) {
        self.last_ticket += 1;
        self.job = None;
        self.result = None;
    }

    /// The newest result not yet taken, if any.
    pub fn poll(&mut self) -> Option<SolveResult> {
        self.result.take()
    }

    /// Whether a submission has stages left.
    pub const fn is_busy(&self) -> bool {
        self.job.is_some()
    }

    /// Calls `waker` whenever a result is ready (e.g. to request a repaint).
    pub fn set_waker(&mut self, waker: Waker) {
        self.waker = Some(waker);
    }

    /// Runs exactly one stage. Returns whether stages remain.
    pub fn step(&mut self) -> bool {
        let Some(job) = &mut self.job else {
            return false;
        };
        let work = job.next_work();
        let start = clock::now_ms();
        let result = job.step(&mut self.engine);
        if let Some(w) = work {
            self.estimates[w as usize] = Some(clock::now_ms() - start);
        }
        if job.is_done() {
            self.job = None;
        }
        if let Some(r) = result {
            self.result = Some(r);
            if let Some(w) = &self.waker {
                w();
            }
        }
        self.job.is_some()
    }

    /// One frame's work: at least one stage, then more while the last measured duration
    /// of the next stage fits in what is left of [`Config::frame_budget_ms`] (a stage
    /// not measured yet waits for the next frame). Returns whether stages remain, i.e.
    /// whether to request another frame.
    pub fn tick(&mut self) -> bool {
        let start = clock::now_ms();
        let mut more = self.step();
        while more {
            let Some(work) = self.job.as_ref().and_then(Job::next_work) else {
                break;
            };
            let Some(estimate) = self.estimates[work as usize] else {
                break;
            };
            if clock::now_ms() - start + estimate > self.config.frame_budget_ms {
                break;
            }
            more = self.step();
        }
        more
    }

    /// Runs every remaining stage (headless use and tests). Returns the last result.
    pub fn finish(&mut self) -> Option<SolveResult> {
        while self.step() {}
        self.poll()
    }
}

// Keeps `Work`'s discriminants and `estimates`' indices in step.
const _: () = assert!(Work::Composite as usize == 0 && Work::Full as usize == 2);
