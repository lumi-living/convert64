//! eframe/egui user interface; native via `cargo run -p convert64-app`, web via `trunk build` (`docs/TAD.md §4`).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window on Windows in release

use convert64_app::document::Document;
use convert64_app::files::{FileEvent, Files};
use convert64_app::shell::{Action, Shell};
use convert64_app::theme;
use convert64_app::workspace::Workspace;
use convert64_core::solve::SolveMode;

/// The storage key of the View › Theme choice (eframe's app storage: a file natively,
/// `localStorage` in the browser).
const THEME_KEY: &str = "theme";

/// Top-level UI state; every frame is drawn from the held document.
#[derive(Debug, Default)]
struct App {
    /// The open document and its preview, if any.
    workspace: Option<Workspace>,
    /// The chrome: working-view mode, tool, dock tab, what is hidden (#203).
    shell: Shell,
    /// Solves off the frame: a worker thread natively, a stage per frame in wasm (#194).
    scheduler: convert64_app::scheduler::Scheduler,
    /// Open, save and export dialogs, downloads and dropped files (#205).
    files: Files,
    /// The window title last set, so it is sent only when it changes.
    title: String,
    /// `--screenshot`: save the window to a PNG after a few frames, then quit (native).
    #[cfg(not(target_arch = "wasm32"))]
    screenshot: Option<screenshot::Request>,
}

