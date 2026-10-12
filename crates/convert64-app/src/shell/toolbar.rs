//! The toolbar (Figma `toolbar`): the four tools, zoom, the working-view switch and A/B
//! compare as 24 px icon buttons in three groups, and a hint on the right naming the
//! working view.

use super::{Shell, Tool, ViewMode, icon};
use crate::icons;
use crate::view::Compare;
use egui::{Button, Ui};

/// A toolbar button: an icon, a tooltip, `selected` drawn as the toggle's on state.
fn button(ui: &mut Ui, glyph: icons::Icon, tooltip: &str, selected: bool, enabled: bool) -> bool {
    let side = crate::theme::layout::TOOLBAR_HEIGHT - 4.0;
    let button = Button::selectable(selected, icon(glyph)).min_size(egui::vec2(side, side));
    ui.add_enabled(enabled, button)
        .on_hover_text(tooltip)
        .on_disabled_hover_text(tooltip)
        .clicked()
}

pub(super) fn show(shell: &mut Shell, ui: &mut Ui) {
    ui.horizontal_centered(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for tool in Tool::ALL {
            let tip = format!("{} ({})", tool.label(), tool.key().name());
            if button(ui, tool.icon(), &tip, shell.tool == tool, true) {
                shell.tool = tool;
            }
        }
        ui.separator();
        let ctx = ui.ctx().clone();
        let tip = |label: &str, shortcut| format!("{label} ({})", ctx.format_shortcut(shortcut));
        if button(
            ui,
            icons::ZOOM_IN,
            &tip("Zoom in", &super::shortcuts::ZOOM_IN),
            false,
            true,
        ) {
            shell.zoom_step(true);
        }
        if button(
            ui,
            icons::ZOOM_OUT,
            &tip("Zoom out", &super::shortcuts::ZOOM_OUT),
            false,
            true,
        ) {
            shell.zoom_step(false);
        }
        button(
            ui,
            icons::FIT,
            &tip("Zoom to fit", &super::shortcuts::ZOOM_FIT),
            false,
            false,
        );
        ui.separator();
        let result = shell.view == ViewMode::Result;
        if button(
            ui,
            icons::SOURCE_RESULT,
            "Source ⇄ Result (Q)",
            result,
            true,
        ) {
            shell.view = shell.view.flipped();
        }
        let compare = format!(
            "Compare: {} (, cycles · hold \\ to flip)",
            shell.compare.label()
        );
        if button(
            ui,
            icons::COMPARE,
            &compare,
            shell.compare != Compare::Off,
            true,
        ) {
            shell.compare = shell.compare.next();
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let other = shell.view.flipped().label();
            ui.label(
                egui::RichText::new(format!(
                    "Working view: {} (Q → {other} · hold \\ to flip)",
                    shell.view.label()
                ))
                .small()
                .weak(),
            );
        });
    });
}
