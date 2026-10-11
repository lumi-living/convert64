# Epics and milestones

Milestones: **M0 Scaffold → M1 Bitmap MVP → M2 Charsets & Sprites → M3 Scene features**. Each epic is a GitHub issue labelled `epic`; child issues reference it. Labels: `epic`, `core`, `formats`, `cli`, `ui`, `tauri`, `wasm`, `docs`, `fixtures`, `bug`, `enhancement`, `good first issue`.

## M0 — Scaffold
- **E1 Workspace & CI** — cargo + pnpm workspace, crate skeletons, CI (test on macOS/Linux/Windows, wasm32 build, clippy/fmt), Tauri dev shell boots with an empty window.
- **E2 Image model & validator** — palette object (Pepto/Colodore), mode definitions, C64 image model structs, validator for §1.3 rules, property test "any solver output validates".
- **E3 Fixtures** — 5 rights-clear source images, png2c64 baseline outputs, golden-test harness with `insta`.

## M1 — Bitmap MVP
- **E4 Compositor & attribute map** — f32 layers, float transforms with supersampling, masks, feather, levels/HSL/curves, blend modes, attribute-map compositing.
- **E5 Bitmap solver** — MC + hi-res: bg search, per-cell brute force in OKLab, cell-clamped diffusion, ordered dithers with strength/phase, refinement, pins/locks, Force/Adaptive, allow/deny sets.
- **E6 Formats v1** — Koala, Art Studio, raw bins, PRG display stub, PNG preview, asm emitters (KickAss/ACME/64tass/ca65), C header.
- **E7 CLI** — `convert64 convert|validate|export|batch`, `.convert64` project file, Makefile example for No Glory.
- **E8 UI v1** — Tauri/Svelte shell: open image, layer stack, per-layer recipe panel, cell pin/lock tool, clash overlay, palette presets, A/B, export dialog; browser build.
- **E9 Release 0.1** — macOS bundle, README with screenshots, CSDb announcement draft.

## M2 — Charsets & sprites
- **E10 Charset solver** — hi-res/MC/ECM, char budget with greedy + k-means merge, tile sizes, CharPad colour-mode targets, CharPad-layout bins.
- **E11 Sprite solver** — hi-res/MC/overlay, shared MC1/MC2 across a set, sprite-sheet import, `.spd` + raw + asm.
- **E12 UI v2** — mode switcher, char-budget counter, sprite sheet view.

## M3 — Scene features (opportunistic)
- **E13 FLI** — FLI solver with FLI-bug columns, `.fli` export, PRG displayer.
- **E14 PAL preview shader** — chroma/luma blur, scanlines, opt-in.
- **E15 Interlace modes** — IFLI/MCI/NUFLI with flicker-blend preview.
- **E16 Integrations** — CharPad `.ctm`, D64 wrapper, C64 Ultimate HTTP push, Aseprite bridge, import C64 files as locked layers.
