//! Method thumbnails for the picker (UX §6a): real solves of a small tile
//! ([`convert64_core::preview`]), rendered off the frame and cached as textures.
//!
//! - [`MethodThumbs::prepare`], once per frame before drawing: brings the tile solver
//!   (palette and solve options) and the sources (the gradient; the picture crop at the
//!   last hovered cell) up to date and turns finished tiles into textures.
//! - While drawing, [`MethodThumbs::texture`] gives a card its thumbnail, or `None` (the
//!   card draws a neutral placeholder), and [`MethodThumbs::job`] what to render.
//! - [`MethodThumbs::request`] replaces the render queue with this frame's wanted
//!   thumbnails, visible cards first; anything no longer wanted is cancelled.
//!
//! **Drivers.** Natively one worker thread renders the queue in order. In wasm there are
//! no threads: [`MethodThumbs::tick`] renders a small batch each frame, within a budget,
//! at least one tile.

use crate::document::Document;
use crate::scheduler::{Waker, clock};
use convert64_core::layer::{Method, Recipe};
use convert64_core::mode::Mode;
use convert64_core::model::CellPos;
use convert64_core::preview::{SourceTile, Tile, TileKey, TileSolver, settings_hash};
use convert64_core::solve::SolveOptions;
use convert64_core::{ColourIndex, Palette};
use egui::{ColorImage, TextureHandle, TextureOptions};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

/// What the thumbnails dither (UX §6a, *Preview on*).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PreviewSource {
    /// A diagonal ramp through the palette's greys plus one hue.
    #[default]
    Gradient,
    /// A crop of the picture at the last cell the cursor was over (or the centre).
    Picture,
}

impl PreviewSource {
    /// The stored form (eframe storage).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Gradient => "gradient",
            Self::Picture => "picture",
        }
    }

    /// From [`Self::as_str`].
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "gradient" => Some(Self::Gradient),
            "picture" => Some(Self::Picture),
            _ => None,
        }
    }
}

/// One thumbnail to render.
#[derive(Clone, Debug)]
pub struct ThumbJob {
    /// Its cache key.
    pub key: TileKey,
    method: Method,
    recipe: Recipe,
    source: Arc<SourceTile>,
    solver: Arc<TileSolver>,
}

impl ThumbJob {
    /// Renders it (on whichever thread).
    pub fn render(&self) -> Tile {
        self.solver.render(self.method, &self.recipe, &self.source)
    }
}

/// The renderer's queue and results.
#[derive(Default)]
struct Queue {
    jobs: VecDeque<ThumbJob>,
    done: Vec<(TileKey, Arc<Tile>, f64)>,
    /// The job the worker is rendering (native).
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    running: Option<TileKey>,
    /// The app is closing (native).
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    shutdown: bool,
    waker: Option<Waker>,
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use super::Queue;
    use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

    #[derive(Default)]
    pub(super) struct Shared {
        pub(super) queue: Mutex<Queue>,
        pub(super) work: Condvar,
    }

    impl Shared {
        pub(super) fn lock(&self) -> MutexGuard<'_, Queue> {
            self.queue.lock().unwrap_or_else(PoisonError::into_inner)
        }
    }

    /// The worker thread: renders the queue front to back.
    pub(super) fn spawn(shared: Arc<Shared>) -> Option<std::thread::JoinHandle<()>> {
        std::thread::Builder::new()
            .name("convert64-thumbs".to_owned())
            .spawn(move || {
                loop {
                    let job = {
                        let mut q = shared.lock();
                        loop {
                            if q.shutdown {
                                return;
                            }
                            if let Some(j) = q.jobs.pop_front() {
                                q.running = Some(j.key);
                                break j;
                            }
                            q = shared.work.wait(q).unwrap_or_else(PoisonError::into_inner);
                        }
                    };
                    let start = crate::scheduler::clock::now_ms();
                    let tile = std::panic::catch_unwind(|| job.render());
                    let ms = crate::scheduler::clock::now_ms() - start;
                    let mut q = shared.lock();
                    q.running = None;
                    if let Ok(tile) = tile {
                        q.done.push((job.key, Arc::new(tile), ms));
                    }
                    let waker = q.waker.clone();
                    drop(q);
                    if let Some(w) = waker {
                        w();
                    }
                }
            })
            .ok()
    }
}

