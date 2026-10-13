# Architecture Decision Records

One file per decision, `ADR-NNN-short-title.md`, copied from `ADR-000-template.md`. Status flows Proposed → Accepted → (Deprecated | Superseded by ADR-NNN). Never edit an accepted ADR's decision; write a new one that supersedes it.

| ADR | Title | Status |
|---|---|---|
| [001](ADR-001-rust-core-with-tauri-svelte-ui.md) | Rust core, Tauri 2 + Svelte 5 UI, CLI parity | Superseded by 006 |
| [002](ADR-002-dither-as-per-pixel-attribute.md) | Dithering as a per-pixel attribute carried through compositing | Accepted |
| [003](ADR-003-converter-not-pixel-editor.md) | Converter, not a pixel editor | Accepted |
| [004](ADR-004-open-source-mit-under-lumi-living.md) | Open source, MIT, under Lumi Living | Accepted |
| [005](ADR-005-command-engine-with-cli-json-mcp.md) | One command engine behind UI, CLI and MCP | Accepted |
| [006](ADR-006-egui-single-rust-workspace.md) | egui/eframe UI in a single Rust workspace (supersedes 001's UI choice) | Accepted |
| [007](ADR-007-light-dark-theme-and-material-icons.md) | Light and dark themes; Material Symbols icon font | Accepted |
| [008](ADR-008-layer-masks-raster-plus-parametric.md) | Layer masks: one raster plus parametric modifiers, edited in-app | Accepted |
| [009](ADR-009-project-file.md) | The `.convert64` project file: versioned JSON in the CLI library | Accepted |
| [010](ADR-010-png2c64-reference-solver.md) | png2c64's pipeline, ported, as the default bitmap solver | Accepted |
| [011](ADR-011-layer-transform-affine-subpixel.md) | Layer transform: translate, scale, rotate; sub-pixel by default, snapping on request | Accepted (resampling amended by 012) |
| [012](ADR-012-mitchell-resampling-every-layer.md) | png2c64's Mitchell bicubic for every layer, on the target grid | Accepted |
