//! The right dock (Figma `SidePanel::right / dock`): the result header, the picture row
//! (mode, palette and background, #288) and the C64 result (fixed, never scrolls away),
//! then the Recipe, Layers and Export tabs. Without a project
//! every group is there but disabled (`docs/UX.md` §11).

use super::status::mode_name;
use super::{Action, DockTab, Shell, ViewMode, icon};
use crate::icons;
use crate::theme::{MEDIUM, Theme, layout};
use crate::view::{Compare, SCREEN, cell_rect};
use crate::workspace::Workspace;
use convert64_core::command::BackgroundPin;
use convert64_core::mode::Mode;
use convert64_core::solve::Background;
use convert64_core::{ColourIndex, Palette};
use convert64_project::{JsonCommand, PaletteSpec};
use egui::{Button, Rect, RichText, Ui, pos2};

/// The result's inset in the panel, as fractions of its size (Figma `Result 320×200 @1×`:
/// the bitmap sits 16 px in from the sides and 12 px from the top and bottom).
const RESULT_INSET: egui::Vec2 = egui::vec2(16.0 / 320.0, 12.0 / 200.0);

/// Below this window height (points) the result collapses to a strip (UX §4.3, §14 Q1;
/// usability review m8: at 1024 × 640 the form had about 300 px under the result).
pub(super) const COLLAPSE_BELOW: f32 = 800.0;

/// The collapsed result's scale: 160 × 100.
const COLLAPSED_SCALE: f32 = 0.5;

/// The border above and below the collapsed result, in points.
const COLLAPSED_BORDER: f32 = 6.0;

/// The result panel's height in a window `window_height` tall: the Figma 1× panel, or
/// below [`COLLAPSE_BELOW`] the 160 × 100 picture and its border (112).
pub(super) fn result_height(window_height: f32) -> f32 {
    if window_height < COLLAPSE_BELOW {
        SCREEN.y * COLLAPSED_SCALE + 2.0 * COLLAPSED_BORDER
    } else {
        layout::RESULT_HEIGHT
    }
}

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
        .show(ui, |ui| {
            result_header(shell, ws, ui);
            picture_row(ws, ui, actions);
        });
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

/// `Result Header`: the title, the method picker's preview, the colour-limited cell count
/// (a button to the worst one) and the compare and fit buttons.
fn result_header(shell: &mut Shell, ws: Option<&Workspace>, ui: &mut Ui) {
    // Figma's header is 26 px: the toolbar's height less its edge stroke and padding.
    let size = egui::vec2(ui.available_width(), layout::TOOLBAR_HEIGHT - 2.0);
    let row = egui::Layout::left_to_right(egui::Align::Center);
    ui.allocate_ui_with_layout(size, row, |ui| {
        // The right-hand part first; the title and the preview take what is left, so
        // the preview never draws over the count (UX §4.3).
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
                // Neutral text, as in the status bar (#243).
                let label = RichText::new(super::status::limited_cells(n)).small();
                let tip = format!(
                    "{}\nClick to show the worst one on the result.",
                    super::status::limited_cells_tip(ui.ctx())
                );
                let clicked = ui
                    .add(Button::new(label).frame(false))
                    .on_hover_text(tip)
                    .clicked();
                if clicked {
                    shell.show_worst(ws);
                }
            }
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                let title = RichText::new("Result").family(egui::FontFamily::Name(MEDIUM.into()));
                ui.add(egui::Label::new(title).extend());
                // The method picker's live preview (UX §6a), cut short to fit.
                if let Some(p) = ws.and_then(|ws| ws.preview.as_ref()) {
                    let accent = Theme::current(ui.ctx()).accent;
                    let text = format!("Preview: {}", p.name);
                    let label = RichText::new(&text).small().color(accent);
                    ui.add(egui::Label::new(label).truncate())
                        .on_hover_text(text);
                }
            });
        });
    });
}

/// The picture row's height (UX §3, §4.3): a compact control's.
const PICTURE_ROW_HEIGHT: f32 = 22.0;

/// The picture row under the header (UX §4.3, usability review M5, #288): the
/// picture-wide Mode, Palette and Background, as compact dropdowns. The Recipe tab has
/// none of them.
fn picture_row(ws: Option<&Workspace>, ui: &mut Ui, actions: &mut Vec<Action>) {
    let size = egui::vec2(ui.available_width(), PICTURE_ROW_HEIGHT);
    let row = egui::Layout::left_to_right(egui::Align::Center);
    ui.allocate_ui_with_layout(size, row, |ui| {
        ui.add_enabled_ui(ws.is_some(), |ui| {
            mode_dropdown(ws, ui, actions);
            palette_dropdown(ws, ui, actions);
            background_dropdown(ws, ui, actions);
        });
    });
}

