//! The browser's solve worker (#265): a dedicated module worker
//! (`solve-worker.js`, copied next to the app by trunk) that loads the app's own wasm
//! module. There `main` finds no window and calls [`serve`] instead of starting the
//! user interface. The page's [`Remote`] and the worker's [`serve`] exchange
//! [`super::protocol`] messages, each one `ArrayBuffer` transferred (not copied, and no
//! `SharedArrayBuffer`). The browser API is reached by name through `js_sys::Reflect`
//! on eframe's re-exports, so the web build needs no further crates or `web-sys`
//! features (TAD §4).
//!
//! **Cancellation.** The worker sees a newer request or a cancel between stages and
//! drops superseded work there ([`Serve`]). A stage is one call into the core and runs
//! to its end, so for a full submission or a cancel that finds the worker busy, the
//! page also keeps a second, idle worker warm: it terminates the busy one, which stops
//! its stage at once, sends the request to the spare and starts a new spare. The worker
//! holds no caches but the solver's palette tables (the composite is made on the page),
//! so nothing is lost. Interactive submissions (a drag, many per second) never
//! terminate a worker; they wait for at most one fast-path stage.
//!
//! If the worker cannot start or fails later, [`Remote::failed`] turns true and the
//! scheduler continues with the stepping driver on the page (#194), solving the
//! submission that was in the worker.

use super::protocol::{self, Header, Kind};
use super::remote::{Client, Serve};
use super::{Config, Snapshot, SolveResult, Stepper, Waker, clock};
use convert64_core::solve::SolveMode;
use eframe::wasm_bindgen::closure::Closure;
use eframe::wasm_bindgen::{JsCast as _, JsValue};
use eframe::web_sys::js_sys::{self, Array, Function, Object, Reflect, Uint8Array};
use std::cell::RefCell;
use std::rc::{Rc, Weak};

/// The worker script, relative to the page (`index.html` copies it into the bundle).
const SCRIPT: &str = "./solve-worker.js";

/// `target[name]` as a function.
fn method(target: &JsValue, name: &str) -> Result<Function, JsValue> {
    Reflect::get(target, &JsValue::from_str(name))?.dyn_into::<Function>()
}

/// Posts `msg` to `port` (a `Worker`, or the worker's global scope), transferring its
/// buffer.
fn post(port: &JsValue, msg: &[u8]) -> Result<(), JsValue> {
    let len = u32::try_from(msg.len()).map_err(|_| JsValue::from_str("message too large"))?;
    let bytes = Uint8Array::new_with_length(len);
    bytes.copy_from(msg);
    let buffer = bytes.buffer();
    method(port, "postMessage")?.call2(port, &buffer, &Array::of1(&buffer))?;
    Ok(())
}

/// A message event's bytes.
fn data(event: &JsValue) -> Option<Vec<u8>> {
    let data = Reflect::get(event, &JsValue::from_str("data")).ok()?;
    data.is_instance_of::<js_sys::ArrayBuffer>()
        .then(|| Uint8Array::new(&data).to_vec())
}

/// One worker and its handlers; terminated when dropped.
struct Port {
    id: u32,
    worker: JsValue,
    /// The worker has said it is ready.
    ready: bool,
    /// The newest ticket sent to it, and the newest it said it was idle at.
    sent: u64,
    idle_at: u64,
    /// When it was created, ms.
    started: f64,
    _on_message: Closure<dyn FnMut(JsValue)>,
    _on_error: Closure<dyn FnMut(JsValue)>,
}

impl Port {
    /// Creates worker `id`, whose events go to `shared`.
    fn spawn(shared: &Weak<RefCell<Shared>>, id: u32) -> Result<Self, JsValue> {
        let global = js_sys::global();
        let options = Object::new();
        Reflect::set(&options, &"type".into(), &"module".into())?;
        Reflect::set(&options, &"name".into(), &"convert64-solve".into())?;
        let worker = Reflect::construct(
            &method(&global, "Worker")?,
            &Array::of2(&SCRIPT.into(), &options),
        )?;
        let on_message = Closure::<dyn FnMut(JsValue)>::new({
            let shared = Weak::clone(shared);
            move |event: JsValue| {
                if let Some(shared) = shared.upgrade() {
                    shared.borrow_mut().message(id, data(&event));
                }
            }
        });
        let on_error = Closure::<dyn FnMut(JsValue)>::new({
            let shared = Weak::clone(shared);
            move |event: JsValue| {
                let why = Reflect::get(&event, &"message".into())
                    .ok()
                    .and_then(|m| m.as_string())
                    .unwrap_or_else(|| "it did not load".to_owned());
                if let Some(shared) = shared.upgrade() {
                    shared.borrow_mut().error(id, &why);
                }
            }
        });
        Reflect::set(&worker, &"onmessage".into(), on_message.as_ref())?;
        Reflect::set(&worker, &"onerror".into(), on_error.as_ref())?;
        Reflect::set(&worker, &"onmessageerror".into(), on_error.as_ref())?;
        Ok(Self {
            id,
            worker,
            ready: false,
            sent: 0,
            idle_at: 0,
            started: clock::now_ms(),
            _on_message: on_message,
            _on_error: on_error,
        })
    }

