//! The steering tools in the working view (UX §7): Pin `P` and Lock `L` (click a cell,
//! drag a rectangle), Nudge `V` (click selects the layer under the cursor, drag moves
//! it, sub-pixel), the right-click cell menu and worst-cell navigation. Every edit is a
//! registry command; a rectangle is one undo step, a drag is one gesture.

use super::{Action, Shell, Tool};
use crate::theme::Theme;
use crate::view::{Compare, cell_rect, target_at};
use crate::workspace::{Cell, Workspace};
use convert64_core::ColourIndex;
use convert64_core::command::{CellClear, CellLock, CellPin};
use convert64_core::model::CellPos;
use convert64_project::{JsonCommand, LayerRef};
use egui::{Pos2, Rect, Response, Ui};

/// Gesture id of a nudge drag.
const NUDGE_GESTURE: u64 = 0x6e75_6467;

/// A drag in progress, in screen units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ToolDrag {
    /// Where it started.
    pub start: Pos2,
    /// Where the pointer is.
    pub now: Pos2,
}

fn pos(c: Cell) -> CellPos {
    CellPos {
        x: c.0 as u16,
        y: c.1 as u16,
    }
}

/// The cells of the rectangle between two picture points, row by row, cut to the
/// picture. A rectangle wholly outside it (a click on the border or the pasteboard) has
/// none: it used to clamp to the edge cell and pin or lock that (#242).
pub(super) fn cells_between(ws: &Workspace, a: Pos2, b: Pos2) -> Vec<Cell> {
    let grid = ws.doc.grid();
    let r = Rect::from_two_pos(a, b);
    let screen = crate::view::SCREEN;
    if r.max.x < 0.0 || r.max.y < 0.0 || r.min.x >= screen.x || r.min.y >= screen.y {
        return Vec::new();
    }
    let clamp = |p: Pos2| {
        p.clamp(
            Pos2::ZERO,
            (crate::view::SCREEN - egui::vec2(0.01, 0.01)).to_pos2(),
        )
    };
    let (Some((_, c0)), Some((_, c1))) = (target_at(grid, clamp(a)), target_at(grid, clamp(b)))
    else {
        return Vec::new();
    };
    let (x0, x1) = (c0.0.min(c1.0), c0.0.max(c1.0));
    let (y0, y1) = (c0.1.min(c1.1), c0.1.max(c1.1));
    (y0..=y1)
        .flat_map(|y| (x0..=x1).map(move |x| (x, y)))
        .collect()
}

/// Commands that pin each cell to the colours it has now (its own set; the background
/// is global), one undo step.
pub(super) fn pin_current(ws: &Workspace, cells: &[Cell]) -> Vec<JsonCommand> {
    let Some(sol) = &ws.solution else {
        return Vec::new();
    };
    let cols = ws.doc.grid().cells().0;
    cells
        .iter()
        .filter_map(|&c| {
            let colours: Vec<ColourIndex> =
                sol.sets.get((c.1 * cols + c.0) as usize)?.iter().collect();
            (!colours.is_empty()).then(|| {
                JsonCommand::CellPin(CellPin {
                    cells: vec![pos(c)],
                    colours,
                    full: true,
                })
            })
        })
        .collect()
}

/// The topmost visible layer whose box contains picture point `p`.
fn layer_at(ws: &Workspace, p: Pos2) -> Option<usize> {
    let layers = &ws.doc.session().project().layers;
    layers.iter().enumerate().rev().find_map(|(i, l)| {
        let t = l.transform();
        let (w, h) = l.source().size();
        let min = egui::pos2(t.translate().x, t.translate().y);
        let size = egui::vec2(w as f32 * t.scale().x.abs(), h as f32 * t.scale().y.abs());
        (l.visible() && Rect::from_min_size(min, size).contains(p)).then_some(i)
    })
}

