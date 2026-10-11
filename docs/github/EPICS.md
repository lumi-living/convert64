# Epics and milestones

Tracking board: **[Convert64 project](https://github.com/orgs/lumi-living/projects/3)** (GitHub Projects, board by Status: Backlog → Ready → In progress → In review → Done; `Work type` field: Epic / Task / Bug / Docs).

| # | Milestone | Issues |
|---|---|---|
| [1](https://github.com/lumi-living/convert64/milestone/1) | **M0 Scaffold** | [#1](https://github.com/lumi-living/convert64/issues/1) [#2](https://github.com/lumi-living/convert64/issues/2) [#3](https://github.com/lumi-living/convert64/issues/3) |
| [2](https://github.com/lumi-living/convert64/milestone/2) | **M1 Bitmap MVP** | #4 #5 #6 #7 #8 #9 |
| [3](https://github.com/lumi-living/convert64/milestone/3) | **M2 Charsets & Sprites** | #10 #11 #12 #17 |
| [4](https://github.com/lumi-living/convert64/milestone/4) | **M3 Scene features** | #13 #14 #15 #16 |

Each epic is a GitHub issue labelled `epic`; child issues are GitHub sub-issues of the epic and say "Part of #N" in their body. Labels: `epic`, `core`, `formats`, `cli`, `ui`, `tauri`, `wasm`, `docs`, `fixtures`, `solver`, `mcp`, `release`, `bug`, `enhancement`, `good first issue`, `needs-vice-check` (exported file must be verified in VICE before merge).

## M0 — Scaffold
- **E1 Workspace & CI** ([#1](https://github.com/lumi-living/convert64/issues/1), children #18–#25) — cargo + pnpm workspace, crate skeletons, CI (test on macOS/Linux/Windows, wasm32 build, clippy/fmt), Tauri dev shell boots with an empty window.
- **E2 Image model & validator** ([#2](https://github.com/lumi-living/convert64/issues/2), children #26–#33) — palette object (Pepto/Colodore), mode definitions, C64 image model structs, validator for §1.3 rules, property test "any solver output validates".
- **E3 Fixtures** ([#3](https://github.com/lumi-living/convert64/issues/3), children #34–#41) — 5 rights-clear source images, png2c64 baseline outputs, golden-test harness with `insta`.

## M1 — Bitmap MVP
- **E4 Compositor & attribute map** — f32 layers, float transforms with supersampling, masks, feather, levels/HSL/curves, blend modes, attribute-map compositing.
- **E5 Bitmap solver** (#5) — MC + hi-res: bg search, per-cell brute force in OKLab, cell-clamped diffusion, ordered dithers with strength/phase, refinement, pins/locks, Force/Adaptive, allow/deny sets.
- **E6 Formats v1** (#6) — Koala, Art Studio, raw bins, PRG display stub, PNG preview, asm emitters (KickAss/ACME/64tass/ca65), C header.
- **E7 CLI** (#7) — `convert64 convert|validate|export|batch`, `.convert64` project file, Makefile example for No Glory.
- **E8 UI v1** (#8) — Tauri/Svelte shell: open image, layer stack, per-layer recipe panel, cell pin/lock tool, clash overlay, palette presets, A/B, export dialog; browser build.
- **E9 Release 0.1** (#9) — macOS bundle, README with screenshots, CSDb announcement draft.

## M2 — Charsets & sprites
- **E10 Charset solver** (#10) — hi-res/MC/ECM, char budget with greedy + k-means merge, tile sizes, CharPad colour-mode targets, CharPad-layout bins.
- **E11 Sprite solver** (#11) — hi-res/MC/overlay, shared MC1/MC2 across a set, sprite-sheet import, `.spd` + raw + asm.
- **E12 UI v2** (#12) — mode switcher, char-budget counter, sprite sheet view.
- **E17 MCP server** (#17) — `convert64-mcp` exposing the command registry (open/convert/set recipe/pin/lock/solve/clash report/export) as MCP tools; stdio transport; usable from Claude Code in a game repo.

## M3 — Scene features (opportunistic)
- **E13 FLI** (#13) — FLI solver with FLI-bug columns, `.fli` export, PRG displayer.
- **E14 PAL preview shader** (#14) — chroma/luma blur, scanlines, opt-in.
- **E15 Interlace modes** (#15) — IFLI/MCI/NUFLI with flicker-blend preview.
- **E16 Integrations** (#16) — CharPad `.ctm`, D64 wrapper, C64 Ultimate HTTP push, Aseprite bridge, import C64 files as locked layers, layered PSD/OpenRaster import via `photocraft-psd`.
