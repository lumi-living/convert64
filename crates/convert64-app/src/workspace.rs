//! An open document with what the views draw from it: the newest solution and the
//! textures of the source composite and of the result, rebuilt only when they change.
//! The working view and the result panel share the result texture (UX §4.3: no second
//! render).

use crate::document::{Document, result_rgba};
use crate::drag::{DragState, SourceRequest, Via};
use convert64_core::layer::Transform;
use convert64_core::model::CellPos;
use convert64_core::solve::Solution;
use convert64_project::{JsonCommand, ProjectError};
use egui::{Color32, ColorImage, Pos2, TextureHandle, TextureOptions, pos2};
use std::sync::Arc;

/// A cell, column and row.
pub type Cell = (u32, u32);

/// A document and its preview.
pub struct Workspace {
    /// The open document.
    pub doc: Document,
    /// The newest solution, once the first solve is in.
    pub solution: Option<Arc<Solution>>,
    /// The newest solve's composite + solve time, in milliseconds.
    pub solve_ms: Option<f64>,
    /// A full search is running and the solution on screen is provisional or stale.
    pub solving: bool,
    /// When the running solve was submitted (egui seconds), for the slow-solve state.
    pub solve_started: Option<f64>,
    /// The user paused the solve (UX §11); the result on screen is stale until Resume.
    pub paused: bool,
    /// The method picker's thumbnails (UX §6a).
    pub method_thumbs: crate::thumbs::MethodThumbs,
    /// The method picker's live preview on the result, while one is showing.
    pub preview: Option<MethodPreview>,
    /// The 4× composite (`Session::PREVIEW_N`) and the revision it shows.
    source: Option<(u64, TextureHandle)>,
    /// The solve's composite (1×), shown while a slider is dragged (#264).
    coarse: Option<(u64, TextureHandle)>,
    /// The textures a transform drag moves on the GPU (#264).
    moving: Option<Moving>,
    /// The drag in progress, if any, and what the Source view draws for it this frame.
    drag: DragState,
    request: SourceRequest,
    /// How many composites the Source view has made, by kind.
    composites: Composites,
    result: Option<(*const Solution, TextureHandle)>,
    /// One thumbnail per layer, bottom first, keyed by the source it shows.
    thumbs: Vec<(*const convert64_core::SourceImage, TextureHandle)>,
    /// One mask thumbnail per layer with a mask, rebuilt on every edit.
    mask_thumbs: (u64, Vec<Option<TextureHandle>>),
    /// The quick-mask overlay: for which revision, layer and style, and its texture.
    mask_overlay: Option<((u64, usize, bool), TextureHandle)>,
    /// All regions' masks as one white texture (alpha = the strongest region), 320 × 200,
    /// for the Region tool's tint; with the revision it was made for.
    regions: Option<(u64, TextureHandle)>,
}

impl std::fmt::Debug for Workspace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Workspace")
            .field("doc", &self.doc)
            .field("solved", &self.solution.is_some())
            .field("solving", &self.solving)
            .finish_non_exhaustive()
    }
}

/// A method previewing on the result (UX §6a): which, and the committed result to
/// show again when the preview ends.
#[derive(Debug, Clone)]
pub struct MethodPreview {
    /// The method's display name, for the result header (`Preview: Atkinson`).
    pub name: &'static str,
    /// The result before the preview started.
    pub committed: Option<Arc<Solution>>,
    /// A full solve was running when the preview started (its result never came).
    pub was_solving: bool,
}

/// How many composites the Source view has made since the workspace opened, by kind: for
/// tests and timing probes (#264).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Composites {
    /// The 4× composite of every layer (`Session::PREVIEW_N`).
    pub full: u64,
    /// The solve's composite (1×), during a drag.
    pub coarse: u64,
    /// The 4× composites of some layers (below, the dragged one, above) when a transform
    /// drag starts.
    pub split: u64,
}

/// What a transform drag draws (#264): textures made when it started, moved on the GPU.
struct Moving {
    /// The drag's start revision and its layer: a new drag makes new textures.
    start: u64,
    layer: usize,
    /// The layer's source size and the transform `layer_tex` shows it at.
    size: (u32, u32),
    at: Transform,
    below: Option<TextureHandle>,
    layer_tex: TextureHandle,
    above: Option<TextureHandle>,
    /// The layer reached past the screen in `layer_tex`, so what lay outside is missing
    /// from it: the coarse composite is drawn underneath and shows there.
    clipped: bool,
    /// Where `layer_tex`'s corners are now, in screen units: the canvas corners through
    /// the affine map from `at` to the layer's current transform.
    quad: [Pos2; 4],
}

/// What the Source view draws this frame.
#[derive(Clone, Copy)]
pub enum SourcePicture<'a> {
    /// One texture over the whole picture: the 4× composite, or during a drag the
    /// solve's 1× composite (drawn with nearest filtering).
    Flat(&'a TextureHandle),
    /// A transform drag: textures laid over each other on the GPU.
    Moving(MovingPicture<'a>),
}

/// A transform drag's textures, bottom first (#264).
#[derive(Clone, Copy)]
pub struct MovingPicture<'a> {
    /// The current composite at 1×, under everything, when the moved layer's texture
    /// is missing what lay past the screen.
    pub backdrop: Option<&'a TextureHandle>,
    /// The layers below the moved one, at 4×, where they are.
    pub below: Option<&'a TextureHandle>,
    /// The moved layer by itself on the whole canvas, at 4×: its corners (0, 0), (1, 0),
    /// (1, 1), (0, 1) go to `quad`.
    pub layer: &'a TextureHandle,
    /// The layers above the moved one, at 4×, where they are.
    pub above: Option<&'a TextureHandle>,
    /// Where `layer`'s corners are, in screen units (a parallelogram).
    pub quad: [Pos2; 4],
    /// Draw `below` and `above` only inside `quad`: outside it `backdrop` is the
    /// composite.
    pub clipped: bool,
}