/// The Mode dropdown: the only Mode control (#288).
fn mode_dropdown(ws: Option<&Workspace>, ui: &mut Ui, actions: &mut Vec<Action>) {
    let current = ws.map(|ws| ws.doc.mode());
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
        })
        .response
        .on_hover_text("Mode: the C64 graphics mode");
}

/// The palettes the Palette dropdown offers before the `.vpl` file.
fn palette_presets() -> [(PaletteSpec, Palette, &'static str); 3] {
    [
        (PaletteSpec::PeptoPal, Palette::pepto_pal(), "Pepto PAL"),
        (PaletteSpec::PeptoNtsc, Palette::pepto_ntsc(), "Pepto NTSC"),
        (
            PaletteSpec::Colodore,
            Palette::colodore(50.0, 100.0, 50.0),
            "Colodore",
        ),
    ]
}

/// The Palette dropdown: the presets and a VICE palette file.
fn palette_dropdown(ws: Option<&Workspace>, ui: &mut Ui, actions: &mut Vec<Action>) {
    let palette = ws.map(|ws| ws.doc.palette());
    let presets = palette_presets();
    let current = palette.map_or_else(
        || "—".to_owned(),
        |palette| {
            presets
                .iter()
                .find(|(_, p, _)| p == palette)
                .map_or_else(|| palette.name().to_owned(), |(_, _, n)| (*n).to_owned())
        },
    );
    egui::ComboBox::from_id_salt("result_palette")
        .selected_text(RichText::new(current).small())
        .width(0.0)
        .show_ui(ui, |ui| {
            for (spec, p, name) in presets {
                let on = palette == Some(&p);
                if ui.selectable_label(on, name).clicked() && !on {
                    actions.push(Action::Run(JsonCommand::PaletteSet { palette: spec }));
                }
            }
            ui.separator();
            if ui.button("VICE palette file (.vpl)…").clicked() {
                actions.push(Action::LoadPalette);
            }
        })
        .response
        .on_hover_text("Palette: the colours the result is solved and shown in");
}

/// The background setting and the colour the result has: the pinned one, or the
/// solver's pick under Auto.
fn shown_background(ws: &Workspace) -> (Background, Option<ColourIndex>) {
    let bg = ws.doc.session().project().solve.background;
    let solved = ws.solution.as_ref().and_then(|s| s.background);
    let shown = match bg {
        Background::Fixed(c) => Some(c),
        Background::Auto => solved,
    };
    (bg, shown)
}

/// The Background dropdown's button text: `Auto` while the solver picks, else the
/// pinned colour's name.
fn background_text(bg: Background) -> String {
    match bg {
        Background::Auto => "Auto".to_owned(),
        Background::Fixed(c) => c.name().to_owned(),
    }
}

/// The Background dropdown (UX §4.3): a swatch and `Auto` or the colour's name; it opens
/// on *Auto* and the 16 colours, and stays open until a click outside.
fn background_dropdown(ws: Option<&Workspace>, ui: &mut Ui, actions: &mut Vec<Action>) {
    let theme = Theme::current(ui.ctx());
    let (bg, shown) = ws.map_or((Background::Auto, None), shown_background);
    let font = egui::TextStyle::Small.resolve(ui.style());
    let galley = ui
        .painter()
        .layout_no_wrap(background_text(bg), font, egui::Color32::PLACEHOLDER);
    let pad = ui.spacing().button_padding;
    let swatch = 12.0;
    let arrow = ui.spacing().icon_width;
    let width = pad.x + swatch + 4.0 + galley.size().x + 4.0 + arrow + pad.x;
    let height = ui.spacing().interact_size.y;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click());
    let response = response.on_hover_text(match (bg, shown) {
        (Background::Auto, Some(c)) => format!("Background: Auto, the solver picked {c}"),
        (Background::Auto, None) => "Background: Auto, the solver picks".to_owned(),
        (Background::Fixed(c), _) => format!("Background: {c}, pinned"),
    });
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, ui.is_enabled(), "Background")
    });
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact(&response);
        let painter = ui.painter();
        painter.rect(
            rect,
            visuals.corner_radius,
            visuals.weak_bg_fill,
            visuals.bg_stroke,
            egui::StrokeKind::Inside,
        );
        let s = Rect::from_center_size(
            pos2(rect.left() + pad.x + swatch / 2.0, rect.center().y),
            egui::vec2(swatch, swatch),
        );
        match (ws, shown) {
            (Some(ws), Some(c)) => {
                let rgb = ws.doc.palette().rgb(c);
                painter.rect_filled(s, 1.0, egui::Color32::from_rgb(rgb.r, rgb.g, rgb.b));
                // Pinned: the accent ring; the solver's pick: the text colour's.
                let ring = if matches!(bg, Background::Fixed(_)) {
                    theme.accent
                } else {
                    visuals.fg_stroke.color
                };
                painter.rect_stroke(s, 1.0, (1.0, ring), egui::StrokeKind::Outside);
            }
            _ => {
                painter.rect_stroke(s, 1.0, visuals.fg_stroke, egui::StrokeKind::Inside);
            }
        }
        let text_pos = pos2(s.right() + 4.0, rect.center().y - galley.size().y / 2.0);
        painter.galley(text_pos, galley, visuals.text_color());
        let a = Rect::from_center_size(
            pos2(rect.right() - pad.x - arrow / 2.0, rect.center().y),
            egui::vec2(arrow, arrow) * 0.5,
        );
        painter.add(egui::Shape::convex_polygon(
            vec![a.left_top(), a.right_top(), a.center_bottom()],
            visuals.fg_stroke.color,
            egui::Stroke::NONE,
        ));
    }
    let Some(ws) = ws else {
        return;
    };
    egui::Popup::from_toggle_button_response(&response)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| background_grid(ws, bg, ui, actions));
}

