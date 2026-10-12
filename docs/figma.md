# Figma ↔ code contract

Convert64's UI is designed in Figma with a library that mirrors egui's
`Style` one-to-one, so anything designed there can be read back into
`convert64-app` without translation.

File: **Convert64 — UI** (`Si4GAB17Vq8PSLmTWf5BxH`).

## Pages

| Page | Contents |
|---|---|
| 📘 Cover | Library map and read-back notes |
| 🎨 Foundations | Specimens of every variable (Light and Dark side by side), spacing scale, text styles |
| 🧩 Widgets | `Button`, `SelectableLabel`, `Checkbox`, `RadioButton`, `Slider`, `DragValue`, `ComboBox`, `TextEdit`, `CollapsingHeader`, `ProgressBar`, `Separator`, `Label`, `Hyperlink`, `Icon / Placeholder` |
| 🪟 Containers | `Panel` (Side L/R, Top, Bottom, Central), `Window`, `Popup Menu`, `Menu Item`, `Tooltip`, `Menu Bar`, `Menu Button`, `ScrollBar` |
| 🖼 Convert64 | `Toolbar Button`, `Palette Swatch`, `Cell Badge`, `Layer Row`, `Status Bar`, `Result Header`, `Dock Tabs`, and `App Shell / Light` + `App Shell / Dark` (the UX.md v0.2 layout) |
| 🧪 Playground | Scratch space for new screens — build from instances, never detach |

Widget states follow egui's `WidgetVisuals` slots: `Inactive`, `Hovered`,
`Active`, `Disabled` (= `noninteractive`), plus `Open` where egui has it
(`ComboBox`, `Menu Button`). A component's description names the egui type
and the style fields it uses.

## Variables

Five collections. **Every variable's code syntax (shown in Dev Mode and
exported by the REST API) is the Rust path it maps to.** That is the
contract: designers change values, not names.

| Collection | Modes | Maps to |
|---|---|---|
| `egui / Visuals` | Light, Dark | `egui::Visuals` — `widgets.{noninteractive,inactive,hovered,active,open}.{weak_bg_fill,bg_fill,bg_stroke,fg_stroke}`, `panel_fill`, `window_fill`, `window_stroke`, `faint_bg_color`, `extreme_bg_color`, `code_bg_color`, `selection.*`, `hyperlink_color`, `warn_fg_color`, `error_fg_color`, `text_cursor`, shadows, corner radii, stroke widths |
| `egui / Spacing` | Default | `egui::style::Spacing` (`item_spacing`, `button_padding`, `window_margin`, `menu_margin`, `indent`, `interact_size`, `slider_width`, `combo_width`, `text_edit_width`, `icon_width`, …) plus Convert64 layout constants under `convert64/*` → `layout::DOCK_WIDTH`, `RESULT_HEIGHT`, `TOOLBAR_HEIGHT`, `STATUS_HEIGHT`, `LAYER_ROW_HEIGHT`, `ICON_SIZE`, `SWATCH_SIZE` |
| `egui / Text` | Default | `TextStyle::{Small,Body,Button,Monospace,Heading}` sizes, `FontFamily::{Proportional,Monospace}`, `weak_text_alpha` |
| `Convert64 / Semantic` | Light, Dark | the app `Theme` struct: `overlay/{clash,pin,lock,region,nudge,cell_grid,char_grid}`, `canvas/{bg,checker_a,checker_b}`, `status/{ok,warn,error}`, `accent`. `warn`/`error`/`accent` alias egui visuals |
| `C64 / Palette` | Colodore | `C64Color::*` — 16 Colodore colours. Generated from `convert64-core`; never hand-edited |

Values are egui 0.36 defaults (`Visuals::light()` / `Visuals::dark()`,
`Spacing::default()`), so the baseline Figma file renders what an untouched
eframe app renders. Any deviation we design becomes an override in
`convert64-app/src/theme.rs`.

Fonts: Inter stands in for egui's bundled Ubuntu-Light, JetBrains Mono for
Hack. Sizes are egui's (9 / 12.5 / 12 / 18). If we bundle different fonts in
the app, update `egui / Text` first.

Deviation from the usual Figma convention: the WEB code syntax holds a Rust
path, not a `var(--css)` name, because the consumer is `theme.rs`, not CSS.

## Reading Figma back into code

1. Export variables into `design/tokens.json` (committed, so diffs are
   reviewable). The REST Variables API is Enterprise-only, so the export uses
   the Plugin API — see `design/README.md`.
2. `cargo xtask tokens` (to be written, tracked in the app epic) turns the
   JSON into `convert64-app/src/theme/generated.rs`:
   - `egui / Visuals` → `fn apply_light(v: &mut Visuals)` / `apply_dark`
     by walking each variable's code-syntax path.
   - `egui / Spacing`, `egui / Text` → `fn apply_spacing(s: &mut Spacing)`,
     `fn text_styles() -> BTreeMap<TextStyle, FontId>`.
   - `Convert64 / Semantic` → `impl Theme { const LIGHT; const DARK; }`.
   - `C64 / Palette` is **not** imported; CI asserts it equals
     `convert64_core::palette::COLODORE` and fails if Figma drifts.
3. Layout constants under `convert64/*` land in `layout.rs` as `pub const`.

Until the xtask exists, read values from the 🎨 Foundations page (each row
shows the Rust path next to the value) and hand-port them.

## Rules for designers and agents

- Design with instances from 🧩 / 🪟 / 🖼; new patterns become components
  on those pages, with the egui type named in the description.
- Bind every colour, padding, radius and gap to a variable. An unbound value
  cannot be read back.
- To add a theme knob, add the variable to the right collection with its
  Rust path as code syntax; do not restyle instances ad hoc.
- Light/Dark are modes, not duplicate components. Preview with
  "Change variable mode" on a frame (the two App Shells are set up this way).
- Icons: the `Icon` component set on 🧩 Widgets holds every glyph from the
  ADR-007 map plus a few chrome glyphs (`close`, `check`, `expand_more`,
  `chevron_right`, `swap_horiz`, `fit_screen`), imported as SVG from
  `google/material-design-icons` at Outlined / wght 400 / GRAD 0 / opsz 20 —
  the cut `egui_material_icons` renders. Variant name = icon id =
  `ICON_<ID>`. Swap via the `Icon` property on `Button` and `Toolbar Button`;
  never paste glyphs from the Material Symbols *font* or the M3 Design Kit.
  To add a glyph, fetch its `<id>_20px.svg`, add a variant with that name,
  bind the fill to `widgets/inactive/fg_stroke`, and add the id to ADR-007.