impl App {
    /// The app with the generated look installed, the theme restored from `cc`'s storage
    /// and the scheduler asking for a repaint when a result is ready.
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::install(&cc.egui_ctx);
        let mut app = Self::default();
        let stored = cc
            .storage
            .and_then(|s| s.get_string(THEME_KEY))
            .and_then(|s| theme_from_str(&s));
        if let Some(preference) = stored {
            app.set_theme(&cc.egui_ctx, preference);
        }
        let ctx = cc.egui_ctx.clone();
        app.scheduler
            .set_waker(std::sync::Arc::new(move || ctx.request_repaint()));
        app
    }

    fn set_theme(&mut self, ctx: &egui::Context, preference: egui::ThemePreference) {
        self.shell.theme = preference;
        theme::set_theme(ctx, preference);
    }

    /// Opens `doc`: the working view fits it and a full solve starts.
    fn open(&mut self, doc: Document) {
        self.scheduler.cancel();
        let ws = Workspace::new(doc);
        self.scheduler
            .submit(ws.doc.snapshot(None), SolveMode::Full);
        self.workspace = Some(ws);
        self.shell.fit_on_next_frame();
        self.shell.last_export = None;
    }

    /// Re-solves after an edit: the whole picture, since #205's edits change settings or
    /// add layers (the interactive path arrives with dragging, #206).
    fn resolve(&mut self) {
        if let Some(ws) = &mut self.workspace {
            ws.solving = true;
            let previous = ws.solution.clone();
            self.scheduler
                .submit(ws.doc.snapshot(previous), SolveMode::Full);
        }
    }

    /// A file arrived (picked, dropped or named on the command line): a picture dropped
    /// on an open project becomes a new layer; anything else opens.
    fn receive(
        &mut self,
        ctx: &egui::Context,
        name: &str,
        bytes: &[u8],
        path: Option<&std::path::Path>,
        dropped: bool,
    ) {
        if dropped
            && !convert64_app::document::is_project(name)
            && let Some(ws) = &mut self.workspace
        {
            match ws.doc.add_layer(name, bytes, path) {
                Ok(()) => {
                    self.shell.layer = ws.doc.session().project().layers.len() - 1;
                    self.shell
                        .toast(ctx, format!("Added {name} as a layer"), false);
                    self.resolve();
                }
                Err(e) => self
                    .shell
                    .toast(ctx, format!("Can't add {name}: {e}"), true),
            }
            return;
        }
        match Document::open(name, bytes, path) {
            Ok(doc) => self.open(doc),
            Err(e) => self.shell.toast(
                ctx,
                format!("Can't open {name}: {e}. Open a PNG or a .convert64 project."),
                true,
            ),
        }
    }

    /// Saves the project: natively to its file (or a new one), in the browser as a
    /// download with every source embedded.
    fn save(&mut self, ctx: &egui::Context, ask: bool) {
        let Some(ws) = &mut self.workspace else {
            return;
        };
        let file_name = format!("{}.convert64", ws.doc.stem());
        #[cfg(not(target_arch = "wasm32"))]
        {
            let path = match ws.doc.path() {
                Some(p) if !ask => Some(p.to_owned()),
                _ => self
                    .files
                    .pick_save_path(&file_name, ("Convert64 project", &["convert64"])),
            };
            let Some(path) = path else {
                return;
            };
            match ws.doc.save_to(&path) {
                Ok(()) => self
                    .shell
                    .toast(ctx, format!("Saved {}", path.display()), false),
                Err(e) => self.shell.toast(ctx, format!("Can't save: {e}"), true),
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = ask;
            match ws
                .doc
                .project_json(std::path::Path::new(""), convert64_project::Embed::All)
            {
                Ok(json) => {
                    ws.doc.mark_saved();
                    self.files
                        .save_files(ctx, vec![(file_name, json.into_bytes())], "Saved");
                }
                Err(e) => self.shell.toast(ctx, format!("Can't save: {e}"), true),
            }
        }
    }

    /// Exports the result on screen.
    fn export(
        &mut self,
        ctx: &egui::Context,
        settings: convert64_app::export::Settings,
        name: String,
    ) {
        let Some(ws) = &self.workspace else {
            return;
        };
        let Some(solution) = ws.solution.as_ref().filter(|_| !ws.solving) else {
            self.shell.toast(
                ctx,
                "Can't export yet: the full solve is still running.",
                true,
            );
            return;
        };
        match convert64_app::export::files(
            solution,
            ws.doc.palette(),
            ws.doc.mode(),
            &name,
            &settings,
        ) {
            Ok(files) => {
                self.shell.last_export = Some((settings, name));
                self.files.save_files(ctx, files, "Exported");
            }
            Err(e) => self.shell.toast(ctx, e, true),
        }
    }

    /// Runs a registry command on the document (as part of drag `gesture`, if any) and
    /// re-solves: the fast path during a drag, in full otherwise. Keeps the layer
    /// selection on the layer an edit made or moved.
    fn edit(
        &mut self,
        ctx: &egui::Context,
        gesture: Option<u64>,
        command: convert64_project::JsonCommand,
    ) {
        use convert64_project::{JsonCommand, LayerRef};
        let Some(ws) = &mut self.workspace else {
            return;
        };
        let select = match &command {
            JsonCommand::LayerDuplicate {
                layer: LayerRef::Index(i),
            } => Some(i + 1),
            JsonCommand::LayerRemove {
                layer: LayerRef::Index(i),
            } => Some(i.saturating_sub(1)),
            _ => None,
        };
        let done = match gesture {
            Some(id) => ws.doc.run_in_gesture(id, command),
            None => ws.doc.run(command),
        };
        match done {
            Ok(()) => {
                if let Some(i) = select {
                    self.shell.layer = i;
                }
                if gesture.is_some() {
                    ws.solving = true;
                    let previous = ws.solution.clone();
                    let dirty = convert64_core::CellSet::EMPTY;
                    self.scheduler
                        .submit(ws.doc.snapshot(previous), SolveMode::Interactive { dirty });
                } else {
                    self.resolve();
                }
            }
            Err(e) => self.shell.toast(ctx, format!("{e}"), true),
        }
    }

    /// Carries out what the shell asked for.
    fn act(&mut self, ctx: &egui::Context, action: Action) {
        match action {
            Action::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Action::SetTheme(preference) => self.set_theme(ctx, preference),
            Action::SetMode(mode) => {
                let Some(ws) = &mut self.workspace else {
                    return;
                };
                let command =
                    convert64_project::JsonCommand::ModeSet(convert64_core::command::ModeSet {
                        mode,
                    });
                match ws.doc.run(command) {
                    Ok(()) => {
                        ws.solution = None;
                        self.resolve();
                    }
                    Err(e) => self.shell.toast(ctx, e.to_string(), true),
                }
            }
            Action::Open => self.files.pick_open(ctx, false),
            Action::NewLayer if self.workspace.is_some() => self.files.pick_open(ctx, true),
            Action::NewLayer => self.files.pick_open(ctx, false),
            Action::Run(command) => self.edit(ctx, None, command),
            Action::Gesture(id, command) => self.edit(ctx, Some(id), command),
            Action::EndGesture => {
                if let Some(ws) = &mut self.workspace {
                    ws.doc.end_gesture();
                }
                // The drag's last frame got the fast path: finish with a full solve.
                self.resolve();
            }
            Action::ToggleVisible(i) => {
                let visible = self
                    .workspace
                    .as_ref()
                    .and_then(|ws| ws.doc.session().project().layers.get(i))
                    .map(|l| !l.visible());
                if let Some(visible) = visible {
                    let layer = convert64_project::LayerRef::Index(i);
                    self.edit(
                        ctx,
                        None,
                        convert64_project::JsonCommand::LayerSetVisible { layer, visible },
                    );
                }
            }
            Action::Undo | Action::Redo => {
                let Some(ws) = &mut self.workspace else {
                    return;
                };
                let done = if action == Action::Undo {
                    Ok(ws.doc.undo())
                } else {
                    ws.doc.redo()
                };
                match done {
                    Ok(true) => self.resolve(),
                    Ok(false) => {}
                    Err(e) => self.shell.toast(ctx, format!("Can't redo: {e}"), true),
                }
            }
            Action::Save => self.save(ctx, false),
            Action::SaveAs => self.save(ctx, true),
            Action::Export(settings, name) => self.export(ctx, settings, name),
        }
    }

    /// Handles finished file operations.
    fn poll_files(&mut self, ctx: &egui::Context) {
        self.files.take_dropped(ctx);
        for event in self.files.poll() {
            match event {
                // Dialog opens replace the project; a picture dropped onto one adds a layer.
                FileEvent::Opened {
                    name,
                    bytes,
                    path,
                    dropped,
                } => self.receive(ctx, &name, &bytes, path.as_deref(), dropped),
                FileEvent::Saved(files, verb) => {
                    let list: Vec<String> = files
                        .iter()
                        .map(|(n, b)| format!("{n} ({} B)", group(*b)))
                        .collect();
                    self.shell
                        .toast(ctx, format!("{verb} {}", list.join(", ")), false);
                }
                FileEvent::Failed(e) => self.shell.toast(ctx, e, true),
            }
        }
    }

    /// The window title: the document's name, `*` while it has unsaved edits (UX §9).
    fn update_title(&mut self, ctx: &egui::Context) {
        let title = match &self.workspace {
            Some(ws) => format!(
                "{}{} — Convert64",
                ws.doc.name,
                if ws.doc.is_dirty() { "*" } else { "" }
            ),
            None => "Convert64".to_owned(),
        };
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }
}