/// Renders thumbnails off the frame: a worker thread natively, a batch per frame in wasm.
struct Renderer {
    #[cfg(not(target_arch = "wasm32"))]
    shared: Arc<native::Shared>,
    #[cfg(not(target_arch = "wasm32"))]
    thread: Option<std::thread::JoinHandle<()>>,
    /// The stepping driver's queue (wasm, or natively when no thread could start).
    local: Queue,
    threaded: bool,
}

impl Renderer {
    fn new(threaded: bool) -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let shared = Arc::new(native::Shared::default());
            let thread = threaded
                .then(|| native::spawn(Arc::clone(&shared)))
                .flatten();
            let threaded = thread.is_some();
            Self {
                shared,
                thread,
                local: Queue::default(),
                threaded,
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = threaded;
            Self {
                local: Queue::default(),
                threaded: false,
            }
        }
    }

    fn set_queue(&mut self, jobs: VecDeque<ThumbJob>) {
        #[cfg(not(target_arch = "wasm32"))]
        if self.threaded {
            self.shared.lock().jobs = jobs;
            self.shared.work.notify_one();
            return;
        }
        self.local.jobs = jobs;
    }

    /// Jobs queued, and the one rendering now.
    fn queued(&self) -> (usize, Option<TileKey>) {
        #[cfg(not(target_arch = "wasm32"))]
        if self.threaded {
            let q = self.shared.lock();
            return (q.jobs.len(), q.running);
        }
        (self.local.jobs.len(), None)
    }

    fn take_done(&mut self) -> Vec<(TileKey, Arc<Tile>, f64)> {
        #[cfg(not(target_arch = "wasm32"))]
        if self.threaded {
            return std::mem::take(&mut self.shared.lock().done);
        }
        std::mem::take(&mut self.local.done)
    }

    fn set_waker(&mut self, waker: Waker) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.shared.lock().waker = Some(Arc::clone(&waker));
        }
        self.local.waker = Some(waker);
    }

    /// The stepping driver: renders jobs until `budget_ms` is spent, at least one.
    /// Returns whether jobs remain.
    fn tick(&mut self, budget_ms: f64) -> bool {
        if self.threaded {
            return false;
        }
        let start = clock::now_ms();
        while let Some(job) = self.local.jobs.pop_front() {
            let t = clock::now_ms();
            let tile = job.render();
            self.local
                .done
                .push((job.key, Arc::new(tile), clock::now_ms() - t));
            if clock::now_ms() - start >= budget_ms {
                break;
            }
        }
        !self.local.jobs.is_empty()
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.shared.lock().shutdown = true;
            self.shared.work.notify_one();
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
        }
    }
}

/// What the gradient tile is made from: mode, palette (its debug form) and background.
type GradientKey = (Mode, String, Option<ColourIndex>);

/// What the picture tile is cropped from: the document revision, the cell and the
/// background.
type PictureKey = (u64, CellPos, Option<ColourIndex>);

/// A batch of renders being timed for the log (the benchmark of UX §6a).
#[derive(Clone, Copy, Debug)]
struct Batch {
    started: f64,
    tiles: usize,
    render_ms: f64,
}

/// The picker's thumbnails: sources, the solver, the render queue and the textures.
pub struct MethodThumbs {
    renderer: Renderer,
    textures: HashMap<TileKey, TextureHandle>,
    order: VecDeque<TileKey>,
    solver: Option<(String, Arc<TileSolver>)>,
    gradient: Option<(GradientKey, Arc<SourceTile>)>,
    picture: Option<(PictureKey, Arc<SourceTile>)>,
    /// The settings hash of the last recipe asked about.
    settings: std::cell::RefCell<Option<(Recipe, u64)>>,
    batch: Option<Batch>,
    /// This frame's wanted thumbnails, collected while drawing ([`Self::want`]).
    wanted: std::cell::RefCell<Vec<ThumbJob>>,
    /// Which source the cards show.
    pub source: PreviewSource,
}

