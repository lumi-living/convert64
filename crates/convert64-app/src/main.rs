//! eframe/egui user interface; native via `cargo run -p convert64-app`, web via `trunk build` (`docs/TAD.md §4`).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window on Windows in release

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
    /// Solves off the frame: a worker thread natively, a stage per frame in wasm (#194).
    scheduler: convert64_app::scheduler::Scheduler,
    /// The newest solve result, drawn every frame (once E8 wires in the preview).
    result: Option<convert64_app::scheduler::SolveResult>,
}

impl App {
    /// The app, with the scheduler asking `ctx` for a repaint when a result is ready.
    fn new(ctx: &egui::Context) -> Self {
        let mut app = Self::default();
        let ctx = ctx.clone();
        app.scheduler
            .set_waker(std::sync::Arc::new(move || ctx.request_repaint()));
        app
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

        egui::Panel::top("menu_bar").show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Quit").clicked() {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
            });
        });

        egui::CentralPanel::default().show(ui, |_ui| {});
    }
}

/// Native entry point: opens a desktop window.
#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([960.0, 640.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Convert64",
        native_options,
        Box::new(|cc| Ok(Box::new(App::new(&cc.egui_ctx)))),
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
                Box::new(|cc| Ok(Box::new(App::new(&cc.egui_ctx)))),
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
