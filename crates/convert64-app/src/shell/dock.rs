//! The right dock (Figma `SidePanel::right / dock`): the result header and the C64 result
//! (fixed, never scrolls away), then the Recipe, Layers and Export tabs. Without a project
//! every group is there but disabled (`docs/UX.md` §11).

use super::status::mode_name;
use super::{Action, DockTab, Shell, ViewMode, icon};
use crate::icons;
use crate::theme::{MEDIUM, Theme, layout};
use crate::view::{Compare, SCREEN, cell_rect};
use crate::workspace::Workspace;
use convert64_core::mode::Mode;
use egui::{Button, Rect, RichText, Ui, pos2};

/// The result's inset in the panel, as fractions of its size (Figma `Result 320×200 @1×`:
/// the bitmap sits 16 px in from the sides and 12 px from the top and bottom).
const RESULT_INSET: egui::Vec2 = egui::vec2(16.0 / 320.0, 12.0 / 200.0);

/// The modes the solver solves, in the header's dropdown.
const MODES: [Mode; 2] = [Mode::MulticolorBitmap, Mode::HiresBitmap];

pub(super) fn show(
    shell: &mut Shell,
    ws: Option<&Workspace>,
    ui: &mut Ui,
    actions: &mut Vec<Action>,
) {
    let margin = ui.spacing().window_margin;
    egui::Frame::NONE
        .inner_margin(egui::Margin::symmetric(margin.left, 0))
        .show(ui, |ui| result_header(shell, ws, ui, actions));
    result(shell, ws, ui);
    egui::Frame::NONE
        .inner_margin(egui::Margin::symmetric(margin.left, 0))
        .show(ui, |ui| tabs(shell, ui));
    ui.separator();
    let mut scroll = egui::ScrollArea::vertical();
    if let Some(offset) = shell.dock_scroll {
        scroll = scroll.vertical_scroll_offset(offset);
    }
    scroll.show(ui, |ui| {
        egui::Frame::NONE.inner_margin(margin).show(ui, |ui| {
            ui.set_width(ui.available_width());
            tab_content(shell, ws, ui, actions);
        });
    });
}

/// `Result Header`: the title, the mode dropdown, the clash count (a button to the worst
/// clash) and the compare and fit buttons.
fn result_header(
    shell: &mut Shell,
    ws: Option<&Workspace>,
    ui: &mut Ui,
    actions: &mut Vec<Action>,
) {
    // Figma's header is 26 px: the toolbar's height less its edge stroke and padding.
    let size = egui::vec2(ui.available_width(), layout::TOOLBAR_HEIGHT - 2.0);
    let row = egui::Layout::left_to_right(egui::Align::Center);
    ui.allocate_ui_with_layout(size, row, |ui| {
        ui.label(RichText::new("Result").family(egui::FontFamily::Name(MEDIUM.into())));
        let current = ws.map(|ws| ws.doc.mode());
        ui.add_enabled_ui(ws.is_some(), |ui| {
            let text = current.map_or("—", mode_name);
            egui::ComboBox::from_id_salt("result_mode")
                .selected_text(RichText::new(text).small())
                .width(0.0)
                .show_ui(ui, |ui| {
                    for mode in MODES {
                        if ui
                            .selectable_label(current == Some(mode), mode_name(mode))
                            .clicked()
                            && current != Some(mode)
                        {
                            actions.push(Action::SetMode(mode));
                        }
                    }
                });
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let enabled = ws.is_some();
            let fit = ui.add_enabled(enabled, Button::new(icon(icons::FIT)).frame(false));
            if fit.on_hover_text("Fit the working view").clicked() {
                shell.fit();
            }
            let on = shell.compare != Compare::Off;
            let compare = ui.add_enabled(
                enabled,
                Button::selectable(on, icon(icons::COMPARE)).frame(false),
            );
            if compare.on_hover_text("Compare (,)").clicked() {
                shell.compare = shell.compare.next();
            }
            if let Some((ws, n)) = ws.and_then(|ws| Some((ws, ws.clashes()?))) {
                let colour = Theme::current(ui.ctx()).clash;
                let word = if n == 1 { "clash" } else { "clashes" };
                let label = RichText::new(format!("{n} {word}")).small().color(colour);
                let clicked = ui
                    .add(Button::new(label).frame(false))
                    .on_hover_text("Show the worst clash")
                    .clicked();
                if clicked {
                    shell.show_worst(ws);
                }
            }
        });
    });
}

