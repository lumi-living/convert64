//! The status bar (Figma `Status Bar`, UX §4.2 cursor readout): mode and size, the working
//! view, the cell and pixel under the cursor with the cell's colours, error and steering,
//! and the zoom on the left; the colour-limited cell count (neutral text, #243) and the
//! last solve on the right.

use super::{Action, Shell};
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

/// The count of colour-limited cells ("clashes" in the core's report) as the status bar
/// and the result header show it: a neutral diagnostic, not an error, since the result
/// is legal either way (#243, usability review M1).
pub(super) fn limited_cells(n: usize) -> String {
    format!("Colour-limited cells: {n}")
}

/// What the colour-limited cell count measures: the tooltip on it.
pub(super) fn limited_cells_tip(ctx: &egui::Context) -> String {
    format!(
        "Cells where the solver wanted more colours than the mode allows, so it used the \
         closest ones it may. The picture is still legal and exports as it is. {} shows \
         them.",
        ctx.format_shortcut(&super::shortcuts::CLASHES)
    )
}

/// The status bar's link that turns the overlay on, suggested once (UX §11).
pub(super) fn show_limited_cells(ctx: &egui::Context) -> String {
    format!(
        "Show colour-limited cells ({})",
        ctx.format_shortcut(&super::shortcuts::CLASHES)
    )
}

/// A solve running longer than this (seconds) shows progress and Pause (UX §11).
const SLOW_SOLVE: f64 = 0.2;

pub(super) fn show(
    shell: &mut Shell,
    ws: Option<&Workspace>,
    ui: &mut Ui,
    actions: &mut Vec<Action>,
) {
    let small = |text: String| RichText::new(text).small();
    ui.horizontal_centered(|ui| {
        error(shell, ui);
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
        if shell.tool == super::Tool::Transform
            && shell.mask.tool.is_none()
            && let Some(l) = ws.doc.session().project().layers.get(shell.layer)
        {
            ui.label(small(super::transform::readout(&l.transform())));
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let now = ui.input(|i| i.time);
            let slow = ws.solve_started.is_some_and(|t| now - t > SLOW_SOLVE);
            if ws.paused {
                if ui.small_button("Resume").clicked() {
                    actions.push(Action::ResumeSolve);
                }
                ui.label(small("Solve paused".to_owned()));
            } else if ws.solving && slow {
                if ui.small_button("Pause").clicked() {
                    actions.push(Action::PauseSolve);
                }
                ui.label(small(format!(
                    "Solving full search… {:.1} s",
                    now - ws.solve_started.unwrap_or(now)
                )));
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(100));
            } else if ws.solving {
                ui.label(small("Solving…".to_owned()));
                if let Some(t) = ws.solve_started {
                    // Wake up when the solve becomes slow, to show Pause.
                    let left = (SLOW_SOLVE - (now - t)).max(0.0);
                    ui.ctx()
                        .request_repaint_after(std::time::Duration::from_secs_f64(left + 0.01));
                }
            } else if let Some(ms) = ws.solve_ms {
                ui.label(small(format!("solve {ms:.0} ms")));
            }
            if let Some(n) = ws.clashes() {
                // Neutral text: red is kept for what blocks export (#243).
                ui.label(small(limited_cells(n)))
                    .on_hover_text(limited_cells_tip(ui.ctx()));
                // Suggested once, never switched on by itself (UX §11).
                if n > 0 && !shell.overlays.clashes && !shell.clash_hint_dismissed {
                    let close =
                        egui::Button::new(crate::icons::CLOSE.rich_text().small()).frame(false);
                    if ui.add(close).on_hover_text("Dismiss").clicked() {
                        shell.clash_hint_dismissed = true;
                    }
                    if ui.link(small(show_limited_cells(ui.ctx()))).clicked() {
                        shell.overlays.clashes = true;
                        shell.clash_hint_dismissed = true;
                    }
                }
            }
        });
    });
}

/// The last error, inline in the error colour; a click shows the details and a way to
/// dismiss it (UX §11: never a modal).
fn error(shell: &mut Shell, ui: &mut Ui) {
    let Some((summary, details)) = shell.error.clone() else {
        return;
    };
    let colour = Theme::current(ui.ctx()).error;
    let text = RichText::new(format!("{} {summary}", crate::icons::WARNING.codepoint))
        .small()
        .color(colour);
    let response = ui.add(egui::Label::new(text).sense(egui::Sense::click()));
    let response = response.on_hover_text("Click for details");
    egui::Popup::from_toggle_button_response(&response).show(|ui| {
        ui.set_max_width(360.0);
        ui.label(RichText::new(&summary).strong());
        ui.label(&details);
        if ui.button("Dismiss").clicked() {
            shell.error = None;
        }
    });
    ui.separator();
}
