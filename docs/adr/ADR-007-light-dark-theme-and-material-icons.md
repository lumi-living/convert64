# ADR-007: Light and dark themes; Material Symbols as the icon font

**Status:** Accepted
**Date:** 2026-10-11
**Deciders:** Jaakko Haaparanta

## Context

The egui app (ADR-006) needs to work in light and dark mode, and needs one icon set that covers an image-editor-style UI (layers, visibility, lock, pin, nudge, feather, compare, undo/redo, export). egui renders icons as font glyphs, so the icon choice and theming interact: glyphs take the current text colour and scale with text size.

## Decision

1. **Both themes, system-following by default.** The app uses egui's built-in `Visuals::light()` / `Visuals::dark()`, follows the OS preference via eframe, and offers View → Theme: System / Light / Dark, persisted in app settings. Convert64's own semantic colours (clash heatmap, pin/lock markers, attribute-map overlay, cell grid, selection) are defined once per theme in a small `Theme` struct so they stay legible on both. **The C64 preview is never themed** — it shows hardware palette colours inside a border; only the chrome around it (frame, transparency checkerboard) changes with the theme.
2. **Material Symbols via the `egui_material_icons` crate** (Apache-2.0, MIT-compatible). Font-glyph icons inherit theme colour automatically. Crate checked 2026-10-11: v0.8.0, ~100k downloads, actively maintained.

## Options considered

| Option | Verdict |
|---|---|
| Material Symbols via `egui_material_icons` | **Chosen** — maintained, font-based, complete coverage (`layers`, `visibility`, `lock`, `push_pin`, `open_with`, `tune`, `grid_on`, `palette`, `blur_on`, `contrast`, `compare`, `flip`, `undo`, `redo`, `download`, `folder_open`, `save`) |
| Lucide via `egui-lucide` | Rejected for now — v0.1.0 with ~300 downloads, SVG-based (one texture per icon per colour and size), young API |
| Lucide as a hand-loaded font | Feasible (the Lucide TTF exists) but no maintained crate; revisit if Material's look proves wrong for the tool |
| Phosphor via `egui-phosphor` | Mature and font-based, but the user preference was Material or Lucide |

## Consequences
- Easier: theme switch is one call; icons recolour for free; one dependency for icons.
- Harder: Material Symbols is a variable font and egui renders one static instance (Outlined, regular weight, unfilled); "filled" variants are not available without a second font file.
- Revisit when: a dedicated icon style for Convert64 is wanted (then a custom font or SVG set, same loader pattern).

## Action items
- [ ] E8: `Theme` struct with per-theme semantic colours; View → Theme menu; persist choice
- [ ] E8: add `egui_material_icons` to the workspace; icon constants in one module
- [ ] Screenshot both themes in the README (E9)
