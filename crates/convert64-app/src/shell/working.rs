//! The working view (`docs/UX.md` §4): the source composite or the result, zoomed and
//! panned, with the overlays and the compare modes. Without a project it shows the empty
//! state.

use super::{Shell, ViewMode, icon};
use crate::icons;
use crate::theme::Theme;
use crate::view::{BORDER, Compare, cell_rect};
use crate::workspace::Workspace;
use egui::{Color32, Painter, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

/// The ghost's opacity (UX §4.1: "at 30 %").
const GHOST: f32 = 0.3;

/// The checkerboard's square, in points (zoom-independent, as image editors draw it).
const CHECKER: f32 = 16.0;

/// The smallest cell, in points, the grid overlay is drawn at; finer would be a grey wash.
const GRID_MIN_CELL: f32 = 6.0;

/// The smallest cell, in points, that gets a pin or lock glyph (Figma `Cell Badge`).
const GLYPH_MIN_CELL: f32 = 16.0;

/// A/B flicker: each image's time on screen (2 Hz).
const FLICKER_S: f64 = 0.25;

pub(super) fn show(
    shell: &mut Shell,
    ui: &mut Ui,
    ws: Option<&Workspace>,
    actions: &mut Vec<super::Action>,
) {
    let Some(ws) = ws else {
        shell.hover = None;
        empty(ui);
        return;
    };
    let view = ui.max_rect();
    let response = ui.allocate_rect(view, Sense::click_and_drag());
    // Screen readers: the view is one focusable area with a name (UX §12).
    let name = format!("Working view, {}", shell.view.label());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Other, true, &name));
    shell.view_rect = Some(view);
    if shell.fit_pending {
        shell.fit_pending = false;
        shell.camera.fit(view, ViewMode::Source);
        shell.camera.fit(view, ViewMode::Result);
    }
    let wheel_scales = shell.tool == super::Tool::Transform
        && shell.mask.tool.is_none()
        && response
            .hover_pos()
            .is_some_and(|p| super::transform::over_box(shell, ws, view, p));
    if !wheel_scales {
        navigate(shell, ui, &response, view);
    }
    if shell.hand {
        // The hand tool only pans.
    } else if shell.mask.tool.is_none() && shell.tool == super::Tool::Transform {
        super::transform::interact(shell, ws, ui, &response, view, actions);
        super::tools::interact(shell, ws, ui, &response, view, actions);
    } else if shell.mask.tool.is_none() && shell.tool == super::Tool::Region {
        super::mask::region_interact(shell, ws, ui, &response, view, actions);
    } else if shell.mask.tool.is_some() {
        super::mask::interact(shell, ws, ui, &response, view, actions);
    } else {
        super::tools::interact(shell, ws, ui, &response, view, actions);
    }

    let painter = ui.painter_at(view);
    let pointer = response.hover_pos();
    match shell.compare {
        Compare::Split => {
            let x = view.left() + view.width() * shell.split;
            let (left, right) = view.split_left_right_at_x(x);
            draw(
                shell,
                ws,
                &painter.with_clip_rect(left),
                view,
                ViewMode::Source,
            );
            draw(
                shell,
                ws,
                &painter.with_clip_rect(right),
                view,
                ViewMode::Result,
            );
            divider(shell, ui, view, x);
        }
        Compare::SideBySide => {
            let (left, right) = view.split_left_right_at_fraction(0.5);
            draw(
                shell,
                ws,
                &painter.with_clip_rect(left),
                left,
                ViewMode::Source,
            );
            draw(
                shell,
                ws,
                &painter.with_clip_rect(right),
                right,
                ViewMode::Result,
            );
            // The linked cursor: the same picture point in the other half.
            if let Some(p) = pointer {
                let (here, there) = if left.contains(p) {
                    (left, right)
                } else {
                    (right, left)
                };
                let picture = shell.camera.to_picture(here, shell.view, p);
                let other = shell.camera.picture_rect(there, shell.view);
                let q = other.min + picture.to_vec2() * shell.camera.scale(shell.view);
                crosshair(
                    &painter.with_clip_rect(there),
                    q,
                    Theme::current(ui.ctx()).accent,
                );
            }
            painter.vline(left.right(), view.y_range(), ui.visuals().window_stroke);
        }
        Compare::Flicker => {
            let t = ui.input(|i| i.time);
            let source = ((t / FLICKER_S) as u64).is_multiple_of(2);
            let mode = if source {
                ViewMode::Source
            } else {
                ViewMode::Result
            };
            draw(shell, ws, &painter, view, mode);
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs_f64(FLICKER_S / 2.0));
        }
        Compare::Off => draw(shell, ws, &painter, view, shell.shown()),
    }

    // The regions, tinted (UX §7: "a tinted outline"), with the Region tool.
    if let Some(tex) = shell.region_overlay {
        let picture = shell.camera.picture_rect(view, shell.view);
        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        let tint = Theme::current(ui.ctx()).region.gamma_multiply(0.45);
        painter.image(tex, picture, uv, tint);
    }
    if shell.tool == super::Tool::Region
        && shell.mask.tool.is_none()
        && let Some(h) = response.hover_pos()
    {
        let s = shell.camera.scale(shell.view);
        let accent = Theme::current(ui.ctx()).accent;
        painter.circle_stroke(h, shell.mask.size * s, (1.5, accent));
    }
    // The quick-mask overlay (UX §7a): over the picture, in the view mode's geometry.
    if let Some(tex) = shell.mask_overlay {
        let picture = shell.camera.picture_rect(view, shell.view);
        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        painter.image(tex, picture, uv, Color32::WHITE);
    }
    super::tools::draw_drag(shell, ws, &painter, view);
    if shell.mask.tool.is_none() && shell.tool == super::Tool::Transform {
        super::transform::draw(shell, ws, &painter, view);
    }
    let hover = response
        .hover_pos()
        .map(|p| shell.camera.to_picture(view, shell.view, p));
    super::mask::draw(shell, ws, &painter, view, hover);

    // The picture point under the pointer, for the status bar.
    shell.hover = pointer.map(|p| {
        let rect = match shell.compare {
            Compare::SideBySide => {
                let (left, right) = view.split_left_right_at_fraction(0.5);
                if left.contains(p) { left } else { right }
            }
            _ => view,
        };
        shell.camera.to_picture(rect, shell.view, p)
    });
}

