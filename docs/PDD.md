# Convert64 — Product Design Document

**Status:** v0.2, accepted direction (2026-10-11). Technical detail lives in [TAD.md](TAD.md); decisions in [adr/](adr/). UX foundations (layout, shortcuts, vocabulary, states) in [UX.md](UX.md).

## One-liner

A layered, non-destructive, high-precision image *converter* whose only output is legal VIC-II graphics — with a live, colour-accurate C64 preview and per-layer control over how each part of the picture gets quantised and dithered. Not a pixel editor.

## Problem

Every existing converter (retropixels, png2c64, C64CC, Pixcen/Multipaint/Project One importers, GIMP/Photoshop palette plugins) treats conversion as **one global operation on one flattened image**: pick a palette, pick a dither, press go. The artist then fixes the result pixel by pixel in an 8-bit editor.

What is missing is the step *between* the source art and the pixel editor — where you can say:

- "The sky layer gets ordered 4×4 dither at 60 % strength, the soldier's face gets Floyd–Steinberg at 100 %, the rifle gets no dither at all."
- "Nudge the face layer 0.5 px right so the eye lands in a cell that can afford a third colour."
- "Feather the fog layer's edge over 6 px so the dither fades instead of cutting."
- "Lift the midtones of the background only, so the quantiser stops picking Brown where I want Dark Grey."

…and see the **actual C64 result** (correct palette, colour clashes resolved, 2:1 pixel aspect) update as you drag the slider.

## Users

1. **C64 game developers** converting painted or AI-generated concept art into title screens, portraits, tilesets and sprites inside a Make/CI pipeline (first user: *No Glory*).
2. **Scene graphicians** who want precise control over conversion before finishing in Pixcen / Multipaint / ArcPaint64.
3. **Demo coders** who need legal data in assembler-ready form.

## Scope decisions

| Decision | Choice |
|---|---|
| Scope | Converter, **not** a pixel editor. Everything that steers conversion is in: cell colour pins, cell locks, per-region dither/metric overrides, Force vs Adaptive colour fitting, region masks, allow/deny colour subsets. Freehand painting is out ([ADR-003](adr/ADR-003-converter-not-pixel-editor.md)). |
| Licence | MIT, open source from the first commit, under Lumi Living ([ADR-004](adr/ADR-004-open-source-mit-under-lumi-living.md)). |
| Stack | Single Rust workspace: core + egui/eframe app + CLI + MCP, one command engine ([ADR-005](adr/ADR-005-command-engine-with-cli-json-mcp.md), [ADR-006](adr/ADR-006-egui-single-rust-workspace.md)). |
| Core algorithm | Dithering as a per-pixel attribute carried through compositing ([ADR-002](adr/ADR-002-dither-as-per-pixel-attribute.md)). |

## Feature list (✅ = M1/M2, ⏩ = later)

### Layers & editing
- ✅ Import PNG/JPG/TIFF at 8/16-bit; internal storage f32 linear RGBA. ⏩ Layered PSD / OpenRaster import (via `photocraft-psd`), each PSD layer becoming a Convert64 layer.
- ✅ Unlimited layers; opacity; blend modes (normal, multiply, screen, overlay).
- ✅ Float-precision transform: translate (sub-pixel), scale, flip; ⏩ rotate.
- ✅ Raster mask per layer, paintable; ⏩ vector/gradient masks.
- ✅ Feather / blur of mask edge (Gaussian, radius in target pixels).
- ✅ Per-layer adjustments: levels, brightness/contrast, hue/saturation/lightness, curves; ⏩ selective colour.
- ✅ Per-layer dither recipe: method (none / Bayer 2×2, 4×4, 8×8 / Floyd–Steinberg / Atkinson / Jarvis–Judice–Ninke / Sierra / blue-noise / line- and 2:1-biased variants), strength 0–100 %, pattern phase (screen-locked or layer-locked), orientation preference.
- ✅ Per-layer colour-distance metric override (OKLab / CIELAB / Pepto YUV luma-weighted / RGB) and optional allowed-colour subset.
- ✅ Per-layer luma-pair bias (reward same-luminance dither pairs for PAL).

### C64 target
- ✅ M1: multicolor bitmap, hi-res bitmap. ✅ M2: multicolor / hi-res / ECM charsets with char budget, hi-res / multicolor / overlay sprites. ⏩ M3: FLI; ⏩⏩ IFLI/MCI/NUFLI.
- ✅ Global colour strategy: auto-search (all 16 backgrounds, keep best) or user-pinned.
- ✅ **Conversion steering:** pin a cell's colour set (whole or partial), lock a cell, paint per-region dither/metric overrides, **Force vs Adaptive** colour fitting per layer/region, allow/deny colour lists.
- ✅ Clash visualiser: which cells are colour-starved and what the solver dropped.
- ✅ Charset mode: unique-char counter, merge tolerance, tile size (1×1…4×4), CharPad colour-mode target (global / per-tile / per-cell).