/// The Background dropdown's contents: *Auto* and the 16 colours, the pinned one ringed
/// in the accent colour and the solver's pick in the text colour (not by colour alone:
/// the rings' widths differ too).
fn background_grid(ws: &Workspace, bg: Background, ui: &mut Ui, actions: &mut Vec<Action>) {
    let pin = |background| Action::Run(JsonCommand::BackgroundPin(BackgroundPin { background }));
    let solved = ws.solution.as_ref().and_then(|s| s.background);
    let mut auto = bg == Background::Auto;
    if ui.checkbox(&mut auto, "Auto (the solver picks)").changed() {
        actions.push(pin(if auto {
            Background::Auto
        } else {
            Background::Fixed(solved.unwrap_or(ColourIndex::ALL[0]))
        }));
    }
    let side = layout::SWATCH_SIZE;
    let palette = ws.doc.palette();
    let accent = Theme::current(ui.ctx()).accent;
    ui.spacing_mut().item_spacing = egui::vec2(2.0, 2.0);
    for row in ColourIndex::ALL.chunks(8) {
        ui.horizontal(|ui| {
            for &c in row {
                let (rect, resp) =
                    ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::click());
                let rgb = palette.rgb(c);
                ui.painter()
                    .rect_filled(rect, 1.0, egui::Color32::from_rgb(rgb.r, rgb.g, rgb.b));
                let pinned = bg == Background::Fixed(c);
                if pinned || solved == Some(c) {
                    let (w, colour) = if pinned {
                        (2.5, accent)
                    } else {
                        (1.5, ui.visuals().text_color())
                    };
                    ui.painter()
                        .rect_stroke(rect, 1.0, (w, colour), egui::StrokeKind::Outside);
                }
                let tip = format!(
                    "Background {c}{}",
                    if pinned {
                        " (pinned)"
                    } else if solved == Some(c) {
                        " (solver's pick)"
                    } else {
                        ""
                    }
                );
                if resp.on_hover_text(tip).clicked() && !pinned {
                    actions.push(pin(Background::Fixed(c)));
                }
            }
        });
    }
}

