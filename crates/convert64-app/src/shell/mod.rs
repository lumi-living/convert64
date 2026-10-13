//! The app shell (`docs/UX.md` §3, Figma `App Shell / Light` and `/ Dark`): menu bar,
//! toolbar, the working view, the right dock (Result on top, then the Recipe, Layers and
//! Export tabs) and the status bar.
//!
//! [`Shell`] holds what the chrome shows and [`Shell::show`] draws it, returning the
//! [`Action`]s the user asked for this frame; the app carries them out. Sizes come from
//! [`crate::theme::layout`] and colours from [`crate::theme::Theme`].

mod dock;
mod export_dialog;
mod layers;
mod mask;
mod menu;
mod recipe;
pub mod shortcuts;
mod status;
mod toolbar;
mod tools;
mod working;

use crate::icons::Icon;
use crate::theme::layout;
use crate::view::{Camera, Compare, Overlays};
use crate::workspace::Workspace;
use convert64_core::mode::Mode;

pub use crate::view::ViewMode;
pub use export_dialog::ExportDialog;
pub use mask::{MaskState, MaskTool};

/// The four steering tools (`docs/UX.md` §7), in toolbar order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tool {
    /// Pin a cell's colour set.
    #[default]
    Pin,
    /// Lock a cell.
    Lock,
    /// Paint a region.
    Region,
    /// Nudge a layer.
    Nudge,
}

impl Tool {
    /// Every tool, in toolbar order.
    pub const ALL: [Self; 4] = [Self::Pin, Self::Lock, Self::Region, Self::Nudge];

    /// The tool's name.
    pub fn label(self) -> &'static str {
        match self {
            Self::Pin => "Pin",
            Self::Lock => "Lock",
            Self::Region => "Region",
            Self::Nudge => "Nudge",
        }
    }

    /// The tool's icon.
    pub fn icon(self) -> Icon {
        match self {
            Self::Pin => crate::icons::PIN,
            Self::Lock => crate::icons::LOCK,
            Self::Region => crate::icons::REGION,
            Self::Nudge => crate::icons::NUDGE,
        }
    }

    /// The tool's single-key shortcut (`docs/UX.md` §8).
    pub fn key(self) -> egui::Key {
        match self {
            Self::Pin => egui::Key::P,
            Self::Lock => egui::Key::L,
            Self::Region => egui::Key::R,
            Self::Nudge => egui::Key::V,
        }
    }
}

/// The dock's tabs, under the result (Figma `Dock Tabs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DockTab {
    /// The selected layer's recipe and adjustments.
    #[default]
    Recipe,
    /// The layer stack.
    Layers,
    /// Export formats and settings.
    Export,
}

impl DockTab {
    /// Every tab, in order.
    pub const ALL: [Self; 3] = [Self::Recipe, Self::Layers, Self::Export];

    /// The tab's name.
    pub fn label(self) -> &'static str {
        match self {
            Self::Recipe => "Recipe",
            Self::Layers => "Layers",
            Self::Export => "Export",
        }
    }
}

/// Something the user asked for that the shell cannot do itself.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Close the app (native) or do nothing (web).
    Quit,
    /// Follow the system theme, or force light or dark (View › Theme).
    SetTheme(egui::ThemePreference),
    /// Change the target mode (the result header's dropdown): `mode_set`, then a full
    /// solve.
    SetMode(Mode),
    /// File › Open… (`Cmd O`).
    Open,
    /// File › Save (`Cmd S`): to the project's file, or as Save As.
    Save,
    /// File › Save As… (`Shift Cmd S`).
    SaveAs,
    /// Export with these settings, files named after the stem (the dialog, or Export
    /// Again).
    Export(crate::export::Settings, String),
    /// Run a registry command (ADR-005).
    Run(convert64_project::JsonCommand),
    /// Run several commands as one undo step, then show the message ("Pinned 7 cells").
    RunAll(Vec<convert64_project::JsonCommand>, String),
    /// Run a command that is part of a drag: one undo step per drag (UX §11).
    Gesture(u64, convert64_project::JsonCommand),
    /// The drag ended.
    EndGesture,
    /// Undo (`Cmd Z`).
    Undo,
    /// Redo (`Shift Cmd Z`, `Cmd Y`).
    Redo,
    /// Layer › New Layer from File… (`Shift Cmd N`): pick a picture, add it on top.
    NewLayer,
    /// Show or hide the selected layer (`Cmd ,`).
    ToggleVisible(usize),
    /// Load a greyscale PNG as the selected layer's mask.
    LoadMask,
}