impl std::fmt::Debug for MethodThumbs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MethodThumbs")
            .field("textures", &self.textures.len())
            .field("source", &self.source)
            .finish_non_exhaustive()
    }
}

/// Thumbnail textures kept (every method on two sources, twice over).
const CAPACITY: usize = 4 * convert64_core::preview::catalogue::CATALOGUE.len();

/// The wasm driver's work per frame, ms (the solve scheduler has its own 10 ms).
pub const FRAME_BUDGET_MS: f64 = 4.0;

impl Default for MethodThumbs {
    fn default() -> Self {
        Self::new(cfg!(not(target_arch = "wasm32")))
    }
}

impl MethodThumbs {
    /// Thumbnails rendered on a worker thread (`threaded`, native only) or by
    /// [`Self::tick`].
    pub fn new(threaded: bool) -> Self {
        Self {
            renderer: Renderer::new(threaded),
            textures: HashMap::new(),
            order: VecDeque::new(),
            solver: None,
            gradient: None,
            picture: None,
            settings: std::cell::RefCell::new(None),
            batch: None,
            wanted: std::cell::RefCell::new(Vec::new()),
            source: PreviewSource::default(),
        }
    }

    /// Asks for a repaint (natively from the worker) when a thumbnail is ready.
    pub fn set_waker(&mut self, waker: Waker) {
        self.renderer.set_waker(waker);
    }

    /// Once per frame before drawing: the solver for `doc`'s palette and solve options,
    /// the current source (the picture crop only when `picture` is wanted: at `cell`, or
    /// the centre), and textures for finished tiles. `background` is the result's.
    pub fn prepare(
        &mut self,
        ctx: &egui::Context,
        doc: &mut Document,
        background: Option<ColourIndex>,
        cell: Option<CellPos>,
    ) {
        let palette: &Palette = doc.palette();
        let options: SolveOptions = doc
            .session()
            .project()
            .solve
            .options(convert64_core::solve::SolveMode::Full, Vec::new());
        let solver_key = format!("{palette:?}{options:?}");
        if self.solver.as_ref().is_none_or(|(k, _)| *k != solver_key) {
            self.solver = Some((solver_key, Arc::new(TileSolver::new(palette, options))));
            *self.settings.borrow_mut() = None;
        }
        let mode = doc.mode();
        let pal_key = format!("{palette:?}");
        let gkey = (mode, pal_key, background);
        if self.gradient.as_ref().is_none_or(|(k, _)| *k != gkey) {
            let tile = SourceTile::gradient(mode, palette, background);
            self.gradient = Some((gkey, Arc::new(tile)));
        }
        if self.source == PreviewSource::Picture {
            let cell = cell.unwrap_or(CellPos::new(20, 12));
            let pkey = (doc.revision(), cell, background);
            if self.picture.as_ref().is_none_or(|(k, _)| *k != pkey) {
                let tile = SourceTile::crop(doc.target_frame(), cell, background);
                self.picture = Some((pkey, Arc::new(tile)));
            }
        }
        for (key, tile, ms) in self.renderer.take_done() {
            let image = ColorImage::from_rgba_unmultiplied(
                [tile.width as usize, tile.height as usize],
                &tile.rgba,
            );
            // C64 pixels: never smoothed.
            let tex = ctx.load_texture("method_thumb", image, TextureOptions::NEAREST);
            if self.textures.insert(key, tex).is_none() {
                self.order.push_back(key);
            }
            while self.textures.len() > CAPACITY {
                if let Some(old) = self.order.pop_front() {
                    self.textures.remove(&old);
                }
            }
            if let Some(b) = &mut self.batch {
                b.tiles += 1;
                b.render_ms += ms;
            }
        }
        if let Some(b) = self.batch
            && self.renderer.queued() == (0, None)
        {
            clock::log(&format!(
                "convert64: rendered {} method thumbnails ({:?}) in {:.1} ms, {:.1} ms of solving (#260)",
                b.tiles,
                self.source,
                clock::now_ms() - b.started,
                b.render_ms
            ));
            self.batch = None;
        }
    }