/// Handles the active tool's clicks and drags on the working view's `response`.
pub(super) fn interact(
    shell: &mut Shell,
    ws: &Workspace,
    ui: &Ui,
    response: &Response,
    view: Rect,
    actions: &mut Vec<Action>,
) {
    if shell.compare == Compare::SideBySide {
        return;
    }
    let space = ui.input(|i| i.key_down(egui::Key::Space));
    let to_picture = |p: Pos2| shell.camera.to_picture(view, shell.view, p);
    let primary = response.dragged_by(egui::PointerButton::Primary) && !space;

    if response.drag_started_by(egui::PointerButton::Primary)
        && !space
        && let Some(p) = response.interact_pointer_pos()
    {
        let p = to_picture(p);
        shell.drag = Some(ToolDrag { start: p, now: p });
        if shell.tool == Tool::Nudge
            && let Some(i) = layer_at(ws, p)
        {
            shell.layer = i;
        }
    }
    if primary && let (Some(drag), Some(p)) = (&mut shell.drag, response.interact_pointer_pos()) {
        drag.now = to_picture(p);
        if shell.tool == Tool::Nudge {
            let by = drag.now - drag.start;
            let by = convert64_core::Vec2::new(by.x, by.y);
            let cmd = JsonCommand::LayerNudge {
                layer: LayerRef::Index(shell.layer),
                by,
            };
            actions.push(Action::Gesture(NUDGE_GESTURE, cmd));
        }
    }
    if response.drag_stopped()
        && let Some(drag) = shell.drag.take()
    {
        let cells = cells_between(ws, drag.start, drag.now);
        match shell.tool {
            Tool::Pin => pin(ws, &cells, actions),
            Tool::Lock => lock(ws, &cells, actions),
            Tool::Nudge => actions.push(Action::EndGesture),
            Tool::Region | Tool::Transform => {}
        }
    }
    if response.clicked()
        && let Some(p) = response.interact_pointer_pos()
    {
        let p = to_picture(p);
        let alt = ui.input(|i| i.modifiers.alt);
        match shell.tool {
            Tool::Pin if alt => pin_pixel(ws, p, actions),
            Tool::Pin => pin(ws, &cells_between(ws, p, p), actions),
            Tool::Lock => lock(ws, &cells_between(ws, p, p), actions),
            Tool::Nudge | Tool::Transform => {
                if let Some(i) = layer_at(ws, p) {
                    shell.layer = i;
                }
            }
            Tool::Region => {}
        }
    }
    if response.secondary_clicked()
        && let Some(p) = response.interact_pointer_pos()
    {
        shell.menu_cell = target_at(ws.doc.grid(), to_picture(p)).map(|(_, c)| c);
    }
    response.context_menu(|ui| cell_menu(shell, ws, ui, actions));
}

fn pin(ws: &Workspace, cells: &[Cell], actions: &mut Vec<Action>) {
    let cmds = pin_current(ws, cells);
    if !cmds.is_empty() {
        let n = cmds.len();
        actions.push(Action::RunAll(cmds, format!("Pinned {n} {}", plural(n))));
    }
}

/// `Alt`-click with Pin: the colour under the cursor must stay in the cell; the solver
/// picks the rest.
fn pin_pixel(ws: &Workspace, p: Pos2, actions: &mut Vec<Action>) {
    let (Some(sol), Some(((x, y), cell))) = (&ws.solution, target_at(ws.doc.grid(), p)) else {
        return;
    };
    let colour = sol.image.pixel(x as u16, y as u16);
    if sol.background == Some(colour) {
        return;
    }
    let cmd = JsonCommand::CellPin(CellPin {
        cells: vec![pos(cell)],
        colours: vec![colour],
        full: false,
    });
    actions.push(Action::RunAll(
        vec![cmd],
        format!("Pinned colour {} in 1 cell", colour.get()),
    ));
}

/// Locks the cells, or unlocks them if they are all locked already.
fn lock(ws: &Workspace, cells: &[Cell], actions: &mut Vec<Action>) {
    if cells.is_empty() {
        return;
    }
    let locks = ws.doc.session().project().steering.locks();
    let locked = !cells.iter().all(|&c| locks.contains(pos(c)));
    let n = cells.len();
    let cmd = JsonCommand::CellLock(CellLock {
        cells: cells.iter().map(|&c| pos(c)).collect(),
        locked,
    });
    let verb = if locked { "Locked" } else { "Unlocked" };
    actions.push(Action::RunAll(
        vec![cmd],
        format!("{verb} {n} {}", plural(n)),
    ));
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "cell" } else { "cells" }
}

