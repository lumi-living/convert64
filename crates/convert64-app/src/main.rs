//! eframe/egui user interface; native via `cargo run -p convert64-app`, web via `trunk build` (`docs/TAD.md §4`).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window on Windows in release

use convert64_app::shell::{Action, Shell, Status};
use convert64_app::theme;

/// The storage key of the View › Theme choice (eframe's app storage: a file natively,
/// `localStorage` in the browser).
const THEME_KEY: &str = "theme";

/// Placeholder for the document being converted.
///
/// Will be replaced by `convert64_core::Project` in E2.
#[derive(Debug, Default)]
struct Project;

/// Top-level UI state; every frame is drawn from the held document.
#[derive(Debug, Default)]
struct App {
    /// The open document (placeholder until E2).
    #[expect(
        dead_code,
        reason = "placeholder; read once E2 wires in the core Project"
    )]
    project: Project,
    /// The chrome: working-view mode, tool, dock tab, what is hidden (#203).
    shell: Shell,
    /// Solves off the frame: a worker thread natively, a stage per frame in wasm (#194).
    scheduler: convert64_app::scheduler::Scheduler,
    /// The newest solve result, drawn every frame (once E8 wires in the preview).
    result: Option<convert64_app::scheduler::SolveResult>,
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

    /// What the status bar reports. Filled from the project and the result once they
    /// reach the app (#204, #205).
    fn status(&self) -> Status {
        Status {
            solve_ms: self
                .result
                .as_ref()
                .map(|r| (r.timing.composite_ms + r.timing.solve_ms) as f32),
            ..Status::default()
        }
    }
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
        if let Some(result) = self.scheduler.poll() {
            self.result = Some(result);
        }

        let status = self.status();
        for action in self.shell.show(ui, &status) {
            match action {
                Action::Quit => ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close),
                Action::SetTheme(preference) => self.set_theme(ui.ctx(), preference),
            }
        }

        #[cfg(not(target_arch = "wasm32"))]
        if let Some(request) = &mut self.screenshot {
            request.step(ui.ctx());
        }
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

/// `--screenshot <file.png> [--theme light|dark] [--size <w>x<h>]`: draws a few frames,
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
        frame: u32,
    }

    impl Request {
        /// The request in the command line, if any.
        pub(super) fn from_args() -> Result<Option<Self>, String> {
            let mut args = std::env::args().skip(1);
            let mut request = None;
            let mut theme = None;
            let mut size = [1280.0, 800.0];
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
                    "--size" => {
                        let v = value()?;
                        let (w, h) = v.split_once('x').ok_or("--size is <w>x<h>")?;
                        size = [
                            w.parse().map_err(|_| "--size: bad width")?,
                            h.parse().map_err(|_| "--size: bad height")?,
                        ];
                    }
                    _ => return Err(format!("unknown argument {arg}")),
                }
            }
            Ok(request.map(|path| Self {
                path,
                theme,
                size,
                frame: 0,
            }))
        }

        /// Called every frame: asks for the screenshot once the frames have settled,
        /// writes it when it arrives and closes the window.
        pub(super) fn step(&mut self, ctx: &egui::Context) {
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
    let request = match screenshot::Request::from_args() {
        Ok(request) => request,
        Err(e) => {
            eprintln!("convert64-app: {e}");
            std::process::exit(2);
        }
    };
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
            if let Some(request) = request {
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