/// The C64 result at 1× with its border (`layout::RESULT_HEIGHT` tall, the dock's
/// width), the colour-limited cell dots while that overlay is on, and, in Result mode,
/// the navigator rectangle. A click or drag
/// centres the working view there (switching a Source view to Result, UX §3).
fn result(shell: &mut Shell, ws: Option<&Workspace>, ui: &mut Ui) {
    let theme = Theme::current(ui.ctx());
    let window = ui.ctx().content_rect().height();
    let collapsed = window < COLLAPSE_BELOW;
    let size = egui::vec2(ui.available_width(), result_height(window));
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
    // The screen inside the border, fitted at 320:200 and never above 1×; collapsed, at
    // half size (never above what the dock's width holds).
    let scale = if collapsed {
        let inner = rect.shrink2(egui::vec2(RESULT_INSET.x * rect.width(), COLLAPSED_BORDER));
        (inner.width() / SCREEN.x).min(COLLAPSED_SCALE)
    } else {
        let inner = rect.shrink2(rect.size() * RESULT_INSET);
        (inner.width() / SCREEN.x)
            .min(inner.height() / SCREEN.y)
            .min(1.0)
    };
    let screen = Rect::from_center_size(rect.center(), SCREEN * scale);
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
    let grid = ws.doc.grid();
    for cell in panel_dots(shell, ws) {
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

/// The cells dotted on the result panel: the colour-limited ones, only while their
/// overlay (`Shift C`) is on, so the main output is clean by default (UX §4.3, #243).
/// Dots only, not the heatmap.
fn panel_dots(shell: &Shell, ws: &Workspace) -> Vec<(u32, u32)> {
    if shell.overlays.clashes {
        ws.clash_cells()
    } else {
        Vec::new()
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
            DockTab::Export => return super::export_panel::show(shell, ws, ui, actions),
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
                let open = crate::shell::shortcuts::format(ui.ctx(), &super::shortcuts::OPEN);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use convert64_core::solve::SolveMode;

    /// A busy 64×40 picture, solved: plenty of cells want more colours than multicolor
    /// allows.
    fn solved() -> Workspace {
        let rgba = (0..64u32 * 40)
            .flat_map(|i| {
                let v = |k: u32| ((i * k) % 256) as u8;
                [v(37), v(91), v(53), 255]
            })
            .collect();
        let png = convert64_project::codec::encode_rgba8(64, 40, rgba);
        let mut ws = Workspace::new(Document::from_png("busy.png", &png).unwrap());
        let mut scheduler = crate::scheduler::Scheduler::stepping(Default::default());
        scheduler.submit(ws.doc.snapshot(None), SolveMode::Full);
        ws.solution = loop {
            scheduler.tick();
            if let Some(r) = scheduler.poll().filter(|r| !r.provisional) {
                break Some(r.solution);
            }
        };
        ws
    }

    /// Found by the release-prep run: with the method picker open and the pointer on a
    /// card, `Preview: …` was drawn on top of `Colour-limited cells: N`. The preview now
    /// takes only the space left of the count, cut short with `…` (UX §4.3).
    #[test]
    fn the_preview_label_never_covers_the_count() {
        use super::super::ui_test::frame;
        let mut ws = solved();
        let n = ws.clashes().unwrap();
        ws.preview = Some(crate::workspace::MethodPreview {
            name: "Riemersma (Hilbert curve, 16-colour history, with a long name to cut)",
            committed: None,
            was_solving: false,
        });
        for theme in [egui::Theme::Light, egui::Theme::Dark] {
            let ctx = egui::Context::default();
            crate::theme::install(&ctx);
            ctx.set_theme(theme);
            let mut shell = Shell::default();
            let size = egui::vec2(1280.0, 900.0);
            frame(&ctx, &mut shell, Some(&mut ws), size, vec![]);
            let (texts, _) = frame(&ctx, &mut shell, Some(&mut ws), size, vec![]);
            // Menu bar, toolbar, options bar (#311), then the header.
            let header =
                |t: &super::super::ui_test::Text| t.rect.top() < 4.0 * layout::TOOLBAR_HEIGHT;
            let count = texts
                .iter()
                .find(|t| header(t) && t.text == super::super::status::limited_cells(n))
                .expect("the count in the header");
            let preview = texts
                .iter()
                .find(|t| header(t) && t.text.starts_with("Preview:"))
                .expect("the preview in the header");
            assert!(
                preview.rect.right() <= count.rect.left(),
                "{theme:?}: preview {:?} over count {:?}",
                preview.rect,
                count.rect
            );
            // The name is far wider than the space: what is drawn stops short of the count.
            let font = egui::TextStyle::Small.resolve(&ctx.global_style());
            let full = ctx.fonts_mut(|f| {
                f.layout_no_wrap(preview.text.clone(), font, egui::Color32::WHITE)
                    .size()
                    .x
            });
            assert!(preview.rect.width() < full, "{theme:?}: not cut short");
        }
    }

    /// Usability review m4: the status bar's parts are set off by `·`, its numbers are
    /// in the monospace font and padded, and the cell's swatches are labelled.
    #[test]
    fn the_status_bar_separates_and_labels_its_parts() {
        let ws = solved();
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let mut shell = Shell {
            hover: Some(egui::pos2(13.0, 9.0)),
            ..Shell::default()
        };
        let mut sections = Vec::new();
        for _ in 0..2 {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                super::super::status::show(&mut shell, Some(&ws), ui, &mut Vec::new());
            });
            output.textures_delta.clear();
            sections.clear();
            for s in &output.shapes {
                if let egui::Shape::Text(t) = &s.shape {
                    for sec in &t.galley.job.sections {
                        let r = sec.byte_range.start.0..sec.byte_range.end.0;
                        let family = sec.format.font_id.family.clone();
                        sections.push((t.galley.job.text[r].to_owned(), family));
                    }
                }
            }
        }
        let texts: Vec<&str> = sections.iter().map(|(t, _)| t.as_str()).collect();
        let mono = |s: &str| {
            sections
                .iter()
                .any(|(t, f)| t == s && *f == egui::FontFamily::Monospace)
        };
        // Screen point 13,9: cell 1,1 and multicolor pixel 6,9, padded, monospace.
        assert!(mono(" 1, 1"), "{texts:?}");
        assert!(mono("  6,  9"), "{texts:?}");
        assert!(
            texts.contains(&super::super::status::CELL_COLOURS),
            "{texts:?}"
        );
        assert!(
            texts.iter().filter(|t| **t == "·").count() >= 5,
            "{texts:?}"
        );
        // The error and the zoom are separate texts with a `·` between them.
        let error = texts.iter().position(|t| *t == "error ").expect("error");
        let zoom = texts.iter().position(|t| t.ends_with('%')).expect("zoom");
        assert!(
            zoom > error + 1 && texts[error + 2..zoom].contains(&"·"),
            "{texts:?}"
        );
    }

    /// #243: the result panel shows no dots until the overlay is turned on.
    #[test]
    fn the_result_panel_has_no_dots_by_default() {
        let ws = solved();
        assert!(ws.clashes().unwrap() > 0, "the test picture has none");
        let mut shell = Shell::default();
        assert!(!shell.overlays.clashes);
        assert_eq!(panel_dots(&shell, &ws), vec![]);
        shell.overlays.clashes = true;
        assert_eq!(panel_dots(&shell, &ws), ws.clash_cells());
        // The result header's count still turns the overlay on, and with it the dots.
        let mut shell = Shell::default();
        shell.show_worst(&ws);
        assert!(!panel_dots(&shell, &ws).is_empty());
    }

    /// #243: the count reads as what it measures, in neutral text (not the `clash` red),
    /// in both the status bar and the result header, with a tooltip.
    #[test]
    fn the_count_is_neutral_and_says_what_it_measures() {
        let mut ws = solved();
        let n = ws.clashes().unwrap();
        assert_eq!(
            super::super::status::limited_cells(367),
            "Colour-limited cells: 367"
        );
        for theme in [egui::Theme::Light, egui::Theme::Dark] {
            let ctx = egui::Context::default();
            crate::theme::install(&ctx);
            ctx.set_theme(theme);
            let mut shell = Shell::default();
            let mut texts = Vec::new();
            for _ in 0..2 {
                let input = egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1400.0, 900.0),
                    )),
                    ..Default::default()
                };
                let mut output = ctx.run_ui(input, |ui| {
                    shell.show(ui, Some(&mut ws));
                });
                output.textures_delta.clear();
                texts = output
                    .shapes
                    .into_iter()
                    .filter_map(|s| match s.shape {
                        egui::Shape::Text(t) => Some(t),
                        _ => None,
                    })
                    .filter(|t| t.galley.text().contains("Colour-limited cells:"))
                    .collect::<Vec<_>>();
            }
            // One in the status bar, one in the result header.
            assert_eq!(texts.len(), 2, "{theme:?}");
            let tip = super::super::status::limited_cells_tip(&ctx);
            assert!(tip.contains("still legal"), "{tip}");
            let clash = Theme::current(&ctx).clash;
            for t in &texts {
                assert_eq!(t.galley.text(), format!("Colour-limited cells: {n}"));
                let colours = t
                    .galley
                    .job
                    .sections
                    .iter()
                    .map(|s| s.format.color)
                    .chain(t.override_text_color)
                    .chain([t.fallback_color]);
                for c in colours {
                    assert_ne!(c, clash, "{theme:?}");
                }
            }
        }
    }
}