/// What the status bar says about one cell of the result.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CellInfo {
    /// The cell's colours, background first in multicolor.
    pub colours: Vec<Color32>,
    /// The cell's error (Σ weight × distance), if solved.
    pub error: Option<f32>,
    /// The cell's colour set is pinned.
    pub pinned: bool,
    /// The cell is locked.
    pub locked: bool,
    /// The cell clashes (colours wanted but dropped).
    pub clash: bool,
}

impl Workspace {
    /// A workspace for `doc`, with nothing solved yet.
    pub fn new(doc: Document) -> Self {
        Self {
            doc,
            solution: None,
            solve_ms: None,
            solving: true,
            solve_started: None,
            paused: false,
            method_thumbs: crate::thumbs::MethodThumbs::default(),
            preview: None,
            source: None,
            coarse: None,
            moving: None,
            drag: DragState::default(),
            request: SourceRequest::Full,
            composites: Composites::default(),
            result: None,
            thumbs: Vec::new(),
            mask_thumbs: (u64::MAX, Vec::new()),
            mask_overlay: None,
            regions: None,
        }
    }

    /// Starts (or moves) the method picker's live preview (UX §6a): what to solve on
    /// the interactive path, the project as it is with `method` in `target`'s recipe and
    /// the committed result as the previous solution. Nothing enters the history.
    pub fn preview_snapshot(
        &mut self,
        target: crate::shell::RecipeTarget,
        method: convert64_core::layer::Method,
    ) -> crate::scheduler::Snapshot {
        use crate::shell::RecipeTarget;
        let name = convert64_core::preview::catalogue::info(method.kind).name;
        let committed = match &mut self.preview {
            Some(p) => {
                p.name = name;
                p.committed.clone()
            }
            None => {
                let committed = self.solution.clone();
                self.preview = Some(MethodPreview {
                    name,
                    committed: committed.clone(),
                    was_solving: self.solving,
                });
                committed
            }
        };
        let mut snapshot = self.doc.snapshot(committed);
        match target {
            RecipeTarget::Layer(i) => {
                if let Some(l) = snapshot.layers.get_mut(i) {
                    let r = l.recipe().with_dither_method(method);
                    l.set_recipe(r);
                }
            }
            RecipeTarget::Region(i) => {
                if let Some(r) = snapshot.regions.get_mut(i) {
                    let recipe = r.recipe().with_dither_method(method);
                    r.set_recipe(recipe);
                }
            }
        }
        self.solving = true;
        snapshot
    }

    /// Ends the live preview, showing the committed result again. Returns whether the
    /// app must solve instead: there was no committed result, or a full solve was
    /// running when the preview started.
    pub fn revert_preview(&mut self) -> bool {
        let Some(p) = self.preview.take() else {
            return false;
        };
        if p.was_solving || p.committed.is_none() {
            if p.committed.is_some() {
                self.solution = p.committed;
            }
            return true;
        }
        self.solution = p.committed;
        self.solving = false;
        false
    }

    /// Ends the live preview for an edit (a commit from the picker, or anything else):
    /// the result on screen stays until the edit's solve replaces it, so a committed
    /// preview does not flash back to the old method.
    pub fn end_preview(&mut self) {
        self.preview = None;
    }

    /// Brings the method picker's thumbnails up to date (once per frame before
    /// drawing): `source` is what they show, `cell` the last cell hovered. They are
    /// solved against the committed result's background.
    pub fn prepare_thumbs(
        &mut self,
        ctx: &egui::Context,
        source: crate::thumbs::PreviewSource,
        cell: Option<CellPos>,
    ) {
        self.method_thumbs.source = source;
        let shown = match &self.preview {
            Some(p) => p.committed.as_ref(),
            None => self.solution.as_ref(),
        };
        let background = shown.and_then(|s| s.background);
        self.method_thumbs
            .prepare(ctx, &mut self.doc, background, cell);
    }

    /// Runs a registry command (ADR-005) as part of drag `gesture`, or by itself, and
    /// keeps track of the drag for the Source view (#264). `held_key`: the command comes
    /// from an arrow key that is still down, so its repeats move the view like a drag.
    pub fn edit(
        &mut self,
        gesture: Option<u64>,
        command: JsonCommand,
        held_key: bool,
    ) -> Result<(), ProjectError> {
        let via = match gesture {
            Some(_) => Via::Gesture,
            None if held_key => Via::HeldKey,
            None => Via::Single,
        };
        let layers = &self.doc.session().project().layers;
        self.drag
            .command(&command, via, layers, self.doc.revision());
        match gesture {
            Some(id) => self.doc.run_in_gesture(id, command)?,
            None => self.doc.run(command)?,
        }
        self.drag.ran(self.doc.revision());
        Ok(())
    }

    /// Ends the drag in progress: the next [`Self::prepare`] composites the Source view
    /// at 4× once.
    pub fn end_drag(&mut self) {
        self.drag.end();
    }

    /// Whether a drag is in progress.
    pub fn dragging(&self) -> bool {
        self.drag.current(self.doc.revision()).is_some()
    }

