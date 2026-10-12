# Design tokens

`tokens.json` is the machine-readable export of the variables in the Figma
file **Convert64 — UI** (`Si4GAB17Vq8PSLmTWf5BxH`): five collections
(`egui / Visuals` Light/Dark, `egui / Spacing`, `egui / Text`,
`Convert64 / Semantic` Light/Dark, `C64 / Palette`), each variable with its
type, scopes, per-mode value (aliases resolved to the target's name and Rust
path) and **`rust`**, the egui/Convert64 path it maps to. That field is the
contract described in `docs/figma.md`.

## How it is produced

Figma's REST Variables API (`GET /v1/files/:key/variables/local`) is
Enterprise-only, so the export runs through the Figma Plugin API instead (the
Figma MCP `use_figma` tool, or the same script in a dev plugin): read
`figma.variables.getLocalVariableCollectionsAsync()` /
`getLocalVariablesAsync()`, write name, type, scopes, `codeSyntax.WEB` and
`valuesByMode` per mode, colours as `#rrggbb[aa]`. Re-export whenever the
Figma variables change and commit the diff; the diff is the review.

## How it is consumed

`cargo xtask tokens` (#195) reads this file and generates
`crates/convert64-app/src/theme/generated.rs`: `apply_light/apply_dark` for
`egui / Visuals`, `apply_spacing`, `text_styles()`, `fonts::*`,
`Theme::{LIGHT,DARK}`, and `layout::*` constants. It fails on any `rust` path
egui 0.36 or the app does not have. After re-exporting, run it and commit both
files; CI runs `cargo xtask tokens --check` and fails if `generated.rs` is
stale. `C64 / Palette` is **not** imported; an xtask test asserts it equals the
core's Colodore palette and fails if Figma drifts. Fonts named in
`egui / Text` must exist under `crates/convert64-app/assets/fonts/` as
`<Name without spaces>-Regular.ttf` (see the README there).