    /// Whether it is running (or about to run) a stage.
    const fn busy(&self) -> bool {
        self.sent > self.idle_at
    }
}

impl Drop for Port {
    fn drop(&mut self) {
        if let Ok(terminate) = method(&self.worker, "terminate") {
            let _ = terminate.call0(&self.worker);
        }
    }
}

/// What the page's handlers share.
struct Shared {
    client: Client,
    /// The worker requests go to.
    active: Option<Port>,
    /// A started, idle worker to switch to.
    spare: Option<Port>,
    next_id: u32,
    /// The newest message for the active worker while it is not ready yet.
    outbox: Option<(u64, Vec<u8>)>,
    failed: bool,
    waker: Option<Waker>,
    me: Weak<RefCell<Self>>,
}

impl Shared {
    /// Sends `msg` (for `ticket`) to the active worker. With `preempt`, a busy worker is
    /// replaced by the spare first, if one is ready.
    fn send(&mut self, ticket: u64, msg: Vec<u8>, preempt: bool) {
        let busy = self.active.as_ref().is_some_and(|p| p.ready && p.busy());
        if preempt && busy && self.spare.as_ref().is_some_and(|p| p.ready) {
            // Dropping the busy worker terminates it.
            self.active = self.spare.take();
            self.spawn_spare();
        }
        let Some(port) = self.active.as_mut() else {
            return;
        };
        if !port.ready {
            self.outbox = Some((ticket, msg));
            return;
        }
        port.sent = ticket;
        if let Err(e) = post(&port.worker, &msg) {
            self.fail(&format!("{e:?}"));
        }
    }

    fn spawn_spare(&mut self) {
        self.next_id += 1;
        match Port::spawn(&self.me, self.next_id) {
            Ok(p) => self.spare = Some(p),
            Err(e) => clock::log(&format!("convert64: no spare solve worker: {e:?}")),
        }
    }

    /// A message from worker `id`.
    fn message(&mut self, id: u32, msg: Option<Vec<u8>>) {
        let Some(msg) = msg else {
            return self.error(id, "a message without data");
        };
        let header = match protocol::header(&msg) {
            Ok(h) => h,
            Err(e) => return self.error(id, &e.to_string()),
        };
        if self.spare.as_ref().is_some_and(|p| p.id == id) {
            if header.kind == Kind::Ready
                && let Some(p) = &mut self.spare
            {
                p.ready = true;
            }
            return;
        }
        let Some(port) = self.active.as_mut().filter(|p| p.id == id) else {
            // A terminated worker's last message.
            return;
        };
        match header {
            Header {
                kind: Kind::Ready, ..
            } => {
                port.ready = true;
                clock::log(&format!(
                    "convert64: solve worker ready in {:.0} ms (#265)",
                    clock::now_ms() - port.started
                ));
                if let Some((ticket, m)) = self.outbox.take() {
                    self.send(ticket, m, false);
                }
                // Started after the first, so the two do not compete at page load.
                self.spawn_spare();
            }
            Header {
                kind: Kind::Idle,
                ticket,
            } => port.idle_at = ticket,
            Header {
                kind: Kind::Result, ..
            } => match self.client.receive(&msg) {
                Ok(true) => self.wake(),
                Ok(false) => {}
                Err(e) => self.fail(&e.to_string()),
            },
            Header { .. } => {}
        }
    }

    /// An error from worker `id`: the spare is dropped, the active one fails the driver.
    fn error(&mut self, id: u32, why: &str) {
        if self.spare.as_ref().is_some_and(|p| p.id == id) {
            clock::log(&format!("convert64: the spare solve worker failed ({why})"));
            self.spare = None;
        } else if self.active.as_ref().is_some_and(|p| p.id == id) {
            self.fail(why);
        }
    }

    fn fail(&mut self, why: &str) {
        if !self.failed {
            clock::log(&format!(
                "convert64: the solve worker failed ({why}); solving on the page instead"
            ));
        }
        self.failed = true;
        self.wake();
    }

    fn wake(&self) {
        if let Some(w) = &self.waker {
            w();
        }
    }
}

/// The page's end of the solve worker.
pub struct Remote {
    shared: Rc<RefCell<Shared>>,
    config: Config,
}

impl std::fmt::Debug for Remote {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = self.shared.borrow();
        f.debug_struct("Remote")
            .field("client", &s.client)
            .field("active", &s.active.as_ref().map(|p| (p.id, p.ready)))
            .field("spare", &s.spare.as_ref().map(|p| (p.id, p.ready)))
            .field("failed", &s.failed)
            .finish_non_exhaustive()
    }
}

impl Remote {
    /// Starts the worker.
    ///
    /// # Errors
    /// If the browser cannot create it.
    pub fn new(config: Config) -> Result<Self, JsValue> {
        let shared = Rc::new_cyclic(|me| {
            RefCell::new(Shared {
                client: Client::new(config),
                active: None,
                spare: None,
                next_id: 0,
                outbox: None,
                failed: false,
                waker: None,
                me: Weak::clone(me),
            })
        });
        let port = Port::spawn(&Rc::downgrade(&shared), 0)?;
        shared.borrow_mut().active = Some(port);
        Ok(Self { shared, config })
    }

