//! The toolbar (Figma `toolbar`): the four tools, zoom, the working-view switch and A/B
//! compare as 24 px icon buttons in three groups, and a hint on the right naming the
//! working view.

use super::{Action, MaskTool, Shell, Tool, ViewMode, icon};
use crate::icons;
use crate::view::Compare;
use crate::workspace::Workspace;
use egui::{Button, RichText, Ui};

/// What a disabled tool, zoom or Snap control says with no picture open (UX §5, #292).
pub(in crate::shell) const NEEDS_PICTURE: &str = "Open a picture first";

/// A toolbar button: an icon, a tooltip, `selected` drawn as the toggle's on state.
/// Disabled, its tooltip says why ([`NEEDS_PICTURE`]).
fn button(ui: &mut Ui, glyph: icons::Icon, tooltip: &str, selected: bool, enabled: bool) -> bool {
    let side = crate::theme::layout::TOOLBAR_HEIGHT - 4.0;
    let button = Button::selectable(selected, icon(glyph)).min_size(egui::vec2(side, side));
    ui.add_enabled(enabled, button)
        .on_hover_text(tooltip)
        .on_disabled_hover_text(format!("{tooltip}: {}", NEEDS_PICTURE.to_lowercase()))
        .clicked()
}

/// The Snap setting (ADR-011): Off, C64 pixel, Cell (`Shift Cmd ;` cycles).
pub(in crate::shell) fn snap_menu(ws: Option<&Workspace>, ui: &mut Ui, actions: &mut Vec<Action>) {
    use convert64_core::layer::Snap;
    let current = ws.map_or(Snap::Off, |ws| ws.doc.session().project().snap);
    let name = |s: Snap| match s {
        Snap::Off => "Off",
        Snap::C64Pixel => "C64 pixel",
        Snap::Cell => "Cell",
    };
    ui.add_enabled_ui(ws.is_some(), |ui| {
        egui::ComboBox::from_id_salt("snap")
            .selected_text(RichText::new(format!("Snap: {}", name(current))).small())
            .width(0.0)
            .show_ui(ui, |ui| {
                for s in [Snap::Off, Snap::C64Pixel, Snap::Cell] {
                    if ui.selectable_label(s == current, name(s)).clicked() && s != current {
                        let cmd = convert64_core::command::SnapSet { snap: s };
                        actions.push(Action::Run(convert64_project::JsonCommand::SnapSet(cmd)));
                    }
                }
            })
            .response
            .on_hover_text("Snap transforms: Off (sub-pixel), C64 pixel, Cell (Shift Cmd ;)")
            .on_disabled_hover_text(format!("Snap: {}", NEEDS_PICTURE.to_lowercase()));
    });
}

pub(super) fn show(
    shell: &mut Shell,
    ws: Option<&Workspace>,
    ui: &mut Ui,
    actions: &mut Vec<Action>,
) {
    // The tools, zoom and Snap wait for a picture (UX §5, #292).
    let open = ws.is_some();
    ui.horizontal_centered(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        if shell.mask.active {
            // Mask mode: the toolbar swaps to the mask tools (UX §7a).
            for tool in MaskTool::ALL {
                let tip = format!("{} ({})", tool.label(), tool.key().name());
                if button(ui, tool.icon(), &tip, shell.mask.tool == Some(tool), true) {
                    shell.mask.tool = Some(tool);
                }
            }
            if ui
                .button(RichText::new("Done").small())
                .on_hover_text("Leave mask mode (M or Esc)")
                .clicked()
            {
                shell.toggle_mask_mode();
            }
        } else {
            for tool in Tool::ALL {
                let mut tip = format!("{} ({}", tool.label(), tool.key().name());
                // Transform's Cmd T beside its S, where the platform has it (#286).
                if let Some(s) = shell.keys.transform.filter(|_| tool == Tool::Transform) {
                    tip.push_str(&format!(" or {}", ui.ctx().format_shortcut(&s)));
                }
                tip.push(')');
                let on = open && shell.tool == tool && shell.mask.tool.is_none();
                if button(ui, tool.icon(), &tip, on, open) {
                    shell.tool = tool;
                    shell.mask.tool = None;
                }
            }
        }
        snap_menu(ws, ui, actions);
        ui.separator();
        let ctx = ui.ctx().clone();
        let tip = |label: &str, shortcut| format!("{label} ({})", ctx.format_shortcut(shortcut));
        if button(
            ui,
            icons::ZOOM_IN,
            &tip("Zoom in", &super::shortcuts::ZOOM_IN),
            false,
            open,
        ) {
            shell.zoom_step(true);
        }
        if button(
            ui,
            icons::ZOOM_OUT,
            &tip("Zoom out", &super::shortcuts::ZOOM_OUT),
            false,
            open,
        ) {
            shell.zoom_step(false);
        }
        if button(
            ui,
            icons::FIT,
            &tip("Zoom to fit", &super::shortcuts::ZOOM_FIT),
            false,
            open,
        ) {
            shell.fit();
        }
        ui.separator();
        let result = shell.view == ViewMode::Result;
        if button(
            ui,
            icons::SOURCE_RESULT,
            "Source or Result (Q)",
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

/// The options bar (UX §3, usability review M8): one row under the toolbar, always
/// there, so nothing jumps when a tool or mask mode changes. It shows the current
/// tool's options: the mask tools' and the Region brush's, Pin and Lock's usage, or
/// with Nudge and Transform the selected layer's transform readout. Empty with no
/// picture.
pub(super) fn options_bar(
    shell: &mut Shell,
    ws: Option<&Workspace>,
    ui: &mut Ui,
    actions: &mut Vec<Action>,
) {
    let Some(ws) = ws else { return };
    ui.horizontal_centered(|ui| {
        super::mask::narrow_sliders(ui);
        if shell.mask.tool.is_some() {
            super::mask::options_bar(shell, Some(ws), ui, actions);
            return;
        }
        let name = |text: &str| RichText::new(text).small().strong();
        let hint = |text: &str| RichText::new(text).small().weak();
        match shell.tool {
            Tool::Region => super::mask::region_bar(shell, Some(ws), ui),
            Tool::Pin => {
                ui.label(name("Pin"));
                ui.separator();
                ui.label(hint(
                    "Click a cell to keep its colours, drag for a rectangle; Alt-click keeps one colour",
                ));
            }
            Tool::Lock => {
                ui.label(name("Lock"));
                ui.separator();
                ui.label(hint(
                    "Click a cell to freeze it as solved, drag for a rectangle; again to unlock",
                ));
            }
            Tool::Nudge | Tool::Transform => {
                ui.label(name(if shell.tool == Tool::Nudge {
                    "Move"
                } else {
                    "Transform"
                }));
                ui.separator();
                if let Some(l) = ws.doc.session().project().layers.get(shell.layer) {
                    let readout = super::transform::readout(&l.transform(), l.source().size());
                    ui.label(RichText::new(format!("Layer {} · {readout}", l.name())).small());
                }
            }
        }
    });
}
