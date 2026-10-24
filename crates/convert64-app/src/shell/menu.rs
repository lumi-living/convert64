//! The menu bar (Figma `Menu Bar`): File, Edit, View, Layer, Recipe, Export, Help. Items
//! whose feature has not landed yet are shown disabled, so the menus already have their
//! final shape (`docs/UX.md` §11: disabled, not hidden). On/off items carry a check mark
//! and Show/Hide items say which they will do (UX §8, usability review m5).

use super::{Action, Shell, ViewMode, shortcuts as s};
use crate::icons;
use crate::view::Compare;
use crate::workspace::Workspace;
use convert64_core::command::RecipeChange;
use convert64_project::{JsonCommand, LayerRef};
use egui::{Button, KeyboardShortcut, RichText, Ui};

/// A menu item with its accelerator; `enabled` false until its feature lands.
fn item(ui: &mut Ui, label: &str, shortcut: Option<&KeyboardShortcut>, enabled: bool) -> bool {
    let mut button = Button::new(label);
    if let Some(shortcut) = shortcut {
        button = button.shortcut_text(s::format(ui.ctx(), shortcut));
    }
    ui.add_enabled(enabled, button).clicked()
}

/// The check mark before an on/off item: the glyph when `on`, the same space blank when
/// off, so the labels line up and a toggle never looks like a hovered item.
pub(super) fn check(ui: &Ui, on: bool) -> RichText {
    let colour = if on {
        ui.visuals().text_color()
    } else {
        egui::Color32::TRANSPARENT
    };
    icons::CHECK.rich_text().color(colour)
}

/// An on/off menu item: a check mark when `on`, its accelerator on the right.
fn toggle(ui: &mut Ui, label: &str, on: bool, shortcut: Option<&KeyboardShortcut>) -> bool {
    let mut button = Button::new((check(ui, on), label));
    if let Some(shortcut) = shortcut {
        button = button.shortcut_text(s::format(ui.ctx(), shortcut));
    }
    ui.add(button).clicked()
}

/// A menu item beside on/off items: its label after the check mark's blank space, so
/// the View menu's labels line up.
fn plain(ui: &mut Ui, label: &str, shortcut: Option<&KeyboardShortcut>, enabled: bool) -> bool {
    let mut button = Button::new((check(ui, false), label));
    if let Some(shortcut) = shortcut {
        button = button.shortcut_text(s::format(ui.ctx(), shortcut));
    }
    ui.add_enabled(enabled, button).clicked()
}

/// The ghost overlay's item, named for what it shows: the other image (UX §4).
pub(super) fn overlay_label(view: ViewMode) -> &'static str {
    match view {
        ViewMode::Source => "Overlay the Result",
        ViewMode::Result => "Overlay the Source",
    }
}