/// The C64 result at 1× with its border (`layout::RESULT_HEIGHT` tall, the dock's
/// width), clash dots and, in Result mode, the navigator rectangle. A click or drag
/// centres the working view there (switching a Source view to Result, UX §3).
fn result(shell: &mut Shell, ws: Option<&Workspace>, ui: &mut Ui) {
    let theme = Theme::current(ui.ctx());
    let size = egui::vec2(ui.available_width(), layout::RESULT_HEIGHT);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Other,
            true,
            "C64 result, click to inspect",
        )
    });
    let painter = ui.painter_at(rect);
    let Some(ws) = ws else {
        painter.rect_filled(rect, 0.0, theme.canvas_bg);
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "No result yet",
            egui::TextStyle::Small.resolve(ui.style()),
            ui.visuals().weak_text_color(),
        );
        return;
    };
    painter.rect_filled(rect, 0.0, ws.border_colour());
    // The screen inside the border, fitted at 320:200 and never above 1×.
    let inner = rect.shrink2(rect.size() * RESULT_INSET);
    let scale = (inner.width() / SCREEN.x)
        .min(inner.height() / SCREEN.y)
        .min(1.0);
    let screen = Rect::from_center_size(inner.center(), SCREEN * scale);
    match ws.result_texture() {
        Some(tex) => {
            let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
            painter.image(tex.id(), screen, uv, egui::Color32::WHITE);
            // A full search running over 200 ms: the last result, slightly faded, with
            // "Solving…" (UX §4.3, §11).
            let now = ui.input(|i| i.time);
            if ws.solving && ws.solve_started.is_some_and(|t| now - t > 0.2) {
                painter.rect_filled(screen, 0.0, theme.canvas_bg.gamma_multiply(0.25));
                painter.text(
                    screen.center(),
                    egui::Align2::CENTER_CENTER,
                    "Solving…",
                    egui::TextStyle::Small.resolve(ui.style()),
                    ui.visuals().strong_text_color(),
                );
            }
        }
        None => {
            painter.rect_filled(screen, 0.0, theme.canvas_bg);
            painter.text(
                screen.center(),
                egui::Align2::CENTER_CENTER,
                "Solving…",
                egui::TextStyle::Small.resolve(ui.style()),
                ui.visuals().weak_text_color(),
            );
        }
    }
    let to_panel = |p: egui::Pos2| screen.min + p.to_vec2() * scale;
    // Clash dots only, not the heatmap (UX §4.3).
    let grid = ws.doc.grid();
    for cell in ws.clash_cells() {
        painter.circle_filled(to_panel(cell_rect(grid, cell).center()), 1.5, theme.clash);
    }
    if shell.view == ViewMode::Result
        && let Some(view) = shell.view_rect
    {
        let visible = shell.camera.visible(view, ViewMode::Result);
        let nav =
            Rect::from_min_max(to_panel(visible.min), to_panel(visible.max)).intersect(screen);
        painter.rect_stroke(nav, 0.0, (1.0, theme.accent), egui::StrokeKind::Inside);
    }
    if (response.clicked() || response.dragged())
        && let Some(p) = response.interact_pointer_pos()
    {
        let picture = ((p - screen.min) / scale).to_pos2();
        shell.camera.centre = picture.clamp(egui::Pos2::ZERO, SCREEN.to_pos2());
        if response.clicked() {
            shell.view = ViewMode::Result;
        }
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
fn tab_content(shell: &mut Shell, ws: Option<&Workspace>, ui: &mut Ui, actions: &mut Vec<Action>) {
    let has_project = ws.is_some();
    if let Some(ws) = ws {
        match shell.tab {
            DockTab::Export => return export_tab(shell, ws, ui),
            DockTab::Layers => {
                super::layers::show(shell, ws, ui, actions);
                // A press on the panel gives it the Delete key (#245).
                let rect = ui.min_rect();
                shell.layers_rect = Some(rect);
                if ui.input(|i| i.pointer.any_pressed()) && ui.rect_contains_pointer(rect) {
                    shell.key_focus = super::KeyFocus::Layers;
                }
                return;
            }
            DockTab::Recipe => return super::recipe::show(shell, ws, ui, actions),
        }
    }
    let (glyph, what) = match shell.tab {
        DockTab::Recipe => (icons::RECIPE, "the recipe"),
        DockTab::Layers => (icons::LAYERS, "the layers"),
        DockTab::Export => (icons::EXPORT, "export"),
    };
    ui.add_enabled_ui(has_project, |ui| {
        ui.horizontal(|ui| {
            ui.label(icon(glyph).weak());
            if has_project {
                ui.label(
                    RichText::new(format!("{} arrives with its own issue.", capitalise(what)))
                        .weak(),
                );
            } else {
                let open = ui.ctx().format_shortcut(&super::shortcuts::OPEN);
                ui.label(RichText::new(format!("Open an image ({open}) to see {what}.")).weak());
            }
        });
    });
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map_or_else(String::new, |f| f.to_uppercase().chain(c).collect())
}

/// The Export tab: the dialog's button and what Export Again would repeat.
fn export_tab(shell: &mut Shell, ws: &Workspace, ui: &mut Ui) {
    let shortcut = ui.ctx().format_shortcut(&super::shortcuts::EXPORT);
    let button = Button::new("Export…").shortcut_text(shortcut);
    if ui.add(button).clicked() {
        shell.open_export(ws);
    }
    if let Some((settings, name)) = &shell.last_export {
        let again = ui.ctx().format_shortcut(&super::shortcuts::EXPORT_AGAIN);
        ui.label(
            RichText::new(format!(
                "Last: {} as {name} · Export Again {again}",
                settings.format.label(ws.doc.mode())
            ))
            .small()
            .weak(),
        );
    }
}