    /// How many composites the Source view has made, by kind.
    pub fn composites(&self) -> Composites {
        self.composites
    }

    /// Rebuilds whichever texture is out of date. Call once per frame before drawing.
    /// `ghost`: the source may be drawn as a ghost over the result, which needs one flat
    /// picture even while a layer is moved.
    ///
    /// The Source view's picture follows the drag in progress (#264, [`crate::drag`]):
    /// the 4× composite without one, and once more on its release; the solve's 1×
    /// composite during a slider drag; textures moved on the GPU during a transform drag.
    pub fn prepare(&mut self, ctx: &egui::Context, ghost: bool) {
        let rev = self.doc.revision();
        let drag = self.drag.current(rev).copied();
        let layers = &self.doc.session().project().layers;
        self.request = crate::drag::request(drag.as_ref(), layers);
        match self.request {
            SourceRequest::Full => {
                self.moving = None;
                self.coarse = None;
                if self.source.as_ref().is_none_or(|(r, _)| *r != rev) {
                    let (w, h, rgba) = self.doc.source_rgba();
                    let tex = ctx.load_texture("source", image(w, h, &rgba), smooth());
                    self.source = Some((rev, tex));
                    self.composites.full += 1;
                }
            }
            SourceRequest::Coarse => {
                self.moving = None;
                self.prepare_coarse(ctx, rev);
            }
            SourceRequest::Moving { layer, before } => {
                let start = drag.map_or(rev, |d| d.start);
                if self
                    .moving
                    .as_ref()
                    .is_none_or(|m| m.start != start || m.layer != layer)
                {
                    self.moving = Some(self.make_moving(ctx, start, layer, before));
                }
                let now = self.doc.session().project().layers[layer].transform();
                let clipped = self.moving.as_mut().is_some_and(|m| {
                    m.quad = moved_quad(m.at, now, m.size);
                    m.clipped
                });
                if clipped || ghost {
                    self.prepare_coarse(ctx, rev);
                } else {
                    self.coarse = None;
                }
            }
        }
        let layers = &self.doc.session().project().layers;
        let mut thumbs = Vec::with_capacity(layers.len());
        for layer in layers {
            let ptr = Arc::as_ptr(layer.source());
            let tex = match self.thumbs.iter().find(|(p, _)| *p == ptr) {
                Some((_, tex)) => tex.clone(),
                None => {
                    let (w, h, rgba) = thumbnail(layer.source(), THUMB);
                    let image = ColorImage::from_rgba_unmultiplied([w, h], &rgba);
                    ctx.load_texture("layer", image, TextureOptions::LINEAR)
                }
            };
            thumbs.push((ptr, tex));
        }
        self.thumbs = thumbs;
        if self.mask_thumbs.0 != rev {
            let masks = layers
                .iter()
                .map(|l| {
                    let mask = l.mask()?;
                    let (w, h, rgba) = mask_thumbnail(mask, THUMB);
                    let image = ColorImage::from_rgba_unmultiplied([w, h], &rgba);
                    Some(ctx.load_texture("mask", image, TextureOptions::LINEAR))
                })
                .collect();
            self.mask_thumbs = (rev, masks);
        }
        if let Some(sol) = &self.solution {
            let ptr = Arc::as_ptr(sol);
            if self.result.as_ref().is_none_or(|(p, _)| *p != ptr) {
                let (w, h, rgba) = result_rgba(sol, self.doc.palette(), self.doc.grid());
                let image = ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba);
                // C64 pixels: never smoothed (CLAUDE.md conventions).
                let tex = ctx.load_texture("result", image, TextureOptions::NEAREST);
                self.result = Some((ptr, tex));
            }
        }
    }

    /// The quick-mask overlay of `layer` for mask mode (UX §7a): red at 50 % where the
    /// mask hides the layer, or with `alone` the mask itself in grey. On the super grid,
    /// covering the screen. `None` if the layer has no mask.
    pub fn mask_overlay(
        &mut self,
        ctx: &egui::Context,
        layer: usize,
        alone: bool,
    ) -> Option<&TextureHandle> {
        let key = (self.doc.revision(), layer, alone);
        if self.mask_overlay.as_ref().is_none_or(|(k, _)| *k != key) {
            let (w, h, values) = self.doc.evaluated_mask(layer)?;
            let rgba: Vec<u8> = values
                .iter()
                .flat_map(|&v| {
                    let m = (v >> 8) as u8;
                    if alone {
                        [m, m, m, 255]
                    } else {
                        [255, 0, 0, (255 - m) / 2]
                    }
                })
                .collect();
            let image = ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba);
            let tex = ctx.load_texture("mask_overlay", image, TextureOptions::LINEAR);
            self.mask_overlay = Some((key, tex));
        }
        self.mask_overlay.as_ref().map(|(_, t)| t)
    }

    /// The regions' coverage (white, alpha = mask) for the tint overlay; `None` without
    /// regions.
    pub fn region_overlay(&mut self, ctx: &egui::Context) -> Option<&TextureHandle> {
        let rev = self.doc.revision();
        let regions = &self.doc.session().project().regions;
        if regions.is_empty() {
            self.regions = None;
            return None;
        }
        if self.regions.as_ref().is_none_or(|(r, _)| *r != rev) {
            let (w, h) = (
                convert64_core::region::CANVAS_WIDTH,
                convert64_core::region::CANVAS_HEIGHT,
            );
            let mut alpha = vec![0.0f32; (w * h) as usize];
            for r in regions {
                for (a, v) in alpha.iter_mut().zip(r.mask().values()) {
                    *a = a.max(*v);
                }
            }
            let rgba: Vec<u8> = alpha
                .iter()
                .flat_map(|a| [255, 255, 255, (a.clamp(0.0, 1.0) * 255.0) as u8])
                .collect();
            let image = ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba);
            self.regions = Some((
                rev,
                ctx.load_texture("regions", image, TextureOptions::LINEAR),
            ));
        }
        self.regions.as_ref().map(|(_, t)| t)
    }

    /// Layer `i`'s mask thumbnail, if it has a mask.
    pub fn mask_thumbnail(&self, i: usize) -> Option<&TextureHandle> {
        self.mask_thumbs.1.get(i)?.as_ref()
    }

    /// Layer `i`'s thumbnail (stack index, bottom first).
    pub fn thumbnail(&self, i: usize) -> Option<&TextureHandle> {
        self.thumbs.get(i).map(|(_, t)| t)
    }

    /// The solve's composite (1×) as a texture of this revision, for a drag.
    fn prepare_coarse(&mut self, ctx: &egui::Context, rev: u64) {
        if self.coarse.as_ref().is_none_or(|(r, _)| *r != rev) {
            let (w, h, rgba) = self.doc.solve_canvas_rgba();
            // Upscaled with nearest filtering: the pixels the solve reads.
            let tex = ctx.load_texture("source_1x", image(w, h, &rgba), TextureOptions::NEAREST);
            self.coarse = Some((rev, tex));
            self.composites.coarse += 1;
        }
    }

    /// The textures a transform drag of `layer` moves, for the drag that started at
    /// revision `start` with the layer at `before`. If the layer is the only visible one
    /// and the 4× composite is the one from before the drag, that is the layer's texture
    /// and nothing is composited. Otherwise the layers below, the layer and the layers
    /// above are each composited at 4×, once per drag.
    fn make_moving(
        &mut self,
        ctx: &egui::Context,
        start: u64,
        layer: usize,
        before: Transform,
    ) -> Moving {
        let session = self.doc.session();
        let layers = &session.project().layers;
        let l = &layers[layer];
        let size = l.source().size();
        let alone = l.visible()
            && layers
                .iter()
                .enumerate()
                .all(|(i, o)| i == layer || !o.visible());
        if alone
            && let Some((r, tex)) = &self.source
            && *r == start
        {
            return Moving {
                start,
                layer,
                size,
                at: before,
                below: None,
                layer_tex: tex.clone(),
                above: None,
                clipped: !on_screen(before, size),
                quad: [Pos2::ZERO; 4],
            };
        }
        let texture = |(w, h, rgba): (u32, u32, Vec<u8>)| {
            ctx.load_texture("source_layers", image(w, h, &rgba), smooth())
        };
        let any_visible = |ls: &[convert64_core::Layer]| ls.iter().any(|o| o.visible());
        let below =
            any_visible(&layers[..layer]).then(|| texture(session.layers_canvas_rgba(0..layer)));
        // At its transform from before the drag, so a layer that was on the screen is
        // wholly in its texture.
        let layer_tex = texture(session.layer_canvas_rgba_at(layer, before));
        let above = any_visible(&layers[layer + 1..])
            .then(|| texture(session.layers_canvas_rgba(layer + 1..layers.len())));
        self.composites.split += 1 + u64::from(below.is_some()) + u64::from(above.is_some());
        Moving {
            start,
            layer,
            size,
            at: before,
            below,
            layer_tex,
            above,
            clipped: !on_screen(before, size),
            quad: [Pos2::ZERO; 4],
        }
    }

    /// What the Source view draws this frame.
    pub fn source_picture(&self) -> Option<SourcePicture<'_>> {
        if let (SourceRequest::Moving { .. }, Some(m)) = (self.request, &self.moving) {
            return Some(SourcePicture::Moving(MovingPicture {
                backdrop: self.coarse.as_ref().map(|(_, t)| t).filter(|_| m.clipped),
                below: m.below.as_ref(),
                layer: &m.layer_tex,
                above: m.above.as_ref(),
                quad: m.quad,
                clipped: m.clipped,
            }));
        }
        self.source_texture().map(SourcePicture::Flat)
    }

    /// The source as one texture: the 4× composite, or during a drag the solve's 1×
    /// composite (the ghost over the result draws it).
    pub fn source_texture(&self) -> Option<&TextureHandle> {
        match self.request {
            SourceRequest::Full => self.source.as_ref(),
            _ => self.coarse.as_ref().or(self.source.as_ref()),
        }
        .map(|(_, t)| t)
    }

    /// The result's texture.
    pub fn result_texture(&self) -> Option<&TextureHandle> {
        self.result.as_ref().map(|(_, t)| t)
    }

    /// The border's colour: the solution's background, black in hi-res (the project has
    /// no border colour of its own yet).
    pub fn border_colour(&self) -> Color32 {
        self.solution
            .as_ref()
            .and_then(|s| s.background)
            .map_or(Color32::BLACK, |bg| rgb(self.doc.palette().rgb(bg)))
    }

    /// Cells that clash in the newest solution, and their count.
    pub fn clashes(&self) -> Option<usize> {
        self.solution.as_ref().map(|s| s.report.summary().clashes)
    }

    /// The worst cell, if any clashes.
    pub fn worst(&self) -> Option<(u32, u32)> {
        let worst = self.solution.as_ref()?.report.summary().worst?;
        Some((u32::from(worst.x), u32::from(worst.y)))
    }

    /// What the status bar shows for `cell`.
    pub fn cell_info(&self, cell: (u32, u32)) -> CellInfo {
        let pos = CellPos {
            x: cell.0 as u16,
            y: cell.1 as u16,
        };
        let steering = &self.doc.session().project().steering;
        let mut info = CellInfo {
            pinned: steering.pin(pos).is_some(),
            locked: steering.locks().contains(pos),
            ..CellInfo::default()
        };
        if let Some(sol) = &self.solution {
            let palette = self.doc.palette();
            let cols = self.doc.grid().cells().0;
            info.colours
                .extend(sol.background.map(|bg| rgb(palette.rgb(bg))));
            if let Some(set) = sol.sets.get((cell.1 * cols + cell.0) as usize) {
                info.colours.extend(set.iter().map(|c| rgb(palette.rgb(c))));
            }
            if let Some(report) = sol.report.cell(pos) {
                info.error = Some(report.error);
                info.clash = report.is_clash();
            }
        }
        info
    }

    /// Every cell's error, row-major, with the largest, for the clash heatmap.
    pub fn cell_errors(&self) -> Option<(Vec<f32>, f32)> {
        let sol = self.solution.as_ref()?;
        let (cols, rows) = self.doc.grid().cells();
        let mut errors = vec![0.0; (cols * rows) as usize];
        for r in &sol.report.cells {
            errors[(u32::from(r.cell.y) * cols + u32::from(r.cell.x)) as usize] = r.error;
        }
        let max = errors.iter().copied().fold(0.0, f32::max);
        Some((errors, max))
    }

    /// Cells that clash (colours wanted but dropped).
    pub fn clash_cells(&self) -> Vec<(u32, u32)> {
        self.solution.as_ref().map_or_else(Vec::new, |s| {
            s.report
                .cells
                .iter()
                .filter(|r| r.is_clash())
                .map(|r| (u32::from(r.cell.x), u32::from(r.cell.y)))
                .collect()
        })
    }

    /// Pinned and locked cells.
    pub fn steered_cells(&self) -> (Vec<Cell>, Vec<Cell>) {
        let steering = &self.doc.session().project().steering;
        let xy = |c: CellPos| (u32::from(c.x), u32::from(c.y));
        (
            steering.pins().map(|(c, _)| xy(c)).collect(),
            steering.locks().iter().map(xy).collect(),
        )
    }
}

