//! The working view (`docs/UX.md` §4): the source composite or the result, zoomed and
//! panned, with the overlays and the compare modes. Without a project it shows the empty
//! state.

use super::{Shell, ViewMode, icon};
use crate::icons;
use crate::theme::Theme;
use crate::view::{BORDER, Compare, SCREEN, cell_rect};
use crate::workspace::{SourcePicture, Workspace};
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
        empty(ui, actions);
        return;
    };
    let view = ui.max_rect();
    let response = ui.allocate_rect(view, Sense::click_and_drag());
    // Screen readers: the view is one focusable area with a name (UX §12).
    let name = format!("Working view, {}", shell.view.label());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Other, true, &name));
    shell.view_rect = Some(view);
    shell.canvas_id = Some(response.id);
    if ui.input(|i| i.pointer.any_pressed()) && response.contains_pointer() {
        shell.key_focus = super::KeyFocus::Canvas;
    }
    if shell.fit_pending {
        shell.fit_pending = false;
        let border = shell.overlays.border;
        shell.camera.fit(view, ViewMode::Source, border);
        shell.camera.fit(view, ViewMode::Result, border);
    }
    let wheel_scales = shell.transform_handles()
        && response
            .hover_pos()
            .is_some_and(|p| super::transform::over_box(shell, ws, view, p));
    if !wheel_scales {
        navigate(shell, ui, &response, view);
    }
    if shell.hand {
        // The hand tool only pans.
    } else if shell.transform_handles() {
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
    if shell.tool == super::Tool::Region && shell.mask.tool.is_none() {
        region_feedback(shell, ws, &painter, view, response.hover_pos());
    }
    // The quick-mask overlay (UX §7a): over the picture, in the view mode's geometry.
    if let Some(tex) = shell.mask_overlay {
        let picture = shell.camera.picture_rect(view, shell.view);
        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        painter.image(tex, picture, uv, Color32::WHITE);
    }
    super::tools::draw_drag(shell, ws, &painter, view);
    if shell.transform_handles() {
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
/// The Region tool's feedback (UX §7, usability review M7): each region outlined in the
/// `region` colour (the selected one thicker), the stroke being painted as a swath in
/// that colour before its release, and the brush under the pointer.
fn region_feedback(
    shell: &Shell,
    ws: &Workspace,
    painter: &Painter,
    view: Rect,
    hover: Option<Pos2>,
) {
    let picture = shell.camera.picture_rect(view, shell.view);
    let s = shell.camera.scale(shell.view);
    let at = |p: Pos2| picture.min + p.to_vec2() * s;
    let theme = Theme::current(painter.ctx());
    let mut shapes = Vec::new();
    for (k, outline) in ws.region_outlines().iter().enumerate() {
        let width = if k == shell.region { 2.0 } else { 1.0 };
        let stroke = Stroke::new(width, theme.region);
        shapes.extend(
            outline
                .iter()
                .map(|[a, b]| egui::Shape::line_segment([at(*a), at(*b)], stroke)),
        );
    }
    painter.extend(shapes);
    let radius = shell.mask.region.size * s;
    let swath = theme.region.gamma_multiply(0.45);
    match shell.mask.stroke.as_slice() {
        [] => {}
        [p] => {
            painter.circle_filled(at(*p), radius, swath);
        }
        points => {
            painter.add(egui::Shape::line(
                points.iter().map(|p| at(*p)).collect(),
                Stroke::new(2.0 * radius, swath),
            ));
        }
    }
    if let Some(h) = hover {
        painter.circle_stroke(h, radius, (1.5, theme.accent));
    }
}

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
        shell.camera.pan(mode, super::drag_step(response));
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
            source(ws, painter, picture);
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

/// The Source view's picture in `picture` (#264): the 4× composite; during a slider drag
/// the solve's 1× composite; during a transform drag the cached 4× textures, the moved
/// layer's through the drag's affine map (an egui mesh), the others where they are.
fn source(ws: &Workspace, painter: &Painter, picture: Rect) {
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    match ws.source_picture() {
        None => {}
        Some(SourcePicture::Flat(tex)) => {
            painter.image(tex.id(), picture, uv, Color32::WHITE);
        }
        Some(SourcePicture::Moving(m)) => {
            // The canvas ends at the screen's edge; the moved layer must not draw past it.
            let painter = painter.with_clip_rect(painter.clip_rect().intersect(picture));
            let scale = picture.width() / SCREEN.x;
            let quad = m.quad.map(|p| picture.min + p.to_vec2() * scale);
            if let Some(tex) = m.backdrop {
                painter.image(tex.id(), picture, uv, Color32::WHITE);
            }
            // The layers below and above stay put. When the moved layer's texture lacks what
            // lay past the screen, they are drawn only where it has its pixels, and the
            // backdrop shows elsewhere.
            let fixed = |tex: &egui::TextureHandle| {
                if m.clipped {
                    let uv = m.quad.map(|p| pos2(p.x / SCREEN.x, p.y / SCREEN.y));
                    painter.add(quad_mesh(tex.id(), quad, uv));
                } else {
                    painter.image(tex.id(), picture, uv, Color32::WHITE);
                }
            };
            if let Some(tex) = m.below {
                fixed(tex);
            }
            let corners = [
                pos2(0.0, 0.0),
                pos2(1.0, 0.0),
                pos2(1.0, 1.0),
                pos2(0.0, 1.0),
            ];
            painter.add(quad_mesh(m.layer.id(), quad, corners));
            if let Some(tex) = m.above {
                fixed(tex);
            }
        }
    }
}

/// A textured parallelogram: `pos` and `uv` are its corners in order.
fn quad_mesh(texture: egui::TextureId, pos: [Pos2; 4], uv: [Pos2; 4]) -> egui::Shape {
    let mut mesh = egui::Mesh::with_texture(texture);
    for (pos, uv) in pos.into_iter().zip(uv) {
        mesh.vertices.push(egui::epaint::Vertex {
            pos,
            uv,
            color: Color32::WHITE,
        });
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    egui::Shape::mesh(mesh)
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

/// The empty state's sentence (UX §5).
pub(super) const EMPTY_TEXT: &str =
    "Drop an image here or open one. Convert64 shows the C64 result as soon as it loads.";

/// The empty state (UX §5, §11; usability review M10, #292): what to do first, and two
/// ways to do it: the primary **Open image…** and the **Try a sample picture** link.
fn empty(ui: &mut Ui, actions: &mut Vec<super::Action>) {
    use crate::theme::layout::{ICON_SIZE, TOOLBAR_HEIGHT};
    let open = crate::shell::shortcuts::format(ui.ctx(), &super::shortcuts::OPEN);
    let rect = ui.max_rect();
    let glyph = 2.0 * ICON_SIZE;
    let button = vec2(160.0, TOOLBAR_HEIGHT);
    // The glyph, two lines of text, the button and the link, centred as one block.
    let block = glyph + 3.0 * TOOLBAR_HEIGHT + button.y;
    let column = Rect::from_center_size(rect.center(), vec2(rect.width().min(420.0), block));
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(column)
            .layout(egui::Layout::top_down(egui::Align::Center)),
        |ui| {
            ui.label(icon(icons::ADD_IMAGE).size(glyph).weak());
            ui.add(egui::Label::new(RichText::new(EMPTY_TEXT)).wrap());
            ui.add_space(ui.spacing().item_spacing.y * 2.0);
            let primary = super::primary_button(ui, "Open image…");
            if ui
                .add_sized(button, primary)
                .on_hover_text(format!("Open a picture or a .convert64 project ({open})"))
                .clicked()
            {
                actions.push(super::Action::Open);
            }
            if ui
                .link("Try a sample picture")
                .on_hover_text("Open the title painting that comes with Convert64")
                .clicked()
            {
                actions.push(super::Action::OpenSample);
            }
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #263: a pan with the Hand moves the picture by the whole drag, counting the
    /// movement inside egui's drag threshold.
    #[test]
    fn a_hand_drag_pans_by_the_whole_distance() {
        use super::super::drag_test::{VIEW, drag, path};
        let mut shell = Shell {
            hand: true,
            ..Shell::default()
        };
        let before = shell.camera.centre;
        let from = VIEW.center();
        drag(
            from,
            &path(from, from + vec2(40.0, 20.0)),
            |ui, response| {
                navigate(&mut shell, ui, response, VIEW);
            },
        );
        // 40 × 20 points at the default 2× zoom is 20 × 10 screen units.
        let moved = before - shell.camera.centre;
        assert!(
            (moved.x - 20.0).abs() < 1e-3 && (moved.y - 10.0).abs() < 1e-3,
            "{moved:?}"
        );
    }

    /// Usability review m3 (UX §4.1): the Transform tool's box and 8 handles are drawn in
    /// Source mode only; in Result mode, or while `\` shows the result, there are none.
    #[test]
    fn transform_handles_are_drawn_in_source_mode_only() {
        let rgba = (0..64 * 40)
            .flat_map(|i| [(i % 64 * 4) as u8, 90, 90, 255])
            .collect();
        let png = convert64_project::codec::encode_rgba8(64, 40, rgba);
        let ws = Workspace::new(crate::document::Document::from_png("ramp.png", &png).unwrap());
        let handles = |view: ViewMode, flip: bool| {
            let ctx = egui::Context::default();
            crate::theme::install(&ctx);
            let mut shell = Shell {
                tool: super::super::Tool::Transform,
                view,
                flip_held: flip,
                ..Shell::default()
            };
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 700.0))),
                ..Default::default()
            };
            let mut actions = Vec::new();
            let mut output = ctx.run_ui(input, |ui| show(&mut shell, ui, Some(&ws), &mut actions));
            output.textures_delta.clear();
            fn count(shape: &egui::Shape) -> usize {
                match shape {
                    egui::Shape::Rect(r) if r.rect.size() == vec2(7.0, 7.0) => 1,
                    egui::Shape::Vec(v) => v.iter().map(count).sum(),
                    _ => 0,
                }
            }
            let n: usize = output.shapes.iter().map(|s| count(&s.shape)).sum();
            (n, shell.transform_handles())
        };
        // Each handle is a filled square and its outline: 8 handles, 16 squares.
        assert_eq!(handles(ViewMode::Source, false), (16, true));
        assert_eq!(handles(ViewMode::Result, false), (0, false));
        assert_eq!(
            handles(ViewMode::Source, true),
            (0, false),
            "\\ shows the result"
        );
        assert_eq!(
            handles(ViewMode::Result, true),
            (16, true),
            "\\ shows the source"
        );
    }

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
        // Result opens at the largest integer zoom with half the border in view (#247).
        let z = shell.camera.result_zoom;
        let framed = |z: u8| {
            (crate::view::SCREEN + egui::Vec2::splat(2.0 * crate::view::FIT_BORDER)) * f32::from(z)
        };
        let fits = |z: u8| framed(z).x <= view.width() && framed(z).y <= view.height();
        assert!(fits(z), "{z}× in {view:?}");
        let next = crate::view::RESULT_STEPS.into_iter().find(|&s| s > z);
        assert!(next.is_none_or(|n| !fits(n)), "{z}× in {view:?}");
    }
}
