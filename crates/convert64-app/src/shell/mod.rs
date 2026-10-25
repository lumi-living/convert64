//! The app shell (`docs/UX.md` §3, Figma `App Shell / Light` and `/ Dark`): menu bar,
//! toolbar, the working view, the right dock (Result on top, then the Recipe, Layers and
//! Export tabs) and the status bar.
//!
//! [`Shell`] holds what the chrome shows and [`Shell::show`] draws it, returning the
//! [`Action`]s the user asked for this frame; the app carries them out. Sizes come from
//! [`crate::theme::layout`] and colours from [`crate::theme::Theme`].

mod dock;
mod export_panel;
mod layers;
mod mask;
mod menu;
pub mod method_picker;
mod recipe;
mod session;
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
pub use mask::{BrushOptions, MaskState, MaskTool};
pub use method_picker::{PickerState, RecipeTarget};
pub use recipe::{GROUP_ORDER, RecipeGroups};

/// The four steering tools (`docs/UX.md` §7), in toolbar order. Nudge (`V`, Move in
/// Photoshop and Figma) is the default: a stray click with it selects a layer and
/// changes nothing, where Pin would pin a cell (#242).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tool {
    /// Pin a cell's colour set.
    Pin,
    /// Lock a cell.
    Lock,
    /// Paint a region.
    Region,
    /// Nudge a layer.
    #[default]
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

/// Where the keyboard's single keys go when no text is being typed: the canvas (the
/// working view), the Layers panel, or some other part of the window. `Delete` and
/// `Backspace` delete the selected layer only from the first two (#245).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KeyFocus {
    /// The working view: the default, and where `Esc` returns.
    #[default]
    Canvas,
    /// The Layers tab.
    Layers,
    /// Anything else: the Recipe form, the result panel, the toolbar.
    Other,
}

/// Something the user asked for that the shell cannot do itself.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Close the app (native) or do nothing (web).
    Quit,
    /// The answer to "Save changes before quitting?" (native, #241).
    AnswerQuit(QuitAnswer),
    /// The restore bar's Restore: open the autosaved session (#241).
    RestoreSession,
    /// The restore bar's Discard: delete the autosaved session.
    DiscardSession,
    /// Dismiss the autosave warning in the status bar.
    DismissAutosaveWarning,
    /// Follow the system theme, or force light or dark (View › Theme).
    SetTheme(egui::ThemePreference),
    /// Change the target mode (the result header's dropdown): `mode_set`, then a full
    /// solve.
    SetMode(Mode),
    /// File › Open… (`Cmd O`).
    Open,
    /// The empty state's *Try a sample picture*: open the bundled `title` painting
    /// ([`crate::document::SAMPLE`]) as if it had been opened from a file (#292).
    OpenSample,
    /// File › Save (`Cmd S`): to the project's file, or as Save As.
    Save,
    /// File › Save As… (`Shift Cmd S`).
    SaveAs,
    /// Write the export set (UX §6b): the panel's Export button asks for a folder;
    /// `again` (Export Again, `Shift Cmd E`) writes into the last one without asking.
    Export {
        /// Export Again: the last folder, if there is one.
        again: bool,
    },
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
    /// The method picker's live preview (UX §6a): solve the result with this method in
    /// place of the recipe's (interactively, nothing in the history), or with `None`
    /// show the committed result again.
    PreviewMethod(Option<(RecipeTarget, convert64_core::layer::Method)>),
}

