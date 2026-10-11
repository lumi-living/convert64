# Convert64 — Concept, Architecture & Feasibility (initial plan, archived)

> Archived planning document from 2026-10-11. The living versions are [../PDD.md](../PDD.md) (product), [../TAD.md](../TAD.md) (architecture) and [../adr/](../adr/) (decisions). Kept for the reasoning trail; do not update.

**Status:** Accepted direction (v0.2 — decisions in §0 resolved 2026-10-11)
**Name:** **Convert64** — availability check 2026-10-11: GitHub user/org `convert64` free; no `convert64` on crates.io, npm or PyPI; `convert64.com/.app/.dev` have no DNS records. No C64 tool of that name found on CSDb or the web.
**One-liner:** A layered, non-destructive, high-precision image *converter* whose only output is legal VIC-II graphics — with a live, colour-accurate C64 preview and per-layer control over how each part of the picture gets quantised and dithered. Not a pixel editor.

## 0. Decisions taken (2026-10-11)

| Decision | Choice | Rationale |
|---|---|---|
| Scope | Converter, **not** a pixel editor. Everything that steers *conversion* is in scope: per-cell colour pins, cell locks, per-cell dither/metric overrides, Force vs Adaptive colour fitting, region masks. Freehand pixel painting is out. | Keeps the differentiator sharp; the scene already has excellent pixel editors. |
| Licence | **Open source from day one** — MIT. Public repo from the first commit. | Scene goodwill, contributors, golden-file test images from the community. |
| Core language | **Rust from the beginning**. One crate serves CLI, WASM and desktop. | See §5. |
| UI stack | **Svelte 5 + TypeScript + Vite**, Canvas 2D preview, packaged with **Tauri 2** from Phase 1 (browser build as a by-product). | Fastest to build a canvas-centric tool UI; runes give fine-grained slider→re-solve reactivity. If React is faster day-to-day, swap it — the architecture does not change. |
| Repo home | Public repo under the existing **Lumi Living** GitHub org, MIT © Lumi Living Oy. No new org. | Lumi Living already hosts multiple projects. |
| Phase 0 | Skip the Python prototype; the first Rust solver + CLI answers the go/no-go. | Produces shippable code instead of throwaway. |

## 1. Problem statement

Every existing converter treats conversion as **one global operation on one flattened image**: pick a palette, pick a dither, press go. The artist then fixes the result pixel by pixel in an 8-bit editor. What is missing is the step *between* the source art and the pixel editor — per-layer dither recipes, sub-pixel nudging, feathered masks, per-layer adjustments — with the **actual C64 result** updating live.

## 2. Hardware constraints

See TAD §1 (palette, modes table, validator rules). Now maintained in docs/reference/.

## 3. Product scope

See PDD (feature list, conversion steering, preview, export, non-goals). Core concept: high-precision compositor → attribute map → constraint solver → validated C64 image model → preview + exporters. Dithering is a per-pixel attribute carried through compositing (ADR-002).

## 4. Algorithm design notes

See TAD §3: per-cell brute force in OKLab with cell-clamped diffusion and refinement; charset budget via greedy + k-means; sub-pixel nudging through a supersampled composite; preview palette == quantisation palette; attribute-map semantics.

## 5. Technology options (ADR)

Options considered: A browser-only (by-product), **B Tauri 2 + Svelte 5 + Rust core (chosen)**, C native Rust GUI (rejected: immature widgets), D Python app (rejected: perf/packaging), E editor plugins (rejected: data-model mismatch), F Electron (rejected: size). Full reasoning in ADR-001.

Repository layout: `crates/convert64-core`, `crates/convert64-formats`, `crates/convert64-cli`, `crates/convert64-wasm`, `app/`, `src-tauri/`, `fixtures/`, `docs/`.

## 6. Feasibility verdict

Plausible and worth building. The differentiated core is a few thousand lines of numeric code on top of a conventional layer stack. Risk is scope (resist FLI, resist becoming a pixel editor) and UI effort, not technical feasibility. Reference-tool analysis (retropixels — archived May 2026; png2c64 — the solver quality bar; ArcPaint64 — Adaptive/Force vocabulary; C64CC — GUI/CLI parity; ALBERT — curves, luma-weighted distance) is in PDD §Reference tools.

## 7. Roadmap

M0 Scaffold → M1 Bitmap MVP → M2 Charsets & Sprites → M3 Scene features. Epics in [../github/EPICS.md](../github/EPICS.md) and as GitHub issues #1–#16.

## 8. Open items at the time of writing

- Domain: register `convert64.com/.app/.dev` only if the browser build goes public; confirm availability first.

---

*Constraint sources are now maintained in docs/reference/. Reference-tool facts checked on 2026-10-11.*
