//! An open document with what the views draw from it: the newest solution and the
//! textures of the source composite and of the result, rebuilt only when they change.
//! The working view and the result panel share the result texture (UX §4.3: no second
//! render).

use crate::document::{Document, result_rgba};
use convert64_core::model::CellPos;
use convert64_core::solve::Solution;
use egui::{Color32, ColorImage, TextureHandle, TextureOptions};
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
    source: Option<(u64, TextureHandle)>,
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

    /// Rebuilds whichever texture is out of date. Call once per frame before drawing.
    pub fn prepare(&mut self, ctx: &egui::Context) {
        let rev = self.doc.revision();
        if self.source.as_ref().is_none_or(|(r, _)| *r != rev) {
            let (w, h, rgba) = self.doc.source_rgba();
            let image = ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba);
            // A photograph: smooth when scaled.
            let options = TextureOptions {
                mipmap_mode: Some(egui::TextureFilter::Linear),
                ..TextureOptions::LINEAR
            };
            let tex = ctx.load_texture("source", image, options);
            self.source = Some((rev, tex));
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

    /// The source composite's texture.
    pub fn source_texture(&self) -> Option<&TextureHandle> {
        self.source.as_ref().map(|(_, t)| t)
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
