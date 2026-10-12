# ADR-007: Light and dark themes; Material Symbols as the icon font

**Status:** Accepted
**Date:** 2026-10-11
**Deciders:** Jaakko Haaparanta

## Context

The egui app (ADR-006) needs to work in light and dark mode, and needs one icon set that covers an image-editor-style UI (layers, visibility, lock, pin, nudge, feather, compare, undo/redo, export). egui renders icons as font glyphs, so the icon choice and theming interact: glyphs take the current text colour and scale with text size.

## Decision

1. **Both themes, system-following by default.** The app uses egui's built-in `Visuals::light()` / `Visuals::dark()`, follows the OS preference via eframe, and offers View → Theme: System / Light / Dark, persisted in app settings. Convert64's own semantic colours (clash heatmap, pin/lock markers, attribute-map overlay, cell grid, selection) are defined once per theme in a small `Theme` struct so they stay legible on both. **The C64 preview is never themed** — it shows hardware palette colours inside a border; only the chrome around it (frame, transparency checkerboard) changes with the theme.
2. **Material Symbols via the `egui_material_icons` crate** (Apache-2.0, MIT-compatible). Font-glyph icons inherit theme colour automatically. Crate checked 2026-10-11: v0.8.0, ~100k downloads, actively maintained.

## Glyph map (verified against Material Symbols 0.33, outlined 400)

Confirmed 2026-10-11 in a side-by-side comparison with Phosphor at 12–32 px; Material chosen for legibility at 12–14 px and complete coverage.

| UI need | Glyph | UI need | Glyph |
|---|---|---|---|
| Layers panel | `layers` | Flip layer | `flip` |
| Layer visible / hidden | `visibility` / `visibility_off` | Undo / Redo | `undo` / `redo` |
| Lock / unlock cell | `lock` / `lock_open` | Export | `download` |
| Pin colour set | `keep` | Open / Save | `folder_open` / `save` |
| Nudge / move | `open_with` | Add / Delete layer | `add` / `delete` |
| Dither recipe | `tune` | Drag handle | `drag_indicator` |
| Cell grid | `grid_on` | Zoom in / out | `zoom_in` / `zoom_out` |
| Palette preset | `palette` | Paint mask | `brush` |
| Feather mask | `gradient` | Clash warning | `warning` |
| Contrast / levels | `contrast` | Settings | `settings` |
| A/B compare | `compare` | Light / dark theme | `light_mode` / `dark_mode` |

## Axis values (binding for code and Figma)

`egui_material_icons` renders one static cut of the variable font. Figma uses the same cut, imported as SVG from `google/material-design-icons` (`symbols/web/<id>/materialsymbolsoutlined/<id>_20px.svg`), so the two never drift:

| Axis | Value |
|---|---|
| Style | Outlined |
| FILL | 0 |
| wght | 400 |
| GRAD | 0 |
| opsz | 20 (rendered at `layout::ICON_SIZE` = 18 px) |

In the Figma library the `Icon` component set has one variant per glyph, named by icon id (`keep`, `open_with`…), which maps to `egui_material_icons::icons::ICON_<ID>`. Google's Material 3 Design Kit library was evaluated and rejected as the source: its icons are vector components too, but it ships only a curated subset without most of the glyphs above. See `docs/figma.md`.

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
- [x] E8: `Theme` struct with per-theme semantic colours (#195: generated from `design/tokens.json` by `cargo xtask tokens`; system-following by default, `theme::set_theme` to switch)
- [ ] E8: View → Theme menu; persist choice
- [x] E8: add `egui_material_icons` to the workspace (#195: 0.8.0, `outline` feature only)
- [x] E8: icon constants in one module (#203: `convert64_app::icons`)
- [ ] Decide on the icon cut: `egui_material_icons` 0.8.0 ships Material Symbols **Rounded** (its `outline` feature is the unfilled Rounded font), not the Outlined cut the axis table and the Figma `Icon` set use (found in #195)
- [ ] Screenshot both themes in the README (E9)
