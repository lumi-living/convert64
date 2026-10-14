//! The Transform tool (UX §7, ADR-011; `S` or `Cmd T`): the selected layer's rotated
//! box with eight handles and its anchor. Inside moves, a corner scales uniformly
//! (`Shift`: free aspect), an edge scales one axis, just outside a corner rotates
//! (`Shift`: 15° steps), the anchor moves the pivot; the wheel over the box scales ±1 %.
//! Hold `Cmd` to invert the project's Snap for one drag. A drag is one gesture (one undo
//! step, the fast path while dragging); `Esc` reverts it.

use super::{Action, Shell};
use crate::theme::Theme;
use crate::workspace::Workspace;
use convert64_core::Vec2;
use convert64_core::layer::{Snap, Transform};
use convert64_project::{JsonCommand, LayerRef};
use egui::{Pos2, Rect, Response, Ui, pos2};

/// Gesture id of a transform drag.
const GESTURE: u64 = 0x7472_616e;

/// Handle hit radius, in points: larger than the 7 pt square drawn, so a handle is
/// easy to catch (Figma's handles have a larger invisible hit area too; #239).
const HANDLE: f32 = 9.0;

/// How far outside a corner the rotate zone reaches, in points.
const ROTATE_ZONE: f32 = 22.0;

/// What a drag grabbed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Grab {
    /// The box: move.
    Move,
    /// A corner (0–3, source corner order): uniform scale.
    Corner(usize),
    /// An edge (0 top, 1 right, 2 bottom, 3 left): one axis.
    Edge(usize),
    /// Just outside a corner: rotate.
    Rotate,
    /// The anchor: move the pivot.
    Anchor,
}

/// A transform drag in progress.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TransformDrag {
    grab: Grab,
    start: Pos2,
    before: Transform,
}

/// The snap a command uses: the project's, inverted while `Cmd` is held.
fn snap(ws: &Workspace, invert: bool) -> Option<Snap> {
    let s = ws.doc.session().project().snap;
    invert.then_some(if s == Snap::Off {
        Snap::C64Pixel
    } else {
        Snap::Off
    })
}

fn v2(p: Pos2) -> Vec2 {
    Vec2::new(p.x, p.y)
}

fn p2(v: Vec2) -> Pos2 {
    pos2(v.x, v.y)
}

/// The layer's corners, edge midpoints and anchor, in screen points.
fn handles(
    shell: &Shell,
    view: Rect,
    t: &Transform,
    size: (u32, u32),
) -> ([Pos2; 4], [Pos2; 4], Pos2) {
    let picture = shell.camera.picture_rect(view, shell.view);
    let s = shell.camera.scale(shell.view);
    let at = |v: Vec2| picture.min + p2(v).to_vec2() * s;
    let quad = t.canvas_quad(size).map(at);
    let mid = [0, 1, 2, 3].map(|i| quad[i].lerp(quad[(i + 1) % 4], 0.5));
    let a = t.anchor();
    let anchor = at(t.layer_to_canvas(size, Vec2::new(a.x * size.0 as f32, a.y * size.1 as f32)));
    (quad, mid, anchor)
}

fn grab_at(quad: &[Pos2; 4], mid: &[Pos2; 4], anchor: Pos2, p: Pos2) -> Option<Grab> {
    if anchor.distance(p) < HANDLE {
        return Some(Grab::Anchor);
    }
    if let Some(i) = quad.iter().position(|c| c.distance(p) < HANDLE) {
        return Some(Grab::Corner(i));
    }
    if let Some(i) = mid.iter().position(|c| c.distance(p) < HANDLE) {
        return Some(Grab::Edge(i));
    }
    if inside(quad, p) {
        return Some(Grab::Move);
    }
    quad.iter()
        .any(|c| c.distance(p) < ROTATE_ZONE)
        .then_some(Grab::Rotate)
}

