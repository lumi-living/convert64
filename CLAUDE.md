# Convert64 — instructions for AI coding agents

Read this before touching the repo. The human-facing overview is `README.md`; the product is `docs/PDD.md`; the architecture is `docs/TAD.md`; decisions live in `docs/adr/`; hardware facts and byte layouts live in `docs/reference/`.

## What this is

A layered, high-precision image converter that outputs *legal* Commodore 64 (VIC-II) graphics. Rust core (`crates/convert64-core`, pure, WASM-safe) + formats crate + CLI + Svelte 5 / Tauri 2 UI. Converter, **not** a pixel editor (ADR-003).

## Non-negotiables

1. Output must satisfy `docs/reference/vic-ii-constraints.md`. Never relax the validator to make a test pass; if the reference is wrong, fix the reference in the same PR with evidence (VICE or hardware screenshot).
2. `convert64-core` stays free of `std::fs`, threads-by-default and platform code; `rayon` only behind the `parallel` feature so the crate compiles to `wasm32-unknown-unknown` unchanged.
3. Dithering is a per-pixel attribute carried through compositing (ADR-002). Do not implement "quantise each layer then composite indices".
4. GUI and CLI have feature parity: anything the UI can export, `convert64` can export headlessly.
5. The palette used for preview is the same object used for quantisation.
6. Architecture, format or scope changes get an ADR (`docs/adr/ADR-000-template.md`); update `docs/TAD.md` in the same PR.
7. File writers match `docs/reference/file-formats.md` byte for byte; add a round-trip test for every new format.

## Working method

- Check library APIs (Tauri 2, Svelte 5, wasm-bindgen, `image`, `clap`, `insta`) against current documentation rather than memory; these move fast.
- Solver changes: run the golden tests in `fixtures/` and compare against the png2c64 baseline before and after; report the OKLab-MSE delta in the PR.
- Any exported file is only "done" once it loads and displays in VICE (`x64sc`).

## Conventions

- Rust 2024 edition, `cargo fmt`, `clippy -D warnings`. Snapshot tests with `insta`; golden image tests in `fixtures/`.
- TypeScript strict; Svelte 5 runes; no UI state in the Rust core.
- Commit messages: imperative, scoped (`core: clamp error diffusion at cell edges`).
- Issues: epics labelled `epic`, children reference the epic; milestones M0–M3.

Maintainers may keep personal tooling notes in `.claude/*.local.md` (git-ignored); nothing in the project depends on them.
