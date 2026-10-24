# Changelog

All notable changes to Convert64 are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/). Numbers in brackets are pull requests on
[lumi-living/convert64](https://github.com/lumi-living/convert64/pulls?q=is%3Apr+is%3Amerged).

## [Unreleased]

## [0.1.0] — date TBD

The first release (#9): multicolor (Koala) and hi-res (Art Studio) bitmaps from layered
sources, in a desktop app, a browser build, a CLI and an MCP server that share one
command engine. Desktop builds are unsigned.

0.1.0 is the first release, so everything in it is new. *Changed* and *Fixed* list what
moved during development, for anyone who tried `main` before the release.

### Added

#### App (desktop and browser)

- The egui/eframe app, natively on macOS, Windows and Linux and in the browser from the
  same code (ADR-006) [#46, #47].
- The shell: menu bar, toolbar, right dock, status bar, and a working view that shows the
  source or the result [#214].
- The result panel, always on at the top of the dock, and the working view's zoom, pan,
  compare and overlays [#215].
- Open, save and export: native file dialogs, and a file picker and downloads in the
  browser [#217].
- Layers tab: rows, drag to reorder, rename, blend mode, opacity, undo [#219].
- Recipe tab: adjustments, curves, dither method and strength, colour fitting
  (Adaptive / Force), and the png2c64 divergences under Advanced [#220].
- Cell tools: pin, lock, nudge, the cell menu, and navigation to the worst cell [#221].
- Mask tools: mask mode, brush, lasso, gradient, luma/chroma key, selection (ADR-008)
  [#222].
- Palette and background pickers, keyboard shortcuts, Pause for slow solves, inline
  errors, accessibility labels [#223].
- Transform group and tool: position, scale, rotate, anchor, snap (ADR-011) [#224].
- Region tool: paint a region and give it its own recipe, shown as a pseudo-layer [#225].
- Solve scheduling: a worker thread natively, stepping in the browser [#200, #226], then a
  Web Worker in the browser [#271].
- Dither method picker with live thumbnails, search and hover preview (UX §6a)
  [#259, #272].
- Sharpen adjustment per layer, png2c64-compatible [#234, #252].
- Light and dark themes generated from the Figma tokens; Inter and JetBrains Mono bundled
  [#212].
- Autosave and Restore, an unsaved-changes guard, and direct saves in the browser [#285].

#### Conversion

- Pepto (PAL, NTSC) and Colodore palettes, and VICE `.vpl` palette files [#57, #58].
- Multicolor and hi-res bitmap modes, the image model and a validator for every VIC-II
  rule, tested by hand-built cases and properties [#60, #61, #62, #63, #64].
- Layers in float, with adjustments (levels, brightness/contrast, HSL, curves), blend
  modes, sub-pixel placement and a supersampled compositor [#88, #90, #91].
- Layer masks: raster plus parametric sources and modifiers, with feather [#92, #95].
- Dithering as a per-pixel attribute carried through compositing (ADR-002), and region
  overrides [#93, #94, #130].
- The solver: cell colour-set search, background search, refinement, interactive and
  full modes [#126, #127, #131, #133].
- Steering: pins, locks, allow/deny colour lists and Force fitting [#132]; the clash
  report [#134].
- Dither methods: ordered and blue noise [#128], cell-clamped error diffusion [#129],
  every ordered, noise and effect pattern [#145], every diffusion kernel and Riemersma's
  Hilbert walk [#147], palette-aware methods (Yliluoma 1–3, Knoll, luma, pair) [#148].
- Solver knobs: cell clamp, max luma difference, adaptive, hysteresis [#143]; calm flat
  areas with mix levels and linear-light mixes [#144]; blur and SSIM cell metrics
  [#149]; per-channel error clamp [#151].
- png2c64's pipeline ported bit for bit as the reference solver, with our extras as hooks
  and its divergences as named options (ADR-010) [#182, #183, #184, #185, #186, #187,
  #188]; png2c64's front end for a plain picture [#201].
- Affine layer transform: rotate, anchor, rotated resampler, snap [#213].
- Large shrinks sampled from a cached mip chain [#257].
- png2amiga's C64 path as a comparison solver, `--solver png2amiga` (ADR-014) [#273], and
  a line-art recipe on it [#283].
- png2amiga's 32 C64 dither methods that png2c64 lacks, on png2c64's pipeline in the
  reference solver: 90 methods in the picker [#280].

#### Files, CLI and MCP

- Koala and Art Studio writers with round trips and bitpair policies [#168, #176]; a
  self-displaying PRG [#89]; raw bitmap sets with a JSON manifest [#170]; palette-exact
  PNG previews [#172]; assembler source for KickAssembler, ACME, 64tass, ca65 and C
  [#173].
- `convert64 validate` for every format it reads [#65, #177].
- The `.convert64` project file with linked or embedded sources [#169], in its own
  wasm-safe crate [#199].
- `convert64 convert` from PNGs or projects to every format, with exit codes and `--json`
  [#171, #175]; `convert64 apply` runs any registry command on a project [#174]; batch
  conversion with a manifest [#177].
- Registry commands for every edit the app makes, shared by the app, the CLI and the MCP
  server [#95, #211, #213, #218].
- The *No Glory* title screen built by `make` with the CLI alone [#180].

#### Quality checks and tooling

- Golden-image tests against png2c64 baselines, regenerated only with a reason [#69, #70,
  #71, #75, #141].
- VICE acceptance: every bitmap export loads and displays in `x64sc` [#80, #178].
- png2c64 and png2amiga parity tests in CI [#185, #273].
- Comparison sheets for dither methods and divergences [#152, #189].
- CI on Linux, macOS and Windows, a wasm job, and unsigned macOS archives with dist
  [#48, #51, #52]; a compositor benchmark [#96].

### Changed

- The default bitmap solver is the png2c64 reference pipeline [#186]; it stays the
  reference after the png2amiga trial (ADR-014) [#273].
- Every layer is resampled with png2c64's Mitchell bicubic on the target grid (ADR-012)
  [#233]; adjustments apply after resampling [#201].
- A layer's opacity also scales its dither strength [#101].
- "Colour-limited cells: N" in neutral text replaces the red clash count, and the dots on
  the result panel are off by default [#268].
- The default tool is Nudge, and a click outside the picture pins nothing [#255].
- Show/Hide UI moved to `Cmd \`; `Esc` always shows it again, and `Tab` is focus
  traversal again [#249].
- Fit leaves a 5 % margin around the picture [#253]; Result opens at the largest integer
  zoom that fits [#258].
- Deleting a layer shows a toast with Undo, and the Delete keys act only from the canvas
  or the Layers tab [#256].
- Version-2 projects migrate to the 1× default on load [#251].
- The Recipe tab runs in the order of use (Dither recipe first; Transform, Mask and
  Advanced collapsed, remembered per user); Mode, Palette and Background moved to a row
  under the result header [#301].
- Transform in screen pixels (W × H of the 320 × 200 screen); Fit · Fill · 1:1 · 2:1
  presets; a newly opened picture gets Fill; flip toggles and a larger anchor control
  [#305].
- Faster compositing and solving with unchanged output [#125, #136, #146]; no 4× Source
  composite while dragging [#269].

### Fixed

- Transform handles grab where the button went down, so corners scale instead of moving
  the layer [#248].
- Canvas tools start their drags where the button went down [#266].
- Single-key shortcuts keep working after a slider or number field [#254].
- After a mode change the solve runs on the new grid [#281].

[Unreleased]: https://github.com/lumi-living/convert64/commits/main
[0.1.0]: https://github.com/lumi-living/convert64/commits/main
