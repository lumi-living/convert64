# Tooling — task → skill / MCP map

Use the right skill or MCP for each kind of task. Agents read this; humans can ignore it.

| Task | Use |
|---|---|
| VIC-II modes, palette values, colour-per-cell rules, dithering patterns, file byte layouts | `c64-designer` skill (references: palette, graphics-modes, dithering, data-formats, sprites, tools) |
| Assembler output (KickAssembler / ACME / 64tass / ca65), memory alignment, VIC banks, PRG display stubs | `c64-coder` skill (references: kickassembler_reference, charpad_spritepad_integration, vic_reference, memory_map) |
| Architecture or scope change | `engineering:architecture` → write an ADR in `docs/adr/` and update `docs/TAD.md` |
| Test strategy, golden fixtures | `engineering:testing-strategy` |
| Code review before merge | `engineering:code-review` |
| Tauri 2, Svelte 5, Vite, wasm-bindgen, `image`, `clap`, `insta` API questions | Context7 (`resolve-library-id` → `query-docs`) before guessing |
| UI layout / widget design | `frontend-design` skill; `design:design-critique` for review |
| Verifying an exported file | Load in VICE `x64sc`; for PRG stubs also test on C64 Ultimate via its HTTP API |
| Issues, PRs, labels | GitHub MCP / `gh` |

Conventions: Rust 2024, `cargo fmt`, `clippy -D warnings`; TypeScript strict; Svelte 5 runes; commits imperative and scoped.
