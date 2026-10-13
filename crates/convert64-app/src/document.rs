//! The open document: a [`Session`] (the project, its history, its own compositor for the
//! Source view) and what the app derives from it for the scheduler and the views.

use crate::scheduler::Snapshot;
use convert64_core::mode::Mode;
use convert64_core::solve::{Solution, SolveMode};
use convert64_core::{Palette, SuperGrid};
use convert64_project::{Embed, JsonCommand, MemoryStore, ProjectError, ProjectFile, Session};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// An open project.
#[derive(Debug)]
pub struct Document {
    /// What the title and the status bar call it: the file name.
    pub name: String,
    session: Session,
    /// Bumped by every edit, so views know when to rebuild what they derive.
    revision: u64,
    /// The revision last saved; the title marks a difference (UX §9).
    saved: u64,
    /// Where the project was opened from or last saved to (native).
    path: Option<PathBuf>,
    /// The gesture (a slider or field drag) the last command belongs to: the next
    /// command of the same gesture replaces it, so a drag is one undo step (UX §11).
    gesture: Option<u64>,
    /// How many editor commands each undo step holds, oldest first (a rectangle of pins
    /// is one step of many commands), and the same for redo.
    undo_steps: Vec<usize>,
    redo_steps: Vec<usize>,
}

/// Whether `name` is a project file rather than a picture.
pub fn is_project(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".convert64") || lower.ends_with(".json")
}

impl Document {
    /// Opens a picture or a project from its bytes. `path` (native) is where the file
    /// is: a project's linked files are then read relative to it, and Save writes back
    /// there. Without it (browser) a project must embed its sources.
    pub fn open(name: &str, bytes: &[u8], path: Option<&Path>) -> Result<Self, ProjectError> {
        if !is_project(name) {
            #[cfg(not(target_arch = "wasm32"))]
            if let Some(path) = path {
                let source = convert64_project::codec::decode_source(bytes, name)?;
                let stem = path
                    .file_stem()
                    .map_or_else(|| name.to_owned(), |s| s.to_string_lossy().into_owned());
                let file =
                    ProjectFile::from_png(Mode::MulticolorBitmap, &stem, path, source.size());
                let session = Session::from_project(file, Path::new(""))?;
                return Ok(Self::new(name, session, None));
            }
            return Self::from_png(name, bytes);
        }
        let text = std::str::from_utf8(bytes).map_err(|e| ProjectError::Json(e.to_string()))?;
        let file = ProjectFile::parse(text)?;
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(path) = path {
            let base = path.parent().map(Path::to_owned).unwrap_or_default();
            let session = Session::from_project(file, &base)?;
            return Ok(Self::new(name, session, Some(path.to_owned())));
        }
        let _ = path;
        drop(file);
        let store = Arc::new(MemoryStore::new());
        store.insert(name, bytes.to_vec());
        let session = Session::open_in(Box::new(store), Path::new(name))?;
        Ok(Self::new(name, session, None))
    }

    fn new(name: &str, session: Session, path: Option<PathBuf>) -> Self {
        Self {
            name: name.to_owned(),
            session,
            revision: 0,
            saved: 0,
            path,
            gesture: None,
            undo_steps: Vec::new(),
            redo_steps: Vec::new(),
        }
    }

    /// The name without its extension, for saved and exported files.
    pub fn stem(&self) -> String {
        Path::new(&self.name)
            .file_stem()
            .map_or_else(|| self.name.clone(), |s| s.to_string_lossy().into_owned())
    }

    /// Where the project lives (native), once opened from or saved to a file.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Whether there are edits since the last save.
    pub fn is_dirty(&self) -> bool {
        self.revision != self.saved
    }

    /// The project file as JSON, with links relative to `dir`, or every source embedded
    /// (the browser's downloads stand alone).
    pub fn project_json(&self, dir: &Path, embed: Embed) -> Result<String, ProjectError> {
        Ok(self.session.to_project_file(dir, embed)?.to_json())
    }

