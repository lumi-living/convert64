//! The status bar (Figma `Status Bar`, UX §4.2 cursor readout): mode and size, the working
//! view, the cell and pixel under the cursor with the cell's colours, error and steering,
//! and the zoom on the left; clashes (in the `clash` colour) and the last solve on the
//! right.

use super::Shell;
use crate::theme::{Theme, layout};
use crate::view::{Compare, target_at};
use crate::workspace::Workspace;
use egui::{RichText, Ui};

/// A mode's name as the status bar and the result header show it.
pub(super) fn mode_name(mode: convert64_core::mode::Mode) -> &'static str {
    use convert64_core::mode::Mode;
    match mode {
        Mode::MulticolorBitmap => "Multicolor",
        Mode::HiresBitmap => "Hi-res",
        Mode::MulticolorCharset => "Multicolor charset",
        Mode::HiresCharset => "Hi-res charset",
        Mode::EcmCharset => "ECM charset",
        _ => "Sprite",
    }
}

pub(super) fn show(shell: &Shell, ws: Option<&Workspace>, ui: &mut Ui) {
    let small = |text: String| RichText::new(text).small();
    ui.horizontal_centered(|ui| {
        let Some(ws) = ws else {
            ui.label(small("No project".to_owned()));
            return;
        };
        let grid = ws.doc.grid();
        let (w, h) = grid.target_size();
        ui.label(small(format!("{} {w}×{h}", mode_name(ws.doc.mode()))));
        let view = match shell.compare {
            Compare::Off => shell.shown().label().to_owned(),
            c => format!("{} · {}", shell.view.label(), c.label()),
        };
        ui.label(small(view));
        if let Some((px, cell)) = shell.hover.and_then(|p| target_at(grid, p)) {
            ui.label(small(format!(
                "cell {},{} · px {},{}",
                cell.0, cell.1, px.0, px.1
            )));
            let info = ws.cell_info(cell);
            let side = layout::SWATCH_SIZE * 0.75;
            ui.spacing_mut().item_spacing.x = 2.0;
            for c in &info.colours {
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::hover());
                ui.painter().rect_filled(rect, 1.0, *c);
                ui.painter().rect_stroke(
                    rect,
                    1.0,
                    ui.visuals().widgets.noninteractive.bg_stroke,
                    egui::StrokeKind::Inside,
                );
            }
            ui.spacing_mut().item_spacing.x = ui.style().spacing.item_spacing.x;
            if let Some(e) = info.error {
                ui.label(small(format!("error {e:.3}")));
            }
            let theme = Theme::current(ui.ctx());
            if info.pinned {
                ui.label(small("pinned".to_owned()).color(theme.pin));
            }
            if info.locked {
                ui.label(small("locked".to_owned()).color(theme.lock));
            }
        }
        ui.label(small(format!("{}%", shell.camera.percent(shell.view))));

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            match (ws.solving, ws.solve_ms) {
                (true, _) => ui.label(small("Solving…".to_owned())),
                (false, Some(ms)) => ui.label(small(format!("solve {ms:.0} ms"))),
                (false, None) => ui.label(small(String::new())),
            };
            if let Some(n) = ws.clashes() {
                let colour = Theme::current(ui.ctx()).clash;
                let word = if n == 1 { "clash" } else { "clashes" };
                ui.label(small(format!("{n} {word}")).color(colour));
            }
        });
    });
}