/// `10003` as `10 003` (UX §9 sample strings).
fn group(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push('\u{2009}');
        }
        out.push(c);
    }
    out
}

/// `View › Theme`'s stored form.
fn theme_to_str(preference: egui::ThemePreference) -> &'static str {
    match preference {
        egui::ThemePreference::System => "system",
        egui::ThemePreference::Light => "light",
        egui::ThemePreference::Dark => "dark",
    }
}

fn theme_from_str(s: &str) -> Option<egui::ThemePreference> {
    match s {
        "system" => Some(egui::ThemePreference::System),
        "light" => Some(egui::ThemePreference::Light),
        "dark" => Some(egui::ThemePreference::Dark),
        _ => None,
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // This frame's share of the solve work (wasm); never blocks on a solve.
        if self.scheduler.tick() {
            ui.ctx().request_repaint();
        }
        if let Some(result) = self.scheduler.poll()
            && let Some(ws) = &mut self.workspace
        {
            ws.solve_ms = Some(result.timing.composite_ms + result.timing.solve_ms);
            ws.solving = result.provisional;
            ws.solution = Some(result.solution);
        }

        self.poll_files(ui.ctx());
        if let Some(ws) = &mut self.workspace {
            self.shell.history = ws.doc.history();
        }
        for action in self.shell.show(ui, self.workspace.as_mut()) {
            self.act(ui.ctx(), action);
        }
        self.update_title(ui.ctx());

        #[cfg(not(target_arch = "wasm32"))]
        if let Some(request) = &mut self.screenshot {
            // Wait for the first full solve when a picture is open.
            let ready = self.workspace.as_ref().is_none_or(|ws| !ws.solving);
            request.step(ui.ctx(), ready);
        }
    }

    fn persist_egui_memory(&self) -> bool {
        // A screenshot run leaves the user's panel sizes and window state alone.
        #[cfg(not(target_arch = "wasm32"))]
        if self.screenshot.is_some() {
            return false;
        }
        true
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        // A screenshot run leaves the user's stored choice alone.
        #[cfg(not(target_arch = "wasm32"))]
        if self.screenshot.is_some() {
            return;
        }
        storage.set_string(THEME_KEY, theme_to_str(self.shell.theme).to_owned());
    }
}