    /// Composites `snapshot` here and sends it to the worker, superseding every earlier
    /// submission. Returns its ticket.
    pub fn submit(&mut self, snapshot: Snapshot, mode: SolveMode) -> u64 {
        let mut s = self.shared.borrow_mut();
        let (ticket, msg) = s.client.submit(snapshot, mode);
        s.send(ticket, msg, mode == SolveMode::Full);
        ticket
    }

    /// Drops the current submission.
    pub fn cancel(&mut self) {
        let mut s = self.shared.borrow_mut();
        let msg = s.client.cancel();
        let ticket = protocol::header(&msg).map_or(0, |h| h.ticket);
        s.send(ticket, msg, true);
    }

    /// The newest result not yet taken, if any.
    pub fn poll(&mut self) -> Option<SolveResult> {
        self.shared.borrow_mut().client.poll()
    }

    /// Whether a submission is in the worker.
    pub fn is_busy(&self) -> bool {
        self.shared.borrow().client.is_busy()
    }

    /// Calls `waker` when a result arrives.
    pub fn set_waker(&mut self, waker: Waker) {
        self.shared.borrow_mut().waker = Some(waker);
    }

    /// Whether the worker failed; the scheduler then solves on the page.
    pub fn failed(&self) -> bool {
        self.shared.borrow().failed
    }

    /// The stepping driver that takes over from a failed worker: the same settings,
    /// waker and ticket numbers, with the submission that was in the worker. The
    /// workers are terminated.
    pub(super) fn into_fallback(self) -> Stepper {
        let mut s = self.shared.borrow_mut();
        let mut stepper = Stepper::new(self.config);
        stepper.continue_tickets(s.client.last_ticket());
        if let Some(w) = s.waker.clone() {
            stepper.set_waker(w);
        }
        if let Some((snapshot, mode)) = s.client.take_pending() {
            // Keeps the submission's ticket.
            stepper.continue_tickets(s.client.last_ticket() - 1);
            stepper.submit(snapshot, mode);
        }
        stepper
    }
}

impl Drop for Remote {
    fn drop(&mut self) {
        // Terminates both workers and frees their handlers.
        let mut s = self.shared.borrow_mut();
        s.active = None;
        s.spare = None;
    }
}

/// What the worker's handlers share.
struct Worker {
    serve: Serve,
    /// A step is queued on the event loop.
    scheduled: bool,
    /// The step, as a function for `setTimeout`.
    run: Option<Function>,
}

impl Worker {
    /// Queues a step on the worker's event loop (after any messages already queued), so
    /// a newer request or a cancel is seen between stages; with nothing to do, tells
    /// the page the worker is idle.
    fn schedule(&mut self) {
        let global = js_sys::global();
        if !self.serve.has_work() {
            let _ = post(&global, &protocol::encode_idle(self.serve.ticket()));
            return;
        }
        if self.scheduled {
            return;
        }
        let queued = self.run.as_ref().is_some_and(|run| {
            method(&global, "setTimeout")
                .and_then(|f| f.call2(&global, run, &JsValue::from(0)))
                .is_ok()
        });
        self.scheduled = queued;
    }
}

/// The worker's entry point: serves solve requests from the page until terminated.
pub fn serve() {
    std::panic::set_hook(Box::new(|info| {
        clock::log(&format!("convert64: the solve worker panicked: {info}"));
    }));
    let global = js_sys::global();
    let worker = Rc::new(RefCell::new(Worker {
        serve: Serve::default(),
        scheduled: false,
        run: None,
    }));
    let run = Closure::<dyn FnMut()>::new({
        let worker = Rc::clone(&worker);
        let global = global.clone();
        move || {
            let mut w = worker.borrow_mut();
            w.scheduled = false;
            match w.serve.step() {
                Ok(Some(msg)) => {
                    if let Err(e) = post(&global, &msg) {
                        clock::log(&format!("convert64: the solve worker cannot reply: {e:?}"));
                    }
                }
                Ok(None) => {}
                Err(e) => clock::log(&format!("convert64: {e}")),
            }
            w.schedule();
        }
    });
    worker.borrow_mut().run = Some(run.into_js_value().unchecked_into());
    let on_message = Closure::<dyn FnMut(JsValue)>::new({
        let worker = Rc::clone(&worker);
        move |event: JsValue| {
            let mut w = worker.borrow_mut();
            match data(&event).map(|msg| w.serve.receive(msg)) {
                Some(Ok(())) => w.schedule(),
                Some(Err(e)) => clock::log(&format!("convert64: {e}")),
                None => clock::log("convert64: the solve worker got a message without data"),
            }
        }
    });
    let ready = Reflect::set(&global, &"onmessage".into(), on_message.as_ref())
        .and_then(|_| post(&global, &protocol::encode_ready()));
    if let Err(e) = ready {
        clock::log(&format!("convert64: the solve worker cannot start: {e:?}"));
    }
    // The handlers live as long as the worker.
    on_message.forget();
}