/// The smallest and largest dock width (`docs/UX.md` §3: 260–420 px). Not a Figma
/// variable yet; see the #203 follow-ups.
pub const DOCK_WIDTH_RANGE: std::ops::RangeInclusive<f32> = 260.0..=420.0;

/// What the chrome shows: the working view's mode, camera, compare mode and overlays,
/// the tool, the dock tab and what is hidden. Everything else lives in the project.
#[derive(Debug, Clone, PartialEq)]
pub struct Shell {
    /// Source or Result.
    pub view: ViewMode,
    /// The active steering tool.
    pub tool: Tool,
    /// The dock tab under the result.
    pub tab: DockTab,
    /// `Tab`: menu, toolbar, dock and status bar hidden.
    pub chrome_hidden: bool,
    /// `Shift Tab`: the dock (and with it the result) hidden.
    pub dock_hidden: bool,
    /// Where the working view looks and how close.
    pub camera: Camera,
    /// The working view's overlays.
    pub overlays: Overlays,
    /// How the working view compares source and result.
    pub compare: Compare,
    /// The split divider's position, as a fraction of the view's width.
    pub split: f32,
    /// The theme preference View › Theme shows as chosen.
    pub theme: egui::ThemePreference,
    /// The selected layer (stack index, bottom first): what the Recipe tab edits and
    /// nudging moves.
    pub layer: usize,
    /// The layer being renamed and the name typed so far.
    renaming: Option<(usize, String)>,
    /// The curve the Recipe tab's curves editor shows: master, red, green, blue.
    curve_channel: usize,
    /// A Pin, Lock or Nudge drag in progress.
    drag: Option<tools::ToolDrag>,
    /// The cell the right-click menu is for.
    menu_cell: Option<crate::workspace::Cell>,
    /// The cell worst-cell navigation last went to.
    nav_cell: Option<crate::workspace::Cell>,
    /// `Shift Cmd ]` (true) or `[` (false) this frame.
    nav_request: Option<bool>,
    /// Mask mode, the mask tools and the selection (UX §7a).
    pub mask: MaskState,
    /// This frame's quick-mask overlay texture, in mask mode.
    mask_overlay: Option<egui::TextureId>,
    /// Whether Edit › Undo and Redo have something to do (set by the app each frame).
    pub history: (bool, bool),
    /// `\` is held: the working view shows the other image.
    flip_held: bool,
    /// Fit the picture once the working view's size is known (after an open).
    fit_pending: bool,
    /// The working view's rectangle last frame, for fit.
    view_rect: Option<egui::Rect>,
    /// The picture point under the pointer, in screen units.
    hover: Option<egui::Pos2>,
    /// The export dialog, while open.
    pub export: Option<ExportDialog>,
    /// `Cmd E` this frame: open the dialog once the workspace is at hand.
    export_requested: bool,
    /// The last export, for Export Again.
    pub last_export: Option<(crate::export::Settings, String)>,
    /// A message shown at the bottom of the window until the time (egui seconds) passes.
    toast: Option<(String, bool, f64)>,
}

impl Default for Shell {
    fn default() -> Self {
        Self {
            view: ViewMode::default(),
            tool: Tool::default(),
            tab: DockTab::default(),
            chrome_hidden: false,
            dock_hidden: false,
            camera: Camera::default(),
            overlays: Overlays::default(),
            compare: Compare::default(),
            split: 0.5,
            theme: egui::ThemePreference::System,
            layer: 0,
            renaming: None,
            curve_channel: 0,
            drag: None,
            menu_cell: None,
            nav_cell: None,
            nav_request: None,
            mask: MaskState::default(),
            mask_overlay: None,
            history: (false, false),
            flip_held: false,
            fit_pending: true,
            view_rect: None,
            hover: None,
            export: None,
            export_requested: false,
            last_export: None,
            toast: None,
        }
    }
}

impl Shell {
    /// Shows `message` for a few seconds (UX §11: 3 s for success); errors stay 6 s and
    /// are drawn in the error colour.
    pub fn toast(&mut self, ctx: &egui::Context, message: impl Into<String>, error: bool) {
        let now = ctx.input(|i| i.time);
        self.toast = Some((message.into(), error, now + if error { 6.0 } else { 3.0 }));
        ctx.request_repaint_after(std::time::Duration::from_secs(3));
    }

