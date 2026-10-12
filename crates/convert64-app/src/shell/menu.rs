//! The menu bar (Figma `Menu Bar`): File, Edit, View, Layer, Recipe, Export, Help. Items
//! whose feature has not landed yet are shown disabled, so the menus already have their
//! final shape (`docs/UX.md` §11: disabled, not hidden).

use super::{Action, Shell, ViewMode, shortcuts as s};
use egui::{Button, KeyboardShortcut, Ui};

/// A menu item with its accelerator; `enabled` false until its feature lands.
fn item(ui: &mut Ui, label: &str, shortcut: Option<&KeyboardShortcut>, enabled: bool) -> bool {
    let mut button = Button::new(label);
    if let Some(shortcut) = shortcut {
        button = button.shortcut_text(ui.ctx().format_shortcut(shortcut));
    }
    ui.add_enabled(enabled, button).clicked()
}

pub(super) fn show(shell: &mut Shell, ui: &mut Ui, actions: &mut Vec<Action>) {
    egui::MenuBar::new().ui(ui, |ui| {
        ui.menu_button("File", |ui| {
            item(ui, "Open…", Some(&s::OPEN), false);
            ui.separator();
            item(ui, "Save", Some(&s::SAVE), false);
            item(ui, "Save As…", Some(&s::SAVE_AS), false);
            if cfg!(not(target_arch = "wasm32")) {
                ui.separator();
                if item(ui, "Quit", Some(&s::QUIT), true) {
                    actions.push(Action::Quit);
                }
            }
        });
        ui.menu_button("Edit", |ui| {
            item(ui, "Undo", Some(&s::UNDO), false);
            item(ui, "Redo", Some(&s::REDO), false);
        });
        ui.menu_button("View", |ui| view_menu(shell, ui, actions));
        ui.menu_button("Layer", |ui| {
            item(ui, "New Layer from File…", Some(&s::NEW_LAYER), false);
            item(ui, "Duplicate", Some(&s::DUPLICATE_LAYER), false);
            item(ui, "Delete", None, false);
        });
        ui.menu_button("Recipe", |ui| {
            item(ui, "Reset to Defaults", None, false);
        });
        ui.menu_button("Export", |ui| {
            item(ui, "Export…", Some(&s::EXPORT), false);
            item(ui, "Export Again", Some(&s::EXPORT_AGAIN), false);
        });
        ui.menu_button("Help", |ui| {
            ui.label(format!("Convert64 {}", env!("CARGO_PKG_VERSION")));
            ui.hyperlink_to("Documentation", env!("CARGO_PKG_REPOSITORY"));
        });
    });
}

fn view_menu(shell: &mut Shell, ui: &mut Ui, actions: &mut Vec<Action>) {
    for mode in [ViewMode::Source, ViewMode::Result] {
        let button = Button::selectable(shell.view == mode, mode.label())
            .shortcut_text(ui.ctx().format_shortcut(&s::SOURCE_RESULT));
        if ui.add(button).clicked() {
            shell.view = mode;
        }
    }
    ui.separator();
    if item(ui, "Zoom In", Some(&s::ZOOM_IN), true) {
        shell.zoom_in();
    }
    if item(ui, "Zoom Out", Some(&s::ZOOM_OUT), true) {
        shell.zoom_out();
    }
    item(ui, "Zoom to Fit", Some(&s::ZOOM_FIT), false);
    if item(ui, "Actual Pixels", Some(&s::ZOOM_100), true) {
        shell.zoom = 100;
    }
    ui.separator();
    let grid = Button::selectable(shell.grid, "Cell Grid")
        .shortcut_text(ui.ctx().format_shortcut(&s::GRID));
    if ui.add(grid).clicked() {
        shell.grid = !shell.grid;
    }
    ui.separator();
    if item(ui, "Hide Dock", Some(&s::HIDE_DOCK), true) {
        shell.dock_hidden = !shell.dock_hidden;
    }
    if item(ui, "Hide All Chrome", Some(&s::HIDE_CHROME), true) {
        shell.chrome_hidden = true;
    }
    ui.separator();
    ui.menu_button("Theme", |ui| {
        for (preference, label) in [
            (egui::ThemePreference::System, "System"),
            (egui::ThemePreference::Light, "Light"),
            (egui::ThemePreference::Dark, "Dark"),
        ] {
            if ui
                .add(Button::selectable(shell.theme == preference, label))
                .clicked()
            {
                shell.theme = preference;
                actions.push(Action::SetTheme(preference));
            }
        }
    });
}
