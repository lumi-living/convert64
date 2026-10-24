//! The keyboard shortcuts of `docs/UX.md` §8 that have a menu item or a shell action.
//! `COMMAND` is `Cmd` on macOS and `Ctrl` elsewhere; menus show each one with
//! `ctx.format_shortcut`.
//!
//! A browser keeps some keys from the page (usability review M3, #286): Chrome opens a
//! tab on `Cmd T`, a window on `Cmd N`, an incognito window on `Shift Cmd N` and closes
//! the tab on `Cmd W`, whatever the page does. The few bindings those keys had differ by
//! platform, in [`Keymap`]: the browser build binds and shows alternatives, and
//! [`Keymap::WEB`] holds none of [`BROWSER_RESERVED`].

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
/// New layer from file, as the browser build binds it (and natively too, beside
/// `Shift Cmd N`): Chrome keeps `Shift Cmd N` for an incognito window (#286).
pub const NEW_LAYER_ALT: KeyboardShortcut = shift_cmd(Key::L);
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
/// The colour-limited cells overlay (the clash heatmap; #243).
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
/// Snap: Off → C64 pixel → Cell.
pub const SNAP_CYCLE: KeyboardShortcut = shift_cmd(Key::Semicolon);
/// Scale the selected layer up 1 %.
pub const SCALE_UP: KeyboardShortcut = cmd(Key::CloseBracket);
/// Scale the selected layer down 1 %.
pub const SCALE_DOWN: KeyboardShortcut = cmd(Key::OpenBracket);

/// The keys a browser keeps from the page (#286): Chrome opens a new tab (`Cmd T`), a
/// window (`Cmd N`) or an incognito window (`Shift Cmd N`), closes the tab (`Cmd W`) or
/// the window (`Shift Cmd W`), reopens a closed tab (`Shift Cmd T`), and on macOS
/// `Cmd Q` quits it.
pub const BROWSER_RESERVED: [KeyboardShortcut; 7] = [
    cmd(Key::T),
    cmd(Key::N),
    shift_cmd(Key::N),
    cmd(Key::W),
    shift_cmd(Key::W),
    shift_cmd(Key::T),
    cmd(Key::Q),
];

/// The bindings that differ between the native and the browser build (UX §8, #286).
/// Menus and tooltips show the shell's, so they show the keys that work there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Keymap {
    /// New layer from file: `Shift Cmd N` natively, `Shift Cmd L` in the browser.
    pub new_layer: KeyboardShortcut,
    /// The Transform tool beside its `S`: `Cmd T` natively, none in the browser.
    pub transform: Option<KeyboardShortcut>,
    /// Quit: `Cmd Q` natively; the browser has no quit (and `Cmd Q` quits it on macOS).
    pub quit: Option<KeyboardShortcut>,
}

impl Keymap {
    /// The native build's: the editors' conventions.
    pub const NATIVE: Self = Self {
        new_layer: shift_cmd(Key::N),
        transform: Some(cmd(Key::T)),
        quit: Some(cmd(Key::Q)),
    };

    /// The browser build's: nothing the browser keeps.
    pub const WEB: Self = Self {
        new_layer: NEW_LAYER_ALT,
        transform: None,
        quit: None,
    };

    /// This build's.
    pub const CURRENT: Self = if cfg!(target_arch = "wasm32") {
        Self::WEB
    } else {
        Self::NATIVE
    };

    /// Every shortcut with a modifier the shell binds under this keymap, by name. The
    /// single keys (tools, `Q`, `Shift C`, `Delete`, arrows) are never a browser's.
    pub fn bindings(&self) -> Vec<(&'static str, KeyboardShortcut)> {
        let alt_cmd = Modifiers::COMMAND.plus(Modifiers::ALT);
        let mut all = vec![
            ("open", OPEN),
            ("save", SAVE),
            ("save as", SAVE_AS),
            ("export", EXPORT),
            ("export again", EXPORT_AGAIN),
            ("undo", UNDO),
            ("redo", REDO),
            ("redo (alt)", REDO_ALT),
            ("show or hide layer", TOGGLE_VISIBLE),
            ("new layer", self.new_layer),
            ("new layer (alt)", NEW_LAYER_ALT),
            ("duplicate layer", DUPLICATE_LAYER),
            ("zoom in", ZOOM_IN),
            ("zoom out", ZOOM_OUT),
            ("zoom to fit", ZOOM_FIT),
            ("zoom 100 %", ZOOM_100),
            ("grid", GRID),
            ("border", BORDER),
            ("hide UI", HIDE_UI),
            ("hide dock", HIDE_DOCK),
            ("hide dock (|)", shift_cmd(Key::Pipe)),
            ("next worst", NEXT_WORST),
            ("previous worst", PREVIOUS_WORST),
            ("layer via cut", VIA_CUT),
            ("snap", SNAP_CYCLE),
            ("scale up", SCALE_UP),
            ("scale down", SCALE_DOWN),
            (
                "rotate -0.1°",
                KeyboardShortcut::new(alt_cmd, Key::OpenBracket),
            ),
            (
                "rotate +0.1°",
                KeyboardShortcut::new(alt_cmd, Key::CloseBracket),
            ),
            (
                "rotate -1°",
                KeyboardShortcut::new(alt_cmd.plus(Modifiers::SHIFT), Key::OpenBracket),
            ),
            (
                "rotate +1°",
                KeyboardShortcut::new(alt_cmd.plus(Modifiers::SHIFT), Key::CloseBracket),
            ),
        ];
        all.extend(self.transform.map(|s| ("transform", s)));
        all.extend(self.quit.map(|s| ("quit", s)));
        all
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #286: the browser build binds none of the keys a browser keeps; the native build
    /// keeps the editors' conventions.
    #[test]
    fn the_web_keymap_holds_no_browser_reserved_keys() {
        for (name, s) in Keymap::WEB.bindings() {
            for r in BROWSER_RESERVED {
                assert!(
                    !(s.logical_key == r.logical_key && s.modifiers == r.modifiers),
                    "{name} is browser-reserved"
                );
            }
        }
        let native = Keymap::NATIVE.bindings();
        for (name, s) in [
            ("transform", cmd(Key::T)),
            ("new layer", shift_cmd(Key::N)),
            ("quit", cmd(Key::Q)),
        ] {
            assert!(native.contains(&(name, s)), "native {name}");
        }
        assert!(
            Keymap::WEB
                .bindings()
                .contains(&("new layer", shift_cmd(Key::L)))
        );
        // No two bindings of one keymap share keys.
        for keymap in [Keymap::NATIVE, Keymap::WEB] {
            let all = keymap.bindings();
            for (i, (a, s)) in all.iter().enumerate() {
                for (b, t) in &all[i + 1..] {
                    let same = s.logical_key == t.logical_key && s.modifiers == t.modifiers;
                    assert!(!same || s == &keymap.new_layer, "{a} and {b} share keys");
                }
            }
        }
    }
}