    /// Opens the export dialog for `ws`.
    pub fn open_export(&mut self, ws: &Workspace) {
        let (settings, name) = self
            .last_export
            .clone()
            .unwrap_or_else(|| (crate::export::Settings::default(), ws.doc.stem()));
        self.export = Some(ExportDialog { settings, name });
    }

    /// Fits the picture into the working view on the next frame (after an open).
    pub fn fit_on_next_frame(&mut self) {
        self.fit_pending = true;
    }

    /// Enters or leaves mask mode on the selected layer (`M`); entering picks the brush.
    pub fn toggle_mask_mode(&mut self) {
        self.mask.active = !self.mask.active;
        if self.mask.active {
            self.mask.tool.get_or_insert(MaskTool::Brush);
            self.mask.selection = None;
        } else {
            self.mask.tool = None;
        }
    }

    /// Keeps the camera as set instead of fitting after an open (screenshots).
    pub fn keep_camera(&mut self) {
        self.fit_pending = false;
    }

    /// The image the working view shows now: the view mode, flipped while `\` is held.
    pub fn shown(&self) -> ViewMode {
        if self.flip_held {
            self.view.flipped()
        } else {
            self.view
        }
    }

    /// Zooms one step in or out in the current mode.
    pub fn zoom_step(&mut self, zoom_in: bool) {
        self.camera.step(self.view, zoom_in);
    }

    /// Fits the picture into the working view.
    pub fn fit(&mut self) {
        match self.view_rect {
            Some(rect) => self.camera.fit(rect, self.view),
            None => self.fit_pending = true,
        }
    }

    /// Centres the working view on `cell` in Result mode.
    pub fn go_to_cell(&mut self, ws: &Workspace, cell: crate::workspace::Cell) {
        self.view = ViewMode::Result;
        self.camera.centre = crate::view::cell_rect(ws.doc.grid(), cell).center();
        self.nav_cell = Some(cell);
    }

    /// The next-worst (`forward`) or previous cell after the last one visited, starting
    /// at the worst (`Shift Cmd ]` / `[`).
    fn step_worst(&mut self, ws: &Workspace, forward: bool) {
        let Some(sol) = &ws.solution else { return };
        let next = self
            .nav_cell
            .and_then(|(x, y)| {
                let pos = convert64_core::model::CellPos {
                    x: x as u16,
                    y: y as u16,
                };
                sol.report.step(pos, forward)
            })
            .or_else(|| sol.report.worst_first().first().copied());
        if let Some(c) = next {
            self.go_to_cell(ws, (u32::from(c.x), u32::from(c.y)));
        }
    }

    /// Shows the worst clash: Clashes overlay on, Result mode, centred on that cell (the
    /// result header's clash button; UX §4.3).
    pub fn show_worst(&mut self, ws: &Workspace) {
        self.overlays.clashes = true;
        self.view = ViewMode::Result;
        if let Some(cell) = ws.worst() {
            self.camera.centre = crate::view::cell_rect(ws.doc.grid(), cell).center();
        }
    }