/// The right-click menu (UX §7): Pin colours · Lock · Clear pin/lock · Jump to worst
/// neighbour · Copy cell colours.
fn cell_menu(shell: &mut Shell, ws: &Workspace, ui: &mut Ui, actions: &mut Vec<Action>) {
    let Some(cell) = shell.menu_cell else {
        ui.label("Not on the picture");
        return;
    };
    ui.label(
        egui::RichText::new(format!("Cell {},{}", cell.0, cell.1))
            .small()
            .weak(),
    );
    if ui.button("Pin colours").clicked() {
        pin(ws, &[cell], actions);
        ui.close();
    }
    let locked = ws
        .doc
        .session()
        .project()
        .steering
        .locks()
        .contains(pos(cell));
    if ui.button(if locked { "Unlock" } else { "Lock" }).clicked() {
        lock(ws, &[cell], actions);
        ui.close();
    }
    if ui.button("Clear pin/lock").clicked() {
        let cmd = JsonCommand::CellClear(CellClear {
            cells: vec![pos(cell)],
        });
        actions.push(Action::RunAll(vec![cmd], "Cleared 1 cell".to_owned()));
        ui.close();
    }
    if ui.button("Jump to worst neighbour").clicked() {
        if let Some(n) = worst_neighbour(ws, cell) {
            shell.go_to_cell(ws, n);
        }
        ui.close();
    }
    if ui.button("Copy cell colours").clicked() {
        let info = ws.cell_info(cell);
        let names: Vec<String> = info
            .colours
            .iter()
            .map(|c| format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b()))
            .collect();
        ui.ctx().copy_text(names.join(" "));
        ui.close();
    }
}

/// The neighbour (of eight) with the largest error.
fn worst_neighbour(ws: &Workspace, cell: Cell) -> Option<Cell> {
    let (errors, _) = ws.cell_errors()?;
    let (cols, rows) = ws.doc.grid().cells();
    let mut best: Option<(f32, Cell)> = None;
    for dy in -1i64..=1 {
        for dx in -1i64..=1 {
            let (x, y) = (i64::from(cell.0) + dx, i64::from(cell.1) + dy);
            if (dx, dy) == (0, 0) || x < 0 || y < 0 || x >= i64::from(cols) || y >= i64::from(rows)
            {
                continue;
            }
            let e = errors[(y as u32 * cols + x as u32) as usize];
            if best.is_none_or(|(b, _)| e > b) {
                best = Some((e, (x as u32, y as u32)));
            }
        }
    }
    best.map(|(_, c)| c)
}