/// `--screenshot <file.png> [--theme light|dark] [--size <w>x<h>] [--view result]
/// [--overlay grid|clashes|ghost]… [--compare split|flicker|side] [--dialog export]
/// [--tab recipe|layers|export]`: draws a few frames,
/// saves the window as a PNG and quits. The PR screenshots come from it (#203).
#[cfg(not(target_arch = "wasm32"))]
mod screenshot {
    use std::path::PathBuf;

    /// Frames drawn before the screenshot, so fonts and panel sizes have settled.
    const SETTLE_FRAMES: u32 = 5;

    /// What `--screenshot` asked for.
    #[derive(Debug)]
    pub(super) struct Request {
        pub(super) path: PathBuf,
        pub(super) theme: Option<egui::ThemePreference>,
        pub(super) size: [f32; 2],
        /// `--view result`: the working view in Result mode.
        pub(super) result: bool,
        /// `--overlay grid|clashes|ghost` (repeatable).
        pub(super) overlays: Vec<String>,
        /// `--compare split|flicker|side`.
        pub(super) compare: Option<String>,
        /// `--dialog export`: the export dialog open.
        pub(super) dialog: Option<String>,
        /// `--tab recipe|layers|export`: the dock tab.
        pub(super) tab: Option<String>,
        frame: u32,
    }

    /// The command line: a picture to open and an optional screenshot request.
    #[derive(Debug, Default)]
    pub(super) struct Args {
        pub(super) open: Option<PathBuf>,
        /// Further pictures, added as layers on top.
        pub(super) layers: Vec<PathBuf>,
        pub(super) screenshot: Option<Request>,
    }

