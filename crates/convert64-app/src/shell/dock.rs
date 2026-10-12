//! The right dock (Figma `SidePanel::right / dock`): the result header and the C64 result
//! (fixed, never scrolls away), then the Recipe, Layers and Export tabs. Without a project
//! every group is there but disabled (`docs/UX.md` §11).

use super::{DockTab, Shell, Status, icon};
use crate::icons;
use crate::theme::{MEDIUM, Theme, layout};
use egui::{Button, RichText, Ui};

pub(super) fn show(shell: &mut Shell, status: &Status, ui: &mut Ui) {
    let margin = ui.spacing().window_margin;
    egui::Frame::NONE
        .inner_margin(egui::Margin::symmetric(margin.left, 0))
        .show(ui, |ui| result_header(shell, status, ui));
    result(shell, ui);
    egui::Frame::NONE
        .inner_margin(egui::Margin::symmetric(margin.left, 0))
        .show(ui, |ui| tabs(shell, ui));
    ui.separator();
    egui::ScrollArea::vertical().show(ui, |ui| {
        egui::Frame::NONE.inner_margin(margin).show(ui, |ui| {
            ui.set_width(ui.available_width());
            tab_content(shell, ui);
        });
    });
}

/// `Result Header`: the title, the mode at its zoom and the result's own controls.
fn result_header(shell: &Shell, status: &Status, ui: &mut Ui) {
    // Figma's header is 26 px: the toolbar's height less its edge stroke and padding.
    let size = egui::vec2(ui.available_width(), layout::TOOLBAR_HEIGHT - 2.0);
    let row = egui::Layout::left_to_right(egui::Align::Center);
    ui.allocate_ui_with_layout(size, row, |ui| {
        ui.label(RichText::new("Result").family(egui::FontFamily::Name(MEDIUM.into())));
        let mode = status
            .mode
            .as_deref()
            .map_or("—", |m| m.split(' ').next().unwrap_or(m));
        ui.label(RichText::new(format!("{mode} · 1×")).small().weak());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for (glyph, tip) in [
                (icons::FIT, "Fit"),
                (icons::ZOOM_IN, "Zoom"),
                (icons::COMPARE, "Compare"),
            ] {
                ui.add_enabled(shell.has_project, Button::new(icon(glyph)).frame(false))
                    .on_disabled_hover_text(tip)
                    .on_hover_text(tip);
            }
        });
    });
}

/// The C64 result at 1× (`layout::RESULT_HEIGHT` tall, the dock's width), or its empty
/// state. The texture arrives with the result panel (#204).
fn result(shell: &Shell, ui: &mut Ui) {
    let theme = Theme::current(ui.ctx());
    let size = egui::vec2(ui.available_width(), layout::RESULT_HEIGHT);
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    ui.painter().rect_filled(rect, 0.0, theme.canvas_bg);
    if !shell.has_project {
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "No result yet",
            egui::TextStyle::Small.resolve(ui.style()),
            ui.visuals().weak_text_color(),
        );
    }
}

/// `Dock Tabs`: Recipe, Layers, Export.
fn tabs(shell: &mut Shell, ui: &mut Ui) {
    ui.horizontal(|ui| {
        for tab in DockTab::ALL {
            if ui.selectable_label(shell.tab == tab, tab.label()).clicked() {
                shell.tab = tab;
            }
        }
    });
}

/// The selected tab's group. Each tab's form arrives with its own issue (#205, #206,
/// #207); until a project is open it says what to do first.
fn tab_content(shell: &Shell, ui: &mut Ui) {
    let (glyph, what) = match shell.tab {
        DockTab::Recipe => (icons::RECIPE, "the recipe"),
        DockTab::Layers => (icons::LAYERS, "the layers"),
        DockTab::Export => (icons::EXPORT, "export"),
    };
    ui.add_enabled_ui(shell.has_project, |ui| {
        ui.horizontal(|ui| {
            ui.label(icon(glyph).weak());
            let open = ui.ctx().format_shortcut(&super::shortcuts::OPEN);
            ui.label(RichText::new(format!("Open an image ({open}) to see {what}.")).weak());
        });
    });
}