/// Pan (`Space` + drag, middle drag, scroll) and zoom (pinch, `Cmd` + wheel) in the view.
fn navigate(shell: &mut Shell, ui: &Ui, response: &egui::Response, view: Rect) {
    let mode = shell.view;
    let (scroll, zoom, space) = ui.input(|i| {
        (
            i.smooth_scroll_delta,
            i.zoom_delta(),
            i.key_down(egui::Key::Space),
        )
    });
    if response.hovered() {
        if zoom != 1.0 {
            if let Some(p) = response.hover_pos() {
                shell.camera.zoom_about(view, mode, zoom, p);
            }
        } else if scroll != egui::Vec2::ZERO {
            shell.camera.pan(mode, scroll);
        }
    }
    let panning = response.dragged_by(egui::PointerButton::Middle)
        || ((space || shell.hand) && response.dragged_by(egui::PointerButton::Primary));
    if panning {
        shell.camera.pan(mode, response.drag_delta());
    }
}

/// One image, `mode`'s, clipped by `painter`, with the overlays. The geometry is always
/// the view mode's, so a compare or a `\` flip shows the other image in the same place.
fn draw(shell: &Shell, ws: &Workspace, painter: &Painter, view: Rect, mode: ViewMode) {
    let theme = Theme::current(painter.ctx());
    let picture = shell.camera.picture_rect(view, shell.view);
    let scale = shell.camera.scale(shell.view);
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    match mode {
        ViewMode::Source => {
            checker(painter, picture.intersect(view), theme);
            if let Some(tex) = ws.source_texture() {
                painter.image(tex.id(), picture, uv, Color32::WHITE);
            }
        }
        ViewMode::Result => {
            if shell.overlays.border {
                painter.rect_filled(picture.expand(BORDER * scale), 0.0, ws.border_colour());
            }
            match ws.result_texture() {
                Some(tex) => {
                    painter.image(tex.id(), picture, uv, Color32::WHITE);
                    // A full search running for a while: the last result, slightly faded
                    // (UX §11).
                    let now = painter.ctx().input(|i| i.time);
                    if ws.solving && ws.solve_started.is_some_and(|t| now - t > 0.2) {
                        painter.rect_filled(picture, 0.0, theme.canvas_bg.gamma_multiply(0.25));
                    }
                }
                None => {
                    painter.rect_filled(picture, 0.0, theme.canvas_bg);
                    painter.text(
                        picture.center(),
                        egui::Align2::CENTER_CENTER,
                        "Solving…",
                        egui::TextStyle::Body.resolve(&painter.ctx().global_style()),
                        theme.accent,
                    );
                }
            }
        }
    }
    if shell.overlays.ghost {
        let other = match mode {
            ViewMode::Source => ws.result_texture(),
            ViewMode::Result => ws.source_texture(),
        };
        if let Some(tex) = other {
            painter.image(
                tex.id(),
                picture,
                uv,
                Color32::from_white_alpha((GHOST * 255.0) as u8),
            );
        }
    }
    let grid = ws.doc.grid();
    let to_view = |r: Rect| {
        Rect::from_min_max(
            picture.min + r.min.to_vec2() * scale,
            picture.min + r.max.to_vec2() * scale,
        )
    };
    if shell.overlays.clashes
        && let Some((errors, max)) = ws.cell_errors()
        && max > 0.0
    {
        let cols = grid.cells().0;
        for (i, e) in errors.iter().enumerate() {
            if *e > 0.0 {
                let cell = (i as u32 % cols, i as u32 / cols);
                let alpha = 0.6 * e / max;
                painter.rect_filled(
                    to_view(cell_rect(grid, cell)),
                    0.0,
                    theme.clash.gamma_multiply(alpha),
                );
            }
        }
        // Colour-starved cells outlined.
        for cell in ws.clash_cells() {
            painter.rect_stroke(
                to_view(cell_rect(grid, cell)),
                0.0,
                (1.0, theme.clash),
                StrokeKind::Inside,
            );
        }
    }
    if shell.overlays.grid {
        grid_lines(painter, grid, picture, scale, theme.cell_grid);
    }
    if shell.overlays.steering {
        let (pins, locks) = ws.steered_cells();
        for (cells, colour, glyph) in [
            (pins, theme.pin, icons::PIN),
            (locks, theme.lock, icons::LOCK),
        ] {
            for cell in cells {
                let r = to_view(cell_rect(grid, cell));
                painter.rect_stroke(r, 0.0, (1.5, colour), StrokeKind::Inside);
                // The glyph at the top right once a cell is big enough to hold it.
                if r.width() >= GLYPH_MIN_CELL {
                    let font = egui::FontId::new(r.width() * 0.45, glyph.font_family());
                    let at = r.right_top() + vec2(-1.0, 1.0);
                    painter.text(at, egui::Align2::RIGHT_TOP, glyph.codepoint, font, colour);
                }
            }
        }
    }
}