    impl Request {
        /// The command line: `[picture.png] [--screenshot <file.png> …]`.
        pub(super) fn from_args() -> Result<Args, String> {
            let mut args = std::env::args().skip(1);
            let mut open = None;
            let mut layers = Vec::new();
            let mut request = None;
            let mut theme = None;
            let mut size = [1280.0, 800.0];
            let mut result = false;
            let mut overlays = Vec::new();
            let mut compare = None;
            let mut dialog = None;
            let mut tab = None;
            while let Some(arg) = args.next() {
                let mut value = || args.next().ok_or(format!("{arg} needs a value"));
                match arg.as_str() {
                    "--screenshot" => request = Some(PathBuf::from(value()?)),
                    "--theme" => {
                        theme = Some(
                            super::theme_from_str(&value()?)
                                .ok_or("--theme is system, light or dark")?,
                        );
                    }
                    "--view" => result = value()? == "result",
                    "--overlay" => overlays.push(value()?),
                    "--compare" => compare = Some(value()?),
                    "--dialog" => dialog = Some(value()?),
                    "--tab" => tab = Some(value()?),
                    "--size" => {
                        let v = value()?;
                        let (w, h) = v.split_once('x').ok_or("--size is <w>x<h>")?;
                        size = [
                            w.parse().map_err(|_| "--size: bad width")?,
                            h.parse().map_err(|_| "--size: bad height")?,
                        ];
                    }
                    _ if !arg.starts_with("--") && open.is_none() => {
                        open = Some(PathBuf::from(arg))
                    }
                    _ if !arg.starts_with("--") => layers.push(PathBuf::from(arg)),
                    _ => return Err(format!("unknown argument {arg}")),
                }
            }
            Ok(Args {
                open,
                layers,
                screenshot: request.map(|path| Self {
                    path,
                    theme,
                    size,
                    result,
                    overlays,
                    compare,
                    dialog,
                    tab,
                    frame: 0,
                }),
            })
        }

        /// Called every frame: asks for the screenshot once the frames have settled,
        /// writes it when it arrives and closes the window.
        pub(super) fn step(&mut self, ctx: &egui::Context, ready: bool) {
            if !ready {
                ctx.request_repaint();
                return;
            }
            self.frame += 1;
            if self.frame == SETTLE_FRAMES {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            }
            ctx.request_repaint();
            let image = ctx.input(|i| {
                i.raw.events.iter().find_map(|e| match e {
                    egui::Event::Screenshot { image, .. } => Some(image.clone()),
                    _ => None,
                })
            });
            if let Some(image) = image {
                let [w, h] = image.size;
                let rgba = image.pixels.iter().flat_map(|c| c.to_array()).collect();
                let png = convert64_project::codec::encode_rgba8(w as u32, h as u32, rgba);
                match std::fs::write(&self.path, png) {
                    Ok(()) => println!("wrote {} ({w}×{h})", self.path.display()),
                    Err(e) => eprintln!("{}: {e}", self.path.display()),
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }
}

/// Native entry point: opens a desktop window.
#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    let args = match screenshot::Request::from_args() {
        Ok(args) => args,
        Err(e) => {
            eprintln!("convert64-app: {e}");
            std::process::exit(2);
        }
    };
    let request = args.screenshot;
    let size = request.as_ref().map_or([1280.0, 800.0], |r| r.size);
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size(size)
            .with_min_inner_size([640.0, 400.0]),
        // A screenshot run keeps the window size it asked for.
        persist_window: request.is_none(),
        ..Default::default()
    };
    eframe::run_native(
        "Convert64",
        native_options,
        Box::new(move |cc| {
            let mut app = App::new(cc);
            if let Some(path) = args.open {
                app.files.open_path(path);
                app.poll_files(&cc.egui_ctx);
                for layer in args.layers {
                    let name = layer
                        .file_name()
                        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                    match std::fs::read(&layer) {
                        Ok(bytes) => app.receive(&cc.egui_ctx, &name, &bytes, Some(&layer), true),
                        Err(e) => eprintln!("convert64-app: {}: {e}", layer.display()),
                    }
                }
            }
            if let Some(request) = request {
                // Draw from defaults, not from what earlier runs stored.
                cc.egui_ctx.memory_mut(|m| m.data = Default::default());
                if request.result {
                    app.shell.view = convert64_app::shell::ViewMode::Result;
                }
                let o = &mut app.shell.overlays;
                for overlay in &request.overlays {
                    match overlay.as_str() {
                        "grid" => o.grid = true,
                        "clashes" => o.clashes = true,
                        "ghost" => o.ghost = true,
                        _ => eprintln!("convert64-app: unknown overlay {overlay}"),
                    }
                }
                use convert64_app::shell::DockTab;
                match request.tab.as_deref() {
                    Some("layers") => app.shell.tab = DockTab::Layers,
                    Some("export") => app.shell.tab = DockTab::Export,
                    _ => {}
                }
                if request.dialog.as_deref() == Some("export")
                    && let Some(ws) = &app.workspace
                {
                    app.shell.open_export(ws);
                }
                use convert64_app::view::Compare;
                app.shell.compare = match request.compare.as_deref() {
                    Some("split") => Compare::Split,
                    Some("flicker") => Compare::Flicker,
                    Some("side") => Compare::SideBySide,
                    _ => Compare::Off,
                };
                if let Some(preference) = request.theme {
                    app.set_theme(&cc.egui_ctx, preference);
                }
                app.screenshot = Some(request);
            }
            Ok(Box::new(app))
        }),
    )
}