/// Whether screen point `p` is over the selected layer's box (the wheel scales it there
/// instead of panning).
pub(super) fn over_box(shell: &Shell, ws: &Workspace, view: Rect, p: Pos2) -> bool {
    let Some(l) = ws.doc.session().project().layers.get(shell.layer) else {
        return false;
    };
    let (quad, _, _) = handles(shell, view, &l.transform(), l.source().size());
    inside(&quad, p)
}

/// Whether `p` is inside the convex quad.
fn inside(quad: &[Pos2; 4], p: Pos2) -> bool {
    let sign = |a: Pos2, b: Pos2| (b - a).x * (p - a).y - (b - a).y * (p - a).x;
    let s: Vec<f32> = (0..4).map(|i| sign(quad[i], quad[(i + 1) % 4])).collect();
    s.iter().all(|v| *v >= 0.0) || s.iter().all(|v| *v <= 0.0)
}

/// The transform a drag from `d.start` to `now` (both screen units) makes.
fn dragged(d: &TransformDrag, now: Pos2, size: (u32, u32), shift: bool) -> Option<Transform> {
    let t = d.before;
    let (w, h) = (size.0 as f32, size.1 as f32);
    let a = Vec2::new(t.anchor().x * w, t.anchor().y * h);
    match d.grab {
        Grab::Move => t.nudged(v2(now) - v2(d.start)).ok(),
        Grab::Rotate => {
            let pivot = p2(t.layer_to_canvas(size, a));
            let angle = |p: Pos2| (p.y - pivot.y).atan2(p.x - pivot.x).to_degrees();
            let mut deg = t.rotate() + angle(now) - angle(d.start);
            if shift {
                deg = (deg / 15.0).round() * 15.0;
            }
            let deg = (deg + 180.0).rem_euclid(360.0) - 180.0;
            t.with_rotation(deg).ok()
        }
        Grab::Anchor => {
            let q = t.canvas_to_layer(size, v2(now));
            t.reanchored(
                Vec2::new((q.x / w).clamp(0.0, 1.0), (q.y / h).clamp(0.0, 1.0)),
                size,
            )
            .ok()
        }
        Grab::Corner(_) | Grab::Edge(_) => {
            // In the layer's own (unrotated) pixels, relative to the pivot.
            let q0 = t.canvas_to_layer(size, v2(d.start)) - a;
            let q = t.canvas_to_layer(size, v2(now)) - a;
            let ratio = |n: f32, d: f32| {
                if d.abs() < 1e-3 {
                    1.0
                } else {
                    (n / d).max(0.01)
                }
            };
            let (fx, fy) = (ratio(q.x, q0.x), ratio(q.y, q0.y));
            let (fx, fy) = match d.grab {
                Grab::Edge(0 | 2) => (1.0, fy),
                Grab::Edge(_) => (fx, 1.0),
                _ if shift => (fx, fy),
                _ => {
                    let u = (fx * fy).sqrt();
                    (u, u)
                }
            };
            let s = t.scale();
            t.scaled_about_anchor(Vec2::new(s.x * fx, s.y * fy), size)
                .ok()
        }
    }
}