/// RGBA8 (straight alpha) as an egui image.
fn image(w: u32, h: u32, rgba: &[u8]) -> ColorImage {
    ColorImage::from_rgba_unmultiplied([w as usize, h as usize], rgba)
}

/// A photograph's texture options: smooth when scaled.
fn smooth() -> TextureOptions {
    TextureOptions {
        mipmap_mode: Some(egui::TextureFilter::Linear),
        ..TextureOptions::LINEAR
    }
}

/// Whether a layer of source `size` at `t` lies within the screen, so a texture of the
/// canvas holds all of it.
fn on_screen(t: Transform, size: (u32, u32)) -> bool {
    const EPS: f32 = 1e-3;
    let screen = crate::view::SCREEN;
    t.canvas_quad(size)
        .iter()
        .all(|p| (-EPS..=screen.x + EPS).contains(&p.x) && (-EPS..=screen.y + EPS).contains(&p.y))
}

/// The screen's corners (0, 0), (w, 0), (w, h), (0, h) through the affine map that takes
/// a layer of source `size` from transform `from` to `to`, in screen units: where a
/// texture of the canvas with the layer at `from` goes to show the layer at `to`.
fn moved_quad(from: Transform, to: Transform, size: (u32, u32)) -> [Pos2; 4] {
    let screen = crate::view::SCREEN;
    let corners = [
        (0.0, 0.0),
        (screen.x, 0.0),
        (screen.x, screen.y),
        (0.0, screen.y),
    ];
    corners.map(|(x, y)| {
        let p = convert64_core::Vec2::new(x, y);
        let p = to.layer_to_canvas(size, from.canvas_to_layer(size, p));
        pos2(p.x, p.y)
    })
}

