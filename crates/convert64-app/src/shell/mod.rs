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
mod transform;
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
    /// Scale and rotate a layer (ADR-011).
    Transform,
}

impl Tool {
    /// Every tool, in toolbar order.
    pub const ALL: [Self; 5] = [
        Self::Pin,
        Self::Lock,
        Self::Region,
        Self::Nudge,
        Self::Transform,
    ];

    /// The tool's name.
    pub fn label(self) -> &'static str {
        match self {
            Self::Pin => "Pin",
            Self::Lock => "Lock",
            Self::Region => "Region",
            Self::Nudge => "Nudge",
            Self::Transform => "Transform",
        }
    }

    /// The tool's icon.
    pub fn icon(self) -> Icon {
        match self {
            Self::Pin => crate::icons::PIN,
            Self::Lock => crate::icons::LOCK,
            Self::Region => crate::icons::REGION,
            Self::Nudge => crate::icons::NUDGE,
            Self::Transform => crate::icons::TRANSFORM,
        }
    }

    /// The tool's single-key shortcut (`docs/UX.md` §8).
    pub fn key(self) -> egui::Key {
        match self {
            Self::Pin => egui::Key::P,
            Self::Lock => egui::Key::L,
            Self::Region => egui::Key::R,
            Self::Nudge => egui::Key::V,
            Self::Transform => egui::Key::S,
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
    /// Revert the drag in progress (`Esc` during a transform).
    CancelGesture,
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
    /// Load a VICE palette file (`.vpl`) as the palette.
    LoadPalette,
    /// Pause the full search that is running (UX §11).
    PauseSolve,
    /// Resume a paused solve.
    ResumeSolve,
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
    /// `Cmd \`: menu, toolbar, dock and status bar hidden; `Esc` shows them again.
    pub chrome_hidden: bool,
    /// `Shift Cmd \`: the dock (and with it the result) hidden.
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
    /// A Transform drag in progress.
    transform_drag: Option<transform::TransformDrag>,
    /// `Cmd [` / `]` this frame: scale the selected layer by this factor.
    scale_request: Option<f32>,
    /// `Shift Cmd ;` this frame: the next Snap setting.
    snap_cycle: bool,
    /// The Transform group's W/H chain: uniform scale (the default).
    pub scale_linked: bool,
    /// The region the Region tool paints and the Recipe tab edits.
    pub region: usize,
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
    /// This frame's region tint texture, with the Region tool.
    region_overlay: Option<egui::TextureId>,
    /// `H`: the hand tool, a primary drag pans.
    pub hand: bool,
    /// The clash hint was dismissed (UX §11: suggested once).
    clash_hint_dismissed: bool,
    /// The last error, shown in the status bar: summary and details for its popover.
    pub error: Option<(String, String)>,
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
            transform_drag: None,
            scale_request: None,
            snap_cycle: false,
            scale_linked: true,
            region: 0,
            menu_cell: None,
            nav_cell: None,
            nav_request: None,
            mask: MaskState::default(),
            mask_overlay: None,
            region_overlay: None,
            hand: false,
            clash_hint_dismissed: false,
            error: None,
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
    /// Shows a file or command error in the status bar, with `details` in its popover
    /// (UX §11: inline, never a modal).
    pub fn report_error(&mut self, summary: impl Into<String>, details: impl Into<String>) {
        self.error = Some((summary.into(), details.into()));
    }

    /// Shows `message` for a few seconds (UX §11: 3 s for success); errors stay 6 s and
    /// are drawn in the error colour.
    pub fn toast(&mut self, ctx: &egui::Context, message: impl Into<String>, error: bool) {
        let now = ctx.input(|i| i.time);
        self.toast = Some((message.into(), error, now + if error { 6.0 } else { 3.0 }));
        ctx.request_repaint_after(std::time::Duration::from_secs(3));
    }

    /// Hides or shows the menu bar, toolbar, dock and status bar (`Cmd \`, View › Hide
    /// UI). Hiding says how to get them back, since nothing on screen does (#240).
    pub fn set_ui_hidden(&mut self, ctx: &egui::Context, hidden: bool) {
        self.chrome_hidden = hidden;
        if hidden {
            let keys = ctx.format_shortcut(&shortcuts::HIDE_UI);
            self.toast(
                ctx,
                format!("UI hidden — press {keys} or Esc to show it"),
                false,
            );
        } else if self
            .toast
            .as_ref()
            .is_some_and(|t| t.0.starts_with("UI hidden"))
        {
            self.toast = None;
        }
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
            Some(rect) => self.camera.fit(rect, self.view, self.overlays.border),
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
            if std::mem::take(&mut self.snap_cycle) {
                use convert64_core::layer::Snap;
                let snap = match ws.doc.session().project().snap {
                    Snap::Off => Snap::C64Pixel,
                    Snap::C64Pixel => Snap::Cell,
                    Snap::Cell => Snap::Off,
                };
                actions.push(Action::Run(convert64_project::JsonCommand::SnapSet(
                    convert64_core::command::SnapSet { snap },
                )));
            }
            if let Some(f) = self.scale_request.take()
                && let Some(l) = ws.doc.session().project().layers.get(self.layer)
            {
                let t = l.transform();
                let s = t.scale();
                if let Ok(transform) = t.scaled_about_anchor(
                    convert64_core::Vec2::new(s.x * f, s.y * f),
                    l.source().size(),
                ) {
                    let layer = convert64_project::LayerRef::Index(self.layer);
                    actions.push(Action::Run(
                        convert64_project::JsonCommand::LayerSetTransform {
                            layer,
                            transform,
                            snap: None,
                        },
                    ));
                }
            }
            self.region_overlay = if self.tool == Tool::Region && self.mask.tool.is_none() {
                ws.region_overlay(ui.ctx()).map(egui::TextureHandle::id)
            } else {
                None
            };
            self.mask_overlay = if self.mask.active {
                ws.mask_overlay(ui.ctx(), self.layer, self.mask.alone)
                    .map(egui::TextureHandle::id)
            } else {
                None
            };
        }
        self.export_requested = false;
        self.nav_request = None;
        self.scale_request = None;
        self.snap_cycle = false;

        if !self.chrome_hidden {
            egui::Panel::top("menu_bar")
                .show(ui, |ui| menu::show(self, ws.as_deref(), ui, &mut actions));
            egui::Panel::top("toolbar")
                .exact_size(layout::TOOLBAR_HEIGHT)
                .show(ui, |ui| {
                    toolbar::show(self, ws.as_deref(), ui, &mut actions)
                });
            if self.tool == Tool::Region && self.mask.tool.is_none() {
                egui::Panel::top("region_options")
                    .exact_size(layout::TOOLBAR_HEIGHT)
                    .show(ui, |ui| mask::region_bar(self, ui));
            }
            if self.mask.tool.is_some() {
                egui::Panel::top("mask_options")
                    .exact_size(layout::TOOLBAR_HEIGHT)
                    .show(ui, |ui| {
                        mask::options_bar(self, ws.as_deref(), ui, &mut actions)
                    });
            }
            egui::Panel::bottom("status_bar")
                .exact_size(layout::STATUS_HEIGHT)
                .show(ui, |ui| status::show(self, ws.as_deref(), ui, &mut actions));
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
                    // One line, however long: a wrapped toast reads badly.
                    let text = egui::RichText::new(message.as_str()).color(colour);
                    ui.add(egui::Label::new(text).extend());
                });
            });
    }

    /// The shortcuts of `docs/UX.md` §8 that the shell itself handles.
    fn handle_shortcuts(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        use shortcuts as s;
        // Single keys only when no text field wants them.
        let typing = ctx.egui_wants_keyboard_input();
        let mut hide_ui = None;
        ctx.input_mut(|i| {
            // Show/Hide UI and Show/Hide Dock (`Cmd \`, `Shift Cmd \`), even while typing.
            // Shift first: a Cmd pattern also matches with Shift held. Shift `\` usually
            // arrives as `|`.
            if i.consume_shortcut(&s::HIDE_DOCK)
                || i.consume_key(s::HIDE_DOCK.modifiers, egui::Key::Pipe)
            {
                self.dock_hidden = !self.dock_hidden;
            } else if i.consume_shortcut(&s::HIDE_UI) {
                hide_ui = Some(!self.chrome_hidden);
            }
            // `Esc` always brings the UI back (#240); it does nothing else then.
            if self.chrome_hidden && i.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
                hide_ui = Some(false);
            }
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
            // Rotate ±0.1° (with Shift ±1°) about the anchor.
            for (key, sign) in [
                (egui::Key::OpenBracket, -1.0),
                (egui::Key::CloseBracket, 1.0),
            ] {
                let cmd_alt = egui::Modifiers::COMMAND.plus(egui::Modifiers::ALT);
                let step = if i.consume_key(cmd_alt.plus(egui::Modifiers::SHIFT), key) {
                    1.0
                } else if i.consume_key(cmd_alt, key) {
                    0.1
                } else {
                    continue;
                };
                let layer = convert64_project::LayerRef::Index(self.layer);
                actions.push(Action::Run(convert64_project::JsonCommand::LayerRotate {
                    layer,
                    degrees: sign * step,
                    relative: true,
                    anchor: None,
                    snap: None,
                }));
            }
            // With the Transform tool, Shift Cmd [ ] scale ±10 %; otherwise they walk the
            // worst cells (UX §8 gives both the same keys; the tool decides).
            let transforming = self.tool == Tool::Transform && self.mask.tool.is_none();
            if transforming && i.consume_shortcut(&s::NEXT_WORST) {
                self.scale_request = Some(1.1);
            } else if transforming && i.consume_shortcut(&s::PREVIOUS_WORST) {
                self.scale_request = Some(1.0 / 1.1);
            } else if i.consume_shortcut(&s::NEXT_WORST) {
                self.nav_request = Some(true);
            } else if i.consume_shortcut(&s::PREVIOUS_WORST) {
                self.nav_request = Some(false);
            }
            if i.consume_shortcut(&s::SCALE_UP) {
                self.scale_request = Some(1.01);
            }
            if i.consume_shortcut(&s::SCALE_DOWN) {
                self.scale_request = Some(1.0 / 1.01);
            }
            if i.consume_shortcut(&s::TRANSFORM) {
                self.tool = Tool::Transform;
                self.mask.tool = None;
                self.mask.active = false;
            }
            if i.consume_shortcut(&s::SNAP_CYCLE) {
                self.snap_cycle = true;
            }
            // Shift Cmd S before Cmd S: a Cmd pattern also matches with Shift held.
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
            // `Cmd \` is Show/Hide UI, not a flip.
            self.flip_held = !typing && !i.modifiers.command && i.key_down(egui::Key::Backslash);
            if typing {
                return;
            }
            if i.consume_shortcut(&s::CLASHES) {
                self.overlays.clashes = !self.overlays.clashes;
            }
            if i.consume_key(egui::Modifiers::NONE, egui::Key::H) {
                self.hand = !self.hand;
            }
            // Bare 1 is 100 % too, for Krita and Aseprite users (UX §8).
            if i.consume_key(egui::Modifiers::NONE, egui::Key::Num1) {
                self.camera.actual(self.view);
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
        if let Some(hidden) = hide_ui {
            self.set_ui_hidden(ctx, hidden);
        }
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

    /// #240: `Cmd \` hides the UI with a toast that says how to get it back; `Esc` and
    /// `Cmd \` show it again; `Shift Cmd \` hides only the dock.
    #[test]
    fn cmd_backslash_hides_the_ui_and_esc_shows_it() {
        let cmd = egui::Modifiers::COMMAND;
        let (shell, _) = frame(Shell::default(), vec![key(egui::Key::Backslash, cmd)]);
        assert!(shell.chrome_hidden && !shell.dock_hidden);
        let (message, error, _) = shell.toast.clone().expect("a toast on hide");
        // "⌘\" on macOS, "Ctrl+Backslash" elsewhere, as the menus show it.
        let keys = message
            .strip_prefix("UI hidden — press ")
            .and_then(|m| m.strip_suffix(" or Esc to show it"))
            .unwrap_or_else(|| panic!("{message}"));
        assert!(keys == "⌘\\" || keys == "Ctrl+Backslash", "{keys}");
        assert!(!error);

        let (shell, _) = frame(shell, vec![key(egui::Key::Escape, egui::Modifiers::NONE)]);
        assert!(!shell.chrome_hidden);
        assert!(shell.toast.is_none(), "the hint goes with the hidden UI");

        let (shell, _) = frame(Shell::default(), vec![key(egui::Key::Backslash, cmd)]);
        let (shell, _) = frame(shell, vec![key(egui::Key::Backslash, cmd)]);
        assert!(!shell.chrome_hidden, "Cmd \\ toggles");

        let shift_cmd = cmd.plus(egui::Modifiers::SHIFT);
        for k in [egui::Key::Backslash, egui::Key::Pipe] {
            let (shell, _) = frame(Shell::default(), vec![key(k, shift_cmd)]);
            assert!(!shell.chrome_hidden && shell.dock_hidden, "{k:?}");
        }
    }

    /// Holding `Cmd \` hides the UI; it does not flip the working view as a bare `\` does.
    #[test]
    fn cmd_backslash_is_not_hold_to_compare() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let mut shell = Shell::default();
        for (modifiers, flipped) in [
            (egui::Modifiers::NONE, true),
            (egui::Modifiers::COMMAND, false),
        ] {
            let input = egui::RawInput {
                events: vec![
                    egui::Event::ModifiersChanged(modifiers),
                    key(egui::Key::Backslash, modifiers),
                ],
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| {
                shell.show(ui, None);
            });
            output.textures_delta.clear();
            assert_eq!(shell.flip_held, flipped, "{modifiers:?}");
            // Release the key before the next case.
            let input = egui::RawInput {
                events: vec![egui::Event::Key {
                    key: egui::Key::Backslash,
                    physical_key: None,
                    pressed: false,
                    repeat: false,
                    modifiers,
                }],
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| {
                shell.show(ui, None);
            });
            output.textures_delta.clear();
        }
    }

    /// #240: `Tab` and `Shift Tab` are egui's focus traversal and hide nothing.
    #[test]
    fn tab_hides_nothing() {
        for m in [egui::Modifiers::NONE, egui::Modifiers::SHIFT] {
            let (shell, _) = frame(Shell::default(), vec![key(egui::Key::Tab, m)]);
            assert!(!shell.chrome_hidden && !shell.dock_hidden, "{m:?}");
        }
    }

    /// `Esc` with the UI showing still leaves mask mode (it is not swallowed).
    #[test]
    fn esc_with_the_ui_showing_leaves_mask_mode() {
        let mut shell = Shell::default();
        shell.toggle_mask_mode();
        let (shell, _) = frame(shell, vec![key(egui::Key::Escape, egui::Modifiers::NONE)]);
        assert!(!shell.mask.active && !shell.chrome_hidden);
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

    /// UX §8: each shortcut asks for its action (the app carries it out).
    #[test]
    fn the_shortcut_table_asks_for_its_actions() {
        use convert64_project::{JsonCommand, LayerRef};
        let cmd = egui::Modifiers::COMMAND;
        let shift_cmd = egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT);
        let none = egui::Modifiers::NONE;
        let cases: Vec<(egui::Key, egui::Modifiers, Action)> = vec![
            (egui::Key::O, cmd, Action::Open),
            (egui::Key::S, cmd, Action::Save),
            (egui::Key::S, shift_cmd, Action::SaveAs),
            (egui::Key::Z, cmd, Action::Undo),
            (egui::Key::Z, shift_cmd, Action::Redo),
            (egui::Key::Y, cmd, Action::Redo),
            (egui::Key::N, shift_cmd, Action::NewLayer),
            (egui::Key::Comma, cmd, Action::ToggleVisible(0)),
            (
                egui::Key::J,
                cmd,
                Action::Run(JsonCommand::LayerDuplicate {
                    layer: LayerRef::Index(0),
                }),
            ),
            (
                egui::Key::Delete,
                none,
                Action::Run(JsonCommand::LayerRemove {
                    layer: LayerRef::Index(0),
                }),
            ),
            (
                egui::Key::ArrowLeft,
                egui::Modifiers::ALT,
                Action::Run(JsonCommand::LayerNudge {
                    layer: LayerRef::Index(0),
                    by: convert64_core::Vec2::new(-0.25, 0.0),
                }),
            ),
            (
                egui::Key::ArrowDown,
                egui::Modifiers::SHIFT,
                Action::Run(JsonCommand::LayerNudge {
                    layer: LayerRef::Index(0),
                    by: convert64_core::Vec2::new(0.0, 10.0),
                }),
            ),
        ];
        for (k, m, want) in cases {
            let (_, actions) = frame(Shell::default(), vec![key(k, m)]);
            assert_eq!(actions, vec![want], "{k:?} {m:?}");
        }
        // Single keys that change the shell itself.
        let (shell, _) = frame(Shell::default(), vec![key(egui::Key::M, none)]);
        assert!(shell.mask.active && shell.mask.tool == Some(MaskTool::Brush));
        let (shell, _) = frame(shell, vec![key(egui::Key::Escape, none)]);
        assert!(!shell.mask.active && shell.mask.tool.is_none());
        let (shell, _) = frame(
            Shell::default(),
            vec![key(egui::Key::H, none), key(egui::Key::Num1, none)],
        );
        assert!(shell.hand);
        assert_eq!(shell.camera.source_zoom, 1.0);
        let (shell, _) = frame(Shell::default(), vec![key(egui::Key::W, none)]);
        assert_eq!(shell.mask.tool, Some(MaskTool::Lasso));
        assert!(
            !shell.mask.active,
            "a mask tool without mask mode makes a selection"
        );
    }
}