    /// Draws the whole shell into `ui` (the app's root) and returns what the user asked
    /// for. Keyboard shortcuts are read first, unless a text field has focus.
    pub fn show(&mut self, ui: &mut egui::Ui, mut ws: Option<&mut Workspace>) -> Vec<Action> {
        let mut actions = Vec::new();
        self.handle_shortcuts(ui.ctx(), &mut actions);
        if let Some(ws) = ws.as_deref_mut() {
            ws.prepare(ui.ctx());
            if std::mem::take(&mut self.export_requested) {
                self.open_export(ws);
            }
            if let Some(forward) = self.nav_request.take() {
                self.step_worst(ws, forward);
            }
            self.mask_overlay = if self.mask.active {
                ws.mask_overlay(ui.ctx(), self.layer, self.mask.alone)
                    .map(egui::TextureHandle::id)
            } else {
                None
            };
        }
        self.export_requested = false;
        self.nav_request = None;

        if !self.chrome_hidden {
            egui::Panel::top("menu_bar")
                .show(ui, |ui| menu::show(self, ws.as_deref(), ui, &mut actions));
            egui::Panel::top("toolbar")
                .exact_size(layout::TOOLBAR_HEIGHT)
                .show(ui, |ui| toolbar::show(self, ui));
            if self.mask.tool.is_some() {
                egui::Panel::top("mask_options")
                    .exact_size(layout::TOOLBAR_HEIGHT)
                    .show(ui, |ui| {
                        mask::options_bar(self, ws.as_deref(), ui, &mut actions)
                    });
            }
            egui::Panel::bottom("status_bar")
                .exact_size(layout::STATUS_HEIGHT)
                .show(ui, |ui| status::show(self, ws.as_deref(), ui));
            if !self.dock_hidden {
                egui::Panel::right("dock")
                    .resizable(true)
                    .default_size(layout::DOCK_WIDTH)
                    .size_range(DOCK_WIDTH_RANGE)
                    .frame(egui::Frame::side_top_panel(ui.style()).inner_margin(0))
                    .show(ui, |ui| dock::show(self, ws.as_deref(), ui, &mut actions));
            }
        }

        let canvas = crate::theme::Theme::current(ui.ctx()).canvas_bg;
        egui::CentralPanel::default()
            .frame(
                egui::Frame::central_panel(ui.style())
                    .fill(canvas)
                    .inner_margin(0),
            )
            .show(ui, |ui| {
                working::show(self, ui, ws.as_deref(), &mut actions)
            });
        export_dialog::show(self, ws.as_deref(), ui.ctx(), &mut actions);
        self.show_toast(ui.ctx());
        actions
    }

