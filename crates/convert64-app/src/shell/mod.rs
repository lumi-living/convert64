//! The app shell (`docs/UX.md` §3, Figma `App Shell / Light` and `/ Dark`): menu bar,
//! toolbar, the working view, the right dock (Result on top, then the Recipe, Layers and
//! Export tabs) and the status bar.
//!
//! [`Shell`] holds what the chrome shows and [`Shell::show`] draws it, returning the
//! [`Action`]s the user asked for this frame; the app carries them out. Sizes come from
//! [`crate::theme::layout`] and colours from [`crate::theme::Theme`].

mod dock;
mod export_dialog;
mod menu;
pub mod shortcuts;
mod status;
mod toolbar;
mod working;

use crate::icons::Icon;
use crate::theme::layout;
use crate::view::{Camera, Compare, Overlays};
use crate::workspace::Workspace;
use convert64_core::mode::Mode;

pub use crate::view::ViewMode;
pub use export_dialog::ExportDialog;

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
#[derive(Debug, Clone, PartialEq, Eq)]
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
        }
        self.export_requested = false;

        if !self.chrome_hidden {
            egui::Panel::top("menu_bar")
                .show(ui, |ui| menu::show(self, ws.as_deref(), ui, &mut actions));
            egui::Panel::top("toolbar")
                .exact_size(layout::TOOLBAR_HEIGHT)
                .show(ui, |ui| toolbar::show(self, ui));
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
            .show(ui, |ui| working::show(self, ui, ws.as_deref()));
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
            for tool in Tool::ALL {
                if i.consume_key(egui::Modifiers::NONE, tool.key()) {
                    self.tool = tool;
                }
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
}
