//! The status bar (Figma `Status Bar`): mode and size, cell and pixel under the cursor and
//! zoom on the left; clashes (in the `clash` colour) and the last solve time on the right.

use super::Shell;
use crate::theme::Theme;
use egui::{RichText, Ui};

/// What the status bar and the result header report, filled in by the app each frame.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Status {
    /// The mode and its pixel size, e.g. `Multicolor 160×200`; `None` with no project.
    pub mode: Option<String>,
    /// The cell under the cursor.
    pub cell: Option<(u32, u32)>,
    /// The C64 pixel under the cursor.
    pub pixel: Option<(u32, u32)>,
    /// Cells whose colours clash, once a result exists.
    pub clashes: Option<usize>,
    /// The last solve's time in milliseconds.
    pub solve_ms: Option<f32>,
}

pub(super) fn show(shell: &Shell, status: &Status, ui: &mut Ui) {
    let small = |text: String| RichText::new(text).small();
    ui.horizontal_centered(|ui| {
        ui.label(small(
            status
                .mode
                .clone()
                .unwrap_or_else(|| "No project".to_owned()),
        ));
        ui.label(small(shell.view.label().to_owned()));
        if let Some((x, y)) = status.cell {
            let px = status
                .pixel
                .map(|(x, y)| format!(" · px {x},{y}"))
                .unwrap_or_default();
            ui.label(small(format!("cell {x},{y}{px}")));
        }
        ui.label(small(format!("{}%", shell.zoom)));

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if let Some(ms) = status.solve_ms {
                ui.label(small(format!("solve {ms:.0} ms")));
            }
            if let Some(n) = status.clashes {
                let colour = Theme::current(ui.ctx()).clash;
                let word = if n == 1 { "clash" } else { "clashes" };
                ui.label(small(format!("{n} {word}")).color(colour));
            }
        });
    });
}