    fn show_toast(&mut self, ctx: &egui::Context) {
        let Some((message, error, until)) = &self.toast else {
            return;
        };
        if ctx.input(|i| i.time) > *until {
            self.toast = None;
            return;
        }
        let theme = crate::theme::Theme::current(ctx);
        let colour = if *error { theme.error } else { theme.ok };
        egui::Area::new(egui::Id::new("toast"))
            .anchor(
                egui::Align2::CENTER_BOTTOM,
                [0.0, -2.0 * layout::STATUS_HEIGHT],
            )
            .interactable(false)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.colored_label(colour, message.as_str());
                });
            });
    }

    /// The shortcuts of `docs/UX.md` §8 that the shell itself handles.
    fn handle_shortcuts(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        use shortcuts as s;
        // Single keys only when no text field wants them.
        let typing = ctx.egui_wants_keyboard_input();
        ctx.input_mut(|i| {
            if i.consume_shortcut(&s::ZOOM_IN) {
                self.zoom_step(true);
            }
            if i.consume_shortcut(&s::ZOOM_OUT) {
                self.zoom_step(false);
            }
            if i.consume_shortcut(&s::ZOOM_FIT) {
                self.fit();
            }
            if i.consume_shortcut(&s::ZOOM_100) {
                self.camera.actual(self.view);
            }
            if i.consume_shortcut(&s::GRID) {
                self.overlays.grid = !self.overlays.grid;
            }
            if i.consume_shortcut(&s::BORDER) {
                self.overlays.border = !self.overlays.border;
            }
            if i.consume_shortcut(&s::QUIT) {
                actions.push(Action::Quit);
            }
            if i.consume_shortcut(&s::OPEN) {
                actions.push(Action::Open);
            }
            // Redo before Undo: Shift Cmd Z also matches Cmd Z.
            if i.consume_shortcut(&s::REDO) || i.consume_shortcut(&s::REDO_ALT) {
                actions.push(Action::Redo);
            } else if i.consume_shortcut(&s::UNDO) {
                actions.push(Action::Undo);
            }
            if i.consume_shortcut(&s::NEW_LAYER) {
                actions.push(Action::NewLayer);
            }
            if i.consume_shortcut(&s::DUPLICATE_LAYER) {
                let layer = convert64_project::LayerRef::Index(self.layer);
                actions.push(Action::Run(
                    convert64_project::JsonCommand::LayerDuplicate { layer },
                ));
            }
            if i.consume_shortcut(&s::TOGGLE_VISIBLE) {
                actions.push(Action::ToggleVisible(self.layer));
            }
            if i.consume_shortcut(&s::NEXT_WORST) {
                self.nav_request = Some(true);
            }
            if i.consume_shortcut(&s::PREVIOUS_WORST) {
                self.nav_request = Some(false);
            }
            // Shift Cmd S before Cmd S, as with Tab.
            if i.consume_shortcut(&s::SAVE_AS) {
                actions.push(Action::SaveAs);
            } else if i.consume_shortcut(&s::SAVE) {
                actions.push(Action::Save);
            }
            if i.consume_shortcut(&s::EXPORT_AGAIN) {
                if let Some((settings, name)) = self.last_export.clone() {
                    actions.push(Action::Export(settings, name));
                }
            } else if i.consume_shortcut(&s::EXPORT) {
                self.export_requested = true;
            }
            self.flip_held = !typing && i.key_down(egui::Key::Backslash);
            if typing {
                return;
            }
            if i.consume_shortcut(&s::CLASHES) {
                self.overlays.clashes = !self.overlays.clashes;
            }
            // Shift Tab before Tab: a plain-Tab pattern also matches with Shift held.
            if i.consume_shortcut(&s::HIDE_DOCK) {
                self.dock_hidden = !self.dock_hidden;
            } else if i.consume_shortcut(&s::HIDE_CHROME) {
                self.chrome_hidden = !self.chrome_hidden;
            }
            if i.consume_shortcut(&s::SOURCE_RESULT) {
                self.view = self.view.flipped();
            }
            if i.consume_shortcut(&s::COMPARE) {
                self.compare = self.compare.next();
            }
            if i.consume_shortcut(&s::DELETE_LAYER) || i.consume_shortcut(&s::DELETE_LAYER_ALT) {
                let layer = convert64_project::LayerRef::Index(self.layer);
                actions.push(Action::Run(convert64_project::JsonCommand::LayerRemove {
                    layer,
                }));
            }
            for tool in Tool::ALL {
                if i.consume_key(egui::Modifiers::NONE, tool.key()) {
                    self.tool = tool;
                    self.mask.tool = None;
                    self.mask.active = false;
                }
            }
            for tool in MaskTool::ALL {
                if i.consume_key(egui::Modifiers::NONE, tool.key()) {
                    self.mask.tool = Some(tool);
                }
            }
            if i.consume_shortcut(&s::MASK_MODE) {
                self.toggle_mask_mode();
            }
            if self.mask.tool.is_some() && i.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
                self.mask.active = false;
                self.mask.tool = None;
                self.mask.polygon.clear();
            }
            if i.consume_key(egui::Modifiers::NONE, egui::Key::X) {
                use convert64_core::mask::PaintMode;
                self.mask.mode = match self.mask.mode {
                    PaintMode::Add => PaintMode::Subtract,
                    PaintMode::Subtract => PaintMode::Add,
                };
            }
            if i.consume_key(egui::Modifiers::NONE, egui::Key::OpenBracket) {
                self.mask.size = (self.mask.size / 1.25).max(0.5);
            }
            if i.consume_key(egui::Modifiers::NONE, egui::Key::CloseBracket) {
                self.mask.size = (self.mask.size * 1.25).min(64.0);
            }
            // Arrows nudge the selected layer: 10 px with Shift, 0.25 px with Alt (UX §6).
            for (key, dx, dy) in [
                (egui::Key::ArrowLeft, -1.0, 0.0),
                (egui::Key::ArrowRight, 1.0, 0.0),
                (egui::Key::ArrowUp, 0.0, -1.0),
                (egui::Key::ArrowDown, 0.0, 1.0),
            ] {
                let step = if i.consume_key(egui::Modifiers::SHIFT, key) {
                    10.0
                } else if i.consume_key(egui::Modifiers::ALT, key) {
                    0.25
                } else if i.consume_key(egui::Modifiers::NONE, key) {
                    1.0
                } else {
                    continue;
                };
                let by = convert64_core::Vec2::new(dx * step, dy * step);
                let layer = convert64_project::LayerRef::Index(self.layer);
                actions.push(Action::Run(convert64_project::JsonCommand::LayerNudge {
                    layer,
                    by,
                }));
            }
        });
    }
}