/// The transparency checkerboard under the source, in fixed-size squares.
fn checker(painter: &Painter, area: Rect, theme: &Theme) {
    if !area.is_positive() {
        return;
    }
    painter.rect_filled(area, 0.0, theme.checker_a);
    let x0 = (area.left() / CHECKER).floor() as i64;
    let y0 = (area.top() / CHECKER).floor() as i64;
    let x1 = (area.right() / CHECKER).ceil() as i64;
    let y1 = (area.bottom() / CHECKER).ceil() as i64;
    let painter = painter.with_clip_rect(area.intersect(painter.clip_rect()));
    for y in y0..y1 {
        for x in (x0..x1).filter(|x| (x + y).rem_euclid(2) == 1) {
            let r = Rect::from_min_size(
                pos2(x as f32 * CHECKER, y as f32 * CHECKER),
                vec2(CHECKER, CHECKER),
            );
            painter.rect_filled(r, 0.0, theme.checker_b);
        }
    }
}

/// The cell grid over the picture, when cells are big enough to see.
fn grid_lines(
    painter: &Painter,
    grid: convert64_core::SuperGrid,
    picture: Rect,
    scale: f32,
    colour: Color32,
) {
    let cell = cell_rect(grid, (0, 0)).size() * scale;
    if cell.x.min(cell.y) < GRID_MIN_CELL {
        return;
    }
    let (cols, rows) = grid.cells();
    let stroke = Stroke::new(1.0, colour);
    for c in 0..=cols {
        let x = (picture.left() + c as f32 * cell.x).round() + 0.5;
        painter.vline(x, picture.y_range(), stroke);
    }
    for r in 0..=rows {
        let y = (picture.top() + r as f32 * cell.y).round() + 0.5;
        painter.hline(picture.x_range(), y, stroke);
    }
}