/// Handles the Transform tool on the working view.
pub(super) fn interact(
    shell: &mut Shell,
    ws: &Workspace,
    ui: &Ui,
    response: &Response,
    view: Rect,
    actions: &mut Vec<Action>,
) {
    let Some(layer) = ws.doc.session().project().layers.get(shell.layer) else {
        return;
    };
    let size = layer.source().size();
    let t = layer.transform();
    let (quad, mid, anchor) = handles(shell, view, &t, size);
    let to_picture = |p: Pos2| shell.camera.to_picture(view, shell.view, p);
    let (shift, cmd) = ui.input(|i| (i.modifiers.shift, i.modifiers.command));
    let id = LayerRef::Index(shell.layer);

    if let Some(p) = response.hover_pos()
        && let Some(grab) = grab_at(&quad, &mid, anchor, p)
    {
        let icon = match grab {
            Grab::Move | Grab::Anchor => egui::CursorIcon::Move,
            Grab::Rotate => egui::CursorIcon::Alias,
            Grab::Corner(_) => egui::CursorIcon::ResizeNwSe,
            Grab::Edge(0 | 2) => egui::CursorIcon::ResizeVertical,
            Grab::Edge(_) => egui::CursorIcon::ResizeHorizontal,
        };
        ui.ctx().set_cursor_icon(icon);
        // The wheel over the box scales ±1 % per notch.
        let scroll: f32 = ui.input(|i| {
            i.raw
                .events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::MouseWheel { delta, .. } => Some(delta.y),
                    _ => None,
                })
                .sum()
        });
        if scroll != 0.0 && inside(&quad, p) {
            let f = if scroll > 0.0 { 1.01 } else { 1.0 / 1.01 };
            if let Ok(t2) = t.scaled_about_anchor(Vec2::new(t.scale().x * f, t.scale().y * f), size)
            {
                let cmd = JsonCommand::LayerSetTransform {
                    layer: id.clone(),
                    transform: t2,
                    snap: snap(ws, cmd),
                };
                actions.push(Action::Gesture(GESTURE + 1, cmd));
            }
        }
    }
    // What was grabbed is decided where the button went down: egui reports the drag only
    // once the pointer has moved past its drag threshold, by which time it can be off
    // the handle (#239).
    if response.drag_started_by(egui::PointerButton::Primary)
        && let Some(p) = super::drag_origin(ui, response)
        && let Some(grab) = grab_at(&quad, &mid, anchor, p)
    {
        shell.transform_drag = Some(TransformDrag {
            grab,
            start: to_picture(p),
            before: t,
        });
    }
    if response.dragged_by(egui::PointerButton::Primary)
        && let (Some(d), Some(p)) = (shell.transform_drag, response.interact_pointer_pos())
        && let Some(t2) = dragged(&d, to_picture(p), size, shift)
    {
        let cmd = JsonCommand::LayerSetTransform {
            layer: id.clone(),
            transform: t2,
            snap: snap(ws, cmd),
        };
        actions.push(Action::Gesture(GESTURE, cmd));
    }
    if response.drag_stopped() && shell.transform_drag.take().is_some() {
        actions.push(Action::EndGesture);
    }
    if shell.transform_drag.is_some() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        // Esc reverts the whole drag (UX §7): it was one gesture.
        shell.transform_drag = None;
        actions.push(Action::CancelGesture);
    }
}

/// Draws the box, its handles and the anchor.
pub(super) fn draw(shell: &Shell, ws: &Workspace, painter: &egui::Painter, view: Rect) {
    let Some(layer) = ws.doc.session().project().layers.get(shell.layer) else {
        return;
    };
    let (quad, mid, anchor) = handles(shell, view, &layer.transform(), layer.source().size());
    let accent = Theme::current(painter.ctx()).accent;
    painter.add(egui::Shape::closed_line(
        quad.to_vec(),
        egui::Stroke::new(1.5, accent),
    ));
    let fill = painter.ctx().global_style().visuals.panel_fill;
    for p in quad.iter().chain(&mid) {
        let r = Rect::from_center_size(*p, egui::vec2(7.0, 7.0));
        painter.rect_filled(r, 0.0, fill);
        painter.rect_stroke(r, 0.0, (1.5, accent), egui::StrokeKind::Inside);
    }
    painter.circle_stroke(anchor, 5.0, (1.5, accent));
    painter.line_segment(
        [anchor - egui::vec2(8.0, 0.0), anchor + egui::vec2(8.0, 0.0)],
        (1.0, accent),
    );
    painter.line_segment(
        [anchor - egui::vec2(0.0, 8.0), anchor + egui::vec2(0.0, 8.0)],
        (1.0, accent),
    );
}

