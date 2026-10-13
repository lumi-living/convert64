//! The export dialog (`Cmd E`, UX §3): format, its options and the file name. Export
//! Again (`Shift Cmd E`) repeats the last export without it.

use super::{Action, Shell};
use crate::export::{Format, Settings};
use crate::workspace::Workspace;
use convert64_formats::asm::DataStyle;
use egui::{RichText, Ui};

/// The dialog's state while it is open.
#[derive(Debug, Clone, PartialEq)]
pub struct ExportDialog {
    /// The settings being chosen.
    pub settings: Settings,
    /// The file stem the files are named after.
    pub name: String,
}

pub(super) fn show(
    shell: &mut Shell,
    ws: Option<&Workspace>,
    ctx: &egui::Context,
    actions: &mut Vec<Action>,
) {
    let (Some(dialog), Some(ws)) = (&mut shell.export, ws) else {
        shell.export = None;
        return;
    };
    let mode = ws.doc.mode();
    let mut open = true;
    let mut done = false;
    egui::Window::new("Export")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui: &mut Ui| {
            ui.horizontal(|ui| {
                ui.label("Name");
                ui.text_edit_singleline(&mut dialog.name);
            });
            ui.separator();
            for format in Format::ALL {
                ui.radio_value(&mut dialog.settings.format, format, format.label(mode));
            }
            ui.separator();
            match dialog.settings.format {
                Format::Png => {
                    ui.add(
                        egui::Slider::new(
                            &mut dialog.settings.zoom,
                            1..=convert64_formats::png::MAX_ZOOM,
                        )
                        .text("Zoom")
                        .suffix("×"),
                    );
                }
                Format::Source(_) => {
                    ui.label("Picture data");
                    for (style, label) in [
                        (DataStyle::Bytes, "Inline bytes"),
                        (DataStyle::IncludeRaw, "Include the raw set"),
                        (DataStyle::IncludePicture, "Include the picture file"),
                    ] {
                        ui.radio_value(&mut dialog.settings.data, style, label);
                    }
                }
                _ => {}
            }
            if ws.solving || ws.solution.is_none() {
                ui.label(RichText::new("Waiting for the full solve…").weak());
            }
            ui.add_space(ui.spacing().item_spacing.y);
            ui.horizontal(|ui| {
                let ready = ws.solution.is_some() && !ws.solving && !dialog.name.trim().is_empty();
                let label = format!("Export {}", dialog.settings.format.label(mode));
                if ui.add_enabled(ready, egui::Button::new(label)).clicked() {
                    actions.push(Action::Export(
                        dialog.settings,
                        dialog.name.trim().to_owned(),
                    ));
                    done = true;
                }
                if ui.button("Cancel").clicked() {
                    done = true;
                }
            });
        });
    if !open || done {
        shell.export = None;
    }
}