/// Draws the drag rectangle of Pin and Lock.
pub(super) fn draw_drag(shell: &Shell, ws: &Workspace, painter: &egui::Painter, view: Rect) {
    let Some(drag) = shell.drag else { return };
    if !matches!(shell.tool, Tool::Pin | Tool::Lock) {
        return;
    }
    let cells = cells_between(ws, drag.start, drag.now);
    let (Some(first), Some(last)) = (cells.first(), cells.last()) else {
        return;
    };
    let grid = ws.doc.grid();
    let r = cell_rect(grid, *first).union(cell_rect(grid, *last));
    let picture = shell.camera.picture_rect(view, shell.view);
    let s = shell.camera.scale(shell.view);
    let on_screen = Rect::from_min_max(
        picture.min + r.min.to_vec2() * s,
        picture.min + r.max.to_vec2() * s,
    );
    let theme = Theme::current(painter.ctx());
    let colour = if shell.tool == Tool::Pin {
        theme.pin
    } else {
        theme.lock
    };
    painter.rect_stroke(on_screen, 0.0, (2.0, colour), egui::StrokeKind::Inside);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use convert64_core::solve::SolveMode;

    fn solved() -> Workspace {
        let rgba = (0..64 * 40)
            .flat_map(|i| [(i % 64 * 4) as u8, (i / 64 * 6) as u8, 90, 255])
            .collect();
        let png = convert64_project::codec::encode_rgba8(64, 40, rgba);
        let mut ws = Workspace::new(Document::from_png("ramp.png", &png).unwrap());
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

    #[test]
    fn a_drag_covers_the_cells_between_its_ends() {
        let ws = solved();
        // Multicolor cells are 8 × 8 screen units.
        let cells = cells_between(&ws, egui::pos2(20.0, 3.0), egui::pos2(4.0, 17.0));
        assert_eq!(
            cells,
            vec![
                (0, 0),
                (1, 0),
                (2, 0),
                (0, 1),
                (1, 1),
                (2, 1),
                (0, 2),
                (1, 2),
                (2, 2)
            ]
        );
        assert_eq!(
            cells_between(&ws, egui::pos2(-5.0, -5.0), egui::pos2(1.0, 1.0)),
            vec![(0, 0)]
        );
    }

    /// #242: a click outside the picture has no cells (it used to clamp to the edge).
    #[test]
    fn a_click_outside_the_picture_has_no_cells() {
        let ws = solved();
        for p in [
            (-1.0, 50.0),
            (50.0, -0.5),
            (320.0, 10.0),
            (330.0, 210.0),
            (10.0, 200.0),
        ] {
            let p = egui::pos2(p.0, p.1);
            assert_eq!(cells_between(&ws, p, p), vec![], "{p:?}");
        }
        // A rectangle wholly on the border has none either.
        let (a, b) = (egui::pos2(-20.0, -20.0), egui::pos2(-2.0, 100.0));
        assert_eq!(cells_between(&ws, a, b), vec![]);
        // The edge cells are still there.
        let corner = egui::pos2(319.5, 199.5);
        assert_eq!(cells_between(&ws, corner, corner), vec![(39, 24)]);
    }

    /// Clicks with `tool` at `inside` and then `outside` the picture (in Result mode,
    /// whose border is part of the working view) and returns each click's actions.
    fn click_in_and_out(tool: Tool) -> (Vec<Action>, Vec<Action>) {
        let mut ws = solved();
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let mut shell = Shell {
            tool,
            view: crate::view::ViewMode::Result,
            ..Shell::default()
        };
        let mut frame = |shell: &mut Shell, events: Vec<egui::Event>| {
            let input = egui::RawInput {
                events,
                ..Default::default()
            };
            let mut actions = Vec::new();
            let mut out = ctx.run_ui(input, |ui| actions = shell.show(ui, Some(&mut ws)));
            out.textures_delta.clear();
            actions
        };
        frame(&mut shell, vec![]);
        frame(&mut shell, vec![]);
        let view = shell.view_rect.expect("the working view was drawn");
        let picture = shell.camera.picture_rect(view, shell.view);
        // 20 screen units into the 32-unit border: beyond the Region brush (6) too.
        let outside = picture.left_top() - egui::Vec2::splat(20.0 * shell.camera.scale(shell.view));
        assert!(view.contains(outside));
        let mut click = |shell: &mut Shell, pos: Pos2| {
            let button = |pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            frame(shell, vec![egui::Event::PointerMoved(pos), button(true)]);
            let actions = frame(shell, vec![button(false)]);
            frame(shell, vec![egui::Event::PointerGone]);
            actions
        };
        let inside = click(&mut shell, picture.left_top() + egui::vec2(4.0, 4.0));
        let outside = click(&mut shell, outside);
        (inside, outside)
    }

    /// #242: Pin, Lock and Region act on a click inside the picture and do nothing on
    /// a click on the border around it.
    #[test]
    fn a_click_on_the_border_pins_locks_and_paints_nothing() {
        for tool in [Tool::Pin, Tool::Lock, Tool::Region] {
            let (inside, outside) = click_in_and_out(tool);
            assert!(
                matches!(inside.as_slice(), [Action::RunAll(..)]),
                "{tool:?} inside: {inside:?}"
            );
            assert!(outside.is_empty(), "{tool:?} outside: {outside:?}");
        }
    }

    /// #242: the default tool is Nudge (Move), and a new document goes back to it.
    #[test]
    fn the_default_tool_is_nudge() {
        assert_eq!(Shell::default().tool, Tool::Nudge);
        assert_eq!(Tool::Nudge.key(), egui::Key::V);
        let mut shell = Shell {
            tool: Tool::Pin,
            ..Shell::default()
        };
        shell.toggle_mask_mode();
        shell.reset_tool();
        assert_eq!(shell.tool, Tool::Nudge);
        assert!(!shell.mask.active && shell.mask.tool.is_none());
    }

    #[test]
    fn pinning_keeps_each_cell_s_current_colours_in_one_step() {
        let mut ws = solved();
        let cmds = pin_current(&ws, &[(0, 0), (5, 3)]);
        assert_eq!(cmds.len(), 2);
        ws.doc.run_all(cmds).unwrap();
        let steering = &ws.doc.session().project().steering;
        assert_eq!(steering.pins().count(), 2);
        assert!(ws.doc.undo());
        assert_eq!(ws.doc.session().project().steering.pins().count(), 0);
    }
}