    /// Saves the project to `path`, links relative to it (native).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn save_to(&mut self, path: &Path) -> Result<(), ProjectError> {
        self.session.save(path, Embed::Keep)?;
        self.path = Some(path.to_owned());
        if let Some(name) = path.file_name() {
            self.name = name.to_string_lossy().into_owned();
        }
        self.mark_saved();
        Ok(())
    }

    /// Records that the current revision is saved (or downloaded).
    pub fn mark_saved(&mut self) {
        self.saved = self.revision;
    }

    /// Layer `layer`'s evaluated mask on the super grid (sources and modifiers applied),
    /// 16-bit, row-major; `None` if it has no mask or is hidden.
    pub fn evaluated_mask(&mut self, layer: usize) -> Option<(u32, u32, Vec<u16>)> {
        self.session.evaluated_mask(layer).ok()
    }

    /// Loads a greyscale PNG as `layer`'s mask raster (`mask_load`): linked by `path`
    /// natively, from the session's store in the browser.
    pub fn load_mask(
        &mut self,
        layer: usize,
        name: &str,
        bytes: &[u8],
        path: Option<&Path>,
    ) -> Result<(), ProjectError> {
        let path = match path {
            Some(p) => p.to_owned(),
            None => {
                self.session.store().write(Path::new(name), bytes)?;
                PathBuf::from(name)
            }
        };
        self.run(JsonCommand::MaskLoad {
            layer: convert64_project::LayerRef::Index(layer),
            path,
        })
    }

    /// Sets the palette from a VICE `.vpl` file (`palette_set`): linked by `path`
    /// natively, from the session's store in the browser.
    pub fn load_palette(
        &mut self,
        name: &str,
        bytes: &[u8],
        path: Option<&Path>,
    ) -> Result<(), ProjectError> {
        let path = match path {
            Some(p) => p.to_owned(),
            None => {
                self.session.store().write(Path::new(name), bytes)?;
                PathBuf::from(name)
            }
        };
        self.run(JsonCommand::PaletteSet {
            palette: convert64_project::PaletteSpec::Vpl(path),
        })
    }

    /// Adds a picture as a new layer on top (a file dropped on an open project): its
    /// bytes go into the session's store (browser) or it is linked by `path` (native).
    pub fn add_layer(
        &mut self,
        name: &str,
        bytes: &[u8],
        path: Option<&Path>,
    ) -> Result<(), ProjectError> {
        let source = convert64_project::codec::decode_source(bytes, name)?;
        let (w, h) = source.size();
        let spec = match path {
            Some(p) => convert64_project::file::SourceSpec::Path(p.to_owned()),
            None => {
                // A revision prefix, so a second `source.png` never replaces the first in
                // the browser's store.
                let stored = PathBuf::from(format!("layers/{}-{name}", self.revision));
                self.session.store().write(&stored, bytes)?;
                convert64_project::file::SourceSpec::Path(stored)
            }
        };
        // Fit inside the screen, never enlarged; top left.
        let s = (320.0 / w as f32).min(200.0 / h as f32).min(1.0);
        let transform = convert64_core::Transform::new(
            convert64_core::Vec2::ZERO,
            convert64_core::Vec2::new(s, s),
            false,
            false,
        )
        .map_err(|e| ProjectError::Invalid(e.to_string()))?;
        let stem = Path::new(name)
            .file_stem()
            .map_or_else(|| name.to_owned(), |s| s.to_string_lossy().into_owned());
        self.run(JsonCommand::LayerAdd {
            name: stem,
            source: spec,
            index: None,
            visible: true,
            transform,
            opacity: 1.0,
            blend: convert64_core::Blend::default(),
            recipe: Box::default(),
            adjustments: Box::default(),
        })
    }

    /// A new project of one PNG, scaled to fill the screen, in multicolor with the
    /// default recipe and palette (what `convert64 convert picture.png` makes).
    pub fn from_png(name: &str, png: &[u8]) -> Result<Self, ProjectError> {
        let source = convert64_project::codec::decode_source(png, name)?;
        let store = Arc::new(MemoryStore::new());
        store.insert(name, png.to_vec());
        let file =
            ProjectFile::from_png(Mode::MulticolorBitmap, name, Path::new(name), source.size());
        let project = format!("{name}.convert64");
        store.insert(&project, file.to_json());
        let session = Session::open_in(Box::new(store), Path::new(&project))?;
        Ok(Self::new(name, session, None))
    }

    /// The session, for reading.
    pub fn session(&self) -> &Session {
        &self.session
    }

    /// Runs a registry command (ADR-005): the app edits only through these.
    pub fn run(&mut self, command: JsonCommand) -> Result<(), ProjectError> {
        self.run_all(vec![command])
    }

    /// Runs several commands as one undo step (a rectangle of pins, each cell its own
    /// colours). If one fails, the ones before it are undone and nothing changes.
    pub fn run_all(&mut self, commands: Vec<JsonCommand>) -> Result<(), ProjectError> {
        self.gesture = None;
        let mut done = 0;
        for command in commands {
            if let Err(e) = self.session.run(command) {
                for _ in 0..done {
                    self.session.editor_mut().undo();
                }
                return Err(e);
            }
            done += 1;
        }
        if done > 0 {
            self.undo_steps.push(done);
            self.redo_steps.clear();
            self.revision += 1;
        }
        Ok(())
    }

    /// Runs a command that is part of the drag `gesture` (any id unique to the control):
    /// it replaces the gesture's previous command in the history, so the whole drag
    /// undoes in one step. [`Self::end_gesture`] when the drag ends.
    pub fn run_in_gesture(
        &mut self,
        gesture: u64,
        command: JsonCommand,
    ) -> Result<(), ProjectError> {
        if self.gesture == Some(gesture) {
            self.session.editor_mut().undo();
            self.undo_steps.pop();
        }
        self.session.run(command)?;
        self.undo_steps.push(1);
        self.redo_steps.clear();
        self.gesture = Some(gesture);
        self.revision += 1;
        Ok(())
    }

    /// Reverts the drag in progress (its one step), if any.
    pub fn cancel_gesture(&mut self) -> bool {
        if self.gesture.take().is_some() && self.undo_steps.pop().is_some() {
            self.session.editor_mut().undo();
            self.revision += 1;
            return true;
        }
        false
    }

    /// Ends the current drag, so the next command starts a new undo step.
    pub fn end_gesture(&mut self) {
        self.gesture = None;
    }

    /// Undoes the last step (`Cmd Z`); false if there is nothing to undo.
    pub fn undo(&mut self) -> bool {
        self.gesture = None;
        let Some(n) = self.undo_steps.pop() else {
            return false;
        };
        for _ in 0..n {
            self.session.editor_mut().undo();
        }
        self.redo_steps.push(n);
        self.revision += 1;
        true
    }

    /// Redoes the last undone step (`Shift Cmd Z`, `Cmd Y`).
    pub fn redo(&mut self) -> Result<bool, ProjectError> {
        self.gesture = None;
        let Some(n) = self.redo_steps.pop() else {
            return Ok(false);
        };
        for _ in 0..n {
            if let Some(Err(e)) = self.session.editor_mut().redo() {
                return Err(ProjectError::Invalid(e.to_string()));
            }
        }
        self.undo_steps.push(n);
        self.revision += 1;
        Ok(true)
    }

    /// Whether there is something to undo and to redo.
    pub fn history(&self) -> (bool, bool) {
        (!self.undo_steps.is_empty(), !self.redo_steps.is_empty())
    }

    /// Changes with every edit.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// The target mode.
    pub fn mode(&self) -> Mode {
        self.session.target()
    }

    /// The super grid: target mode and supersample factor.
    pub fn grid(&self) -> SuperGrid {
        self.session.grid()
    }

    /// The palette solved with and previewed in (TAD §3.4: the same object).
    pub fn palette(&self) -> &Palette {
        self.session.palette()
    }

    /// Everything a solve reads, for the scheduler.
    pub fn snapshot(&self, previous: Option<Arc<Solution>>) -> Snapshot {
        let p = self.session.project();
        Snapshot {
            layers: p.layers.clone(),
            regions: p.regions.clone(),
            steering: p.steering.clone(),
            grid: self.grid(),
            palette: p.palette.clone(),
            options: p.solve.options(SolveMode::Full, Vec::new()),
            previous,
        }
    }

    /// The composite on the super grid as RGBA8, for the Source view.
    pub fn source_rgba(&mut self) -> (u32, u32, Vec<u8>) {
        self.session.canvas_rgba()
    }
}

