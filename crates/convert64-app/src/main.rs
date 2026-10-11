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
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
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

fn main() -> eframe::Result {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([960.0, 640.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Convert64",
        native_options,
        Box::new(|_cc| Ok(Box::new(App::default()))),
    )
}