### Preview
- ✅ Palette presets: Pepto PAL (default), Colodore with brightness/contrast/saturation, Pepto NTSC, user `.vpl`.
- ✅ Correct 2:1 multicolor aspect, border colour, 1×–4× nearest-neighbour zoom.
- ✅ Side-by-side / split / flicker A-B between source and result.
- ✅ Light and dark UI themes (system-following, user override); the C64 preview itself is never themed. Icons: Material Symbols ([ADR-007](adr/ADR-007-light-dark-theme-and-material-icons.md)).
- ⏩ PAL chroma/luma blur + scanline shader, opt-in.

### Export
- ✅ Koala `.kla`, Art Studio `.art`, raw `.bin` set, PRG with display stub, PNG of preview.
- ✅ Assembler source: KickAssembler, ACME, 64tass, ca65; C header.
- ✅ Charset: `charset.bin` + `map.bin` (8/16-bit) + `colour.bin` + tile bins in CharPad export layout.
- ✅ Sprites: raw 64-byte blocks, SpritePad `.spd`, `.asm`.
- ✅ Project file `.convert64` (JSON + embedded/linked sources) — layers never flattened.
- ⏩ CharPad `.ctm`, FLI formats, D64 + Ultimate HTTP push.

### Automation
- ✅ CLI with feature parity (M1).
- ⏩ **MCP server** (`convert64-mcp`, M2): the same command registry exposed as MCP tools so an AI agent can open a project, adjust recipes, re-solve, read the clash report and export — e.g. from a game repo's Claude Code session. See ADR-005.

## Non-goals
- Freehand pixel painting, brushes, fills.
- Being an emulator (preview is palette-exact and aspect-correct, not cycle-exact).
- Other platforms (Plus/4, VIC-20, Amiga) — the core is mode-driven so it *could* grow, but not before M3.

## Feasibility (summary)

| Area | Plausibility |
|---|---|
| Layered f32 compositor with masks/feather/adjustments | High — standard maths; cost is UI widgets |
| Bitmap solver with per-cell colours + cell-clamped dithering | High — png2c64 is the reference |
| Per-layer dither recipes via attribute map | High, novel — needs the first build to prove it looks right |
| Sub-pixel nudging with re-solve | High |
| Charset mode with 256-char budget | Medium — greedy/k-means merge, perceptual weighting |
| Sprites incl. overlay | High |
| Accurate preview | High |
| FLI / interlace modes | Medium / Low — deferred |
| Solo part-time effort | Realistic if scoped: M0–M1 ≈ 6–10 weekends, M2 ≈ 4–8 |

The risk is scope (resist FLI, resist becoming a pixel editor) and UI effort, not technical feasibility.

## Reference tools (checked 2026-10-11)

| Tool | What it is | What we borrow |
|---|---|---|
| [retropixels](https://github.com/micheldebree/retropixels) | TS core + CLI + web; MIT; **archived May 2026**. Bayer 2/4/8 with strength, palettes, colour spaces, `.kla/.art/.spd/.fli` writers with documented layouts. | Format byte layouts; "dither radius" = our strength. |
| [png2c64](https://github.com/tinic/png2c64) | C++26 CLI + WASM; MIT. Brute-force per-cell search in OKLab, ~30 dither methods, charset k-means, 7 palettes, PRG displayers. | The solver quality bar; golden-test baseline; dither menu. |
| [ArcPaint64](https://www.doussis.com/arcpaint64/) | Browser pixel editor by arch0N (CSDb 2026, 10/10). Hardware mode, **Adaptive vs Force** fitting. | Vocabulary for colour fitting; hand-off target. |
| [C64CC](https://www.elder0010.com/projects/c64cc/) | Charset converter, GUI + CLI parity, Levenshtein char reuse under a budget, KickAss export. | GUI/CLI parity, char-budget UX. |
| [ALBERT](https://www.albertpixels.com/) | Native border-extended image editor; curves, luminance-weighted distance. | Curves UI; metric preset. |
| [PhotoCraft](https://github.com/storytold/photocraft) | Rust clean-room layered editor (egui + wgpu, WASM too), MIT/Apache-2, early alpha, very active. Engine-first: pure-data document, 500+ registered commands behind UI, CLI, JSON channel and MCP server; CoW sparse tiles; `photocraft-psd` crate. | The command-engine pattern (ADR-005); `photocraft-psd` for layered PSD import; CPU-vs-GPU compositor cross-testing idea. |

**The gap:** all convert a single flattened image with one global recipe. None has high-precision layers, masks/feathering, sub-pixel placement, per-region recipes, or pins/locks driving a re-solve with a legal live preview.

## Roadmap

| Milestone | Goal | Exit criterion |
|---|---|---|
| **M0 Scaffold** | Workspace, CI, fixtures, validator | `cargo test` green on 3 platforms + WASM build; validator rejects illegal data |
| **M1 Bitmap MVP** | MC + hi-res bitmap solver, formats, CLI, first egui UI with layers and recipes | A *No Glory* title screen converted from a layered project, loads in VICE, exported via Makefile |
| **M2 Charsets & sprites** | Charset/tile modes with budget, sprite modes incl. overlay, `.spd`, CharPad bins, MCP server | *No Glory* terrain tileset and HUD portraits produced through Convert64 |
| **M3 Scene features** | FLI, PAL shader, interlace modes, `.ctm`, D64/Ultimate, Aseprite bridge | Opportunistic |

Epics per milestone: [github/EPICS.md](github/EPICS.md).