fn rgb(c: convert64_core::palette::Rgb8) -> Color32 {
    Color32::from_rgb(c.r, c.g, c.b)
}

/// The longest side of a layer thumbnail's texture, in pixels (drawn smaller, sharp on
/// high-density screens).
const THUMB: u32 = 64;

/// `src` box-filtered to fit `side` × `side`, as sRGB8 RGBA. Width and height first.
fn thumbnail(src: &convert64_core::SourceImage, side: u32) -> (usize, usize, Vec<u8>) {
    use convert64_core::palette::linear_to_srgb8;
    let (w, h) = src.size();
    let s = (side as f32 / w.max(h) as f32).min(1.0);
    let (tw, th) = (
        ((w as f32 * s).round() as u32).max(1),
        ((h as f32 * s).round() as u32).max(1),
    );
    let px = src.pixels();
    let mut out = Vec::with_capacity((tw * th * 4) as usize);
    for ty in 0..th {
        let (y0, y1) = (ty * h / th, ((ty + 1) * h / th).max(ty * h / th + 1));
        for tx in 0..tw {
            let (x0, x1) = (tx * w / tw, ((tx + 1) * w / tw).max(tx * w / tw + 1));
            let mut sum = [0.0f32; 4];
            for y in y0..y1 {
                for x in x0..x1 {
                    let p = px[(y * w + x) as usize];
                    // Straight alpha: weight colour by coverage.
                    for c in 0..3 {
                        sum[c] += p[c] * p[3];
                    }
                    sum[3] += p[3];
                }
            }
            let n = ((y1 - y0) * (x1 - x0)) as f32;
            let a = sum[3] / n;
            let rgb = if sum[3] > 0.0 {
                [0, 1, 2].map(|c| linear_to_srgb8(sum[c] / sum[3]))
            } else {
                [0; 3]
            };
            out.extend([rgb[0], rgb[1], rgb[2], (a * 255.0).round() as u8]);
        }
    }
    (tw as usize, th as usize, out)
}