/// `solution` as RGBA8 on the target grid (160 × 200 multicolor, 320 × 200 hi-res), every
/// pixel a colour of `palette`. Drawn with nearest filtering at 2:1 for multicolor.
pub fn result_rgba(solution: &Solution, palette: &Palette, grid: SuperGrid) -> (u32, u32, Vec<u8>) {
    let (w, h) = grid.target_size();
    let mut out = Vec::with_capacity(w as usize * h as usize * 4);
    for y in 0..h {
        for x in 0..w {
            let c = palette.rgb(solution.image.pixel(x as u16, y as u16));
            out.extend([c.r, c.g, c.b, 255]);
        }
    }
    (w, h, out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use convert64_project::codec::encode_rgba8;

    fn png() -> Vec<u8> {
        let rgba = (0..64 * 40)
            .flat_map(|i| [(i % 64 * 4) as u8, (i / 64 * 6) as u8, 128, 255])
            .collect();
        encode_rgba8(64, 40, rgba)
    }

    #[test]
    fn a_png_opens_as_a_multicolor_project_and_solves() {
        let doc = Document::from_png("ramp.png", &png()).unwrap();
        assert_eq!(doc.mode(), Mode::MulticolorBitmap);
        assert_eq!(doc.session().project().layers.len(), 1);
        let mut session_doc = doc;
        let (w, h, rgba) = session_doc.source_rgba();
        let (sw, sh) = session_doc.grid().size();
        assert_eq!((w, h), (sw, sh));
        assert_eq!(rgba.len(), (w * h * 4) as usize);
        // Solve the snapshot the scheduler would get, synchronously.
        let mut scheduler = crate::scheduler::Scheduler::stepping(Default::default());
        scheduler.submit(session_doc.snapshot(None), SolveMode::Full);
        let result = loop {
            scheduler.tick();
            if let Some(r) = scheduler.poll().filter(|r| !r.provisional) {
                break r;
            }
        };
        let (rw, rh, px) = result_rgba(&result.solution, session_doc.palette(), session_doc.grid());
        assert_eq!((rw, rh), (160, 200));
        assert!(px.chunks(4).all(|p| p[3] == 255));
    }

    #[test]
    fn edits_bump_the_revision() {
        let mut doc = Document::from_png("ramp.png", &png()).unwrap();
        let before = doc.revision();
        doc.run(JsonCommand::ModeSet(convert64_core::command::ModeSet {
            mode: Mode::HiresBitmap,
        }))
        .unwrap();
        assert_eq!(doc.revision(), before + 1);
        assert_eq!(doc.mode(), Mode::HiresBitmap);
    }

    /// The browser's save: every source embedded, reopened from the bytes alone.
    #[test]
    fn a_project_round_trips_through_its_json() {
        let mut doc = Document::from_png("ramp.png", &png()).unwrap();
        doc.run(JsonCommand::ModeSet(convert64_core::command::ModeSet {
            mode: Mode::HiresBitmap,
        }))
        .unwrap();
        assert!(doc.is_dirty());
        let json = doc.project_json(Path::new(""), Embed::All).unwrap();
        doc.mark_saved();
        assert!(!doc.is_dirty());
        let back = Document::open("ramp.convert64", json.as_bytes(), None).unwrap();
        assert_eq!(back.mode(), Mode::HiresBitmap);
        assert_eq!(back.session().project(), doc.session().project());
        assert_eq!(back.stem(), "ramp");
    }

    /// The native save: links relative to the project, reopened from its path.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_project_saves_and_reopens_natively() {
        let dir = std::env::temp_dir().join(format!("convert64-app-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("art")).unwrap();
        let picture = dir.join("art/ramp.png");
        std::fs::write(&picture, png()).unwrap();
        let mut doc = Document::open("ramp.png", &png(), Some(&picture)).unwrap();
        let project = dir.join("ramp.convert64");
        doc.save_to(&project).unwrap();
        assert_eq!(doc.path(), Some(project.as_path()));
        assert_eq!(doc.name, "ramp.convert64");
        let text = std::fs::read_to_string(&project).unwrap();
        assert!(text.contains("art/ramp.png"), "{text}");
        let bytes = std::fs::read(&project).unwrap();
        let back = Document::open("ramp.convert64", &bytes, Some(&project)).unwrap();
        assert_eq!(back.session().project(), doc.session().project());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_dropped_picture_becomes_a_layer() {
        let mut doc = Document::from_png("ramp.png", &png()).unwrap();
        doc.add_layer("logo.png", &png(), None).unwrap();
        let layers = &doc.session().project().layers;
        assert_eq!(layers.len(), 2);
        assert_eq!(doc.revision(), 1);
    }

    #[test]
    fn a_second_picture_with_the_same_name_is_its_own_layer() {
        let mut doc = Document::from_png("source.png", &png()).unwrap();
        let other = convert64_project::codec::encode_rgba8(8, 4, vec![200; 8 * 4 * 4]);
        doc.add_layer("source.png", &other, None).unwrap();
        let layers = &doc.session().project().layers;
        assert_eq!(layers[0].source().size(), (64, 40));
        assert_eq!(layers[1].source().size(), (8, 4));
    }

    #[test]
    fn a_drag_is_one_undo_step() {
        let mut doc = Document::from_png("ramp.png", &png()).unwrap();
        let layer = convert64_project::LayerRef::Index(0);
        for opacity in [0.9, 0.7, 0.5] {
            let cmd = JsonCommand::LayerSetOpacity {
                layer: layer.clone(),
                opacity,
            };
            doc.run_in_gesture(7, cmd).unwrap();
        }
        doc.end_gesture();
        assert_eq!(doc.session().project().layers[0].opacity(), 0.5);
        assert!(doc.undo());
        assert_eq!(doc.session().project().layers[0].opacity(), 1.0);
        assert!(!doc.undo());
        assert!(doc.redo().unwrap());
        assert_eq!(doc.session().project().layers[0].opacity(), 0.5);
    }

    #[test]
    fn a_group_of_commands_is_one_undo_step() {
        use convert64_core::command::CellLock;
        use convert64_core::model::CellPos;
        let mut doc = Document::from_png("ramp.png", &png()).unwrap();
        let lock = |x| {
            JsonCommand::CellLock(CellLock {
                cells: vec![CellPos { x, y: 0 }],
                locked: true,
            })
        };
        doc.run_all(vec![lock(0), lock(1), lock(2)]).unwrap();
        assert_eq!(doc.session().project().steering.locks().iter().count(), 3);
        assert!(doc.undo());
        assert_eq!(doc.session().project().steering.locks().iter().count(), 0);
        assert!(doc.redo().unwrap());
        assert_eq!(doc.session().project().steering.locks().iter().count(), 3);
        assert_eq!(doc.history(), (true, false));
    }
}
