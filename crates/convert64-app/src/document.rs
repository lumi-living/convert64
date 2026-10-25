//! The open document: a [`Session`] (the project, its history, its own compositor for the
//! Source view) and what the app derives from it for the scheduler and the views.

use crate::scheduler::Snapshot;
use convert64_core::mode::Mode;
use convert64_core::solve::{Solution, SolveMode, SolveOptions};
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

/// The file name the bundled sample picture opens as (the project is named `title`).
pub const SAMPLE_NAME: &str = "title.png";

/// The sample picture the empty state offers (UX §5, #292): the `title` fixture's
/// painting at 640 × 398 (`assets/sample/README.md`), small enough for the browser build.
pub const SAMPLE: &[u8] = include_bytes!("../assets/sample/title.png");

/// `name` without its extension: what a picture's layer is called (`source.png` →
/// `source`).
fn file_stem(name: &str) -> String {
    Path::new(name)
        .file_stem()
        .map_or_else(|| name.to_owned(), |s| s.to_string_lossy().into_owned())
}

/// Whether `name` is a project file rather than a picture.
pub fn is_project(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".convert64") || lower.ends_with(".json")
}

/// The project a picture opened in the app starts as: `convert64 convert picture.png`'s
/// (multicolor, the default recipe and palette), but with the layer at a uniform scale,
/// where the CLI stretches it to 320 × 200 as png2c64 does: 1:1 when it fits inside the
/// screen (#306), Fill otherwise (UX §6 item 1, usability review M6, #289). A 16:10
/// picture of at least 320 × 200 gets the same transform either way. A project file
/// stores its transforms, so opening one changes nothing.
fn picture_file(name: &str, path: &Path, size: (u32, u32)) -> ProjectFile {
    let mut file = ProjectFile::from_png(Mode::MulticolorBitmap, name, path, size);
    for layer in &mut file.layers {
        layer.transform = crate::presets::opened(size, file.target);
    }
    file
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
                let file = picture_file(&stem, path, source.size());
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

    /// Records that `revision` was saved: the browser's save finishes after the user
    /// picks a place, perhaps after more edits, which stay unsaved.
    pub fn mark_saved_at(&mut self, revision: u64) {
        self.saved = revision;
    }

    /// An autosaved session restored (#241): unsaved work until the next Save, which
    /// natively writes to `path`, the file it came from.
    pub fn restored(name: &str, session: Session, path: Option<PathBuf>) -> Self {
        Self {
            revision: 1,
            ..Self::new(name, session, path)
        }
    }

    /// Whether a drag's command is the last one run (the drag may still be going).
    pub fn in_gesture(&self) -> bool {
        self.gesture.is_some()
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
        let name = self.free_name(&file_stem(name));
        self.run(JsonCommand::LayerAdd {
            name,
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

    /// A new project of one PNG in multicolor with the default recipe and palette, the
    /// picture at 1:1 if it fits inside the screen and covering it otherwise
    /// ([`picture_file`]).
    pub fn from_png(name: &str, png: &[u8]) -> Result<Self, ProjectError> {
        let source = convert64_project::codec::decode_source(png, name)?;
        let store = Arc::new(MemoryStore::new());
        store.insert(name, png.to_vec());
        // The layer is named by the file stem, as natively (usability review m7).
        let mut file = picture_file(name, Path::new(name), source.size());
        for layer in &mut file.layers {
            layer.name = file_stem(name);
        }
        let project = format!("{name}.convert64");
        store.insert(&project, file.to_json());
        let session = Session::open_in(Box::new(store), Path::new(&project))?;
        Ok(Self::new(name, session, None))
    }

    /// `stem`, or if a layer already has that name the first free `stem 2`, `stem 3`…
    /// (UX §5, usability review m7: two `source.png` became "source" and "source.png").
    pub fn free_name(&self, stem: &str) -> String {
        let layers = &self.session.project().layers;
        let taken = |n: &str| layers.iter().any(|l| l.name() == n);
        if !taken(stem) {
            return stem.to_owned();
        }
        (2..)
            .map(|k| format!("{stem} {k}"))
            .find(|n| !taken(n))
            .expect("a free number")
    }

    /// The session, for reading.
    pub fn session(&self) -> &Session {
        &self.session
    }

    /// What opening the file changed, for the status bar to say once after the open:
    /// an older project's migration (#236), e.g. "Updated project from v2: now uses 1×
    /// resampling". `None` for a current project or a picture.
    pub fn open_message(&self) -> Option<String> {
        let notes = self.session.notes();
        (!notes.is_empty()).then(|| notes.join(" · "))
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

    /// The composite on the target grid (the solver's input), up to date: the method
    /// picker crops its Picture thumbnails from it.
    pub fn target_frame(&mut self) -> &convert64_core::TargetFrame {
        self.session.target_frame()
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
            options: SolveOptions {
                // Refine is not in the app's full solve (ADR-014 §5): it is experimental,
                // png2amiga-only and costs about 0.6 s; the CLI and MCP run it.
                refine: false,
                ..p.solve.options(SolveMode::Full, Vec::new())
            },
            previous,
        }
    }

    /// The composite on the super grid as RGBA8, for the Source view.
    ///
    /// This is the first composite after a source is loaded, so it is where a large
    /// source's mip levels are built (#235): their time is logged (the browser console
    /// in wasm), so it can be measured where it matters.
    pub fn source_rgba(&mut self) -> (u32, u32, Vec<u8>) {
        use crate::scheduler::clock;
        self.session.set_clock(Some(clock::now_ms));
        let before = self.session.mip_stats();
        let out = self.session.canvas_rgba();
        let after = self.session.mip_stats();
        if after.0 > before.0 {
            clock::log(&format!(
                "convert64: built {} mip level(s) in {:.1} ms (#235)",
                after.0 - before.0,
                after.1 - before.1
            ));
        }
        out
    }

    /// The composite the solve reads (1× by default) as RGBA8, for the Source view while
    /// a slider is dragged (#264).
    pub fn solve_canvas_rgba(&mut self) -> (u32, u32, Vec<u8>) {
        self.session.solve_canvas_rgba()
    }
}

/// The window (natively) or tab (browser) title: `title.png* — Convert64`, the
/// document's name with Photoshop's asterisk while it has changes since the last Save
/// (UX §9), or just `Convert64` with nothing open.
pub fn window_title(doc: Option<&Document>) -> String {
    match doc {
        Some(d) => format!(
            "{}{} — Convert64",
            d.name,
            if d.is_dirty() { "*" } else { "" }
        ),
        None => "Convert64".to_owned(),
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
    /// Usability review m7: a picture's layer is named by its file stem whether it came
    /// with a path or as bytes, and a name already in the stack gets the next number.
    #[test]
    fn layers_are_named_by_file_stem_with_a_number_on_a_clash() {
        let rgba = (0..16 * 10).flat_map(|i| [i as u8, 90, 200, 255]).collect();
        let png = convert64_project::codec::encode_rgba8(16, 10, rgba);
        let mut doc = Document::open("source.png", &png, None).unwrap();
        doc.add_layer("source.png", &png, None).unwrap();
        doc.add_layer("source.png", &png, None).unwrap();
        doc.add_layer("sky.png", &png, None).unwrap();
        let names: Vec<_> = doc
            .session()
            .project()
            .layers
            .iter()
            .map(|l| l.name().to_owned())
            .collect();
        assert_eq!(names, ["source", "source 2", "source 3", "sky"]);
        assert_eq!(doc.free_name("sky"), "sky 2");
        assert_eq!(doc.free_name("ground"), "ground");
    }

    use super::*;
    use convert64_project::codec::encode_rgba8;

    fn png() -> Vec<u8> {
        let rgba = (0..64 * 40)
            .flat_map(|i| [(i % 64 * 4) as u8, (i / 64 * 6) as u8, 128, 255])
            .collect();
        encode_rgba8(64, 40, rgba)
    }

    /// A `w` × `h` grey ramp.
    fn png_sized(w: u32, h: u32) -> Vec<u8> {
        let rgba = (0..w * h)
            .flat_map(|i| [(i % w * 255 / w) as u8, 100, 100, 255])
            .collect();
        encode_rgba8(w, h, rgba)
    }

    /// #292: the bundled sample is the `title` painting at 640 × 398, and opens as a
    /// picture named `title` would, with no file behind it.
    #[test]
    fn the_sample_picture_opens_as_title() {
        let doc = Document::open(SAMPLE_NAME, SAMPLE, None).unwrap();
        assert_eq!(doc.stem(), "title");
        assert!(doc.path().is_none(), "Save asks where to save it");
        let layers = &doc.session().project().layers;
        assert_eq!(layers.len(), 1);
        assert_eq!(layers[0].source().size(), (640, 398));
        // Small enough for the browser build (the fixture itself is 954 kB).
        assert!(SAMPLE.len() < 400_000, "{} bytes", SAMPLE.len());
    }

    /// The documents a picture of `bytes` opens as: in the browser (bytes only) and
    /// natively (a path).
    fn opened_docs(w: u32, h: u32, bytes: &[u8]) -> Vec<Document> {
        let mut docs = vec![Document::from_png("pic.png", bytes).unwrap()];
        #[cfg(not(target_arch = "wasm32"))]
        {
            let dir = std::env::temp_dir().join(format!(
                "convert64-app-opened-{}-{w}x{h}",
                std::process::id()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join("pic.png");
            std::fs::write(&path, bytes).unwrap();
            docs.push(Document::open("pic.png", bytes, Some(&path)).unwrap());
            let _ = std::fs::remove_dir_all(&dir);
        }
        docs
    }

    /// #289 (usability review M6): a picture larger than the screen opens at a uniform
    /// scale, Fill, in the browser (bytes only) and natively (a path) alike.
    #[test]
    fn an_opened_picture_has_a_uniform_scale() {
        use crate::presets::Preset;
        for (w, h) in [(400, 150), (330, 640), (1024, 637)] {
            for doc in opened_docs(w, h, &png_sized(w, h)) {
                let t = doc.session().project().layers[0].transform();
                assert_eq!(t.scale().x, t.scale().y, "{w}×{h}: {t:?}");
                assert_eq!(
                    Preset::matching(&t, (w, h), doc.mode()),
                    Some(Preset::Fill),
                    "{w}×{h}: {t:?}"
                );
                // The screen is covered.
                let (sw, sh) = crate::presets::screen_size(&t, (w, h));
                assert!(sw >= 320.0 - 1e-3 && sh >= 200.0 - 1e-3, "{sw} × {sh}");
            }
        }
    }

    /// #306: a picture that fits inside the screen opens at 1:1, centred on whole C64
    /// pixels, and the Transform group shows 1:1; it stays on whole pixels in hi-res.
    /// 320 × 200 is 1:1 and Fill at once.
    #[test]
    fn a_small_picture_opens_at_1_to_1() {
        use crate::presets::Preset;
        for ((w, h), at) in [((160, 100), (80.0, 50.0)), ((320, 200), (0.0, 0.0))] {
            for mut doc in opened_docs(w, h, &png_sized(w, h)) {
                for mode in [Mode::MulticolorBitmap, Mode::HiresBitmap] {
                    if doc.mode() != mode {
                        doc.run(JsonCommand::ModeSet(convert64_core::command::ModeSet {
                            mode,
                        }))
                        .unwrap();
                    }
                    let t = doc.session().project().layers[0].transform();
                    assert_eq!(t.scale(), convert64_core::Vec2::new(1.0, 1.0), "{w}×{h}");
                    assert_eq!(t.translate(), convert64_core::Vec2::new(at.0, at.1));
                    assert_eq!(t.translate().x % crate::presets::pixel_width(mode), 0.0);
                    assert_eq!(
                        Preset::matching(&t, (w, h), mode),
                        Some(Preset::OneToOne),
                        "{w}×{h} {mode:?}"
                    );
                }
            }
        }
        let full = Document::from_png("pic.png", &png_sized(320, 200)).unwrap();
        let t = full.session().project().layers[0].transform();
        assert_eq!(
            Preset::Fill.apply(
                &convert64_core::Transform::IDENTITY,
                (320, 200),
                full.mode()
            ),
            Some(t)
        );
    }

    /// #289: a 16:10 picture's Fill is png2c64's stretch to 320 × 200, what the CLI
    /// makes, so it keeps png2c64's front end. Since #306 only one of at least 320 × 200
    /// opens at Fill; a smaller one opens at 1:1.
    #[test]
    fn a_16_10_picture_opens_as_the_cli_makes_it() {
        for (w, h) in [(320, 200), (640, 400)] {
            let doc = Document::from_png("ramp.png", &png_sized(w, h)).unwrap();
            let cli = ProjectFile::from_png(Mode::MulticolorBitmap, "ramp", Path::new("x"), (w, h));
            assert_eq!(
                doc.session().project().layers[0].transform(),
                cli.layers[0].transform,
                "{w}×{h}"
            );
        }
    }

    /// #289: projects store their transforms, so an old one (a version-2 file, #251's
    /// migration) opens with its own, png2c64's stretch included, not with Fill.
    #[test]
    fn a_project_keeps_its_transform() {
        let mut doc = Document::from_png("pic.png", &png_sized(48, 40)).unwrap();
        let stretch = convert64_core::Transform::new(
            convert64_core::Vec2::ZERO,
            convert64_core::Vec2::new(320.0 / 48.0, 200.0 / 40.0),
            false,
            false,
        )
        .unwrap();
        doc.run(JsonCommand::LayerSetTransform {
            layer: convert64_project::LayerRef::Index(0),
            transform: stretch,
            snap: None,
        })
        .unwrap();
        let current = doc.project_json(Path::new(""), Embed::All).unwrap();
        let v2 = current
            .replace(r#""schema_version": 3,"#, r#""schema_version": 2,"#)
            .replace(r#""supersample": 1,"#, r#""supersample": 4,"#);
        for text in [current, v2] {
            let old = Document::open("old.convert64", text.as_bytes(), None).unwrap();
            assert_eq!(old.session().project().layers[0].transform(), stretch);
        }
    }

    #[test]
    fn a_png_opens_as_a_multicolor_project_and_solves() {
        let doc = Document::from_png("ramp.png", &png()).unwrap();
        assert_eq!(doc.mode(), Mode::MulticolorBitmap);
        assert_eq!(doc.session().project().layers.len(), 1);
        let mut session_doc = doc;
        let (w, h, rgba) = session_doc.source_rgba();
        // The Source view's own grid, finer than the 1× solve (ADR-012).
        assert_eq!(session_doc.grid().n(), 1);
        let preview = SuperGrid::new(
            Mode::MulticolorBitmap,
            convert64_project::Session::PREVIEW_N,
        );
        assert_eq!((w, h), preview.unwrap().size());
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

    /// UX §9's asterisk: there after an edit, gone after a save, back after an edit and
    /// there for a restored session from the start.
    #[test]
    fn the_title_marks_unsaved_changes() {
        assert_eq!(window_title(None), "Convert64");
        let mut doc = Document::from_png("ramp.png", &png()).unwrap();
        assert_eq!(window_title(Some(&doc)), "ramp.png — Convert64");
        doc.run(JsonCommand::ModeSet(convert64_core::command::ModeSet {
            mode: Mode::HiresBitmap,
        }))
        .unwrap();
        assert_eq!(window_title(Some(&doc)), "ramp.png* — Convert64");
        let saving = doc.revision();
        doc.mark_saved_at(saving);
        assert_eq!(window_title(Some(&doc)), "ramp.png — Convert64");
        // An edit while the browser's Save dialog was open stays unsaved.
        doc.undo();
        doc.mark_saved_at(saving);
        assert!(window_title(Some(&doc)).contains('*'));
        let session = Document::from_png("ramp.png", &png()).unwrap().session;
        let restored = Document::restored("ramp.png", session, None);
        assert_eq!(window_title(Some(&restored)), "ramp.png* — Convert64");
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
        assert_eq!(back.open_message(), None, "a current project");
    }

    /// #236: a version-2 project saved with the old default 4× opens on the 1× grid,
    /// with the message the status bar shows once; saved, it is version 3 and opens
    /// without one.
    #[test]
    fn a_version_2_project_opens_at_1x_and_says_so() {
        let doc = Document::from_png("ramp.png", &png()).unwrap();
        assert_eq!(doc.open_message(), None, "a picture");
        let current = doc.project_json(Path::new(""), Embed::All).unwrap();
        assert!(current.contains(r#""schema_version": 3,"#), "{current}");
        let v2 = current
            .replace(r#""schema_version": 3,"#, r#""schema_version": 2,"#)
            .replace(r#""supersample": 1,"#, r#""supersample": 4,"#);
        let old = Document::open("old.convert64", v2.as_bytes(), None).unwrap();
        assert_eq!(old.grid().n(), 1);
        assert_eq!(
            old.open_message().as_deref(),
            Some("Updated project from v2: now uses 1× resampling")
        );
        assert!(!old.is_dirty());
        let saved = old.project_json(Path::new(""), Embed::All).unwrap();
        assert_eq!(saved, current);
        let again = Document::open("old.convert64", saved.as_bytes(), None).unwrap();
        assert_eq!(again.open_message(), None);
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

    /// The Region tool's first stroke: a region and its paint, one undo step.
    #[test]
    fn the_first_region_stroke_makes_the_region_in_one_step() {
        use convert64_core::Vec2;
        use convert64_project::RegionRef;
        let mut doc = Document::from_png("ramp.png", &png()).unwrap();
        let add = JsonCommand::RegionAdd {
            name: "Region 1".into(),
            recipe: Box::default(),
            index: None,
        };
        let stroke = JsonCommand::RegionStroke {
            region: RegionRef::Index(0),
            points: vec![Vec2::new(100.0, 100.0), Vec2::new(140.0, 110.0)],
            radius: 6.0,
            hardness: 0.5,
            flow: 1.0,
            mode: convert64_core::mask::PaintMode::Add,
        };
        doc.run_all(vec![add, stroke]).unwrap();
        let regions = &doc.session().project().regions;
        assert_eq!(regions.len(), 1);
        assert!(regions[0].mask().values().iter().any(|v| *v > 0.5));
        assert!(doc.undo());
        assert!(doc.session().project().regions.is_empty());
    }
}