/// Split mode's divider: a line with a handle, dragged to move.
fn divider(shell: &mut Shell, ui: &mut Ui, view: Rect, x: f32) {
    let handle = Rect::from_center_size(pos2(x, view.center().y), vec2(8.0, view.height()));
    let response = ui
        .interact(handle, ui.id().with("split"), Sense::drag())
        .on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
    if response.dragged() {
        shell.split =
            ((x + response.drag_delta().x - view.left()) / view.width()).clamp(0.05, 0.95);
    }
    let accent = Theme::current(ui.ctx()).accent;
    ui.painter()
        .vline(x, view.y_range(), Stroke::new(2.0, accent));
    ui.painter()
        .circle_filled(pos2(x, view.center().y), 5.0, accent);
}

fn crosshair(painter: &Painter, p: Pos2, colour: Color32) {
    let s = Stroke::new(1.0, colour);
    painter.line_segment([p - vec2(8.0, 0.0), p + vec2(8.0, 0.0)], s);
    painter.line_segment([p - vec2(0.0, 8.0), p + vec2(0.0, 8.0)], s);
}

/// The empty state (UX §11): what to do first.
fn empty(ui: &mut Ui) {
    let open = ui.ctx().format_shortcut(&super::shortcuts::OPEN);
    ui.centered_and_justified(|ui| {
        ui.vertical_centered(|ui| {
            let block = 3.0 * crate::theme::layout::TOOLBAR_HEIGHT;
            ui.add_space((ui.available_height() - block).max(0.0) / 2.0);
            ui.label(
                icon(icons::ADD_IMAGE)
                    .size(2.0 * crate::theme::layout::ICON_SIZE)
                    .weak(),
            );
            ui.label(RichText::new("Open an image or a .convert64 project").strong());
            ui.label(RichText::new(format!("File › Open… ({open}), or drop a file here")).weak());
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The first frame after an open fits the Source view with a margin around the
    /// picture (#246).
    #[test]
    fn the_first_frame_after_an_open_fits_with_a_margin() {
        let rgba = (0..64 * 40)
            .flat_map(|i| [(i % 64 * 4) as u8, 90, 90, 255])
            .collect();
        let png = convert64_project::codec::encode_rgba8(64, 40, rgba);
        let doc = crate::document::Document::from_png("ramp.png", &png).unwrap();
        let ws = Workspace::new(doc);
        let ctx = egui::Context::default();
        let mut shell = Shell::default();
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 700.0))),
            ..Default::default()
        };
        let mut actions = Vec::new();
        let mut output = ctx.run_ui(input, |ui| show(&mut shell, ui, Some(&ws), &mut actions));
        output.textures_delta.clear();
        let view = shell.view_rect.expect("the working view was laid out");
        let picture = shell.camera.picture_rect(view, ViewMode::Source);
        assert!(view.contains_rect(picture), "{view:?} {picture:?}");
        let gap = ((view.width() - picture.width()) / view.width())
            .min((view.height() - picture.height()) / view.height())
            / 2.0;
        assert!((0.04..=0.06).contains(&gap), "{gap}");
    }
}
