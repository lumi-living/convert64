# ADR-006: egui/eframe UI in a single Rust workspace

**Status:** Accepted (supersedes the UI half of [ADR-001](ADR-001-rust-core-with-tauri-svelte-ui.md))
**Date:** 2026-10-11
**Deciders:** Jaakko Haaparanta

## Context

ADR-001 chose Svelte 5 + Tauri 2 for the UI, partly on the claim that native Rust GUI toolkits were too immature for a layered editor. Reviewing [PhotoCraft](https://github.com/storytold/photocraft) — a layered, non-destructive editor with masks, curves and histograms built on egui + wgpu, compiled natively and to WASM from one codebase — showed that claim to be wrong. With ADR-005 making every action a command on a pure-data `Project`, the UI framework became a two-way door, so the choice was re-evaluated on technical merit alone.

## Decision

The UI is **egui via eframe**, in the same cargo workspace as the core. One codebase builds the native app (macOS, Windows, Linux) and the browser build (WASM via `trunk`). Tauri, Svelte, Vite, pnpm and the separate WASM facade crate are dropped.

Restated from ADR-001 and still in force: Rust core (`convert64-core`, pure, no I/O, WASM-safe), GUI/CLI feature parity, CPU-only compositing (no wgpu — the target is 320×200 and a 4× supersampled composite is milliseconds on a CPU).

## Why egui is the better fit for this product

- **Immediate mode matches "drag a slider → re-solve → redraw".** The UI state *is* the `Project`; every frame draws from it and dispatches commands to it. No reactivity layer, no state mirrored between Rust and a JS front-end, no IPC serialisation of previews and clash maps, no question of which side owns undo.
- **One execution path.** With Tauri the core ran natively on desktop but as WASM in the browser build (plus a Web Worker for the solver). With eframe the same Rust runs everywhere; the browser build is the same app compiled to WASM.
- **It is a tool, not a consumer app.** Users live in Pixcen, CharPad, SpritePad and VICE; a dense, keyboard-driven, tool-looking UI is expected. DOM-level visual polish was the thing the extra architecture would have bought, and the thing this audience values least.
- **Custom widgets are the product.** Cell pin/lock overlay, clash heatmap, attribute-map debug view, 2:1-aspect preview with integer zoom — in egui these are short painter functions over the same data; in a web UI they are canvas code plus a bridge.
- **CLI and MCP link the core directly**, as does the app; identical behaviour by construction.
- **PhotoCraft (MIT/Apache-2) is a readable reference** for egui layer panels, curves and a colour-managed canvas.

## Options considered

| Option | Verdict |
|---|---|
| Svelte 5 + Tauri 2 (ADR-001) | Best visual polish and DOM accessibility; two toolchains, IPC boundary, dual execution paths. Rejected on merit for a tool UI. |
| **egui + eframe** | **Chosen.** |
| iced | Retained/Elm-style; good, but custom canvas-heavy widgets are more work than in egui and there is no PhotoCraft-scale precedent for this kind of app. |
| Slint / Dioxus / GPUI | Declarative or webview-based; licensing (Slint), webview tax (Dioxus desktop) or platform maturity (GPUI) make them worse fits today. |

## Consequences
- Easier: one language, one toolchain (`cargo` + `trunk`), one crate graph for app/CLI/MCP, trivial live preview, small binaries via `cargo-dist`, no signing of a webview shell.
- Harder: accessibility and IME/text input are adequate rather than browser-grade; the web build is a single canvas; visual design is limited to egui theming (fonts, colours, spacing) — acceptable for a tool.
- Revisit when: a consumer-grade or heavily branded UI becomes a goal, or egui accessibility proves insufficient for a contributor.

## Resulting workspace

```
convert64/                 cargo workspace, Rust 2024
├─ crates/convert64-core      document model, command registry, compositor, solvers, validator (pure, WASM-safe)
├─ crates/convert64-formats   byte-exact readers/writers, asm/C emitters
├─ crates/convert64-cli       clap
├─ crates/convert64-mcp       rmcp (official Rust MCP SDK), stdio
├─ crates/convert64-app       eframe/egui; `cargo run -p convert64-app` native, `trunk build` → web
├─ fixtures/
└─ docs/
```
Tests: `insta` + `proptest`. Parallelism: `rayon` behind a `parallel` feature (native). Release: `cargo-dist` for binaries, GitHub Pages for the web build.

## Action items
- [ ] Update TAD §4–§5, README, PDD, EPICS and issues #1, #8, #12 (done in the same change as this ADR)
- [ ] E1: scaffold `convert64-app` with eframe and a `trunk` web build in CI
