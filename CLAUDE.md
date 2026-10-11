# Convert64 — agent instructions

Read this before touching the repo. The human-facing overview is `README.md`; the product is `docs/PDD.md`; the architecture is `docs/TAD.md`; decisions live in `docs/adr/`.

## What this is

A layered, high-precision image converter that outputs *legal* Commodore 64 (VIC-II) graphics. Rust core (`crates/convert64-core`, pure, WASM-safe) + formats crate + CLI + Svelte 5/Tauri 2 UI. Converter, **not** a pixel editor.

## Non-negotiables

1. Output must satisfy the hardware rules in `docs/TAD.md §Hardware constraints`. Never relax the validator to make a test pass.
2. `convert64-core` must stay free of `std::fs`, threads-by-default and platform code; `rayon` only behind a feature flag so the crate compiles to `wasm32-unknown-unknown` unchanged.
3. Dithering is a per-pixel attribute carried through compositing (ADR-002). Do not implement "quantise each layer then composite indices".
4. GUI and CLI have feature parity: anything the UI can export, `convert64` can export headlessly.
5. Palette used for preview == palette used for quantisation. One object.
6. Architecture, format or scope changes get an ADR (`docs/adr/ADR-000-template.md`). Update `docs/TAD.md` in the same PR.

## Skills and tools to use (see docs/TOOLING.md for the full map)

- VIC-II constraints, palette, dithering, file formats → `c64-designer` skill (references: palette, graphics-modes, dithering, data-formats).
- Assembler output, memory alignment, VIC banks, PRG stubs → `c64-coder` skill.
- Architecture changes → `engineering:architecture` (ADR format); testing strategy → `engineering:testing-strategy`.
- Tauri 2 / Svelte 5 / wasm-bindgen APIs → Context7 before guessing.
- Verify format output by loading it in VICE (`x64sc`) — a Koala that displays is the acceptance test.

## Conventions

- Rust 2024 edition, `cargo fmt`, `clippy -D warnings`. Snapshot tests with `insta`; golden image tests in `fixtures/`.
- TypeScript strict; Svelte 5 runes; no UI state in the Rust core.
- Commit messages: imperative, scoped (`core: clamp error diffusion at cell edges`).
- Issues: epics labelled `epic`, children reference the epic; milestones M0–M3.