/// Copy Recipe and Paste Recipe (Edit and Recipe menus): the selected layer's whole
/// recipe into the app's clipboard, and back onto the selected layer as one
/// `recipe_set` (UX §8).
fn copy_paste(shell: &mut Shell, ws: Option<&Workspace>, ui: &mut Ui, actions: &mut Vec<Action>) {
    let layer = ws.and_then(|ws| ws.doc.session().project().layers.get(shell.layer));
    if item(ui, "Copy Recipe", None, layer.is_some())
        && let Some(layer) = layer
    {
        shell.recipe_clipboard = Some(*layer.recipe());
        let message = format!("Copied the recipe of '{}'", layer.name());
        shell.toast(ui.ctx(), message, false);
    }
    let paste = item(
        ui,
        "Paste Recipe",
        None,
        layer.is_some() && shell.recipe_clipboard.is_some(),
    );
    if paste && let Some(recipe) = &shell.recipe_clipboard {
        actions.push(Action::Run(JsonCommand::RecipeSet {
            layer: LayerRef::Index(shell.layer),
            recipe: RecipeChange::all(recipe),
        }));
    }
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
                if item(ui, "Quit", shell.keys.quit.as_ref(), true) {
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
            ui.separator();
            copy_paste(shell, ws, ui, actions);
        });
        ui.menu_button("View", |ui| view_menu(shell, ui, actions));
        ui.menu_button("Layer", |ui| {
            let layer = convert64_project::LayerRef::Index(shell.layer);
            // The browser's binding in the browser, Shift Cmd L (#286).
            if item(
                ui,
                "New Layer from File…",
                Some(&shell.keys.new_layer),
                open,
            ) {
                actions.push(Action::NewLayer);
            }
            if item(ui, "Duplicate", Some(&s::DUPLICATE_LAYER), open) {
                actions.push(Action::Run(JsonCommand::LayerDuplicate {
                    layer: layer.clone(),
                }));
            }
            let visible = ws
                .and_then(|ws| ws.doc.session().project().layers.get(shell.layer))
                .is_none_or(|l| l.visible());
            let label = if visible { "Hide Layer" } else { "Show Layer" };
            if item(ui, label, Some(&s::TOGGLE_VISIBLE), open) {
                actions.push(Action::ToggleVisible(shell.layer));
            }
            if item(ui, "Delete", Some(&s::DELETE_LAYER), open) {
                actions.push(Action::Run(JsonCommand::LayerRemove { layer }));
            }
        });
        ui.menu_button("Recipe", |ui| {
            copy_paste(shell, ws, ui, actions);
            ui.separator();
            if item(ui, "Reset Recipe", None, open) {
                actions.push(Action::Run(JsonCommand::RecipeSet {
                    layer: LayerRef::Index(shell.layer),
                    recipe: RecipeChange::all(&Default::default()),
                }));
            }
        });
        ui.menu_button("Export", |ui| {
            // Export… shows the Export panel (UX §6b); Export Again writes its set.
            if item(ui, "Export…", Some(&s::EXPORT), open) {
                shell.show_export(ui.ctx());
            }
            let again = ws.is_some_and(|ws| !ws.doc.session().project().exports.is_empty());
            if item(ui, "Export Again", Some(&s::EXPORT_AGAIN), again) {
                actions.push(Action::Export { again: true });
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
        if toggle(
            ui,
            mode.label(),
            shell.view == mode,
            Some(&s::SOURCE_RESULT),
        ) {
            shell.view = mode;
        }
    }
    ui.menu_button((check(ui, false), "Compare"), |ui| {
        for compare in [
            Compare::Off,
            Compare::Split,
            Compare::Flicker,
            Compare::SideBySide,
        ] {
            let on = shell.compare == compare;
            if toggle(ui, compare.label(), on, Some(&s::COMPARE)) {
                shell.compare = compare;
            }
        }
    });
    ui.separator();
    if plain(ui, "Zoom In", Some(&s::ZOOM_IN), true) {
        shell.zoom_step(true);
    }
    if plain(ui, "Zoom Out", Some(&s::ZOOM_OUT), true) {
        shell.zoom_step(false);
    }
    if plain(ui, "Zoom to Fit", Some(&s::ZOOM_FIT), true) {
        shell.fit();
    }
    if plain(ui, "Actual Pixels", Some(&s::ZOOM_100), true) {
        shell.camera.actual(shell.view);
    }
    ui.separator();
    let overlay = overlay_label(shell.view);
    let o = &mut shell.overlays;
    for (on, label, shortcut) in [
        (&mut o.grid, "Cell Grid", Some(&s::GRID)),
        (&mut o.clashes, "Colour-Limited Cells", Some(&s::CLASHES)),
        (&mut o.steering, "Pins & Locks", None),
        (&mut o.ghost, overlay, None),
        (&mut o.border, "Border", Some(&s::BORDER)),
    ] {
        if toggle(ui, label, *on, shortcut) {
            *on = !*on;
        }
    }
    ui.separator();
    let dock = if shell.dock_hidden {
        "Show Dock"
    } else {
        "Hide Dock"
    };
    if plain(ui, dock, Some(&s::HIDE_DOCK), true) {
        shell.dock_hidden = !shell.dock_hidden;
    }
    // The menu is part of the UI, so it only ever hides it; `Cmd \` or `Esc` shows it.
    if plain(ui, "Hide UI", Some(&s::HIDE_UI), true) {
        shell.set_ui_hidden(ui.ctx(), true);
    }
    ui.separator();
    ui.menu_button((check(ui, false), "Theme"), |ui| {
        for (preference, label) in [
            (egui::ThemePreference::System, "System"),
            (egui::ThemePreference::Light, "Light"),
            (egui::ThemePreference::Dark, "Dark"),
        ] {
            if toggle(ui, label, shell.theme == preference, None) {
                shell.theme = preference;
                actions.push(Action::SetTheme(preference));
            }
        }
    });
}
