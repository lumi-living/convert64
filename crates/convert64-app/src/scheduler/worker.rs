//! The native driver: one worker thread runs submissions' stages; the UI thread only
//! submits and polls. Between stages the worker checks for a newer submission and drops
//! the rest of a superseded one.

use super::engine::{Engine, Job};
use super::{Config, Snapshot, SolveResult, Waker};
use convert64_core::solve::SolveMode;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// What the UI thread and the worker share.
#[derive(Default)]
struct State {
    /// The newest ticket; a cancel also takes one. Work for any other ticket is stale.
    ticket: u64,
    /// The submission waiting for the worker.
    pending: Option<Job>,
    /// Whether the worker is running a job.
    running: bool,
    /// The newest undelivered result.
    result: Option<SolveResult>,
    /// Set by `Drop`.
    shutdown: bool,
    waker: Option<Waker>,
    /// The message of a panic in the worker.
    error: Option<String>,
}

#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    /// Signalled on a submission and on shutdown.
    work: Condvar,
    /// Signalled when a result is delivered or the worker goes idle.
    done: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Runs submissions on one worker thread (native).
pub struct Worker {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
    config: Config,
}

impl std::fmt::Debug for Worker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Worker")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl Worker {
    /// Starts the worker thread.
    ///
    /// # Panics
    /// If the thread cannot be spawned.
    pub fn new(config: Config) -> Self {
        let shared = Arc::new(Shared::default());
        let thread = std::thread::Builder::new()
            .name("convert64-solve".to_owned())
            .spawn({
                let shared = Arc::clone(&shared);
                move || run(&shared)
            })
            .expect("convert64: cannot spawn the solve thread");
        Self {
            shared,
            thread: Some(thread),
            config,
        }
    }

    /// Queues `snapshot` for solving in `mode`, superseding any earlier submission: the
    /// worker drops the rest of it at its next stage boundary and its result is never
    /// delivered. Returns the submission's ticket (increasing from 1).
    pub fn submit(&self, snapshot: Snapshot, mode: SolveMode) -> u64 {
        let mut s = self.shared.lock();
        s.ticket += 1;
        s.pending = Some(Job::new(s.ticket, snapshot, mode, self.config.preview_full));
        s.result = None;
        let ticket = s.ticket;
        drop(s);
        self.shared.work.notify_one();
        ticket
    }

    /// Drops the current submission; nothing it produces is delivered.
    pub fn cancel(&self) {
        let mut s = self.shared.lock();
        s.ticket += 1;
        s.pending = None;
        s.result = None;
    }

    /// The newest result not yet taken, if any.
    pub fn poll(&self) -> Option<SolveResult> {
        self.shared.lock().result.take()
    }

    /// Waits up to `timeout` for a result (headless use and tests).
    pub fn wait(&self, timeout: Duration) -> Option<SolveResult> {
        let deadline = Instant::now() + timeout;
        let mut s = self.shared.lock();
        loop {
            if let Some(r) = s.result.take() {
                return Some(r);
            }
            let left = deadline.checked_duration_since(Instant::now())?;
            s = self
                .shared
                .done
                .wait_timeout(s, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    /// Waits up to `timeout` until no submission is queued or running; returns whether
    /// that happened.
    pub fn wait_idle(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut s = self.shared.lock();
        while s.pending.is_some() || s.running {
            let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                return false;
            };
            s = self
                .shared
                .done
                .wait_timeout(s, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        true
    }

    /// Whether a submission is queued or running.
    pub fn is_busy(&self) -> bool {
        let s = self.shared.lock();
        s.pending.is_some() || s.running
    }

    /// Calls `waker` from the worker thread whenever a result is ready (e.g. to request
    /// a repaint).
    pub fn set_waker(&self, waker: Waker) {
        self.shared.lock().waker = Some(waker);
    }

    /// The message of a panic in the worker since the last call, if any. The worker
    /// survives it with fresh caches; that submission delivers nothing.
    pub fn take_error(&self) -> Option<String> {
        self.shared.lock().error.take()
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.shared.lock().shutdown = true;
        self.shared.work.notify_one();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// The worker thread's loop.
fn run(shared: &Shared) {
    let mut engine = Engine::default();
    loop {
        let mut job = {
            let mut s = shared.lock();
            loop {
                if s.shutdown {
                    return;
                }
                if let Some(job) = s.pending.take() {
                    s.running = true;
                    break job;
                }
                s = shared.work.wait(s).unwrap_or_else(PoisonError::into_inner);
            }
        };
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            while !job.is_done() {
                let result = job.step(&mut engine);
                let mut s = shared.lock();
                if s.ticket != job.ticket() || s.shutdown {
                    break;
                }
                if let Some(r) = result {
                    s.result = Some(r);
                    let waker = s.waker.clone();
                    drop(s);
                    shared.done.notify_all();
                    if let Some(w) = waker {
                        w();
                    }
                }
            }
        }));
        let mut s = shared.lock();
        if let Err(panic) = outcome {
            engine = Engine::default();
            s.error = Some(
                panic
                    .downcast_ref::<&str>()
                    .map(|m| (*m).to_owned())
                    .or_else(|| panic.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "the solve thread panicked".to_owned()),
            );
        }
        s.running = false;
        drop(s);
        shared.done.notify_all();
    }
}