/// A mask's raster with its invert applied, box-filtered to fit `side`, as grey RGBA.
fn mask_thumbnail(mask: &convert64_core::Mask, side: u32) -> (usize, usize, Vec<u8>) {
    let raster = mask.raster();
    let (w, h) = raster.size();
    let s = (side as f32 / w.max(h) as f32).min(1.0);
    let (tw, th) = (
        ((w as f32 * s).round() as u32).max(1),
        ((h as f32 * s).round() as u32).max(1),
    );
    let invert = mask.modifiers().invert();
    let mut out = Vec::with_capacity((tw * th * 4) as usize);
    for ty in 0..th {
        for tx in 0..tw {
            let (x, y) = ((tx * w / tw).min(w - 1), (ty * h / th).min(h - 1));
            let mut v = raster.get(x, y);
            if invert {
                v = 1.0 - v;
            }
            let g = (v.clamp(0.0, 1.0) * 255.0).round() as u8;
            out.extend([g, g, g, 255]);
        }
    }
    (tw as usize, th as usize, out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scheduler::{Config, Scheduler};
    use crate::shell::RecipeTarget;
    use convert64_core::layer::{DitherKind, Method};
    use convert64_core::solve::SolveMode;
    use convert64_project::codec::encode_rgba8;
    use convert64_project::{JsonCommand, LayerRef};

    fn workspace() -> Workspace {
        let rgba = (0..64 * 40)
            .flat_map(|i| [(i % 64 * 4) as u8, (i / 64 * 6) as u8, 120, 255])
            .collect();
        let doc = Document::from_png("g.png", &encode_rgba8(64, 40, rgba)).unwrap();
        Workspace::new(doc)
    }

    fn solve(s: &mut Scheduler, ws: &mut Workspace, mode: SolveMode) {
        s.submit(ws.doc.snapshot(ws.solution.clone()), mode);
        while s.tick() {}
        let r = s.poll().expect("a result");
        ws.solving = r.provisional;
        ws.solution = Some(r.solution);
    }

    fn method(ws: &Workspace) -> DitherKind {
        ws.doc.session().project().layers[0].recipe().method().kind
    }

    /// UX §6a: a preview solves the picture with the method but changes neither the
    /// project nor the history; moving off shows the committed result again; a commit
    /// is one undo step.
    #[test]
    fn a_preview_reverts_and_a_commit_is_one_undo_step() {
        let mut s = Scheduler::stepping(Config::default());
        let mut ws = workspace();
        solve(&mut s, &mut ws, SolveMode::Full);
        let committed = ws.solution.clone().unwrap();
        let history = ws.doc.history();
        let before = method(&ws);
        assert_eq!(before, DitherKind::FloydSteinberg);

        let target = RecipeTarget::Layer(0);
        let snap = ws.preview_snapshot(target, Method::new(DitherKind::Bayer4));
        assert_eq!(snap.layers[0].recipe().method().kind, DitherKind::Bayer4);
        s.submit(
            snap,
            SolveMode::Interactive {
                dirty: convert64_core::CellSet::all(),
            },
        );
        while s.tick() {}
        ws.solution = Some(s.poll().unwrap().solution);
        assert_ne!(
            ws.solution.as_deref(),
            Some(&*committed),
            "the result shows Bayer 4×4"
        );
        assert_eq!(ws.preview.as_ref().unwrap().name, "Bayer 4×4");
        assert_eq!(method(&ws), before, "the project is unchanged");
        assert_eq!(ws.doc.history(), history, "nothing in the history");

        // Arrowing on moves the preview; the committed result stays the one to return to.
        let _ = ws.preview_snapshot(target, Method::new(DitherKind::Atkinson));
        assert_eq!(ws.preview.as_ref().unwrap().name, "Atkinson");
        assert!(!ws.revert_preview(), "no solve needed to revert");
        assert!(Arc::ptr_eq(ws.solution.as_ref().unwrap(), &committed));
        assert!(ws.preview.is_none());

        // Preview, then commit: one command, one undo step.
        let _ = ws.preview_snapshot(target, Method::new(DitherKind::Atkinson));
        ws.end_preview();
        let recipe = convert64_core::command::RecipeChange {
            method: Some(Method::new(DitherKind::Atkinson)),
            ..Default::default()
        };
        ws.doc
            .run(JsonCommand::RecipeSet {
                layer: LayerRef::Index(0),
                recipe,
            })
            .unwrap();
        assert_eq!(method(&ws), DitherKind::Atkinson);
        assert!(ws.doc.undo());
        assert_eq!(method(&ws), before, "one undo restores the old method");
        assert_eq!(
            ws.doc.history(),
            (history.0, true),
            "back where it was, with a redo"
        );
    }

    /// A preview started while a full solve was running asks for a solve on revert.
    #[test]
    fn reverting_a_preview_over_a_running_solve_solves_again() {
        let mut ws = workspace();
        ws.solving = true;
        let _ = ws.preview_snapshot(RecipeTarget::Layer(0), Method::new(DitherKind::Checker));
        assert!(ws.revert_preview());
    }
}

/// #264: which composite the Source view makes during drags, and that a release makes
/// exactly one 4× composite.
#[cfg(test)]
mod drag_tests {
    use super::*;
    use convert64_core::Vec2;
    use convert64_core::command::AdjustmentsChange;
    use convert64_core::layer::BrightnessContrast;
    use convert64_project::LayerRef;

    /// Gesture ids, as the shell's controls have them.
    const NUDGE: u64 = 1;
    const SLIDER: u64 = 2;

    fn png(w: u32, h: u32) -> Vec<u8> {
        let rgba = (0..w * h)
            .flat_map(|i| [(i % w * 4) as u8, (i / w * 6) as u8, 128, 255])
            .collect();
        convert64_project::codec::encode_rgba8(w, h, rgba)
    }

    /// A picture filling the screen, its first 4× composite made.
    fn workspace(ctx: &egui::Context) -> Workspace {
        let doc = Document::from_png("ramp.png", &png(64, 40)).unwrap();
        let mut ws = Workspace::new(doc);
        ws.prepare(ctx, false);
        assert_eq!(ws.composites().full, 1);
        ws
    }

    fn nudge(layer: usize, x: f32) -> JsonCommand {
        JsonCommand::LayerNudge {
            layer: LayerRef::Index(layer),
            by: Vec2::new(x, 0.0),
        }
    }

    fn brightness(v: f32) -> JsonCommand {
        JsonCommand::AdjustmentsSet {
            layer: LayerRef::Index(0),
            adjustments: AdjustmentsChange {
                brightness_contrast: Some(BrightnessContrast::new(v, 0.0).unwrap()),
                ..Default::default()
            },
        }
    }

    fn composites(full: u64, coarse: u64, split: u64) -> Composites {
        Composites {
            full,
            coarse,
            split,
        }
    }

    /// Releases the drag and draws two frames: one 4× composite, then none.
    fn release(ws: &mut Workspace, ctx: &egui::Context) {
        let before = ws.composites();
        ws.doc.end_gesture();
        ws.end_drag();
        ws.prepare(ctx, false);
        ws.prepare(ctx, false);
        let after = ws.composites();
        assert_eq!(after.full, before.full + 1, "one 4× composite on release");
        assert_eq!((after.coarse, after.split), (before.coarse, before.split));
        assert!(matches!(ws.source_picture(), Some(SourcePicture::Flat(_))));
    }

    #[test]
    fn a_slider_drag_shows_the_1x_composite_and_its_release_composites_4x_once() {
        let ctx = egui::Context::default();
        let mut ws = workspace(&ctx);
        for (i, v) in [0.1, 0.2, 0.3, 0.4].into_iter().enumerate() {
            ws.edit(Some(SLIDER), brightness(v), false).unwrap();
            ws.prepare(&ctx, false);
            assert_eq!(ws.composites(), composites(1, i as u64 + 1, 0));
            // The solve's grid, multicolor 1×: 160 × 200, drawn over the whole picture.
            let Some(SourcePicture::Flat(tex)) = ws.source_picture() else {
                panic!("a flat picture");
            };
            assert_eq!(tex.size(), [160, 200]);
        }
        // A frame without an edit composites nothing.
        ws.prepare(&ctx, false);
        assert_eq!(ws.composites(), composites(1, 4, 0));
        release(&mut ws, &ctx);
        assert_eq!(ws.composites(), composites(2, 4, 0));
    }

    #[test]
    fn a_transform_drag_of_the_only_layer_moves_its_texture_and_composites_nothing() {
        let ctx = egui::Context::default();
        let mut ws = workspace(&ctx);
        for x in [1.0, 2.5, 4.0] {
            ws.edit(Some(NUDGE), nudge(0, x), false).unwrap();
            ws.prepare(&ctx, false);
            // The 4× composite from before the drag is the layer's texture.
            assert_eq!(ws.composites(), composites(1, 0, 0));
            let Some(SourcePicture::Moving(m)) = ws.source_picture() else {
                panic!("moving textures");
            };
            assert!(m.below.is_none() && m.above.is_none() && m.backdrop.is_none());
            assert!(!m.clipped, "the layer fills the screen exactly");
            assert_eq!(m.quad[0], pos2(x, 0.0));
            assert_eq!(m.quad[2], pos2(320.0 + x, 200.0));
        }
        release(&mut ws, &ctx);
    }

    #[test]
    fn a_transform_drag_over_other_layers_composites_each_part_once_when_it_starts() {
        let ctx = egui::Context::default();
        let mut ws = workspace(&ctx);
        ws.doc.add_layer("logo.png", &png(16, 8), None).unwrap();
        ws.prepare(&ctx, false);
        assert_eq!(ws.composites(), composites(2, 0, 0));
        // The logo on top: the layers below, then the logo by itself.
        for x in [1.0, 2.0, 3.0] {
            ws.edit(Some(NUDGE), nudge(1, x), false).unwrap();
            ws.prepare(&ctx, false);
            assert_eq!(ws.composites(), composites(2, 0, 2));
            let Some(SourcePicture::Moving(m)) = ws.source_picture() else {
                panic!("moving textures");
            };
            assert!(m.below.is_some() && m.above.is_none());
        }
        release(&mut ws, &ctx);
        // The picture at the bottom: itself, then the layers above.
        for x in [1.0, 2.0] {
            ws.edit(Some(NUDGE), nudge(0, x), false).unwrap();
            ws.prepare(&ctx, false);
            assert_eq!(ws.composites(), composites(3, 0, 4));
            let Some(SourcePicture::Moving(m)) = ws.source_picture() else {
                panic!("moving textures");
            };
            assert!(m.below.is_none() && m.above.is_some());
            // Its texture is from before the drag, when it filled the screen exactly.
            assert!(!m.clipped && m.backdrop.is_none());
        }
        release(&mut ws, &ctx);
    }

    /// The textures a transform drag lays over each other are the 4× composite: Normal
    /// blending is associative. Pixels the upper texture covers partly (its antialiased
    /// edge) are left out: the GPU blends them in sRGB.
    #[test]
    fn below_layer_and_above_laid_over_each_other_are_the_composite() {
        let doc = Document::from_png("ramp.png", &png(64, 40)).unwrap();
        let mut ws = Workspace::new(doc);
        ws.doc.add_layer("a.png", &png(16, 8), None).unwrap();
        ws.doc.add_layer("b.png", &png(24, 24), None).unwrap();
        for (i, x) in [(1, 30.25), (2, 20.5)] {
            ws.edit(None, nudge(i, x), false).unwrap();
        }
        let (w, h, full) = ws.doc.source_rgba();
        let session = ws.doc.session();
        let before = session.project().layers[1].transform();
        let parts = [
            session.layers_canvas_rgba(0..1),
            session.layer_canvas_rgba_at(1, before),
            session.layers_canvas_rgba(2..3),
        ];
        assert!(parts.iter().all(|p| (p.0, p.1) == (w, h)));
        let mut compared = 0;
        for (i, want) in full.chunks(4).enumerate() {
            let px: Vec<&[u8]> = parts.iter().map(|p| &p.2[i * 4..i * 4 + 4]).collect();
            let mut got = px[0];
            let mut exact = true;
            for part in &px[1..] {
                match part[3] {
                    0 => {}
                    255 => got = part,
                    _ => exact = false,
                }
            }
            if exact {
                assert_eq!(got, want, "pixel {i}");
                compared += 1;
            }
        }
        assert!(compared > full.len() / 4 * 9 / 10, "{compared}");
    }

    #[test]
    fn a_layer_reaching_past_the_screen_has_the_1x_composite_under_it() {
        let ctx = egui::Context::default();
        let mut ws = workspace(&ctx);
        ws.edit(None, nudge(0, 10.0), false).unwrap();
        ws.prepare(&ctx, false);
        assert_eq!(ws.composites(), composites(2, 0, 0));
        for (i, x) in [1.0, 2.0, 3.0].into_iter().enumerate() {
            ws.edit(Some(NUDGE), nudge(0, x), false).unwrap();
            ws.prepare(&ctx, false);
            assert_eq!(ws.composites(), composites(2, i as u64 + 1, 0));
            let Some(SourcePicture::Moving(m)) = ws.source_picture() else {
                panic!("moving textures");
            };
            assert!(m.clipped && m.backdrop.is_some());
        }
        release(&mut ws, &ctx);
    }

    #[test]
    fn a_translucent_layer_is_dragged_on_the_1x_composite() {
        let ctx = egui::Context::default();
        let mut ws = workspace(&ctx);
        let opacity = JsonCommand::LayerSetOpacity {
            layer: LayerRef::Index(0),
            opacity: 0.5,
        };
        ws.edit(None, opacity, false).unwrap();
        ws.prepare(&ctx, false);
        ws.edit(Some(NUDGE), nudge(0, 1.0), false).unwrap();
        ws.prepare(&ctx, false);
        assert_eq!(ws.composites(), composites(2, 1, 0));
        assert!(matches!(ws.source_picture(), Some(SourcePicture::Flat(_))));
        release(&mut ws, &ctx);
    }

    #[test]
    fn held_arrow_keys_move_the_texture_until_released() {
        let ctx = egui::Context::default();
        let mut ws = workspace(&ctx);
        for _ in 0..3 {
            ws.edit(None, nudge(0, 1.0), true).unwrap();
            ws.prepare(&ctx, false);
            assert!(ws.dragging());
            assert!(matches!(
                ws.source_picture(),
                Some(SourcePicture::Moving(_))
            ));
        }
        assert_eq!(ws.composites(), composites(1, 0, 0));
        assert_eq!(
            ws.source_picture().map(|p| match p {
                SourcePicture::Moving(m) => m.quad[0],
                SourcePicture::Flat(_) => Pos2::ZERO,
            }),
            Some(pos2(3.0, 0.0))
        );
        release(&mut ws, &ctx);
        // A tap (the key already up) composites at 4× at once, as before.
        ws.edit(None, nudge(0, 1.0), false).unwrap();
        ws.prepare(&ctx, false);
        assert_eq!(ws.composites(), composites(3, 0, 0));
    }

    #[test]
    fn an_undo_during_a_drag_ends_it() {
        let ctx = egui::Context::default();
        let mut ws = workspace(&ctx);
        ws.edit(Some(SLIDER), brightness(0.2), false).unwrap();
        ws.prepare(&ctx, false);
        assert!(ws.dragging());
        assert!(ws.doc.undo());
        assert!(!ws.dragging());
        ws.prepare(&ctx, false);
        assert_eq!(ws.composites(), composites(2, 1, 0));
    }
}
