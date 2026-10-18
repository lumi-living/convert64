//! The web worker driver's two halves (#265), as plain Rust so they are tested natively
//! connected to each other; [`super::web`] carries their messages between the page and
//! the worker.
//!
//! - [`Client`], in the app: runs a submission's composite stage on its [`Engine`] (a
//!   few milliseconds, and the compositor's caches stay where the edits are), sends the
//!   composite with everything else the solve reads as one [`protocol`] message, and
//!   turns the worker's replies into [`SolveResult`]s, dropping any for a superseded
//!   submission.
//! - [`Serve`], in the worker: keeps the solver (its palette tables) between requests
//!   and runs a request's solve stages (the fast-path preview, then the submitted solve)
//!   one per [`Serve::step`]. The worker returns to its event loop between steps, so a
//!   newer request or a cancel that arrived meanwhile is seen there: a superseded
//!   request is dropped before its next stage, and one that was superseded before it
//!   started is never even decoded.

use super::engine::{self, Engine, Job};
use super::protocol::{self, Header, Kind, ProtocolError, SolveRequest};
use super::{Config, Snapshot, SolveResult};
use convert64_core::solve::{SolveMode, Solver};
use std::sync::Arc;

/// The app's half: composites, sends, and receives.
#[derive(Debug)]
pub struct Client {
    engine: Engine,
    config: Config,
    last_ticket: u64,
    /// The submission in the worker, past its composite stage.
    job: Option<Job>,
    result: Option<SolveResult>,
}

impl Client {
    /// An idle client.
    pub fn new(config: Config) -> Self {
        Self {
            engine: Engine::default(),
            config,
            last_ticket: 0,
            job: None,
            result: None,
        }
    }

    /// Composites `snapshot` and returns its ticket and the request to send the worker;
    /// supersedes every earlier submission.
    pub fn submit(&mut self, snapshot: Snapshot, mode: SolveMode) -> (u64, Vec<u8>) {
        self.last_ticket += 1;
        let mut job = Job::new(self.last_ticket, snapshot, mode, self.config.preview_full);
        // The composite stage: delivers nothing.
        let _ = job.step(&mut self.engine);
        let snap = job.snapshot();
        let msg = protocol::encode_solve(
            self.last_ticket,
            self.engine.frame(),
            &snap.steering,
            &snap.palette,
            job.options(),
            job.preview(),
            job.solve_mode(),
            snap.previous.as_deref(),
        );
        self.job = Some(job);
        self.result = None;
        (self.last_ticket, msg)
    }

    /// Drops the current submission; returns the cancel to send the worker.
    pub fn cancel(&mut self) -> Vec<u8> {
        self.last_ticket += 1;
        self.job = None;
        self.result = None;
        protocol::encode_cancel(self.last_ticket)
    }

    /// Takes a message from the worker. Returns whether it delivered a result.
    ///
    /// # Errors
    /// If the message is malformed.
    pub fn receive(&mut self, msg: &[u8]) -> Result<bool, ProtocolError> {
        let Header { kind, ticket } = protocol::header(msg)?;
        if kind != Kind::Result {
            return Ok(false);
        }
        let Some(job) = self.job.as_mut().filter(|j| j.ticket() == ticket) else {
            // A superseded or cancelled submission's: never decoded.
            return Ok(false);
        };
        let reply = protocol::decode_result(msg)?;
        if !reply.provisional {
            job.finish();
        }
        let result = job.result(
            &mut self.engine,
            Arc::new(reply.solution),
            reply.mode,
            reply.provisional,
            reply.solve_ms,
        );
        if job.is_done() {
            self.job = None;
        }
        self.result = Some(result);
        Ok(true)
    }

    /// The newest result not yet taken, if any.
    pub fn poll(&mut self) -> Option<SolveResult> {
        self.result.take()
    }

    /// Whether a submission is in the worker.
    pub const fn is_busy(&self) -> bool {
        self.job.is_some()
    }

    /// The newest ticket handed out.
    pub const fn last_ticket(&self) -> u64 {
        self.last_ticket
    }

    /// The submission in the worker, to run in the app instead when the worker fails.
    pub fn take_pending(&mut self) -> Option<(Snapshot, SolveMode)> {
        self.job
            .take()
            .map(|j| (j.snapshot().clone(), j.submitted_mode()))
    }
}

/// The worker's half: the newest request and the solver.
#[derive(Debug, Default)]
pub struct Serve {
    solver: Option<Solver>,
    /// The newest ticket: a request or a cancel.
    ticket: u64,
    /// The newest request, not decoded yet.
    pending: Option<Vec<u8>>,
    /// The request being solved and whether its preview is still to run.
    current: Option<(SolveRequest, bool)>,
}

impl Serve {
    /// Takes a message from the app. A request supersedes every earlier one; it is
    /// decoded when its first stage runs.
    ///
    /// # Errors
    /// If the message is malformed.
    pub fn receive(&mut self, msg: Vec<u8>) -> Result<(), ProtocolError> {
        let Header { kind, ticket } = protocol::header(&msg)?;
        match kind {
            Kind::Solve => {
                self.ticket = ticket;
                self.pending = Some(msg);
            }
            Kind::Cancel => {
                self.ticket = ticket;
                self.pending = None;
            }
            Kind::Ready | Kind::Result | Kind::Idle => {}
        }
        Ok(())
    }

    /// The newest ticket received (a request's or a cancel's).
    pub const fn ticket(&self) -> u64 {
        self.ticket
    }

    /// Whether a stage is waiting.
    pub fn has_work(&self) -> bool {
        self.pending.is_some()
            || self
                .current
                .as_ref()
                .is_some_and(|(r, _)| r.ticket == self.ticket)
    }

    /// Runs the next stage of the newest request, dropping a superseded one first;
    /// returns the result message to send the app, if a stage ran.
    ///
    /// # Errors
    /// If the newest request is malformed (it is dropped).
    pub fn step(&mut self) -> Result<Option<Vec<u8>>, ProtocolError> {
        if self
            .current
            .as_ref()
            .is_some_and(|(r, _)| r.ticket != self.ticket)
        {
            self.current = None;
        }
        if self.current.is_none() {
            let Some(msg) = self.pending.take() else {
                return Ok(None);
            };
            let request = protocol::decode_solve(&msg)?;
            let preview = request.preview.is_some();
            self.current = Some((request, preview));
        }
        let Some((request, preview)) = &mut self.current else {
            unreachable!("set above");
        };
        let (mode, provisional) = match (request.preview, *preview) {
            (Some(dirty), true) => (SolveMode::Interactive { dirty }, true),
            _ => (request.mode, false),
        };
        let solver = engine::solver_for(&mut self.solver, &request.palette);
        let (solution, solve_ms) = engine::solve(
            solver,
            &request.frame,
            &request.steering,
            request.previous.as_ref(),
            &request.options,
            mode,
        );
        let msg = protocol::encode_result(request.ticket, provisional, mode, solve_ms, &solution);
        if provisional {
            *preview = false;
        } else {
            self.current = None;
        }
        Ok(Some(msg))
    }
}