/// Web entry point for `trunk`: mounts the app on the `convert64_canvas`
/// element of `index.html` and removes the loading text once it runs.
///
/// `eframe::WebRunner` installs eframe's panic handler, so a Rust panic is
/// logged to the browser console. Everything web-specific is reached through
/// eframe's re-exports, so the app needs no extra wasm dependencies.
#[cfg(target_arch = "wasm32")]
fn main() {
    use eframe::wasm_bindgen::JsCast as _;
    use eframe::web_sys;

    web_sys::js_sys::futures::spawn_local(async {
        let document = web_sys::window()
            .and_then(|window| window.document())
            .expect("convert64: no browser document");

        let canvas = document
            .get_element_by_id("convert64_canvas")
            .expect("convert64: index.html has no #convert64_canvas")
            .dyn_into::<web_sys::HtmlCanvasElement>()
            .expect("convert64: #convert64_canvas is not a <canvas>");

        let start_result = eframe::WebRunner::new()
            .start(
                canvas,
                eframe::WebOptions::default(),
                Box::new(|cc| Ok(Box::new(App::new(cc)))),
            )
            .await;

        if let Some(loading_text) = document.get_element_by_id("loading_text") {
            match start_result {
                Ok(()) => loading_text.remove(),
                Err(err) => {
                    loading_text.set_inner_html(
                        "<p>Convert64 failed to start. See the developer console for details.</p>",
                    );
                    panic!("convert64: failed to start eframe: {err:?}");
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use convert64_core::mode::Mode;
    use convert64_project::codec::encode_rgba8;
    use convert64_project::{Embed, MemoryStore, ProjectFile, Session};
    use std::path::Path;
    use std::sync::Arc;

    /// The app loads and saves projects through `convert64-project` with no file system
    /// (#192): the store holds the files the browser was given.
    #[test]
    fn a_project_loads_and_saves_from_memory() {
        let store = Arc::new(MemoryStore::new());
        let rgba = (0..16 * 10).flat_map(|i| [i as u8, 90, 200, 255]).collect();
        store.insert("art/title.png", encode_rgba8(16, 10, rgba));
        let file =
            ProjectFile::from_png(Mode::HiresBitmap, "title", Path::new("title.png"), (16, 10));
        store.insert("art/title.convert64", file.to_json());

        let session = Session::open_in(
            Box::new(Arc::clone(&store)),
            Path::new("art/title.convert64"),
        )
        .unwrap();
        assert_eq!(session.project().layers.len(), 1);
        session
            .save_to_store(Path::new("art/saved.convert64"), Embed::Keep)
            .unwrap();
        assert_eq!(
            store.get("art/saved.convert64"),
            store.get("art/title.convert64")
        );
    }
}
