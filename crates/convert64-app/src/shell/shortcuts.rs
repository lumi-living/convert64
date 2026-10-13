//! The keyboard shortcuts of `docs/UX.md` §8 that have a menu item or a shell action.
//! `COMMAND` is `Cmd` on macOS and `Ctrl` elsewhere; menus show each one with
//! `ctx.format_shortcut`.

use egui::{Key, KeyboardShortcut, Modifiers};

const fn cmd(key: Key) -> KeyboardShortcut {
    KeyboardShortcut::new(Modifiers::COMMAND, key)
}

const fn shift_cmd(key: Key) -> KeyboardShortcut {
    KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), key)
}

const fn bare(key: Key) -> KeyboardShortcut {
    KeyboardShortcut::new(Modifiers::NONE, key)
}

/// Open image or project.
pub const OPEN: KeyboardShortcut = cmd(Key::O);
/// Save project.
pub const SAVE: KeyboardShortcut = cmd(Key::S);
/// Save project as.
pub const SAVE_AS: KeyboardShortcut = shift_cmd(Key::S);
/// Export.
pub const EXPORT: KeyboardShortcut = cmd(Key::E);
/// Export again with the same settings.
pub const EXPORT_AGAIN: KeyboardShortcut = shift_cmd(Key::E);
/// Quit (native).
pub const QUIT: KeyboardShortcut = cmd(Key::Q);
/// Undo.
pub const UNDO: KeyboardShortcut = cmd(Key::Z);
/// Redo.
pub const REDO: KeyboardShortcut = shift_cmd(Key::Z);
/// Redo, as GIMP users press it.
pub const REDO_ALT: KeyboardShortcut = cmd(Key::Y);
/// Delete the selected layer.
pub const DELETE_LAYER: KeyboardShortcut = bare(Key::Delete);
/// Delete the selected layer (Backspace).
pub const DELETE_LAYER_ALT: KeyboardShortcut = bare(Key::Backspace);
/// Show or hide the selected layer.
pub const TOGGLE_VISIBLE: KeyboardShortcut = cmd(Key::Comma);
/// New layer from file.
pub const NEW_LAYER: KeyboardShortcut = shift_cmd(Key::N);
/// Duplicate layer.
pub const DUPLICATE_LAYER: KeyboardShortcut = cmd(Key::J);
/// Zoom in.
pub const ZOOM_IN: KeyboardShortcut = cmd(Key::Equals);
/// Zoom out.
pub const ZOOM_OUT: KeyboardShortcut = cmd(Key::Minus);
/// Zoom to fit.
pub const ZOOM_FIT: KeyboardShortcut = cmd(Key::Num0);
/// Zoom to 100 %.
pub const ZOOM_100: KeyboardShortcut = cmd(Key::Num1);
/// Cell grid overlay.
pub const GRID: KeyboardShortcut = cmd(Key::Quote);
/// Clash heatmap overlay.
pub const CLASHES: KeyboardShortcut = KeyboardShortcut::new(Modifiers::SHIFT, Key::C);
/// Border around the result.
pub const BORDER: KeyboardShortcut = cmd(Key::B);
/// Compare: cycle split, A/B flicker, side by side, off.
pub const COMPARE: KeyboardShortcut = bare(Key::Comma);
/// Working view: Source ⇄ Result.
pub const SOURCE_RESULT: KeyboardShortcut = bare(Key::Q);
/// Show or hide the UI: menu bar, toolbar, dock and status bar (Figma's `Cmd \`). `Esc`
/// also shows it again. Not `Tab`, which is egui's and the browser's focus traversal.
pub const HIDE_UI: KeyboardShortcut = cmd(Key::Backslash);
/// Show or hide the dock. With Shift held most layouts report `\` as `|`, so the shell
/// matches [`Key::Pipe`] too.
pub const HIDE_DOCK: KeyboardShortcut = shift_cmd(Key::Backslash);
/// Next-worst cell.
pub const NEXT_WORST: KeyboardShortcut = shift_cmd(Key::CloseBracket);
/// Previous worst cell.
pub const PREVIOUS_WORST: KeyboardShortcut = shift_cmd(Key::OpenBracket);
/// Layer via cut (with a selection).
pub const VIA_CUT: KeyboardShortcut = shift_cmd(Key::J);
/// Mask mode on and off.
pub const MASK_MODE: KeyboardShortcut = bare(Key::M);
/// Transform tool.
pub const TRANSFORM: KeyboardShortcut = cmd(Key::T);
/// Snap: Off → C64 pixel → Cell.
pub const SNAP_CYCLE: KeyboardShortcut = shift_cmd(Key::Semicolon);
/// Scale the selected layer up 1 %.
pub const SCALE_UP: KeyboardShortcut = cmd(Key::CloseBracket);
/// Scale the selected layer down 1 %.
pub const SCALE_DOWN: KeyboardShortcut = cmd(Key::OpenBracket);