/// An icon at the toolbar and row size (`layout::ICON_SIZE`).
pub(crate) fn icon(icon: Icon) -> egui::RichText {
    icon.rich_text().size(layout::ICON_SIZE)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs one frame of the shell with `events` and returns the shell afterwards.
    fn frame(mut shell: Shell, events: Vec<egui::Event>) -> (Shell, Vec<Action>) {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let input = egui::RawInput {
            events,
            ..Default::default()
        };
        let mut actions = Vec::new();
        let mut output = ctx.run_ui(input, |ui| {
            actions = shell.show(ui, None);
        });
        // No renderer here: drop the font atlas uploads deliberately.
        output.textures_delta.clear();
        (shell, actions)
    }

    fn key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    #[test]
    fn tab_hides_the_chrome_and_shift_tab_only_the_dock() {
        let (shell, _) = frame(
            Shell::default(),
            vec![key(egui::Key::Tab, egui::Modifiers::NONE)],
        );
        assert!(shell.chrome_hidden && !shell.dock_hidden);
        let (shell, _) = frame(
            Shell::default(),
            vec![key(egui::Key::Tab, egui::Modifiers::SHIFT)],
        );
        assert!(!shell.chrome_hidden && shell.dock_hidden);
    }

    #[test]
    fn q_flips_the_working_view_and_single_keys_pick_tools() {
        let (shell, _) = frame(
            Shell::default(),
            vec![
                key(egui::Key::Q, egui::Modifiers::NONE),
                key(egui::Key::V, egui::Modifiers::NONE),
            ],
        );
        assert_eq!(shell.view, ViewMode::Result);
        assert_eq!(shell.tool, Tool::Nudge);
    }

    #[test]
    fn command_shortcuts_zoom_and_toggle_overlays() {
        let (shell, _) = frame(
            Shell::default(),
            vec![
                key(egui::Key::Equals, egui::Modifiers::COMMAND),
                key(egui::Key::Quote, egui::Modifiers::COMMAND),
                key(egui::Key::C, egui::Modifiers::SHIFT),
                key(egui::Key::Comma, egui::Modifiers::NONE),
            ],
        );
        assert_eq!(shell.camera.source_zoom, 3.0);
        assert!(shell.overlays.grid && shell.overlays.clashes);
        assert_eq!(shell.compare, Compare::Split);
    }

    /// Every tab and view draws with an open document, in both themes, without panics
    /// and without commands nobody asked for.
    #[test]
    fn every_tab_draws_with_a_document() {
        let rgba = (0..64 * 40)
            .flat_map(|i| [(i % 64 * 4) as u8, (i / 64 * 6) as u8, 90, 255])
            .collect();
        let png = convert64_project::codec::encode_rgba8(64, 40, rgba);
        let doc = crate::document::Document::from_png("ramp.png", &png).unwrap();
        let mut ws = crate::workspace::Workspace::new(doc);
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        for theme in [egui::ThemePreference::Light, egui::ThemePreference::Dark] {
            ctx.set_theme(theme);
            for tab in DockTab::ALL {
                for view in [ViewMode::Source, ViewMode::Result] {
                    let mut shell = Shell {
                        tab,
                        view,
                        ..Shell::default()
                    };
                    let mut actions = Vec::new();
                    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                        actions = shell.show(ui, Some(&mut ws));
                    });
                    output.textures_delta.clear();
                    assert!(actions.is_empty(), "{tab:?} {view:?}: {actions:?}");
                }
            }
        }
    }

    /// Values a slider can only show rounded (strength 0.6 → 60.000004 %, opacity ⅓ →
    /// 33 %) must not count as edits, or the form re-sends them every frame.
    #[test]
    fn rounded_values_are_not_edits() {
        use convert64_core::command::RecipeChange;
        use convert64_project::{JsonCommand, LayerRef};
        let rgba = (0..64 * 40)
            .flat_map(|i| [(i % 64 * 4) as u8, 90, 90, 255])
            .collect();
        let png = convert64_project::codec::encode_rgba8(64, 40, rgba);
        let mut doc = crate::document::Document::from_png("ramp.png", &png).unwrap();
        let layer = LayerRef::Index(0);
        let recipe = RecipeChange {
            strength: Some(0.6),
            ..Default::default()
        };
        doc.run(JsonCommand::RecipeSet {
            layer: layer.clone(),
            recipe,
        })
        .unwrap();
        doc.run(JsonCommand::LayerSetOpacity {
            layer,
            opacity: 1.0 / 3.0,
        })
        .unwrap();
        let mut ws = crate::workspace::Workspace::new(doc);
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        for tab in [DockTab::Recipe, DockTab::Layers] {
            let mut shell = Shell {
                tab,
                ..Shell::default()
            };
            for _ in 0..3 {
                let mut actions = Vec::new();
                let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                    actions = shell.show(ui, Some(&mut ws));
                });
                out.textures_delta.clear();
                assert!(actions.is_empty(), "{tab:?}: {actions:?}");
            }
        }
    }
}