/// The status bar readout while transforming: X/Y, W/H % and the angle.
pub(super) fn readout(t: &Transform) -> String {
    let (p, s) = (t.translate(), t.scale());
    format!(
        "X {:.2} Y {:.2} · W {:.2} % H {:.2} % · {:.2}°",
        p.x,
        p.y,
        s.x * 100.0,
        s.y * 100.0,
        t.rotate()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t() -> Transform {
        Transform::new(Vec2::new(10.0, 20.0), Vec2::new(0.5, 0.5), false, false).unwrap()
    }

    #[test]
    fn a_corner_drag_scales_uniformly_about_the_anchor() {
        let size = (100, 60);
        let before = t();
        // The bottom-right corner is at (60, 50); the anchor (centre) at (35, 35).
        let d = TransformDrag {
            grab: Grab::Corner(2),
            start: pos2(60.0, 50.0),
            before,
        };
        let after = dragged(&d, pos2(85.0, 65.0), size, false).unwrap();
        assert!(
            (after.scale().x - 1.0).abs() < 1e-4 && (after.scale().y - 1.0).abs() < 1e-4,
            "{after:?}"
        );
        let pivot = |t: &Transform| t.layer_to_canvas(size, Vec2::new(50.0, 30.0));
        let (a, b) = (pivot(&before), pivot(&after));
        assert!((a.x - b.x).abs() < 1e-3 && (a.y - b.y).abs() < 1e-3);
    }

    #[test]
    fn rotating_with_shift_lands_on_15_degree_steps() {
        let size = (100, 60);
        let d = TransformDrag {
            grab: Grab::Rotate,
            start: pos2(60.0, 35.0),
            before: t(),
        };
        // A quarter turn clockwise about the centre (35, 35), slightly off.
        let after = dragged(&d, pos2(36.0, 60.0), size, true).unwrap();
        assert_eq!(after.rotate(), 90.0);
        let free = dragged(&d, pos2(36.0, 60.0), size, false).unwrap();
        assert!((free.rotate() - 90.0).abs() > 0.5);
    }

    /// A layer of 64×40 placed at (100, 60) at 2×: a 128×80 box well inside the screen.
    fn workspace() -> Workspace {
        let rgba = (0..64 * 40)
            .flat_map(|i| [(i % 64 * 4) as u8, 90, 90, 255])
            .collect();
        let png = convert64_project::codec::encode_rgba8(64, 40, rgba);
        let mut doc = crate::document::Document::from_png("ramp.png", &png).unwrap();
        let transform =
            Transform::new(Vec2::new(100.0, 60.0), Vec2::new(2.0, 2.0), false, false).unwrap();
        doc.run(JsonCommand::LayerSetTransform {
            layer: LayerRef::Index(0),
            transform,
            snap: None,
        })
        .unwrap();
        Workspace::new(doc)
    }

    /// The working view in the headless runs, in points.
    fn view() -> Rect {
        super::super::drag_test::VIEW
    }

    /// Presses the primary button at `from`, moves through `path` one frame per point
    /// and releases (`drag_test::drag`); returns the transform the last gesture command
    /// set, if any.
    fn drag(ws: &Workspace, from: Pos2, path: &[Pos2]) -> Option<Transform> {
        let mut shell = Shell {
            tool: super::super::Tool::Transform,
            ..Shell::default()
        };
        let mut set = None;
        super::super::drag_test::drag(from, path, |ui, response| {
            let mut actions = Vec::new();
            interact(&mut shell, ws, ui, response, view(), &mut actions);
            for a in actions {
                if let Action::Gesture(GESTURE, JsonCommand::LayerSetTransform { transform, .. }) =
                    a
                {
                    set = Some(transform);
                }
            }
        });
        set
    }

    /// The handles of the test layer, in points.
    fn test_handles(ws: &Workspace) -> ([Pos2; 4], [Pos2; 4], Pos2) {
        let l = &ws.doc.session().project().layers[0];
        handles(&Shell::default(), view(), &l.transform(), l.source().size())
    }

    /// A path from `from` towards `to`: two small steps inside egui's 6 pt drag threshold,
    /// then one past the 9 pt handle radius (where the old code decided the grab), then
    /// the target.
    fn path(from: Pos2, to: Pos2) -> Vec<Pos2> {
        super::super::drag_test::path(from, to)
    }

    #[test]
    fn a_corner_drag_from_the_press_point_scales_about_the_anchor() {
        let ws = workspace();
        let before = ws.doc.session().project().layers[0].transform();
        let size = (64, 40);
        let (quad, _, _) = test_handles(&ws);
        // Grab the bottom-right corner 3 pt off its centre and pull it outwards; the
        // drag starts only once the pointer is 12 pt away, off the handle.
        let from = quad[2] + egui::vec2(3.0, 2.0);
        let after = drag(&ws, from, &path(from, from + egui::vec2(64.0, 40.0))).unwrap();
        // Uniform: both axes grow by the same factor, and the layer did not just move.
        let (s0, s1) = (before.scale(), after.scale());
        assert!(s1.x > s0.x * 1.2, "{after:?}");
        assert!((s1.x / s0.x - s1.y / s0.y).abs() < 1e-3, "{after:?}");
        // About the anchor (the centre): it stays where it was on the canvas.
        let pivot = |t: &Transform| t.layer_to_canvas(size, Vec2::new(32.0, 20.0));
        let (a, b) = (pivot(&before), pivot(&after));
        assert!(
            (a.x - b.x).abs() < 1e-3 && (a.y - b.y).abs() < 1e-3,
            "{a:?} {b:?}"
        );
    }

    #[test]
    fn an_edge_drag_from_the_press_point_scales_one_axis() {
        let ws = workspace();
        let before = ws.doc.session().project().layers[0].transform();
        let (_, mid, _) = test_handles(&ws);
        // The right edge's handle, pulled outwards.
        let from = mid[1] + egui::vec2(-2.0, 3.0);
        let after = drag(&ws, from, &path(from, from + egui::vec2(60.0, 0.0))).unwrap();
        assert!(after.scale().x > before.scale().x * 1.2, "{after:?}");
        assert_eq!(after.scale().y, before.scale().y);
        // The bottom edge, pulled up: only H changes.
        let from = mid[2];
        let after = drag(&ws, from, &path(from, from - egui::vec2(0.0, 40.0))).unwrap();
        assert!(after.scale().y < before.scale().y * 0.8, "{after:?}");
        assert_eq!(after.scale().x, before.scale().x);
    }

    #[test]
    fn a_drag_from_inside_the_box_moves_by_the_whole_distance() {
        let ws = workspace();
        let before = ws.doc.session().project().layers[0].transform();
        let (quad, _, _) = test_handles(&ws);
        // A quarter of the way in from the top-left corner: inside, off every handle.
        let from = quad[0].lerp(quad[2], 0.25);
        let after = drag(&ws, from, &path(from, from + egui::vec2(40.0, 20.0))).unwrap();
        assert_eq!(after.scale(), before.scale());
        // 40 × 20 points at the default 2× zoom is 20 × 10 screen units, counted from
        // where the button went down.
        let moved = after.translate() - before.translate();
        assert!(
            (moved.x - 20.0).abs() < 1e-3 && (moved.y - 10.0).abs() < 1e-3,
            "{moved:?}"
        );
    }

    #[test]
    fn handles_catch_the_pointer_within_their_hit_radius() {
        let ws = workspace();
        let (quad, mid, anchor) = test_handles(&ws);
        let off = egui::vec2(6.0, 5.0); // 7.8 pt: outside the old 6 pt radius.
        assert_eq!(
            grab_at(&quad, &mid, anchor, quad[1] + off),
            Some(Grab::Corner(1))
        );
        assert_eq!(
            grab_at(&quad, &mid, anchor, mid[3] - off),
            Some(Grab::Edge(3))
        );
    }

    #[test]
    fn an_edge_drag_changes_one_axis() {
        let size = (100, 60);
        let d = TransformDrag {
            grab: Grab::Edge(1),
            start: pos2(60.0, 35.0),
            before: t(),
        };
        let after = dragged(&d, pos2(85.0, 40.0), size, false).unwrap();
        assert!((after.scale().x - 1.0).abs() < 1e-4);
        assert_eq!(after.scale().y, 0.5);
    }
}