    /// The tile source the cards show now.
    fn current_source(&self) -> Option<&Arc<SourceTile>> {
        match self.source {
            PreviewSource::Gradient => self.gradient.as_ref().map(|(_, s)| s),
            PreviewSource::Picture => self.picture.as_ref().map(|(_, s)| s),
        }
    }

    /// The cache key of `method`'s thumbnail with `recipe`'s other settings, on the
    /// current source.
    pub fn key(&self, method: Method, recipe: &Recipe) -> Option<TileKey> {
        let (_, solver) = self.solver.as_ref()?;
        let source = self.current_source()?;
        let mut cached = self.settings.borrow_mut();
        let settings = match &*cached {
            Some((r, h)) if r == recipe => *h,
            _ => {
                let h = settings_hash(recipe, source.mode(), solver.palette(), solver.options());
                *cached = Some((*recipe, h));
                h
            }
        };
        Some(TileKey {
            method,
            settings,
            source: source.hash(),
        })
    }

    /// The thumbnail of `key`, if rendered.
    pub fn texture(&self, key: &TileKey) -> Option<&TextureHandle> {
        self.textures.get(key)
    }

    /// The job that renders `method`'s thumbnail with `recipe`, unless it is rendered
    /// or on its way.
    pub fn job(&self, method: Method, recipe: &Recipe) -> Option<ThumbJob> {
        let key = self.key(method, recipe)?;
        if self.textures.contains_key(&key) {
            return None;
        }
        Some(ThumbJob {
            key,
            method,
            recipe: *recipe,
            source: Arc::clone(self.current_source()?),
            solver: Arc::clone(&self.solver.as_ref()?.1),
        })
    }

    /// This frame's wanted thumbnails, in the order to render them (visible cards
    /// first): replaces the queue, so anything no longer wanted is dropped. Keys
    /// already rendering stay pending.
    pub fn request(&mut self, jobs: Vec<ThumbJob>) {
        let (queued, running) = self.renderer.queued();
        let mut seen = HashSet::new();
        let queue: VecDeque<ThumbJob> = jobs
            .into_iter()
            .filter(|j| {
                !self.textures.contains_key(&j.key) && Some(j.key) != running && seen.insert(j.key)
            })
            .collect();
        if queue.is_empty() && queued == 0 {
            return;
        }
        if self.batch.is_none() && !queue.is_empty() {
            self.batch = Some(Batch {
                started: clock::now_ms(),
                tiles: 0,
                render_ms: 0.0,
            });
        }
        self.renderer.set_queue(queue);
    }

    /// Adds `jobs` to this frame's wanted thumbnails (while drawing, from a shared
    /// borrow); [`Self::flush`] queues them.
    pub fn want(&self, jobs: Vec<ThumbJob>) {
        self.wanted.borrow_mut().extend(jobs);
    }

    /// Queues this frame's wanted thumbnails in place of the last frame's (after
    /// drawing): what no card asked for this frame is cancelled.
    pub fn flush(&mut self) {
        let jobs = std::mem::take(&mut *self.wanted.borrow_mut());
        self.request(jobs);
    }

    /// Drops every queued thumbnail (the picker closed).
    pub fn cancel(&mut self) {
        self.renderer.set_queue(VecDeque::new());
        self.batch = None;
    }

    /// Whether thumbnails are queued.
    pub fn is_busy(&self) -> bool {
        self.renderer.queued() != (0, None)
    }

    /// The wasm driver's share of this frame (natively nothing). Returns whether to ask
    /// for another frame.
    pub fn tick(&mut self) -> bool {
        self.renderer.tick(FRAME_BUDGET_MS)
    }

    /// Renders everything queued now, on this thread (tests and screenshots).
    pub fn finish(&mut self) {
        while self.is_busy() {
            if self.renderer.threaded {
                std::thread::yield_now();
            } else {
                self.renderer.tick(f64::INFINITY);
            }
        }
    }
}