/// How the user answered "Save changes to … before quitting?".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuitAnswer {
    /// Save, then quit.
    Save,
    /// Quit without saving.
    DontSave,
    /// Keep working.
    Cancel,
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
    /// The autosave's last failure, one line in the status bar's warning colour (#241).
    pub autosave_warning: Option<String>,
    /// The restore bar's question, while an autosaved session is on offer (#241).
    pub restore_offer: Option<String>,
    /// Quitting with unsaved changes (native): the document's name, for the question.
    pub quit_prompt: Option<String>,
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
    /// The Export panel's file previews (UX §6b), rebuilt per solution and set.
    export_cache: export_panel::ExportCache,
    /// The Export panel's file name being typed: the row and the text so far.
    export_edit: Option<(usize, String)>,
    /// The folder the last export went into, by name, for the panel's note.
    pub last_export_dir: Option<String>,
    /// A message shown at the bottom of the window until the time (egui seconds) passes.
    toast: Option<(String, bool, f64)>,
    /// The toast offers Undo (a deleted layer).
    toast_undo: bool,
    /// Where the last press landed, or `Esc` sent the keys (#245).
    pub key_focus: KeyFocus,
    /// The Layers tab's rectangle last frame, for a focused widget inside it.
    layers_rect: Option<egui::Rect>,
    /// The working view's widget id, for keyboard focus on it.
    canvas_id: Option<egui::Id>,
    /// `--dock-scroll <points>` (screenshots only): the dock's tab content held scrolled
    /// down this far, to show a group below the fold.
    pub dock_scroll: Option<f32>,
    /// The dither method picker (UX §6a).
    pub picker: PickerState,
    /// The dock's left edge last frame, where the picker hangs from.
    dock_left: Option<f32>,
    /// The last cell the pointer was over on the picture: the picker's Picture
    /// thumbnails are cropped there.
    pub last_cell: Option<convert64_core::model::CellPos>,
    /// The bindings that differ between the native and the browser build (#286): what
    /// the shortcuts bind and the menus show.
    pub keys: shortcuts::Keymap,
    /// Which Recipe groups are open: the user's choice, kept in the app's storage
    /// (#288).
    pub groups: RecipeGroups,
    /// *Edit › Copy Recipe*: the recipe *Paste Recipe* sets (usability review m5).
    pub recipe_clipboard: Option<convert64_core::layer::Recipe>,
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
            autosave_warning: None,
            restore_offer: None,
            quit_prompt: None,
            history: (false, false),
            flip_held: false,
            fit_pending: true,
            view_rect: None,
            hover: None,
            export_cache: export_panel::ExportCache::default(),
            export_edit: None,
            last_export_dir: None,
            toast: None,
            toast_undo: false,
            key_focus: KeyFocus::default(),
            layers_rect: None,
            canvas_id: None,
            dock_scroll: None,
            picker: PickerState::default(),
            dock_left: None,
            last_cell: None,
            keys: shortcuts::Keymap::CURRENT,
            groups: RecipeGroups::default(),
            recipe_clipboard: None,
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
        self.toast_undo = false;
        ctx.request_repaint_after(std::time::Duration::from_secs(3));
    }

    /// Shows `message` with an **Undo** button for 6 s, for an edit that removed
    /// something (UX §11; "Deleted layer 'source' · Undo", #245).
    pub fn toast_with_undo(&mut self, ctx: &egui::Context, message: impl Into<String>) {
        let now = ctx.input(|i| i.time);
        self.toast = Some((message.into(), false, now + 6.0));
        self.toast_undo = true;
        ctx.request_repaint_after(std::time::Duration::from_secs(6));
    }

    /// Whether `Delete` / `Backspace` may delete the selected layer now: only while the
    /// keys belong to the canvas or the Layers panel, never to the Recipe form (#245).
    fn delete_keys_apply(&self, ctx: &egui::Context) -> bool {
        let here = match ctx.memory(|m| m.focused()) {
            None => self.key_focus,
            // A widget with keyboard focus (`Tab`) decides by where it is.
            Some(id) if Some(id) == self.canvas_id => KeyFocus::Canvas,
            Some(id) => {
                let inside = ctx.read_response(id).zip(self.layers_rect);
                if inside.is_some_and(|(r, layers)| layers.contains(r.rect.center())) {
                    KeyFocus::Layers
                } else {
                    KeyFocus::Other
                }
            }
        };
        matches!(here, KeyFocus::Canvas | KeyFocus::Layers)
    }

    /// Hides or shows the menu bar, toolbar, dock and status bar (`Cmd \`, View › Hide
    /// UI). Hiding says how to get them back, since nothing on screen does (#240).
    pub fn set_ui_hidden(&mut self, ctx: &egui::Context, hidden: bool) {
        self.chrome_hidden = hidden;
        if hidden {
            let keys = crate::shell::shortcuts::format(ctx, &shortcuts::HIDE_UI);
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

    /// Shows the Export panel (`Cmd E`, Export › Export…; UX §6b): the Export tab, with
    /// the dock and the UI shown if they were hidden.
    pub fn show_export(&mut self, ctx: &egui::Context) {
        self.tab = DockTab::Export;
        self.dock_hidden = false;
        if self.chrome_hidden {
            self.set_ui_hidden(ctx, false);
        }
    }

    /// Whether a dialog is open (the quit question, or any egui modal last frame): `Esc`
    /// is its, and no shortcut reaches the app behind it (UX §8, #287).
    pub fn dialog_open(&self, ctx: &egui::Context) -> bool {
        self.quit_prompt.is_some() || ctx.memory(|m| m.top_modal_layer()).is_some()
    }

    /// A new document starts with the default tool (Nudge) and out of mask mode, so a
    /// first click on it changes nothing (#242).
    pub fn reset_tool(&mut self) {
        self.tool = Tool::default();
        self.mask.active = false;
        self.mask.tool = None;
        self.drag = None;
        self.transform_drag = None;
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

    /// The current tool's own Add/Subtract, if it has one: the mask Brush's, the
    /// Lasso's or the Region brush's (usability review M7).
    pub fn paint_mode_mut(&mut self) -> Option<&mut convert64_core::mask::PaintMode> {
        match self.mask.tool {
            Some(MaskTool::Brush) => Some(&mut self.mask.brush.mode),
            Some(MaskTool::Lasso) => Some(&mut self.mask.lasso),
            Some(MaskTool::Gradient | MaskTool::Key) => None,
            None => self.brush_mut().map(|b| &mut b.mode),
        }
    }

    /// The current tool's own brush, if it has one: the mask Brush's or the Region
    /// tool's.
    pub fn brush_mut(&mut self) -> Option<&mut BrushOptions> {
        match self.mask.tool {
            Some(MaskTool::Brush) => Some(&mut self.mask.brush),
            Some(_) => None,
            None if self.tool == Tool::Region => Some(&mut self.mask.region),
            None => None,
        }
    }

    /// Keeps the camera as set instead of fitting after an open (screenshots).
    pub fn keep_camera(&mut self) {
        self.fit_pending = false;
    }

    /// Whether the Transform tool's box and handles are drawn and catch the pointer:
    /// with that tool, outside mask mode, and only while the working view shows the
    /// source (UX §4.1; usability review m3).
    pub fn transform_handles(&self) -> bool {
        self.tool == Tool::Transform && self.mask.tool.is_none() && self.shown() == ViewMode::Source
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

    /// Shows the worst colour-limited cell: that overlay on, Result mode, centred on the
    /// cell (the result header's count is a button; UX §4.3).
    pub fn show_worst(&mut self, ws: &Workspace) {
        self.overlays.clashes = true;
        self.view = ViewMode::Result;
        if let Some(cell) = ws.worst() {
            self.camera.centre = crate::view::cell_rect(ws.doc.grid(), cell).center();
        }
    }

    /// Draws the whole shell into `ui` (the app's root) and returns what the user asked
    /// for. Keyboard shortcuts are read first; single keys wait while a text field is
    /// being edited.
    pub fn show(&mut self, ui: &mut egui::Ui, mut ws: Option<&mut Workspace>) -> Vec<Action> {
        let mut actions = Vec::new();
        // A press anywhere else ends a text edit, as a click does: a drag on a slider
        // must not leave the W field taking the keys (#244).
        let presses = egui::SurrenderFocusOn::Presses;
        if ui.ctx().options(|o| o.input_options.surrender_focus_on) != presses {
            ui.ctx()
                .options_mut(|o| o.input_options.surrender_focus_on = presses);
        }
        self.handle_shortcuts(ui.ctx(), ws.is_some(), &mut actions);
        self.picker.drawn = false;
        if let Some(ws) = ws.as_deref_mut() {
            ws.prepare(ui.ctx(), self.overlays.ghost);
            ws.prepare_thumbs(ui.ctx(), self.picker.source, self.last_cell);
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
            // The options row is always there, so nothing jumps (usability review M8).
            egui::Panel::top("options_bar")
                .exact_size(layout::TOOLBAR_HEIGHT)
                .show(ui, |ui| {
                    toolbar::options_bar(self, ws.as_deref(), ui, &mut actions)
                });
            if let Some(text) = &self.restore_offer {
                egui::Panel::top("restore_bar")
                    .exact_size(layout::TOOLBAR_HEIGHT)
                    .show(ui, |ui| session::restore_bar(text, ui, &mut actions));
            }
            egui::Panel::bottom("status_bar")
                .exact_size(layout::STATUS_HEIGHT)
                .show(ui, |ui| status::show(self, ws.as_deref(), ui, &mut actions));
            if !self.dock_hidden {
                let dock = egui::Panel::right("dock")
                    .resizable(true)
                    .default_size(layout::DOCK_WIDTH)
                    .size_range(DOCK_WIDTH_RANGE)
                    .frame(egui::Frame::side_top_panel(ui.style()).inner_margin(0))
                    .show(ui, |ui| dock::show(self, ws.as_deref(), ui, &mut actions));
                self.dock_left = Some(dock.response.rect.left());
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
        session::quit_prompt(self, ui.ctx(), &mut actions);
        // The picker closes (reverting its preview) when its recipe is no longer shown.
        if self.picker.open.is_some() && !self.picker.drawn {
            self.picker.close(&mut actions);
        }
        if self.picker.open.is_none()
            && let Some(ws) = ws.as_deref()
            && let Some((_, (x, y))) = self
                .hover
                .and_then(|p| crate::view::target_at(ws.doc.grid(), p))
        {
            self.last_cell = Some(convert64_core::model::CellPos::new(x as u16, y as u16));
        }
        if let Some(ws) = ws.as_deref_mut() {
            ws.method_thumbs.flush();
        }
        if let Some(ws) = ws.as_deref() {
            self.announce_deletes(ui.ctx(), ws, &actions);
        }
        self.show_toast(ui.ctx(), &mut actions);
        release_focus(ui.ctx());
        actions
    }

    /// A layer delete, from any key, button or menu, says what went and offers Undo:
    /// it used to happen silently (usability review M9, #245).
    pub fn announce_deletes(&mut self, ctx: &egui::Context, ws: &Workspace, actions: &[Action]) {
        use convert64_project::{JsonCommand, LayerRef};
        let layers = &ws.doc.session().project().layers;
        for action in actions {
            if let Action::Run(JsonCommand::LayerRemove {
                layer: LayerRef::Index(i),
            }) = action
                && let Some(layer) = layers.get(*i)
            {
                self.toast_with_undo(ctx, format!("Deleted layer '{}'", layer.name()));
            }
        }
    }

    fn show_toast(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        let Some((message, error, until)) = &self.toast else {
            return;
        };
        if ctx.input(|i| i.time) > *until {
            self.toast = None;
            return;
        }
        let theme = crate::theme::Theme::current(ctx);
        let colour = if *error { theme.error } else { theme.ok };
        let undo = self.toast_undo;
        let mut undone = false;
        egui::Area::new(egui::Id::new("toast"))
            .anchor(
                egui::Align2::CENTER_BOTTOM,
                [0.0, -2.0 * layout::STATUS_HEIGHT],
            )
            .interactable(undo)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        // One line, however long: a wrapped toast reads badly.
                        let text = egui::RichText::new(message.as_str()).color(colour);
                        ui.add(egui::Label::new(text).extend());
                        if undo {
                            ui.label("·");
                            let keys = crate::shell::shortcuts::format(ctx, &shortcuts::UNDO);
                            undone = ui
                                .link("Undo")
                                .on_hover_text(format!("Undo ({keys})"))
                                .clicked();
                        }
                    });
                });
            });
        if undone {
            actions.push(Action::Undo);
            self.toast = None;
        }
    }

    /// The shortcuts of `docs/UX.md` §8 that the shell itself handles. Without a picture
    /// (`open` false) the tool, zoom and Snap keys do nothing, as their buttons are
    /// disabled (UX §5, #292).
    fn handle_shortcuts(&mut self, ctx: &egui::Context, open: bool, actions: &mut Vec<Action>) {
        use shortcuts as s;
        // Single keys only while no text field is being edited (a `TextEdit`, or a
        // `DragValue` taking typed digits); a focused slider or button leaves them be
        // (#244). Arrows, though, belong to whichever widget has focus.
        // The open method picker takes typing and the arrows (UX §6a), from its first
        // frame on, before its search field has the focus.
        // An open dialog takes the keys, `Esc` among them (UX §8): nothing acts behind it.
        if self.dialog_open(ctx) {
            self.flip_held = false;
            return;
        }
        let picker = self.picker.open.is_some();
        let typing = ctx.text_edit_focused() || picker;
        let widget_focused = ctx.egui_wants_keyboard_input() || picker;
        let delete_applies = self.delete_keys_apply(ctx);
        let mut hide_ui = None;
        let mut show_export = false;
        ctx.input_mut(|i| {
            // A press moves the keys away from the canvas and the Layers panel, unless
            // it lands on one of them (they say so as they draw); `Esc` brings them back.
            if i.pointer.any_pressed() {
                self.key_focus = KeyFocus::Other;
            }
            if i.key_pressed(egui::Key::Escape) {
                self.key_focus = KeyFocus::Canvas;
            }
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
            if i.consume_shortcut(&s::ZOOM_IN) && open {
                self.zoom_step(true);
            }
            if i.consume_shortcut(&s::ZOOM_OUT) && open {
                self.zoom_step(false);
            }
            if i.consume_shortcut(&s::ZOOM_FIT) && open {
                self.fit();
            }
            if i.consume_shortcut(&s::ZOOM_100) && open {
                self.camera.actual(self.view);
            }
            if i.consume_shortcut(&s::GRID) {
                self.overlays.grid = !self.overlays.grid;
            }
            if i.consume_shortcut(&s::BORDER) {
                self.overlays.border = !self.overlays.border;
            }
            if let Some(quit) = self.keys.quit
                && i.consume_shortcut(&quit)
            {
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
            if i.consume_shortcut(&self.keys.new_layer) || i.consume_shortcut(&s::NEW_LAYER_ALT) {
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
            if let Some(transform) = self.keys.transform
                && i.consume_shortcut(&transform)
                && open
            {
                self.tool = Tool::Transform;
                self.mask.tool = None;
                self.mask.active = false;
            }
            if i.consume_shortcut(&s::SNAP_CYCLE) && open {
                self.snap_cycle = true;
            }
            // Shift Cmd S before Cmd S: a Cmd pattern also matches with Shift held.
            if i.consume_shortcut(&s::SAVE_AS) {
                actions.push(Action::SaveAs);
            } else if i.consume_shortcut(&s::SAVE) {
                actions.push(Action::Save);
            }
            // Shift Cmd E before Cmd E, as for Save As.
            if i.consume_shortcut(&s::EXPORT_AGAIN) {
                actions.push(Action::Export { again: true });
            } else if i.consume_shortcut(&s::EXPORT) {
                show_export = true;
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
            if i.consume_key(egui::Modifiers::NONE, egui::Key::Num1) && open {
                self.camera.actual(self.view);
            }
            if i.consume_shortcut(&s::SOURCE_RESULT) {
                self.view = self.view.flipped();
            }
            if i.consume_shortcut(&s::COMPARE) {
                self.compare = self.compare.next();
            }
            if delete_applies
                && (i.consume_shortcut(&s::DELETE_LAYER)
                    || i.consume_shortcut(&s::DELETE_LAYER_ALT))
            {
                let layer = convert64_project::LayerRef::Index(self.layer);
                actions.push(Action::Run(convert64_project::JsonCommand::LayerRemove {
                    layer,
                }));
            }
            for tool in Tool::ALL {
                if i.consume_key(egui::Modifiers::NONE, tool.key()) && open {
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
            // X, [ and ] change the current tool's own options (usability review M7).
            if i.consume_key(egui::Modifiers::NONE, egui::Key::X)
                && let Some(mode) = self.paint_mode_mut()
            {
                *mode = mask::swapped(*mode);
            }
            if i.consume_key(egui::Modifiers::NONE, egui::Key::OpenBracket)
                && let Some(b) = self.brush_mut()
            {
                b.size = (b.size / 1.25).max(0.5);
            }
            if i.consume_key(egui::Modifiers::NONE, egui::Key::CloseBracket)
                && let Some(b) = self.brush_mut()
            {
                b.size = (b.size * 1.25).min(64.0);
            }
            // Arrows nudge the selected layer: 10 px with Shift, 0.25 px with Alt (UX §6).
            if widget_focused {
                return;
            }
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
        if show_export {
            self.show_export(ctx);
        }
    }
}

/// Gives the keyboard back to the shortcuts once a slider or `DragValue` is done with:
/// on `Enter`, and when its drag ends (#244, Figma's behaviour). Text being typed keeps
/// its focus until `Enter`, `Esc` or a press elsewhere, which egui handles.
fn release_focus(ctx: &egui::Context) {
    let Some(id) = ctx.memory(|m| m.focused()) else {
        return;
    };
    if ctx.text_edit_focused() {
        return;
    }
    let drag_widget = ctx.read_response(id).is_some_and(|r| r.sense.senses_drag());
    let enter = ctx.input(|i| i.key_pressed(egui::Key::Enter));
    let drag_ended = ctx.drag_stopped_id() == Some(id);
    if drag_widget && (enter || drag_ended) {
        ctx.memory_mut(|m| m.surrender_focus(id));
    }
}

/// Where the drag `response` reports began: where the button went down. egui reports a
/// drag on a widget that also senses clicks only once the pointer has moved past its
/// drag threshold (6 pt), by which time the pointer is that far from the press, so a
/// tool that reads its start point at `drag_started` must take it from here (#239, #263).
pub(super) fn drag_origin(ui: &egui::Ui, response: &egui::Response) -> Option<egui::Pos2> {
    ui.input(|i| i.pointer.press_origin())
        .or_else(|| response.interact_pointer_pos())
}

/// How far `response`'s drag moved the pointer this frame, counting the movement inside
/// the drag threshold on the frame the drag starts (#263): `drag_delta` alone loses it.
pub(super) fn drag_step(response: &egui::Response) -> egui::Vec2 {
    if response.drag_started() {
        response
            .total_drag_delta()
            .unwrap_or_else(|| response.drag_delta())
    } else {
        response.drag_delta()
    }
}

/// Headless frames of the whole shell, read back as the texts they draw (#292).
#[cfg(test)]
pub(crate) mod ui_test {
    use super::{Action, Shell};
    use crate::workspace::Workspace;
    use egui::{Color32, Event, Pos2, Rect, Shape};

    /// A text a frame drew: what it says, where, and the colour it was painted in (a
    /// disabled widget's fade included).
    #[derive(Debug, Clone)]
    pub struct Text {
        pub text: String,
        pub rect: Rect,
        pub colour: Color32,
    }

    /// The colour `t` is painted in: its override, its first section's colour, or the
    /// widget's text colour, faded by the shape's opacity.
    fn colour(t: &egui::epaint::TextShape) -> Color32 {
        let own = t
            .galley
            .job
            .sections
            .first()
            .map(|s| s.format.color)
            .filter(|c| *c != Color32::PLACEHOLDER);
        t.override_text_color
            .or(own)
            .unwrap_or(t.fallback_color)
            .gamma_multiply(t.opacity_factor)
    }

    fn collect(shape: &Shape, out: &mut Vec<Text>) {
        match shape {
            Shape::Text(t) => out.push(Text {
                text: t.galley.text().to_owned(),
                rect: t.visual_bounding_rect(),
                colour: colour(t),
            }),
            Shape::Vec(v) => v.iter().for_each(|s| collect(s, out)),
            _ => {}
        }
    }

    /// One frame of `shell` in a `size` window with `events`: the texts drawn and the
    /// actions asked for.
    pub fn frame(
        ctx: &egui::Context,
        shell: &mut Shell,
        ws: Option<&mut Workspace>,
        size: egui::Vec2,
        events: Vec<Event>,
    ) -> (Vec<Text>, Vec<Action>) {
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
            events,
            ..Default::default()
        };
        let mut actions = Vec::new();
        let mut ws = ws;
        let mut output = ctx.run_ui(input, |ui| actions = shell.show(ui, ws.as_deref_mut()));
        output.textures_delta.clear();
        let mut texts = Vec::new();
        for clipped in &output.shapes {
            collect(&clipped.shape, &mut texts);
        }
        (texts, actions)
    }

    /// The first text that reads `text`.
    pub fn find<'a>(texts: &'a [Text], text: &str) -> &'a Text {
        texts
            .iter()
            .find(|t| t.text == text)
            .unwrap_or_else(|| panic!("no {text:?} in {:?}", names(texts)))
    }

    /// Every text's string, for a failure message.
    pub fn names(texts: &[Text]) -> Vec<&str> {
        texts.iter().map(|t| t.text.as_str()).collect()
    }

    /// A press and release at `p`, as two frames' events.
    pub fn click(p: Pos2) -> [Vec<Event>; 2] {
        let button = |pressed| Event::PointerButton {
            pos: p,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        [
            vec![Event::PointerMoved(p), button(true)],
            vec![button(false)],
        ]
    }
}

/// Headless drags with real pointer events, for the canvas tools' tests (#248, #263).
#[cfg(test)]
pub(super) mod drag_test {
    use egui::{Pos2, Rect, Response, Ui, pos2};

    /// The working view in the headless runs, in points.
    pub const VIEW: Rect = Rect::from_min_max(pos2(0.0, 0.0), pos2(800.0, 600.0));

    /// A path from `from` towards `to`: two small steps inside egui's 6 pt drag
    /// threshold, one past it (where egui reports the drag), then the target.
    pub fn path(from: Pos2, to: Pos2) -> Vec<Pos2> {
        let dir = (to - from).normalized();
        vec![from + dir * 2.0, from + dir * 4.0, from + dir * 12.0, to]
    }

    /// Presses the primary button at `from`, moves through `path` one frame per point
    /// (as a real mouse does: the first steps stay inside egui's drag threshold) and
    /// releases. Each frame allocates a click-and-drag area over [`VIEW`] and hands its
    /// response to `f`.
    pub fn drag(from: Pos2, path: &[Pos2], mut f: impl FnMut(&Ui, &Response)) {
        let ctx = egui::Context::default();
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let last = *path.last().unwrap_or(&from);
        let mut frames = vec![
            vec![egui::Event::PointerMoved(from)],
            vec![button(from, true)],
        ];
        frames.extend(path.iter().map(|p| vec![egui::Event::PointerMoved(*p)]));
        frames.push(vec![button(last, false)]);
        frames.push(vec![]);
        for (n, events) in frames.into_iter().enumerate() {
            let input = egui::RawInput {
                screen_rect: Some(VIEW),
                time: Some(n as f64 / 60.0),
                events,
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| {
                let response = ui.allocate_rect(VIEW, egui::Sense::click_and_drag());
                f(ui, &response);
            });
            output.textures_delta.clear();
        }
    }
}

/// A primary button (the Export panel's Export, the empty state's Open image…): the
/// selection fill, strong text (dark on the light theme's pale accent, light on the dark
/// theme's deep one).
pub(super) fn primary_button(ui: &egui::Ui, text: impl Into<String>) -> egui::Button<'static> {
    let visuals = ui.visuals();
    egui::Button::new(
        egui::RichText::new(text)
            .strong()
            .color(visuals.strong_text_color()),
    )
    .fill(visuals.selection.bg_fill)
}

/// Shift-drag on a slider or a number field moves a tenth as far (UX §6; usability review
/// m9, the Figma and Photoshop convention).
pub(crate) const FINE: f32 = 10.0;

/// A slider's value during a Shift-drag (UX §6, m9): from where the value stood, the
/// pointer's movement across the rail at a tenth of the plain rate, rounded to
/// `decimals`. `None` when the slider is not being Shift-dragged, and then the slider's own
/// value stands (it follows the pointer again). egui's `DragValue` already moves a tenth
/// as far with Shift held; its `Slider` does not.
pub(super) fn fine_slider(
    ui: &egui::Ui,
    slider: &egui::Response,
    value: f32,
    range: std::ops::RangeInclusive<f32>,
    decimals: usize,
) -> Option<f32> {
    let id = slider.id.with("fine");
    let shift = ui.input(|i| i.modifiers.shift_only());
    let k = 10f32.powi(decimals as i32);
    let held = ui.data(|d| d.get_temp::<f32>(id));
    // The release of a fine drag keeps its value: egui's slider would jump to the
    // pointer on that frame.
    if slider.drag_stopped()
        && let Some(held) = held.filter(|_| shift)
    {
        ui.data_mut(|d| d.remove::<f32>(id));
        return Some((held * k).round() / k);
    }
    if !(slider.dragged() && shift) {
        ui.data_mut(|d| d.remove::<f32>(id));
        return None;
    }
    let (lo, hi) = (*range.start(), *range.end());
    let width = slider.rect.width().max(1.0);
    let dx = ui.input(|i| i.pointer.delta().x);
    let now = (held.unwrap_or(value) + dx / width * (hi - lo) / FINE).clamp(lo, hi);
    ui.data_mut(|d| d.insert_temp(id, now));
    Some((now * k).round() / k)
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

    /// [`frame`] with a picture open (the tool, zoom and Snap keys wait for one, #292).
    fn frame_open(mut shell: Shell, events: Vec<egui::Event>) -> (Shell, Vec<Action>) {
        let (ctx, mut ws) = two_layers();
        shell.keep_camera();
        let input = egui::RawInput {
            events,
            ..Default::default()
        };
        let mut actions = Vec::new();
        let mut output = ctx.run_ui(input, |ui| actions = shell.show(ui, Some(&mut ws)));
        output.textures_delta.clear();
        (shell, actions)
    }

    /// UX §5, usability review M10 (#292): with no picture the working view offers the
    /// primary Open image… and the Try a sample picture link, and each asks for its
    /// action.
    #[test]
    fn the_empty_state_offers_open_and_a_sample() {
        use ui_test::{click, find, frame};
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let mut shell = Shell::default();
        let size = egui::vec2(1280.0, 800.0);
        let (texts, _) = frame(&ctx, &mut shell, None, size, vec![]);
        find(&texts, working::EMPTY_TEXT);
        let view = egui::Rect::from_min_max(
            egui::pos2(0.0, 2.0 * layout::TOOLBAR_HEIGHT),
            egui::pos2(size.x - layout::DOCK_WIDTH, size.y - layout::STATUS_HEIGHT),
        );
        for (label, want) in [
            ("Open image…", Action::Open),
            ("Try a sample picture", Action::OpenSample),
        ] {
            let at = find(&texts, label).rect;
            assert!(view.contains_rect(at), "{label} at {at:?}, view {view:?}");
            let mut asked = Vec::new();
            for events in click(at.center()) {
                asked.extend(frame(&ctx, &mut shell, None, size, events).1);
            }
            assert_eq!(asked, vec![want], "{label}");
        }
        // The primary button sits above the link, both under the sentence.
        let sentence = find(&texts, working::EMPTY_TEXT).rect;
        let open = find(&texts, "Open image…").rect;
        let sample = find(&texts, "Try a sample picture").rect;
        assert!(sentence.bottom() <= open.top() && open.bottom() <= sample.top());
    }

    /// #292: with no picture the tools, zoom and Snap do nothing, by key or by click.
    #[test]
    fn without_a_picture_the_tools_zoom_and_snap_wait() {
        use ui_test::{click, find, frame};
        let none = egui::Modifiers::NONE;
        let cmd = egui::Modifiers::COMMAND;
        let shift_cmd = cmd.plus(egui::Modifiers::SHIFT);
        let (shell, actions) = frame_none(vec![
            key(egui::Key::P, none),
            key(egui::Key::L, none),
            key(egui::Key::S, none),
            key(egui::Key::T, cmd),
            key(egui::Key::Equals, cmd),
            key(egui::Key::Minus, cmd),
            key(egui::Key::Num1, none),
            key(egui::Key::Semicolon, shift_cmd),
        ]);
        assert_eq!(shell.tool, Tool::Nudge);
        assert_eq!(shell.camera, crate::view::Camera::default());
        assert!(!shell.snap_cycle && actions.is_empty(), "{actions:?}");

        // A click on the Pin tool or on Zoom in changes nothing either.
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let mut shell = Shell::default();
        let size = egui::vec2(1280.0, 800.0);
        let (texts, _) = frame(&ctx, &mut shell, None, size, vec![]);
        for glyph in [crate::icons::PIN, crate::icons::ZOOM_IN] {
            let at = find(&texts, glyph.codepoint).rect;
            for events in click(at.center()) {
                frame(&ctx, &mut shell, None, size, events);
            }
        }
        assert_eq!(shell.tool, Tool::Nudge);
        assert_eq!(shell.camera, crate::view::Camera::default());

        // With a picture the same keys act.
        let (shell, _) = frame_open(
            Shell::default(),
            vec![key(egui::Key::P, none), key(egui::Key::Equals, cmd)],
        );
        assert_eq!(shell.tool, Tool::Pin);
        assert_ne!(shell.camera, crate::view::Camera::default());
    }

    /// [`frame`] with no picture, in a context with the app's look.
    fn frame_none(events: Vec<egui::Event>) -> (Shell, Vec<Action>) {
        frame(Shell::default(), events)
    }

    /// #292 (usability review M10): the dock's line before a picture is open is drawn in
    /// the text colour, not faded as a disabled control.
    #[test]
    fn the_empty_dock_text_is_not_faded() {
        use ui_test::{find, frame};
        for theme in [egui::Theme::Light, egui::Theme::Dark] {
            let ctx = egui::Context::default();
            crate::theme::install(&ctx);
            ctx.set_theme(theme);
            let mut shell = Shell::default();
            let size = egui::vec2(1280.0, 800.0);
            frame(&ctx, &mut shell, None, size, vec![]);
            let (texts, _) = frame(&ctx, &mut shell, None, size, vec![]);
            let line = dock::empty_text(&ctx, "the recipe");
            let drawn = find(&texts, &line);
            let visuals = crate::theme::style(theme).visuals;
            assert_eq!(drawn.colour, visuals.text_color(), "{theme:?}");
            assert!(drawn.rect.left() > size.x - layout::DOCK_WIDTH - 1.0);
        }
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
        // "⌘\" on macOS, "Ctrl+\" elsewhere (the key as its symbol, m5), as the menus show it.
        let keys = message
            .strip_prefix("UI hidden — press ")
            .and_then(|m| m.strip_suffix(" or Esc to show it"))
            .unwrap_or_else(|| panic!("{message}"));
        assert!(keys == "⌘\\" || keys == "Ctrl+\\", "{keys}");
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

    /// #290 (usability review M7): every tool keeps its own options. `X` on the mask
    /// Brush no longer makes the Region brush subtract, and `]` grows only the current
    /// tool's brush; the Lasso has its own Add/Subtract too.
    #[test]
    fn tool_options_do_not_leak_between_tools() {
        use convert64_core::mask::PaintMode;
        let none = egui::Modifiers::NONE;
        let press = |shell, k| frame_open(shell, vec![key(k, none)]).0;
        // The mask Brush (with mask mode off it builds a selection): X makes it subtract.
        let shell = press(Shell::default(), egui::Key::B);
        let shell = press(shell, egui::Key::X);
        let shell = press(shell, egui::Key::CloseBracket);
        assert_eq!(shell.mask.brush.mode, PaintMode::Subtract);
        let grown = shell.mask.brush.size;
        assert!(grown > 6.0);
        // The Region brush still adds, at its own size.
        let shell = press(shell, egui::Key::R);
        assert_eq!(shell.tool, Tool::Region);
        assert_eq!(shell.mask.region, BrushOptions::default());
        // Its X and [ change only its own options.
        let shell = press(shell, egui::Key::X);
        let shell = press(shell, egui::Key::OpenBracket);
        assert_eq!(shell.mask.region.mode, PaintMode::Subtract);
        assert!(shell.mask.region.size < 6.0);
        assert_eq!(shell.mask.brush.mode, PaintMode::Subtract);
        assert_eq!(shell.mask.brush.size, grown);
        // The Lasso has its own Add/Subtract; X there leaves both brushes alone.
        let shell = press(shell, egui::Key::W);
        assert_eq!(shell.mask.lasso, PaintMode::Add);
        let shell = press(shell, egui::Key::X);
        assert_eq!(shell.mask.lasso, PaintMode::Subtract);
        let shell = press(shell, egui::Key::B);
        let shell = press(shell, egui::Key::X);
        assert_eq!(shell.mask.brush.mode, PaintMode::Add);
        assert_eq!(shell.mask.region.mode, PaintMode::Subtract);
        assert_eq!(shell.mask.lasso, PaintMode::Subtract);
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

    /// #260: while the method picker is open, arrows move in its grid and letters
    /// search; neither nudges the layer nor picks a tool.
    #[test]
    fn the_open_picker_keeps_arrows_and_letters() {
        let mut shell = Shell::default();
        shell.picker.open(RecipeTarget::Layer(0));
        let none = egui::Modifiers::NONE;
        let (shell, actions) = frame(
            shell,
            vec![key(egui::Key::ArrowRight, none), key(egui::Key::P, none)],
        );
        assert!(actions.is_empty(), "{actions:?}");
        assert_eq!(shell.tool, Tool::Nudge);
        // Not drawn (no document): it closed.
        assert!(shell.picker.open.is_none());
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

    /// Clicks the menu `menu`, then its item `item`, in a 1280 × 800 window: the actions
    /// that asked for.
    fn menu_click(
        ctx: &egui::Context,
        shell: &mut Shell,
        ws: &mut Workspace,
        menu: &str,
        item: &str,
    ) -> Vec<Action> {
        use ui_test::{click, find, frame};
        let size = egui::vec2(1280.0, 800.0);
        let mut actions = Vec::new();
        let (texts, _) = frame(ctx, shell, Some(ws), size, vec![]);
        for events in click(find(&texts, menu).rect.center()) {
            actions.extend(frame(ctx, shell, Some(ws), size, events).1);
        }
        // The menu opens over a few frames.
        let mut texts = Vec::new();
        for _ in 0..3 {
            texts = frame(ctx, shell, Some(ws), size, vec![]).0;
        }
        for events in click(find(&texts, item).rect.center()) {
            actions.extend(frame(ctx, shell, Some(ws), size, events).1);
        }
        actions
    }

    /// Usability review m5: Edit › Copy Recipe takes the selected layer's recipe and
    /// Paste Recipe sets all of it on another layer as one `recipe_set`; Recipe › Reset
    /// Recipe sets the defaults.
    #[test]
    fn copy_and_paste_recipe_set_the_whole_recipe() {
        use convert64_core::command::RecipeChange;
        use convert64_core::layer::Metric;
        use convert64_project::{JsonCommand, LayerRef};
        let (ctx, mut ws) = two_layers();
        let change = RecipeChange {
            strength: Some(0.35),
            metric: Some(Metric::Rgb),
            ..Default::default()
        };
        ws.doc
            .run(JsonCommand::RecipeSet {
                layer: LayerRef::Index(0),
                recipe: change,
            })
            .unwrap();
        let copied = *ws.doc.session().project().layers[0].recipe();
        let mut shell = Shell {
            layer: 0,
            ..Shell::default()
        };
        shell.keep_camera();
        menu_click(&ctx, &mut shell, &mut ws, "Edit", "Copy Recipe");
        assert_eq!(shell.recipe_clipboard, Some(copied));
        shell.layer = 1;
        let actions = menu_click(&ctx, &mut shell, &mut ws, "Edit", "Paste Recipe");
        let want = Action::Run(JsonCommand::RecipeSet {
            layer: LayerRef::Index(1),
            recipe: RecipeChange::all(&copied),
        });
        assert_eq!(actions, vec![want]);
        let actions = menu_click(&ctx, &mut shell, &mut ws, "Recipe", "Reset Recipe");
        let want = Action::Run(JsonCommand::RecipeSet {
            layer: LayerRef::Index(1),
            recipe: RecipeChange::all(&Default::default()),
        });
        assert_eq!(actions, vec![want]);
    }

    /// Usability review m5: toggles carry a check mark (blank, same width, when off),
    /// Show/Hide items say what they will do, and the ghost is "Overlay the …".
    #[test]
    fn menus_check_toggles_and_name_show_or_hide() {
        use ui_test::{find, frame};
        let (ctx, mut ws) = two_layers();
        let mut shell = Shell::default();
        shell.keep_camera();
        shell.overlays.border = true;
        shell.overlays.grid = false;
        menu_click(&ctx, &mut shell, &mut ws, "View", "Cell Grid");
        assert!(shell.overlays.grid, "the click toggled it");
        let size = egui::vec2(1280.0, 800.0);
        // Open the View menu again and read it.
        let (texts, _) = frame(&ctx, &mut shell, Some(&mut ws), size, vec![]);
        for events in ui_test::click(find(&texts, "View").rect.center()) {
            frame(&ctx, &mut shell, Some(&mut ws), size, events);
        }
        let mut texts = Vec::new();
        for _ in 0..3 {
            texts = frame(&ctx, &mut shell, Some(&mut ws), size, vec![]).0;
        }
        let check = crate::icons::CHECK.codepoint;
        let marks: Vec<_> = texts.iter().filter(|t| t.text == check).collect();
        // The menu is drawn last: its item is the last text that reads `label`.
        let on = |label: &str| {
            let row = texts
                .iter()
                .rev()
                .find(|t| t.text == label)
                .unwrap_or_else(|| panic!("no {label}"))
                .rect;
            marks
                .iter()
                .find(|m| (m.rect.center().y - row.center().y).abs() < 4.0)
                .is_some_and(|m| m.colour.a() > 0)
        };
        assert!(
            on("Cell Grid") && on("Border") && on("Source"),
            "{:?}",
            ui_test::names(&texts)
        );
        assert!(!on("Colour-Limited Cells") && !on("Result"));
        find(&texts, "Overlay the Result");
        find(&texts, "Hide Dock");
        assert!(!texts.iter().any(|t| t.text.contains("Ghost")));
        assert_eq!(menu::overlay_label(ViewMode::Result), "Overlay the Source");
    }

    /// Usability review m8 (UX §4.3, §14 Q1): under 800 px of window the result is a
    /// 112 px strip (160 × 100 and its border), so the tabs move up by 88 px.
    #[test]
    fn a_short_window_collapses_the_result_to_a_strip() {
        use ui_test::{find, frame};
        assert_eq!(dock::result_height(640.0), 112.0);
        assert_eq!(dock::result_height(799.0), 112.0);
        assert_eq!(dock::result_height(800.0), layout::RESULT_HEIGHT);
        let tab_top = |h: f32| {
            let (ctx, mut ws) = two_layers();
            let mut shell = Shell::default();
            let size = egui::vec2(1024.0, h);
            frame(&ctx, &mut shell, Some(&mut ws), size, vec![]);
            let (texts, _) = frame(&ctx, &mut shell, Some(&mut ws), size, vec![]);
            texts
                .iter()
                .filter(|t| t.text == "Layers" && t.rect.left() > size.x - layout::DOCK_WIDTH - 1.0)
                .map(|t| t.rect.top())
                .next()
                .unwrap_or_else(|| panic!("no Layers tab in {:?}", ui_test::names(&texts)))
        };
        let (short, tall) = (tab_top(640.0), tab_top(900.0));
        assert!((tall - short - 88.0).abs() < 1.0, "{short} {tall}");
        let _ = find;
    }

    /// Usability review m9 (UX §6): Shift-drag moves a slider a tenth as far as a plain
    /// drag, and egui's `DragValue` does the same.
    #[test]
    fn shift_drag_makes_fine_steps() {
        fn run(shift: bool, drag_value: bool) -> f32 {
            let ctx = egui::Context::default();
            crate::theme::install(&ctx);
            let mods = if shift {
                egui::Modifiers::SHIFT
            } else {
                egui::Modifiers::NONE
            };
            let mut value = 50.0_f32;
            let step = |ctx: &egui::Context, events: Vec<egui::Event>, value: &mut f32| {
                let mut rect = egui::Rect::NOTHING;
                let mut all = vec![egui::Event::ModifiersChanged(mods)];
                all.extend(events);
                let input = egui::RawInput {
                    events: all,
                    ..Default::default()
                };
                let mut output = ctx.run_ui(input, |ui| {
                    ui.spacing_mut().slider_width = 200.0;
                    let mut v = *value;
                    let r = if drag_value {
                        ui.add(egui::DragValue::new(&mut v).range(0.0..=100.0))
                    } else {
                        let r = ui.add(egui::Slider::new(&mut v, 0.0..=100.0).fixed_decimals(2));
                        if let Some(f) = fine_slider(ui, &r, *value, 0.0..=100.0, 2) {
                            v = f;
                        }
                        r
                    };
                    rect = r.rect;
                    *value = v;
                });
                output.textures_delta.clear();
                rect
            };
            let rect = step(&ctx, vec![], &mut value);
            // Press on the slider's knob (its middle at 50) or on the field, and move 40 pt.
            let from = if drag_value {
                rect.center()
            } else {
                egui::pos2(rect.left() + 0.5 * rect.width(), rect.center().y)
            };
            let press = |pressed, pos| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: mods,
            };
            // A press on a slider's rail jumps there; the drag counts from the press.
            step(
                &ctx,
                vec![egui::Event::PointerMoved(from), press(true, from)],
                &mut value,
            );
            let start = value;
            for k in 1..=8 {
                let p = from + egui::vec2(5.0 * k as f32, 0.0);
                step(&ctx, vec![egui::Event::PointerMoved(p)], &mut value);
            }
            let to = from + egui::vec2(40.0, 0.0);
            step(&ctx, vec![press(false, to)], &mut value);
            value - start
        }
        for drag_value in [false, true] {
            let plain = run(false, drag_value);
            let fine = run(true, drag_value);
            assert!(plain > 1.0, "{drag_value}: {plain}");
            let ratio = fine / plain;
            assert!(
                (0.05..=0.2).contains(&ratio),
                "drag value {drag_value}: plain {plain}, fine {fine}"
            );
        }
    }

    /// Usability review m7: a layer row is the name over its subtitle, the recipe in
    /// short form with the opacity when not 100 %; nothing is clipped at the dock's edge;
    /// each strip icon has a tooltip.
    #[test]
    fn layer_rows_show_a_subtitle_and_the_strip_has_tooltips() {
        use convert64_project::{JsonCommand, LayerRef};
        use ui_test::{find, frame};
        let (ctx, mut ws) = two_layers();
        ws.doc
            .run(JsonCommand::LayerSetOpacity {
                layer: LayerRef::Index(1),
                opacity: 0.5,
            })
            .unwrap();
        let mut shell = Shell {
            tab: DockTab::Layers,
            layer: 1,
            ..Shell::default()
        };
        ctx.all_styles_mut(|s| s.interaction.tooltip_delay = 0.0);
        let size = egui::vec2(1280.0, 800.0);
        frame(&ctx, &mut shell, Some(&mut ws), size, vec![]);
        let (texts, _) = frame(&ctx, &mut shell, Some(&mut ws), size, vec![]);
        let top = find(&texts, "top").rect;
        let sub = find(&texts, "FS 100% · OKLab · Adaptive · 50% opacity").rect;
        assert!(sub.top() >= top.bottom() - 1.0 && (sub.left() - top.left()).abs() < 1.0);
        find(&texts, "FS 100% · OKLab · Adaptive");
        let dock_right = size.x - ui_test_margin();
        for t in &texts {
            if t.rect.left() > size.x - layout::DOCK_WIDTH {
                assert!(t.rect.right() <= dock_right + 0.5, "{:?} clipped", t.text);
            }
        }
        assert!(!texts.iter().any(|t| t.text == "50%"), "no opacity column");
        // Hover each strip icon: its tooltip names it.
        for (glyph, tip) in [
            (crate::icons::ADD, "New layer from file"),
            (crate::icons::DUPLICATE, "Duplicate"),
            (crate::icons::FILL, "Add mask"),
            (crate::icons::DELETE, "Delete layer"),
        ] {
            let at = texts
                .iter()
                .rev()
                .find(|t| t.text == glyph.codepoint)
                .expect("the strip icon")
                .rect
                .center();
            let mut shown = Vec::new();
            for _ in 0..3 {
                shown = frame(
                    &ctx,
                    &mut shell,
                    Some(&mut ws),
                    size,
                    vec![egui::Event::PointerMoved(at)],
                )
                .0;
            }
            assert!(
                shown.iter().any(|t| t.text.starts_with(tip)),
                "{tip}: {:?}",
                ui_test::names(&shown)
            );
        }
    }

    /// The dock's inner margin on the right, in points.
    fn ui_test_margin() -> f32 {
        6.0
    }

    /// A document with two layers, "ramp" and "top", and a context to draw it on.
    fn two_layers() -> (egui::Context, Workspace) {
        let png = |g: u8| {
            let rgba = (0..64 * 40)
                .flat_map(|i| [(i % 64 * 4) as u8, g, 90, 255])
                .collect();
            convert64_project::codec::encode_rgba8(64, 40, rgba)
        };
        let mut doc = crate::document::Document::from_png("ramp.png", &png(10)).unwrap();
        doc.add_layer("top.png", &png(200), None).unwrap();
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        (ctx, Workspace::new(doc))
    }

    fn draw(
        ctx: &egui::Context,
        shell: &mut Shell,
        ws: &mut Workspace,
        events: Vec<egui::Event>,
    ) -> Vec<Action> {
        let input = egui::RawInput {
            events,
            ..Default::default()
        };
        let mut actions = Vec::new();
        let mut out = ctx.run_ui(input, |ui| actions = shell.show(ui, Some(ws)));
        out.textures_delta.clear();
        actions
    }

    fn press(pos: egui::Pos2, pressed: bool) -> Vec<egui::Event> {
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            },
        ]
    }

    fn removes(actions: &[Action], i: usize) -> bool {
        use convert64_project::{JsonCommand, LayerRef};
        actions.iter().any(|a| {
            *a == Action::Run(JsonCommand::LayerRemove {
                layer: LayerRef::Index(i),
            })
        })
    }

    /// #245: deleting a layer names it in a toast whose Undo asks for an undo.
    #[test]
    fn deleting_a_layer_says_so_and_offers_undo() {
        let (ctx, mut ws) = two_layers();
        let mut shell = Shell {
            layer: 1,
            ..Shell::default()
        };
        draw(&ctx, &mut shell, &mut ws, vec![]);
        let none = egui::Modifiers::NONE;
        let actions = draw(
            &ctx,
            &mut shell,
            &mut ws,
            vec![key(egui::Key::Backspace, none)],
        );
        assert!(removes(&actions, 1), "{actions:?}");
        let (message, error, _) = shell.toast.clone().expect("a toast");
        assert_eq!(message, "Deleted layer 'top'");
        assert!(shell.toast_undo && !error);

        // The toast's last word is its Undo link.
        draw(&ctx, &mut shell, &mut ws, vec![]);
        let area = ctx
            .memory(|m| m.area_rect(egui::Id::new("toast")))
            .expect("the toast is drawn");
        let undo = area.right_center() - egui::vec2(20.0, 0.0);
        draw(&ctx, &mut shell, &mut ws, press(undo, true));
        let actions = draw(&ctx, &mut shell, &mut ws, press(undo, false));
        assert_eq!(actions, vec![Action::Undo]);
        assert!(shell.toast.is_none());
    }

    /// #245: `Delete` / `Backspace` delete only while the canvas or the Layers panel has
    /// the keys: after a press on either, or `Esc`; not after a press on the Recipe form
    /// or with another widget focused by `Tab`.
    #[test]
    fn delete_keys_act_only_from_the_canvas_or_the_layers_panel() {
        let none = egui::Modifiers::NONE;
        let (ctx, mut ws) = two_layers();
        let mut shell = Shell::default();
        let delete = |shell: &mut Shell, ws: &mut Workspace| {
            let actions = draw(&ctx, shell, ws, vec![key(egui::Key::Delete, none)]);
            shell.toast = None;
            // Whichever layer is selected (a click on the canvas may select one).
            removes(&actions, shell.layer)
        };
        draw(&ctx, &mut shell, &mut ws, vec![]);
        assert_eq!(
            shell.key_focus,
            KeyFocus::Canvas,
            "the canvas has the keys first"
        );
        assert!(delete(&mut shell, &mut ws));

        // A press on the Recipe form takes them away.
        let view = shell.view_rect.unwrap();
        let form = egui::pos2(view.right() + 60.0, view.bottom() - 40.0);
        draw(&ctx, &mut shell, &mut ws, press(form, true));
        draw(&ctx, &mut shell, &mut ws, press(form, false));
        assert_eq!(shell.key_focus, KeyFocus::Other);
        assert!(!delete(&mut shell, &mut ws));

        // `Esc` gives them back to the canvas.
        draw(
            &ctx,
            &mut shell,
            &mut ws,
            vec![key(egui::Key::Escape, none)],
        );
        assert!(delete(&mut shell, &mut ws));

        // A press on the canvas, and one on the Layers panel.
        draw(&ctx, &mut shell, &mut ws, press(form, true));
        draw(&ctx, &mut shell, &mut ws, press(form, false));
        draw(&ctx, &mut shell, &mut ws, press(view.center(), true));
        draw(&ctx, &mut shell, &mut ws, press(view.center(), false));
        assert_eq!(shell.key_focus, KeyFocus::Canvas);
        assert!(delete(&mut shell, &mut ws));
        shell.tab = DockTab::Layers;
        shell.key_focus = KeyFocus::Other;
        draw(&ctx, &mut shell, &mut ws, vec![]);
        let layers = shell.layers_rect.expect("the Layers tab was drawn");
        draw(&ctx, &mut shell, &mut ws, press(layers.center(), true));
        draw(&ctx, &mut shell, &mut ws, press(layers.center(), false));
        assert_eq!(shell.key_focus, KeyFocus::Layers);
        assert!(delete(&mut shell, &mut ws));

        // A widget outside both, focused with `Tab` (the first is the File menu).
        shell.key_focus = KeyFocus::Canvas;
        draw(&ctx, &mut shell, &mut ws, vec![key(egui::Key::Tab, none)]);
        draw(&ctx, &mut shell, &mut ws, vec![]);
        assert!(ctx.memory(|m| m.focused()).is_some());
        assert!(!delete(&mut shell, &mut ws));
    }

    /// The shell and, beside it, a slider and a `DragValue` as the Recipe form has
    /// them, frame after frame on one context (#244).
    struct Form {
        ctx: egui::Context,
        shell: Shell,
        /// A picture is open: the single keys pick tools only then (#292).
        ws: Workspace,
        slider: (f32, Option<egui::Response>),
        drag: (f32, Option<egui::Response>),
    }

    impl Form {
        fn new() -> Self {
            let (ctx, ws) = two_layers();
            let mut shell = Shell::default();
            shell.keep_camera();
            let mut form = Self {
                ctx,
                shell,
                ws,
                slider: (0.5, None),
                drag: (50.0, None),
            };
            form.frame(vec![]);
            form
        }

        fn frame(&mut self, events: Vec<egui::Event>) -> Vec<Action> {
            let input = egui::RawInput {
                events,
                ..Default::default()
            };
            let mut actions = Vec::new();
            let (shell, slider, drag) = (&mut self.shell, &mut self.slider, &mut self.drag);
            let ws = &mut self.ws;
            let mut output = self.ctx.run_ui(input, |ui| {
                actions = shell.show(ui, Some(ws));
                egui::Area::new(egui::Id::new("form"))
                    .fixed_pos([400.0, 300.0])
                    .show(ui.ctx(), |ui| {
                        let r = egui::Slider::new(&mut slider.0, 0.0..=1.0).show_value(false);
                        slider.1 = Some(ui.add(r));
                        drag.1 = Some(ui.add(egui::DragValue::new(&mut drag.0)));
                    });
            });
            output.textures_delta.clear();
            actions
        }

        fn slider_id(&self) -> egui::Id {
            self.slider.1.as_ref().unwrap().id
        }

        fn drag_id(&self) -> egui::Id {
            self.drag.1.as_ref().unwrap().id
        }

        /// Gives `id` keyboard focus, as `Tab` or a click would.
        fn focus(&mut self, id: egui::Id) {
            self.ctx.memory_mut(|m| m.request_focus(id));
            self.frame(vec![]);
            assert_eq!(self.focused(), Some(id));
        }

        fn focused(&self) -> Option<egui::Id> {
            self.ctx.memory(|m| m.focused())
        }

        fn pointer(&mut self, pos: egui::Pos2, pressed: Option<bool>) {
            let mut events = vec![egui::Event::PointerMoved(pos)];
            if let Some(pressed) = pressed {
                events.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                });
            }
            self.frame(events);
        }
    }

    /// #244: a slider with focus leaves single keys to the shortcuts (arrows stay with
    /// the slider), and `Enter` gives the keyboard back.
    #[test]
    fn a_focused_slider_leaves_single_keys_to_the_shortcuts() {
        let none = egui::Modifiers::NONE;
        let mut form = Form::new();
        form.focus(form.slider_id());
        form.frame(vec![key(egui::Key::R, none)]);
        assert_eq!(form.shell.tool, Tool::Region);
        let actions = form.frame(vec![key(egui::Key::ArrowLeft, none)]);
        assert!(actions.is_empty(), "arrows move the slider, not the layer");
        form.frame(vec![key(egui::Key::Enter, none)]);
        assert_eq!(form.focused(), None, "Enter drops focus");
        let actions = form.frame(vec![key(egui::Key::ArrowLeft, none)]);
        assert_eq!(actions.len(), 1, "arrows nudge again: {actions:?}");
    }

    /// #244: the end of a drag on a focused slider gives the keyboard back.
    #[test]
    fn a_slider_drag_ends_its_focus() {
        let mut form = Form::new();
        let id = form.slider_id();
        let rect = form.slider.1.as_ref().unwrap().rect;
        form.focus(id);
        form.pointer(rect.left_center() + egui::vec2(4.0, 0.0), Some(true));
        form.pointer(rect.center(), None);
        form.pointer(rect.right_center(), None);
        form.pointer(rect.right_center(), Some(false));
        form.frame(vec![]);
        assert!(
            form.slider.0 > 0.9,
            "the drag moved the slider: {}",
            form.slider.0
        );
        assert_eq!(form.focused(), None);
    }

    /// #244: digits typed into a `DragValue` hold the single keys until `Enter`.
    #[test]
    fn typing_into_a_drag_value_holds_single_keys_until_enter() {
        let none = egui::Modifiers::NONE;
        let mut form = Form::new();
        form.focus(form.drag_id());
        assert!(
            form.ctx.text_edit_focused(),
            "the DragValue takes typed digits"
        );
        form.frame(vec![key(egui::Key::R, none), egui::Event::Text("r".into())]);
        assert_ne!(form.shell.tool, Tool::Region, "R is typed, not a shortcut");
        form.frame(vec![key(egui::Key::Enter, none)]);
        form.frame(vec![key(egui::Key::R, none)]);
        assert_eq!(form.shell.tool, Tool::Region);
    }

    /// #244: a press elsewhere (the start of a slider drag) ends typing, not only a click.
    #[test]
    fn a_press_elsewhere_ends_typing() {
        let mut form = Form::new();
        form.focus(form.drag_id());
        let slider = form.slider.1.as_ref().unwrap().rect.center();
        form.pointer(slider, Some(true));
        assert!(!form.ctx.text_edit_focused());
        form.pointer(slider, Some(false));
        form.frame(vec![key(egui::Key::R, egui::Modifiers::NONE)]);
        assert_eq!(form.shell.tool, Tool::Region);
    }

    /// #244: `Esc` ends typing and returns the keys to the canvas in the same frame.
    #[test]
    fn esc_returns_the_keys_to_the_canvas() {
        let none = egui::Modifiers::NONE;
        let mut form = Form::new();
        form.focus(form.drag_id());
        form.frame(vec![key(egui::Key::Escape, none), key(egui::Key::R, none)]);
        assert_eq!(form.focused(), None);
        assert_eq!(form.shell.tool, Tool::Region);
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
        let (shell, _) = frame_open(
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

    /// #286: under the browser's keymap the keys a browser keeps do nothing (the browser
    /// acts on them first) and the alternatives work: `Shift Cmd L` adds a layer, `S`
    /// picks Transform. Natively `Cmd T` and `Shift Cmd N` keep working, and
    /// `Shift Cmd L` too.
    #[test]
    fn the_browser_keymap_binds_the_alternatives() {
        let web = || Shell {
            keys: shortcuts::Keymap::WEB,
            ..Shell::default()
        };
        for r in shortcuts::BROWSER_RESERVED {
            let (shell, actions) = frame(web(), vec![key(r.logical_key, r.modifiers)]);
            assert!(actions.is_empty(), "{r:?}: {actions:?}");
            assert_eq!(shell.tool, Tool::Nudge, "{r:?}");
        }
        let shift_cmd = egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT);
        let (_, actions) = frame(web(), vec![key(egui::Key::L, shift_cmd)]);
        assert_eq!(actions, vec![Action::NewLayer]);
        let (shell, _) = frame_open(web(), vec![key(egui::Key::S, egui::Modifiers::NONE)]);
        assert_eq!(shell.tool, Tool::Transform);

        let native = || Shell {
            keys: shortcuts::Keymap::NATIVE,
            ..Shell::default()
        };
        let (shell, _) = frame_open(native(), vec![key(egui::Key::T, egui::Modifiers::COMMAND)]);
        assert_eq!(shell.tool, Tool::Transform);
        for k in [egui::Key::N, egui::Key::L] {
            let (_, actions) = frame(native(), vec![key(k, shift_cmd)]);
            assert_eq!(actions, vec![Action::NewLayer], "{k:?}");
        }
        let (_, actions) = frame(native(), vec![key(egui::Key::Q, egui::Modifiers::COMMAND)]);
        assert_eq!(actions, vec![Action::Quit]);
    }

    /// #287, UX §8: while a dialog is open (the quit question) no shortcut acts behind
    /// it, `Q` and `Shift C` among them (the usability review saw both act behind the
    /// old export dialog); `Esc` closes it, as Cancel, and the shortcuts work again.
    #[test]
    fn an_open_dialog_blocks_shortcuts_and_esc_closes_it() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let mut shell = Shell {
            quit_prompt: Some("title.png".to_owned()),
            ..Shell::default()
        };
        let run = |shell: &mut Shell, events: Vec<egui::Event>| {
            let input = egui::RawInput {
                events,
                ..Default::default()
            };
            let mut actions = Vec::new();
            let mut out = ctx.run_ui(input, |ui| actions = shell.show(ui, None));
            out.textures_delta.clear();
            actions
        };
        let none = egui::Modifiers::NONE;
        let cmd = egui::Modifiers::COMMAND;
        // The modal's first frame.
        assert!(run(&mut shell, vec![]).is_empty());
        let actions = run(
            &mut shell,
            vec![
                key(egui::Key::Q, none),
                key(egui::Key::C, egui::Modifiers::SHIFT),
                key(egui::Key::P, none),
                key(egui::Key::E, cmd),
                key(egui::Key::E, cmd.plus(egui::Modifiers::SHIFT)),
                key(egui::Key::O, cmd),
                key(egui::Key::Backslash, cmd),
            ],
        );
        assert!(actions.is_empty(), "{actions:?}");
        assert_eq!(shell.view, ViewMode::Source, "Q acted behind the dialog");
        assert!(!shell.overlays.clashes, "Shift C acted behind the dialog");
        assert_eq!(shell.tool, Tool::Nudge);
        assert_eq!(shell.tab, DockTab::Recipe);
        assert!(!shell.chrome_hidden);
        assert!(shell.quit_prompt.is_some());

        let actions = run(&mut shell, vec![key(egui::Key::Escape, none)]);
        assert_eq!(actions, vec![Action::AnswerQuit(QuitAnswer::Cancel)]);
        assert!(shell.quit_prompt.is_none());
        // The modal is gone once a frame has drawn without it.
        run(&mut shell, vec![]);
        run(&mut shell, vec![key(egui::Key::Q, none)]);
        assert_eq!(shell.view, ViewMode::Result, "Q works again");
    }

    /// #287, UX §6b: `Cmd E` shows the Export panel, the dock and UI with it;
    /// `Shift Cmd E` asks for Export Again.
    #[test]
    fn cmd_e_shows_the_export_panel_and_shift_cmd_e_exports_again() {
        let cmd = egui::Modifiers::COMMAND;
        let hidden = Shell {
            dock_hidden: true,
            chrome_hidden: true,
            ..Shell::default()
        };
        let (shell, actions) = frame(hidden, vec![key(egui::Key::E, cmd)]);
        assert!(actions.is_empty(), "{actions:?}");
        assert_eq!(shell.tab, DockTab::Export);
        assert!(!shell.dock_hidden && !shell.chrome_hidden);
        let (shell, actions) = frame(
            Shell::default(),
            vec![key(egui::Key::E, cmd.plus(egui::Modifiers::SHIFT))],
        );
        assert_eq!(actions, vec![Action::Export { again: true }]);
        assert_eq!(shell.tab, DockTab::Recipe);
    }

    /// The galley rectangle of the first text shape `ctx` drew last frame that reads
    /// `text`.
    fn text_rect(output: &egui::FullOutput, text: &str) -> Option<egui::Rect> {
        output.shapes.iter().find_map(|s| match &s.shape {
            egui::Shape::Text(t) if t.galley.text() == text => {
                Some(t.galley.rect.translate(t.pos.to_vec2()))
            }
            _ => None,
        })
    }

    /// #287, UX §6b: the Export panel shows each row's file name preview, adds a row
    /// with **+ Add format** and exports with its one primary button, every change an
    /// `export_set` (so it is saved with the project and undoes).
    #[test]
    fn the_export_panel_lists_rows_adds_one_and_exports() {
        use convert64_core::export::{ExportEntry, ExportFormat};
        let (ctx, mut ws) = two_layers();
        let rows = vec![
            ExportEntry::new(ExportFormat::Picture),
            ExportEntry::new(ExportFormat::Prg),
        ];
        ws.doc
            .run(convert64_project::JsonCommand::ExportSet(
                convert64_core::command::ExportSet {
                    exports: rows.clone(),
                },
            ))
            .unwrap();
        let mut scheduler = crate::scheduler::Scheduler::stepping(Default::default());
        scheduler.submit(
            ws.doc.snapshot(None),
            convert64_core::solve::SolveMode::Full,
        );
        ws.solution = loop {
            scheduler.tick();
            if let Some(r) = scheduler.poll().filter(|r| !r.provisional) {
                break Some(r.solution);
            }
        };
        ws.solving = false;
        let mut shell = Shell {
            tab: DockTab::Export,
            ..Shell::default()
        };
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 900.0));
        let mut draw = |shell: &mut Shell, events: Vec<egui::Event>| {
            let input = egui::RawInput {
                screen_rect: Some(screen),
                events,
                ..Default::default()
            };
            let mut actions = Vec::new();
            let mut out = ctx.run_ui(input, |ui| actions = shell.show(ui, Some(&mut ws)));
            out.textures_delta.clear();
            (out, actions)
        };
        let mut out = None;
        for _ in 0..3 {
            let (o, actions) = draw(&mut shell, vec![]);
            assert!(actions.is_empty(), "{actions:?}");
            out = Some(o);
        }
        let out = out.unwrap();
        let texts: Vec<String> = out
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Text(t) => Some(t.galley.text().to_owned()),
                _ => None,
            })
            .collect();
        assert!(text_rect(&out, "ramp.kla · 10 kB").is_some(), "{texts:?}");
        assert!(text_rect(&out, "Koala (.kla)").is_some());
        let add = text_rect(&out, "Add format").expect("the + Add format button");
        let export = text_rect(&out, "Export 2 formats").expect("the Export button");

        draw(&mut shell, press(add.center(), true));
        let (_, actions) = draw(&mut shell, press(add.center(), false));
        let mut more = rows.clone();
        more.push(ExportEntry::new(ExportFormat::Png));
        assert_eq!(
            actions,
            vec![Action::Run(convert64_project::JsonCommand::ExportSet(
                convert64_core::command::ExportSet { exports: more }
            ))]
        );
        draw(&mut shell, press(export.center(), true));
        let (_, actions) = draw(&mut shell, press(export.center(), false));
        assert_eq!(actions, vec![Action::Export { again: false }]);
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
        let (shell, _) = frame_open(
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
