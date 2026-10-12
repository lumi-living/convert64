//! The app shell (`docs/UX.md` §3, Figma `App Shell / Light` and `/ Dark`): menu bar,
//! toolbar, the working view, the right dock (Result on top, then the Recipe, Layers and
//! Export tabs) and the status bar.
//!
//! [`Shell`] holds what the chrome shows and [`Shell::show`] draws it, returning the
//! [`Action`]s the user asked for this frame; the app carries them out. Sizes come from
//! [`crate::theme::layout`] and colours from [`crate::theme::Theme`].

mod dock;
mod menu;
pub mod shortcuts;
mod status;
mod toolbar;
mod working;

use crate::icons::Icon;
use crate::theme::layout;

pub use status::Status;

/// What the working view shows (`Q` switches; `docs/UX.md` §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ViewMode {
    /// The layer composite, at full precision. The default on open.
    #[default]
    Source,
    /// The C64 result, zoomed, with overlays.
    Result,
}

impl ViewMode {
    /// The other mode.
    pub fn flipped(self) -> Self {
        match self {
            Self::Source => Self::Result,
            Self::Result => Self::Source,
        }
    }

    /// The mode's name, as the status bar and the toolbar hint show it.
    pub fn label(self) -> &'static str {
        match self {
            Self::Source => "Source",
            Self::Result => "Result",
        }
    }
}

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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Close the app (native) or do nothing (web).
    Quit,
    /// Follow the system theme, or force light or dark (View › Theme).
    SetTheme(egui::ThemePreference),
}

/// The smallest and largest dock width (`docs/UX.md` §3: 260–420 px). Not a Figma
/// variable yet; see the #203 follow-ups.
pub const DOCK_WIDTH_RANGE: std::ops::RangeInclusive<f32> = 260.0..=420.0;

/// The zoom steps of the working view, in percent (`Cmd =` / `Cmd −`).
const ZOOM_STEPS: [u32; 10] = [25, 50, 100, 200, 300, 400, 600, 800, 1200, 1600];

/// What the chrome shows: the working view's mode and zoom, the tool, the dock tab and
/// what is hidden. Everything else lives in the project.
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
    /// The cell grid overlay (`Cmd '`).
    pub grid: bool,
    /// The working view's zoom, in percent.
    pub zoom: u32,
    /// Whether a project is open: without one the dock's groups are disabled, not hidden
    /// (`docs/UX.md` §11).
    pub has_project: bool,
    /// The theme preference View › Theme shows as chosen.
    pub theme: egui::ThemePreference,
}

impl Default for Shell {
    fn default() -> Self {
        Self {
            view: ViewMode::default(),
            tool: Tool::default(),
            tab: DockTab::default(),
            chrome_hidden: false,
            dock_hidden: false,
            grid: false,
            zoom: 400,
            has_project: false,
            theme: egui::ThemePreference::System,
        }
    }
}

impl Shell {
    /// The next zoom step in, or the largest.
    pub fn zoom_in(&mut self) {
        self.zoom = ZOOM_STEPS
            .into_iter()
            .find(|&z| z > self.zoom)
            .unwrap_or(ZOOM_STEPS[ZOOM_STEPS.len() - 1]);
    }

    /// The next zoom step out, or the smallest.
    pub fn zoom_out(&mut self) {
        self.zoom = ZOOM_STEPS
            .into_iter()
            .rev()
            .find(|&z| z < self.zoom)
            .unwrap_or(ZOOM_STEPS[0]);
    }

    /// Draws the whole shell into `ui` (the app's root) and returns what the user asked
    /// for. Keyboard shortcuts are read first, unless a text field has focus.
    pub fn show(&mut self, ui: &mut egui::Ui, status: &Status) -> Vec<Action> {
        let mut actions = Vec::new();
        self.handle_shortcuts(ui.ctx(), &mut actions);

        if !self.chrome_hidden {
            egui::Panel::top("menu_bar").show(ui, |ui| menu::show(self, ui, &mut actions));
            egui::Panel::top("toolbar")
                .exact_size(layout::TOOLBAR_HEIGHT)
                .show(ui, |ui| toolbar::show(self, ui));
            egui::Panel::bottom("status_bar")
                .exact_size(layout::STATUS_HEIGHT)
                .show(ui, |ui| status::show(self, status, ui));
            if !self.dock_hidden {
                egui::Panel::right("dock")
                    .resizable(true)
                    .default_size(layout::DOCK_WIDTH)
                    .size_range(DOCK_WIDTH_RANGE)
                    .frame(egui::Frame::side_top_panel(ui.style()).inner_margin(0))
                    .show(ui, |ui| dock::show(self, status, ui));
            }
        }

        let canvas = crate::theme::Theme::current(ui.ctx()).canvas_bg;
        egui::CentralPanel::default()
            .frame(egui::Frame::central_panel(ui.style()).fill(canvas))
            .show(ui, |ui| working::show(self, ui));
        actions
    }

    /// The shortcuts of `docs/UX.md` §8 that the shell itself handles.
    fn handle_shortcuts(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        use shortcuts as s;
        // Single keys only when no text field wants them.
        let typing = ctx.egui_wants_keyboard_input();
        ctx.input_mut(|i| {
            if i.consume_shortcut(&s::ZOOM_IN) {
                self.zoom_in();
            }
            if i.consume_shortcut(&s::ZOOM_OUT) {
                self.zoom_out();
            }
            if i.consume_shortcut(&s::ZOOM_100) {
                self.zoom = 100;
            }
            if i.consume_shortcut(&s::GRID) {
                self.grid = !self.grid;
            }
            if i.consume_shortcut(&s::QUIT) {
                actions.push(Action::Quit);
            }
            if typing {
                return;
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

    #[test]
    fn zoom_steps_clamp_at_both_ends() {
        let mut shell = Shell::default();
        assert_eq!(shell.zoom, 400);
        shell.zoom_in();
        assert_eq!(shell.zoom, 600);
        for _ in 0..20 {
            shell.zoom_in();
        }
        assert_eq!(shell.zoom, 1600);
        for _ in 0..20 {
            shell.zoom_out();
        }
        assert_eq!(shell.zoom, 25);
        shell.zoom = 150;
        shell.zoom_out();
        assert_eq!(shell.zoom, 100);
    }

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
            actions = shell.show(ui, &Status::default());
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
    fn command_shortcuts_zoom_and_toggle_the_grid() {
        let (shell, _) = frame(
            Shell::default(),
            vec![
                key(egui::Key::Equals, egui::Modifiers::COMMAND),
                key(egui::Key::Quote, egui::Modifiers::COMMAND),
            ],
        );
        assert_eq!(shell.zoom, 600);
        assert!(shell.grid);
    }
}
