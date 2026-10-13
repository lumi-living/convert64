//! The menu bar (Figma `Menu Bar`): File, Edit, View, Layer, Recipe, Export, Help. Items
//! whose feature has not landed yet are shown disabled, so the menus already have their
//! final shape (`docs/UX.md` §11: disabled, not hidden).

use super::{Action, Shell, ViewMode, shortcuts as s};
use crate::view::Compare;
use crate::workspace::Workspace;
use egui::{Button, KeyboardShortcut, Ui};

/// A menu item with its accelerator; `enabled` false until its feature lands.
fn item(ui: &mut Ui, label: &str, shortcut: Option<&KeyboardShortcut>, enabled: bool) -> bool {
    let mut button = Button::new(label);
    if let Some(shortcut) = shortcut {
        button = button.shortcut_text(ui.ctx().format_shortcut(shortcut));
    }
    ui.add_enabled(enabled, button).clicked()
}

pub(super) fn show(
    shell: &mut Shell,
    ws: Option<&Workspace>,
    ui: &mut Ui,
    actions: &mut Vec<Action>,
) {
    let open = ws.is_some();
    egui::MenuBar::new().ui(ui, |ui| {
        ui.menu_button("File", |ui| {
            if item(ui, "Open…", Some(&s::OPEN), true) {
                actions.push(Action::Open);
            }
            ui.separator();
            if item(ui, "Save", Some(&s::SAVE), open) {
                actions.push(Action::Save);
            }
            if item(ui, "Save As…", Some(&s::SAVE_AS), open) {
                actions.push(Action::SaveAs);
            }
            if cfg!(not(target_arch = "wasm32")) {
                ui.separator();
                if item(ui, "Quit", Some(&s::QUIT), true) {
                    actions.push(Action::Quit);
                }
            }
        });
        ui.menu_button("Edit", |ui| {
            let (undo, redo) = shell.history;
            if item(ui, "Undo", Some(&s::UNDO), open && undo) {
                actions.push(Action::Undo);
            }
            if item(ui, "Redo", Some(&s::REDO), open && redo) {
                actions.push(Action::Redo);
            }
        });
        ui.menu_button("View", |ui| view_menu(shell, ui, actions));
        ui.menu_button("Layer", |ui| {
            let layer = convert64_project::LayerRef::Index(shell.layer);
            if item(ui, "New Layer from File…", Some(&s::NEW_LAYER), open) {
                actions.push(Action::NewLayer);
            }
            if item(ui, "Duplicate", Some(&s::DUPLICATE_LAYER), open) {
                actions.push(Action::Run(
                    convert64_project::JsonCommand::LayerDuplicate {
                        layer: layer.clone(),
                    },
                ));
            }
            if item(ui, "Show / Hide", Some(&s::TOGGLE_VISIBLE), open) {
                actions.push(Action::ToggleVisible(shell.layer));
            }
            if item(ui, "Delete", Some(&s::DELETE_LAYER), open) {
                actions.push(Action::Run(convert64_project::JsonCommand::LayerRemove {
                    layer,
                }));
            }
        });
        ui.menu_button("Recipe", |ui| {
            item(ui, "Reset to Defaults", None, false);
        });
        ui.menu_button("Export", |ui| {
            if item(ui, "Export…", Some(&s::EXPORT), open)
                && let Some(ws) = ws
            {
                shell.open_export(ws);
            }
            let again = open && shell.last_export.is_some();
            if item(ui, "Export Again", Some(&s::EXPORT_AGAIN), again)
                && let Some((settings, name)) = shell.last_export.clone()
            {
                actions.push(Action::Export(settings, name));
            }
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
    ui.menu_button("Compare", |ui| {
        for compare in [
            Compare::Off,
            Compare::Split,
            Compare::Flicker,
            Compare::SideBySide,
        ] {
            let button = Button::selectable(shell.compare == compare, compare.label())
                .shortcut_text(ui.ctx().format_shortcut(&s::COMPARE));
            if ui.add(button).clicked() {
                shell.compare = compare;
            }
        }
    });
    ui.separator();
    if item(ui, "Zoom In", Some(&s::ZOOM_IN), true) {
        shell.zoom_step(true);
    }
    if item(ui, "Zoom Out", Some(&s::ZOOM_OUT), true) {
        shell.zoom_step(false);
    }
    if item(ui, "Zoom to Fit", Some(&s::ZOOM_FIT), true) {
        shell.fit();
    }
    if item(ui, "Actual Pixels", Some(&s::ZOOM_100), true) {
        shell.camera.actual(shell.view);
    }
    ui.separator();
    let o = &mut shell.overlays;
    for (on, label, shortcut) in [
        (&mut o.grid, "Cell Grid", Some(&s::GRID)),
        (&mut o.clashes, "Clashes", Some(&s::CLASHES)),
        (&mut o.steering, "Pins & Locks", None),
        (&mut o.ghost, "Ghost of the Other Image", None),
        (&mut o.border, "Border", Some(&s::BORDER)),
    ] {
        let mut button = Button::selectable(*on, label);
        if let Some(shortcut) = shortcut {
            button = button.shortcut_text(ui.ctx().format_shortcut(shortcut));
        }
        if ui.add(button).clicked() {
            *on = !*on;
        }
    }
    ui.separator();
    let dock = if shell.dock_hidden {
        "Show Dock"
    } else {
        "Hide Dock"
    };
    if item(ui, dock, Some(&s::HIDE_DOCK), true) {
        shell.dock_hidden = !shell.dock_hidden;
    }
    // The menu is part of the UI, so it only ever hides it; `Cmd \` or `Esc` shows it.
    if item(ui, "Hide UI", Some(&s::HIDE_UI), true) {
        shell.set_ui_hidden(ui.ctx(), true);
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
