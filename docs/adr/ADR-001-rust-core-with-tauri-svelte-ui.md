# ADR-001: Rust core, Tauri 2 + Svelte 5 UI, CLI with GUI parity

**Status:** Accepted
**Date:** 2026-10-11
**Deciders:** Jaakko Haaparanta

## Context

Convert64 needs (a) a numeric core doing f32 compositing and per-cell constraint solving at interactive rates, (b) a layered-editor UI with many widgets, (c) headless operation from Makefiles, (d) distribution to the C64 scene on macOS/Windows/Linux, and (e) to stay finishable by one part-time developer whose strengths are TypeScript/frontend.

## Decision

- Core in **Rust** (`convert64-core`, pure, no I/O, compiles natively and to `wasm32`).
- UI in **Svelte 5 + TypeScript + Vite**, Canvas 2D preview, packaged with **Tauri 2**; the same UI is published as a browser build.
- **CLI** (`convert64`) built on the same core with feature parity with the GUI.
- Rust from the first commit — no interim TypeScript solver.

## Options considered

| Option | Verdict | Why |
|---|---|---|
| A. Browser-only web app, Rust→WASM core | By-product of B | No CLI, Chromium-only file handles |
| B. Tauri 2 desktop + same UI + Rust core | **Chosen** | Native file I/O, small bundles, one UI codebase, CLI falls out of the core |
| C. Native Rust GUI (egui/iced) | Rejected | Immature widgets for a layered editor; discards frontend strengths |
| D. Python app (PyQt + numpy/numba) | Rejected | Real-time diffusion painful; packaging; UI ceiling |
| E. Plugin for Aseprite/Krita/GIMP/Photoshop | Rejected | Host data models cannot express the attribute map + live legal preview |
| F. Electron | Rejected | 150 MB bundles, slower native bridge than Tauri |

UI framework: Svelte 5 over React because runes give fine-grained slider→re-solve reactivity with no memo plumbing, minimal boilerplate, tiny bundle, and an official Tauri template. Swapping to React would not change the architecture.

Core language: Rust over C++/Zig/Go/TS for first-class WASM with small binaries, `rayon`, memory safety, and one crate serving CLI + desktop + web.

## Consequences
- Easier: shipping to the scene, CI builds, golden-file regression tests, Makefile integration.
- Harder: two toolchains (cargo + pnpm); a clean WASM boundary (typed arrays only); macOS signing/notarisation.
- Revisit when: FLI/NUFLI search makes WebGPU compute worth it; if Rust proves too slow to learn, bitmap modes are feasible in TS — the UI would not change.

## Action items
- [ ] Scaffold cargo + pnpm workspace (M0)
- [ ] CI: native test on 3 OSes, wasm build, Tauri bundle for macOS
