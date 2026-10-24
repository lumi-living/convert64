# Convert64 — Technical Architecture Document

**Status:** v0.2 (2026-10-11). Product context in [PDD.md](PDD.md). Decisions in [adr/](adr/).

## 1. Hardware constraints (the type system of the output)

Summary only — the full, authoritative version is [reference/vic-ii-constraints.md](reference/vic-ii-constraints.md); byte layouts are in [reference/file-formats.md](reference/file-formats.md).

### 1.1 Palette
Fixed 16 colours. Reference values Pepto / Colodore. Nine luminance levels (0, 8, 10, 12, 15, 16, 20, 24, 31). Same-luma pairs — Blue+Brown, Red+DkGrey, Purple+Orange, MedGrey+LtBlue, Green+LtRed, Cyan+LtGrey, Yellow+LtGreen — blend cleanly on PAL; the quantiser can reward them.

### 1.2 Modes

| Mode | Logical res | Aspect | Colour rule | Global colours | Data |
|---|---|---|---|---|---|
| Multicolor bitmap (Koala) | 160×200 | 2:1 | 4 per 4×8 cell | bg $D021 | 8000 bitmap + 1000 screen + 1000 colour + 1 bg |
| Hi-res bitmap (Art Studio) | 320×200 | 1:1 | 2 per 8×8 cell | none | 8000 bitmap + 1000 screen |
| Multicolor charset | 160×200 | 2:1 | 4 per 4×8; bg, MC1, MC2 global; per-char colour **0–7** only | $D021–$D023 | 2048 charset + screen + colour; **≤256 unique chars** |
| Hi-res charset | 320×200 | 1:1 | 2 per 8×8; bg global, fg per char 0–15 | $D021 | as above |
| ECM charset | 320×200 | 1:1 | 1 fg + 1 of 4 bgs per char | $D021–$D024 | **≤64 unique chars** |
| Hi-res sprite | 24×21 | 1:1 | 1 colour + transparent | — | 64 B, 64-byte aligned |
| Multicolor sprite | 12×21 | 2:1 | 3 + transparent; **MC1/MC2 shared by all sprites** | $D025, $D026 | 64 B |
| Overlay sprite | 24×21 | mixed | hi-res over MC → 6–7 colours | as above | 2×64 B, 2 slots |
| FLI (M3) | 160×200 | 2:1 | screen-RAM colours change per raster line; colour RAM per 4×8; first 3 columns unusable | $D021 | 8×1000 screen + 8000 bitmap |
| IFLI / MCI / NUFLI (M3+) | interlaced | — | two alternating frames | — | ≥2× data, flicker preview needed |

### 1.3 Validator rules
1. Colours per cell never exceeded (2 hi-res / 4 multicolor, counting globals).
2. Only indices 0–15; MC-charset per-char colour 0–7.
3. Charset modes: unique-char budget (256 / 64) — a global constraint.
4. Sprites: 64-byte blocks; MC1/MC2 shared across the set.
5. Export alignment: charset 2 KB, bitmap 8 KB, screen 1 KB, sprites 64 B; never charsets at $1000–$1FFF / $9000–$9FFF (char ROM shadow).
6. Colour RAM is one copy at $D800 — scrolled/double-buffered charset graphics need per-tile colour discipline; the tool targets CharPad colour modes as export profiles.

## 2. Pipeline

```
 Source layers (f32 linear RGBA, any size)
        │  per-layer: float transform → png2c64's Mitchell bicubic onto the super grid
        │             (ADR-012; sub-pixel placement is the kernel's phase; past a 4×
        │             shrink, under Downscale Auto, on a cached mip level of the
        │             source, #235), then the
        │             layer's adjustments (gamma, sharpen, png2c64's preprocessing round
        │             trip, levels/curves/HSL, range matching — after the resample,
        │             png2c64's order, #190, #228), mask (resampled at 4× and box-reduced), feather,
        │             blend, opacity, dither recipe (method, strength, phase, metric,
        │             allowed set, force/adaptive)
        ▼
 Composite on the super grid (default 1×: 160×200 or 320×200, the target itself)
        │  + attribute map: per pixel, which recipe governs it
        │    (top-most non-transparent layer wins; strength blends through alpha/feather)
        ▼
 Box filter to the target grid (a no-op at 1×; 2:1 aspect honoured)
        ▼
 Constraint solver (global colours → per-cell colour sets → cell-clamped dithering → refine)
        ▼
 Validated C64 image model (bitmap/screen/colour/bg | charset+map | sprites)
        ├─► Preview renderer (palette object, border, aspect, optional PAL shader)
        └─► Exporters (formats crate)
```

**A plain project takes png2c64's front end (#190).** One visible, fully opaque 8-bit layer filling the screen exactly, with no mask, no regions and no adjustment but gamma, png2c64's sharpen (its 3 × 3 kernel, no threshold; #228) and range matching, skips the supersampled composite. Its colour is png2c64's prepared image: the bytes scaled straight to the target grid with png2c64's Mitchell bicubic, then gamma and sharpen, then png2c64's preprocessing (`solve::png2c64_ref::prepare`), then range matching. The frame is marked prepared, so the reference solver does not repeat the OKLab round trip. `convert64 convert picture.png` is therefore png2c64's output end to end. Anything more (a nudge, a mask, a second layer) uses the composite. Since ADR-012 (#227) the composite resamples every layer the same way, so for a plain project the two paths give the same frame bit for bit (a test, `the_general_path_is_png2c64_s_front_end_for_a_plain_layer`); the branch stays until that invariant has stood for a while. `Compositor::set_png2c64_front_end(false)` turns it off. A layer shrunk past 4× under `Downscale::Auto` (#235) samples a mip level png2c64 does not have, so it is not plain: `convert64 convert huge.png` composites it, and `--downscale exact` keeps png2c64's sampling and its front end.

**Large shrinks sample a cached mip chain (#235, [ADR-012](adr/ADR-012-mitchell-resampling-every-layer.md) amendment).** Each layer has `downscale`, `auto` (default) or `exact`. Under Auto, an axis shrunk more than 4× (source pixels per hi-res target pixel, `1 / (scale · n)`, turned layers by their footprint) samples level `⌈log2(shrink / 2)⌉` of a rip-map of 2 × 2 box reductions of the premultiplied colour in linear light, with the same Mitchell taps; masks do the same on their 4× grid. The levels live in the `SourceImage` (`composite::mip`), built the first time a composite needs them and kept while the source lives, so every compositor and drag shares them; they are never saved. `Compositor::mip_builds` counts them and `set_clock` times them (the app logs the time, to the browser console in wasm).

**A layer can opt into png2amiga's Lanczos-3 (ADR-014 §3, #273).** `Layer::resample` is `mitchell` (the default above) or `lanczos`: the layer's colour is resampled with png2amiga's Lanczos-3 widened by the shrink (`⌈3 · max(1, shrink)⌉` taps each side, cut at the source's edge and renormalised, each pass clamped to 0..1; rotated layers separably along the source axes from a table of the kernel), on the mip level past 4× under Auto as above. Masks stay Mitchell. The front end after it is png2c64's, so a fitted Lanczos layer is png2amiga's scaler followed by png2c64's preprocessing; it composites through the general path (the plain branch is png2c64's Mitchell). It makes thin lines more even, and is part of the line-art recipe (§3.1).

**The png2amiga solver brings its own front end (ADR-014 §2).** `composite::Pipeline` is `Png2c64` (default) or `Png2amiga`; the session (and the app's solve engine) sets it from `solve.solver`. Under `Png2amiga` every layer's colour is resampled with the Lanczos whatever its `resample`, and the preprocessing (gamma, sharpen, black point, OKLab round trip) uses png2amiga's colour maths; a plain project takes png2amiga's `prepare`. So `--solver png2amiga` is png2amiga's picture end to end (`tests/png2amiga_convert.rs`, 20 cases at 100 %), and the reference and native solvers never see png2amiga's front end.

**Dithering is a per-pixel attribute carried through compositing, not an operation per layer** ([ADR-002](adr/ADR-002-dither-as-per-pixel-attribute.md)). Palette indices cannot be blended; layers stay in float and control only the recipe.

## 3. Algorithms

### 3.1 Per-cell constraint solving (bitmap modes)
**Since #181 the base is png2c64's pipeline, ported** (`solve::png2c64_ref`, [ADR-010](adr/ADR-010-png2c64-reference-solver.md), [reference/png2c64-reference.md](reference/png2c64-reference.md)): per cell, a brute force over every set (455 multicolor triples per background, 120 hi-res pairs) on OKLab distances, an ordered method's threshold biasing each pixel (L by 0.15 t, a and b by 0.03 t) and the picture being the search's nearest colours; for diffusion and `none`, a global Floyd–Steinberg pass against the full palette chooses each cell's colours (most used first) and the diffusion runs within them; the background with the least search error wins. The parity test against png2c64's own output is the standing bar. Our extras are hooks on it (pins, locks, allow/deny, per-pixel recipes, the interactive fast path) and named divergences, off by default. The outline below is the native solver (`SolverKind::Native`), which frames the reference cannot solve fall back to (methods png2c64 lacks, non-OKLab metrics, Force, the native knobs).

**png2amiga's C64 path, ported, is a comparison solver (#261; ADR-013 made it the reference, [ADR-014](adr/ADR-014-png2c64-stays-reference.md) reversed that).** `solve::png2amiga_ref` behind `SolverKind::Png2amiga` (`--solver png2amiga`), with its own front end (§2). The multicolor background is colour 0 unless `solve.background_rule = "search"` (png2c64's 16-way search on its encoder, experimental); for any method but `none` each cell's colours are the most used of a global Floyd–Steinberg pass (multicolor the top three, hi-res the pair whose OKLab segment best mixes over the 3 × 3-blurred cell), for `none` a brute force by the cell metric; then every method dithers within the cell's colours. Its opt-in Refine (`solve.refine`, experimental, CLI and MCP only) descends on the blurred difference across cells and keeps the result only if SSIMULACRA2 improves. A frame it cannot solve goes to the reference, then native. Its parity test is a test of the port, not of quality; [reference/png2amiga-reference.md](reference/png2amiga-reference.md) tables how it differs from png2c64. #273's cause sheet found three differences account for png2amiga's worse pictures by eye: the cell colours from the global Floyd–Steinberg histogram, the widened scaler, and the ordered amplitude (png2c64 applies the strength twice). Its 32 methods png2c64 lacks are `DitherKind`s ([reference/dither-methods.md](reference/dither-methods.md) §7). Jaakko judged them by eye on #280's sheets and kept all 32 (#275): the reference runs them on png2c64's pipeline (`solve::png2c64_ref::amiga`: png2c64's front end and cell stage, png2amiga's threshold biasing the search for the 20 threshold kinds, png2amiga's per-pixel pass within png2c64's cells for the 10 diffusion and palette-aware kinds, and a nearest pick for `gilbert` and `dbs`, which png2amiga runs as one on C64), mixed per pixel with png2c64's methods and under every reference hook. `--solver png2amiga` runs png2amiga's own versions; the native solver keeps its stand-ins (`DitherKind::native`) for frames handed to it.

**The line-art recipe (ADR-014 §4), an opt-in chosen by eye:** hi-res `none` at gamma 1.5, `resample = lanczos`, the black point 0.5 (`adjustments.black_point`, png2c64's `--black-point`: OKLab L levels after the sharpen, in png2c64's order) and the recipe's allowed colours limited to Pepto's greys (0, 11, 12, 15, 1), solved by the png2amiga solver (`solve.solver = "png2amiga"`, opt-in under ADR-014 §2). It is [`examples/recipes/lineart.toml`](../examples/recipes/lineart.toml), a batch manifest with the equivalent `convert` line; golden `fixtures/lineart/expected_lineart-recipe.png`. It suits black-and-white line art only: on coloured art the black point turns dark fills black and the colour limit greys everything (`sheet-lineart.png`, `sheet-lineart-recipe.png`). *As built (#276):* #273 solved it by the reference, which gave 87.6 % of tile 13's pixels; swapping each difference into tile 13 alone (`cargo xtask lineart-sheet`, `sheet-lineart-recipe.md`) showed that the scaler, the preprocessing maths, the black point, the grey limit, the metric and the gamma all give tile 13 to the pixel, and the whole gap is png2c64's hi-res `none` cell stage: after its 120-pair search it replaces each cell's pair with the two most used colours of a global Floyd–Steinberg dither (87.5 % alone). png2amiga keeps the brute-force pair, and png2c64 has no option to skip the replacement, so the recipe takes the png2amiga solver and is tile 13 exactly (100.000 %).
```
solve(target, mode):
  for bg in candidate_bgs (16, or pinned 1):
     cells    = per cell: best k colours given bg (brute force over valid combos in metric space,
                intersected with allow/deny sets and pins)
     dithered = dither(target, cells, attribute_map)      # diffusion clamped to the cell
     cells    = re-pick from dithered + residual error      # 1–2 refinement iterations
     score    = Σ perceptual error (MSE in OKLab; optional SSIM-like)
  return best; locked cells are copied through untouched
```
- Error diffusion is **clamped at cell boundaries**; error never propagates into a cell lacking the needed colour.
- Ordered dither with per-pixel strength is embarrassingly parallel; diffusion is serpentine per row.
- Budget: 16 bg candidates × (cluster + dither + refine) on 160×200 ≈ <100 ms native, 100–300 ms WASM single-thread. Interactive drags use pinned bg + single pass (≈ 5–15 ms); release triggers the full search.
- **Force vs Adaptive:** Force weights a region's intended colours heavily in the cell-set choice (neighbours pay); Adaptive lets the solver minimise total error.

### 3.2 Charset / tile modes
Global budget ≤ 256 (ECM 64) unique 8×8 patterns. Greedy merge by perceptual damage first (what CharPad does), then k-means refinement (what png2c64 does); expose a merge-tolerance slider and a live counter. Levenshtein-style reuse (C64CC) as an alternative heuristic.

### 3.3 Sub-pixel nudging
Layers positioned in float; each is resampled with png2c64's Mitchell bicubic (or, opted in, png2amiga's widened Lanczos-3, §2) straight to the target grid, so a 0.25-px nudge moves the kernel's phase and the layer's edge coverage, changes cell statistics, and the solver legitimately re-picks cell colours (ADR-012; `tests/rotate.rs` checks it on a fixture). Masks are resampled at 4× and box-reduced, because a hard mask edge at 1× is measured off by up to 0.3 in coverage. A higher supersample factor (1–8) is kept as a setting; it composites on the finer grid and box-filters it down, which is softer. Past a 4× shrink a layer under Downscale Auto samples a mip level of its source (#235); the level is a power-of-two snapshot independent of the transform, so a nudge, scale or rotation drag only moves the kernel's phase on it and builds nothing, and the edge coverage stays the original's exact box coverage.

### 3.4 Colour accuracy
- Quantise in a perceptual space: default OKLab; CIELAB; Pepto-YUV with luma weighted 2–3×; raw RGB.
- Preview palette and quantisation palette are **the same object**; changing the preset re-solves.

### 3.5 Attribute map semantics
Each target pixel carries `(method, strength, phase, metric, allowed_set, fit_mode)`. Ordered-dither pixels use their threshold matrix; diffusion pixels join the diffusion pass; `strength` scales threshold/error; `allowed_set` intersects with the cell's candidate colours (a layer can *pull* a colour into a cell); feathered masks give a gradient of strength so dithered regions fade into flat ones.

## 4. Repository layout

```
convert64/                      cargo workspace only (Rust 2024)
├─ crates/
│  ├─ convert64-core/           pure Rust, no I/O: document model (`Project`), command registry,
│  │                            compositor, attribute map, solvers per mode, validators.
│  │                            Builds for wasm32 unchanged.
│  ├─ convert64-formats/        byte-exact readers/writers: Koala, Art Studio, raw bins, .spd,
│  │                            CharPad-style bins, PRG stubs, asm/C emitters
│  ├─ convert64-project/        the `.convert64` project file (schema, load/save, recipe migration,
│  │                            sources, masks) and `Session` (editor + compositor + solver); file
│  │                            access through a `Store` trait, `FsStore` behind the `fs` feature.
│  │                            Builds for wasm32 (no `parallel`, no `fs`).
│  ├─ convert64-cli/            `convert64` binary (clap): convert / validate / export / batch
│  ├─ convert64-mcp/            MCP server (rmcp, stdio) over the command registry
│  └─ convert64-app/            eframe/egui UI; native via `cargo run`, web via `trunk build`
├─ fixtures/                    source images + golden outputs for regression
└─ docs/                        PDD, TAD, adr/, reference/, TOOLING, github/
```

Crates: `image` (decode; PNG preview export in `convert64-formats`), own OKLab code, `rayon` behind a `parallel` feature, `serde` (project file + command args), `clap`, `serde_json` (CLI/MCP JSON output, Figma token import, the raw bitmap set's manifest in `convert64-formats`), `toml` (fixture thresholds, project config), `rmcp`, `eframe`/`egui`, `egui_material_icons` (icons, ADR-007), `egui_dnd` (drag to reorder layers, #206), `rfd` (file dialogs), `insta` + `proptest` (tests). No `std::fs` in core.

As built in E1 (2026-10-11): `eframe` runs on the **`glow`** (OpenGL / WebGL2) renderer, not its default `wgpu`, keeping ADR-006's no-wgpu rule; `rust-toolchain.toml` pins stable with `rustfmt`, `clippy` and the `wasm32-unknown-unknown` target; the app's web build uses the eframe web template with no extra JS-side crates (eframe re-exports what it needs). The solve Web Worker (#265) keeps to that: it is the same wasm module started from a five-line `solve-worker.js`, driven through `js_sys::Reflect` on eframe's re-exports, with no `web-sys` features beyond eframe's. See [ADR-005](adr/ADR-005-command-engine-with-cli-json-mcp.md) and [ADR-006](adr/ADR-006-egui-single-rust-workspace.md).

As built in E3 (2026-10-11): `serde_json` and `toml` replaced the hand-written JSON writer (`convert64 validate --json`) and `thresholds.toml` reader that E3 shipped before they were listed here. `rmcp` already depends on `serde_json`, `thresholds.toml` needs comments and nested tables, and [figma.md](figma.md) plans a `tokens.json` import. Neither crate is a dependency of `convert64-core`'s library; the golden-test harness uses `toml` as a dev-dependency only.

As built in E4 (2026-10-11): the compositor and attribute map ([ADR-002](adr/ADR-002-dither-as-per-pixel-attribute.md), [ADR-008](adr/ADR-008-layer-masks-raster-plus-parametric.md)), in `convert64-core`. #4 tracks the PRs.

**Types**
- **`geom`.** `Vec2`, `PxRect`, `CellSet` (the 40 × 25 cells), and `SuperGrid { n, target }`. `SuperGrid` is the only type that knows the multicolor 2:1 aspect.
- **Canvas space.** Layers are placed in a 320 × 200 square-pixel canvas. Target space is 320 × 200 or 160 × 200, and super space is n × target.
- **`layer`.**
  - `SourceImage` holds f32 linear straight-alpha RGBA, decoded once through an sRGB LUT and shared by `Arc`.
  - `Transform { translate, scale, flip_h, flip_v }` is axis-aligned with scale > 0.
  - `Layer { name, source, transform, opacity, blend: Blend, visible, mask, adjustments, recipe }`.
  - `Recipe { method: Method { kind, orientation }, strength, phase, metric, allowed, fit_mode }`.
  - `Adjustments { levels, brightness_contrast, hsl, curves }` with exact per-byte tables, HSL in OKLCh, and Fritsch–Carlson curves.
- **`mask`.** `Mask { raster: MaskRaster, sources: Vec<SourceEntry>, modifiers: Modifiers }`:
  - `MaskRaster` is f32 in memory and u16 on disk.
  - `SourceEntry` is a `Gradient` or `Key` plus a `Combine`.
  - `Modifiers` holds invert, `Levels`, `SnapToCells` and feather.
- **`region::Region`.** A canvas-space mask plus a `Recipe`.
- **`composite`.** `Compositor`, `Canvas`, `TargetFrame` (straight linear RGB, coverage, `AttributeMap`), `AttrSource` and `debug_colour`.
- **`command`.** `Project { layers, regions }`, `Command`, `Editor`, `REGISTRY` and `Dirty`.

**Behaviour**
- **Supersampling.** The default factor is 1× (1–8) since ADR-012; it was 4×. Project files saved before it (`schema_version` 2, which wrote 4) are migrated to 1× on load and saved as version 3; the CLI's `notes` and the app's status bar say so once (#236, [project-file.md](reference/project-file.md#versions)).
- **Resampling** is separable per axis because transforms are axis-aligned. Since ADR-012 it is png2c64's Mitchell bicubic (B = C = ⅓, four taps, edge taps clamped and the weights renormalised, no widening when downscaling), bit for bit png2c64's scaler when the layer fits the grid. A layer's edge is its exact box coverage, so a sub-pixel nudge moves target pixels continuously. (Before #227: bilinear up, Lanczos-3 down, at 4×.)
- **Feather cache.** Each layer keeps two caches:
  - its resampled raster, keyed by the source `Arc`, adjustments, transform, mask generation and grid;
  - its evaluated mask, keyed by the modifiers and that raster's identity.

  The mask generation changes with any raster or source edit. A modifier slider therefore re-evaluates one mask and never resamples the layer's colour. Since #98 it resamples the mask again from its cached layer-space values (raster ⊕ sources) rather than keeping a pre-modifier copy on the super grid, which saves 4 MB per full-screen hi-res layer at 4×.
- **Feather blur.** An exact Gaussian below σ = 4 super px, three box blurs (f64 prefix sums) above, and half resolution from σ = 4 target px (#98; it was above 8).
- **Incremental updates.** `Compositor::update` diffs what each layer contributes. It recomposites the changed footprints aligned to whole cells, and returns the `CellSet` whose output changed, found as the fold writes the frame. `mask_change_cells` maps a layer-space dirty rect to cells before compositing. Since #98 it also keeps the fold of the layers below the lowest changed one, over the whole grid, while they stay unchanged. A nudge or a slider on layer *k* then re-blends only layers *k* and up. The cache is built when an update recomposites at least a quarter of the grid.
- **Fold.** Colour blends bottom-up with per-blend-mode loops. Attributes are claimed bottom-up in the same loop: each layer whose effective alpha is above zero overwrites, so the top-most governs. Since #98 rasters are stored one plane per channel, and a target row's `n` super rows are blended layer by layer. Per row, a layer touches only the columns where it can contribute, because an all-zero source pixel, or a zero mask, leaves the backdrop unchanged.
- **Performance (#98).** None of the following changes the output. A canvas zero may change sign where a layer is skipped or stored onto a clear band, but such zeros compare equal and the frame does not change. This was checked three ways: the incremental-versus-full tests, the golden fixtures, and, during development, fingerprints of every composite against the E4 code in both sequential and parallel builds.
  - Layers are resampled in parallel, and each layer's colour and mask side by side.
  - On a clear band, the bottom layer is stored straight in, because blending onto transparent gives the source in every mode.
  - Resampling works in chunks of rows, so the horizontal pass stays in cache.
  - The box blur runs on lanes of rows or columns at once.
  - A rebuilt layer reuses the buffers it replaced.
  - Parallel tasks take scratch buffers from per-thread pools (`convert64-core::scratch`), so a drag pages in no fresh memory. On the 3-core CI runner, fresh pages and allocator churn had made the parallel nudge slower than the sequential one.
  - A pure translation still resamples and re-feathers the mask. Reusing the shifted mask would not be bit-identical: f32 rounding of the translated taps differs, and the blur clamps at the clipped edge.
- **Performance (#139): the single-threaded cold composite**, the wasm build's first frame after load. Again nothing changes the output. Besides the checks above, new tests compare the colour resample, the half-resolution mask path, gradient rows and mask extents with the previous code bit for bit. Where the time went, hi-res on the benchmark scene, profiled locally:
  - **Colour resample** 14.3 → 8.8 ms. Bilinear outputs take their two taps from a table. The vertical pass splits each row into the planes as it writes it, with the coverage and the clamps in the same vectorised loop; the coverage scale is skipped where both coverages are 1. Chunks are 32 rows.
  - **Mask resample and feather** 18.1 → 9.6 ms. A half-resolution feather with no snap reduces the resampled rows to half resolution as they are made, so the full-resolution mask is no longer stored and read back before the feather writes it. The box blur keeps its running sums in registers, a group of 16 rows is transposed in 4 × 4 tiles, and the upsample only visits the edge columns outside its vectorised pairs.
  - **Mask sources, extents and premultiplying** 2.8 → 1.0 ms. Gradients and combines run a row at a time, and mask extents are scanned 16 values at a time, in loops that vectorise. Identity adjustments premultiply the source without the adjust cache's copy of it.
  - **The blend** stays at 7.5 ms, now over a quarter of the 26.9 ms total (was 42.7 ms). The blend and the colour resample are close to their arithmetic floor for bit-identical output.
  - **Not done: resampling on decode.** The resampled raster depends on the transform and the super grid, so resampling at load would only move the same work from the first composite into the load, and the first frame would come no sooner. Doing it outside the compositor would also take it out of the benchmark's timed region without making anything faster.
  - **Not done: a quarter-resolution feather.** From σ = 32 super px (8 target px at 4×) it would save about a tenth of the hi-res cold composite. Its error on hard edges stays below one 8-bit step, but on `portrait-over-title` with the feather raised to 8 target px the mask error is 0.0066 (half resolution: 0.0022), and two attribute pixels change source inside the feather. That fails #98's bar, so it was not kept.
- **Front-ends.** `convert64-cli` is also a library, holding the recipe-file JSON and `Session` (and, since #163, the project file). `convert64 recipe apply` and `convert64-mcp` run the same `Session`.

**Deviations from ADR-002, ADR-008 and this document**
- **Effective alpha.** The top-most layer whose PNG alpha × mask is above zero governs a pixel. Its strength is multiplied by PNG alpha × mask × opacity (#99, which amends ADR-008; E4 first shipped it without opacity). On the target grid, the source covering most super pixels wins, and its strength is summed over its super pixels and divided by n².
- **Feather is approximated.** Above σ = 4 super px the Gaussian is approximated by three box blurs (measured σ within ~4 %).
- **Half-resolution feather.** ADR-008 asks for a half-resolution feather in general. Since #98 it is used from σ = 4 target px; below that, full resolution is cheap enough. Each mask command re-blurs once, which matches ADR-008's "on stroke end". Measured against the full-resolution blur (`mask::eval` and `golden.rs` tests):
  - **Hard edges and a small square.** The error is at most 1.4 % (0.0143) at σ = 4 on a 1× grid, where the half-resolution blur is an exact Gaussian of σ = 2 against three box blurs. It stays below one 8-bit step at 4× and 8× (0.0037 at most).
  - **The composite fixtures.** The mask error is 0.0015 and the composite error 1/255 at most. In the attribute map, 28 target pixels of `portrait-over-title` change source, and strength changes by up to 0.15, all where the full-resolution mask is 0. A layer claims every super pixel its mask is above zero on, and the half-resolution blur spreads that support by a pixel or two.
- **The command registry arrived with E4** (ADR-005 planned it for E7/E8), because the mask commands needed it. `Project` is only layers and regions so far.
- **`MaskClear { value }`** sets the raster to one value (ADR-008's "reset").
- **`LayerViaMask` with `cut`** inverts the selection on the original through its modifiers, because modifiers are super-grid operations.
- **The MCP server** is a small synchronous JSON-RPC implementation on `serde_json` rather than `rmcp`, because `rmcp` needs `tokio`, which is not listed above. Moving to `rmcp` changes the transport only.
- **Regions live in canvas space**, not on the composite's super grid. They stay put when layers move and survive a mode switch.

**Benchmark (#87, #98, #139)**
- **Scene:** 10 full-canvas masked layers, σ = 8 feather, all blend modes, 4×.
- **Command:**
  ```
  cargo test --release -p convert64-core [--features parallel] --test bench -- --ignored --nocapture
  ```
  The `bench` workflow runs it on the macOS runner class and puts the table in the run summary.
- **Rows.** Each row is one case, for multicolor and hi-res:
  - **cold composite:** a new compositor composites the scene.
  - **warm recomposite:** everything re-blended, every layer cached.
  - **nudge one layer:** `update` after moving layer 4 by 0.25 px.
  - **feather slider:** `update` after changing layer 7's feather.

  A row reports the median of 7 (cold) or 21 runs, after one untimed run, the budget that applies to the build, and the fastest and slowest run. The runner is shared and noisy: a single run can be 2–5× the median.
- **Before #98.** E4 measured these on the macOS runner class (Apple M1 Virtual, 3 cores), multicolor / hi-res:
  - With `parallel`: warm recomposite 4.37 / 9.50 ms, cold composite 156 / 173 ms, nudge 15 / 28 ms.
  - Single-threaded: warm recomposite 18.5 / 26.2 ms.
- **After #98.** Three runs on the same runner class, 2026-10-12. Each figure is the median of the three run medians, with their range in brackets, multicolor / hi-res.

  | case | `parallel` | single-threaded |
  |---|---|---|
  | cold composite | 19.6 [16.4–49.5] / 42.7 [33.4–49.1] ms | 49.0 [40.8–76.7] / 96.1 [77.7–124.8] ms |
  | warm recomposite | 3.14 [2.87–5.01] / 6.64 [6.49–9.25] ms | 6.83 [6.26–11.08] / 11.8 [11.8–18.8] ms |
  | nudge one layer | 4.85 [3.55–5.05] / 10.4 [7.53–12.0] ms | 8.22 [7.80–16.0] / 18.0 [15.0–21.7] ms |
  | feather slider | 4.15 [2.38–6.51] / 7.27 [5.62–7.65] ms | 5.72 [4.69–9.01] / 13.2 [9.23–16.1] ms |

  These meet #98's budgets: nudge ≤ 10 / 12 ms with `parallel`, cold composite ≤ 60 ms with `parallel`, and single-threaded warm ≤ 12 / 16 ms. The single-threaded hi-res warm recomposite went over in one run of three (18.8 ms), on a host where every row was slow.
- **#139** adds a budget of ≤ 60 ms for the single-threaded cold composite, the wasm build's first frame after load. Measured on the same runner class, 2026-10-12; each batch of runs was started at once. Each figure is the median of the run medians, with their range in brackets, multicolor / hi-res.
  - **Before #139:** `main` (bc7fb50), five runs.

    | case | `parallel` | single-threaded |
    |---|---|---|
    | cold composite | 30.6 [16.2–64.9] / 34.4 [29.8–41.5] ms | 50.8 [41.2–64.7] / 105 [77.9–122] ms |
    | warm recomposite | 3.25 [3.03–5.04] / 6.62 [6.01–9.56] ms | 6.47 [6.18–9.24] / 13.5 [11.3–18.7] ms |
    | nudge one layer | 4.27 [3.47–5.42] / 8.89 [7.23–11.7] ms | 7.77 [6.97–9.24] / 17.8 [15.1–28.2] ms |
    | feather slider | 2.43 [2.10–3.63] / 6.52 [4.96–7.58] ms | 5.43 [4.55–9.43] / 10.8 [9.34–13.8] ms |
  - **After #139:** ten runs, five of the final code and five of a version that differed only in how a growing buffer is allocated, which measured the same.

    | case | `parallel` | single-threaded |
    |---|---|---|
    | cold composite | 14.1 [10.9–87.3] / 31.2 [21.1–64.0] ms | 32.5 [22.4–49.3] / 63.3 [47.9–87.6] ms |
    | warm recomposite | 4.52 [2.81–16.7] / 7.46 [5.77–12.5] ms | 7.86 [5.53–10.7] / 14.3 [10.2–21.7] ms |
    | nudge one layer | 4.64 [2.91–16.6] / 7.96 [6.20–20.2] ms | 7.66 [6.19–13.4] / 17.3 [10.8–21.2] ms |
    | feather slider | 2.11 [1.74–7.27] / 5.35 [3.51–11.0] ms | 4.77 [3.53–6.16] / 11.2 [6.78–14.7] ms |

  The single-threaded cold composite falls from 105 to 63 ms hi-res and from 51 to 33 ms multicolor. Multicolor meets the 60 ms budget in every run. Hi-res does not: it was within it in 4 runs of 10 (47.9–56.3 ms) and over it in 6 (62.6–87.6 ms), so #139's target is not met for hi-res. The warm recomposite is unchanged, and the nudge and the feather slider, which rebuild one layer, gain little; the differences in those rows are the hosts'.
- **Builds.** `convert64-cli` and `convert64-mcp` build the core with `parallel`. The app does not, because it also targets wasm.

As built in E5 (2026-10-12): the bitmap solver (§3.1, §3.4–§3.5), in `convert64-core::solve`. Since #181 it is the native solver (`SolverKind::Native`), no longer the default; see "As built in #181" below. The first run was #126–#136; the follow-ups #123, #142, #114, #115, #120, #150 and #116 are recorded in their own "As built" paragraphs below. #5 tracked them all.

**E5 in summary (at closing)**
- **Methods.** 58 `DitherKind`s; every one has a golden on `gradient` (`fixtures/gradient/expected_m-<id>.png`).

  | Family | Count | Kinds |
  |---|---|---|
  | None | 1 | `none` |
  | Ordered | 27 | `bayer2/4/8/16`, `bayer2x2-mc`, `checker`, `h2x4`, `clustered4/8`, `halftone8`, `dot`, `diamond`, `star`, `line-h2/4/8`, `line-v2/4/8`, `line-checker`, `diag-down/up`, `diagonal8`, `spiral5`, `hex5/8`, `custom` |
  | Noise | 4 | `blue-noise`, `ign`, `r2`, `white-noise` |
  | Effects | 3 | `crosshatch`, `radial`, `value-noise` |
  | Diffusion | 17 | `floyd-steinberg` (`fs`), `false-fs`, `fs-wide`, `line-fs`, `jarvis-judice-ninke` (`jarvis`), `stucki`, `burkes`, `sierra` (`sierra3`), `sierra2`, `sierra-lite`, `atkinson`, `stevenson-arce`, `shiau-fan1/2`, `ostromoukhov`, `zhou-fang`, `riemersma` |
  | Palette-aware | 6 | `yliluoma1/2/3`, `knoll`, `luma`, `pair` |

- **Metrics.**
  - Colour metrics: OKLab (default), CIELAB, Pepto Y'UV (luma-weighted) and RGB, with the signed same-luma pair bias (default 0).
  - Cell metrics for the set search: `mse` (default), `blur` and `ssim`.
- **Recipe knobs and defaults** (all serialised, all on the CLI):
  - `strength` 0–1.
  - `phase` screen.
  - `orientation` auto.
  - `cell_clamp` true.
  - `max_luma_diff` 8 (off).
  - `adaptive` 0.
  - `mix_levels` 4 (ordered, noise and effects; 0 = continuous).
  - `smooth` false.
  - `cell_metric` mse.
  - `error_clamp` 0.1 (diffusion; 0 = off).
  - `custom` matrix (for `custom`).
- **Solve options.**
  - `background` auto.
  - `refinements` 2.
  - `finalists` 3.
  - `hysteresis` off (0.97 is Dithertron's).
  - `luma_pair_bias` 0.
- **Adjustments added for png2c64 parity.** `gamma` (identity) and `match_range` (off), with the "Auto levels" suggestion.
- **Ordered semantics (#156).** Default `OrderedSemantics::Png2c64`, png2c64's `apply_ordered_matrix` formula, with the chroma guard on and cells re-picked by what they render. E5's semantics remain as `Legacy` (`expected_legacy.png` reproduces the old portrait golden).
- **Quality gate (#156).** A human review of `sheet-vs-png2c64.png`, not a number.
  - Per-pixel and blurred OKLab MSE against the gamma-1.5 source are reported for every fixture.
  - Both bars said we beat png2c64 while the eye said png2c64 won: per-pixel MSE rewards a flat wrong colour, and blurred MSE rewards a banded render whose regional averages are right.
  - The sheet adds a gradient-domain error on lightness (differences of horizontal and vertical L gradients after a 1-px blur) for reading. With the png2c64 semantics, our ordered renders score about twice png2c64's on it on the test card (`bayer4` 1.37 vs 0.67). Ours carry more pattern energy: more bridging colours and stepped cell-wide bands in the colour ramps. Diffusion is level or lower (`fs` 1.18 vs 1.31).
- **Determinism.** Byte-identical on 1 and N threads, and across OSes as far as CI shows: platform `sin`/`powf` are kept out of the solver's input since #147.

**Original E5 notes (the first run, #126–#136)**

**Types**
- **`metric`.**
  - `Metric { OkLab, CieLab, PeptoYuv { luma_weight }, Rgb }` measures squared, comparably scaled distances.
  - `MetricTables` holds the active `Palette`'s 16 colours in every space, so a new palette means new tables and a re-solve (§3.4).
  - Cube roots and the sRGB transfer use basic IEEE operations only, so coordinates are identical on every platform.
- **`input::Input`.** Each target pixel's `TargetFrame` colour in its recipe's metric coordinates, with its coverage weight and recipe. Per cell, a 16 × P table of weighted distances, built once per solve and shared by every background.
- **Search and dither.**
  - `cell::search` and `Constraints` (candidates, required) are the cell search.
  - `ordered::{threshold, ordered_pick, blue_noise}` handle ordered and noise patterns.
  - `dither::{pass, Locked, layer_origins}` is the dithering pass.
- **Top level.** `Solver { tables }` with `solve(&TargetFrame, &Steering, previous, &SolveOptions) -> Solution { image: BitmapImage, background, sets, error, report: ClashReport }`. Options are `SolveMode { Full, Interactive { dirty } }` and `Background { Auto, Fixed }`.
- **Steering.** `Steering { pins: Pin { colours, full }, locks, allow, deny }` lives in `Project` and is changed by the `cell_pin`, `cell_lock`, `cell_clear`, `cell_allow` and `cell_deny` commands.
- **Report.** `ClashReport`, `CellReport` and `Summary`.
- **Outside `solve`.** `BitmapImage::encode` packs a solved `IndexedBitmap` into bytes with a deterministic slot order. `convert64-formats` gains the Koala and Art Studio writers (`BitmapFormat::write`). `convert64 convert` runs the whole pipeline from the CLI.

**Search cost model**
- A multicolor cell has 455 sets (background + 3 of 15); a hi-res cell has 120 pairs.
- Each set costs one min-and-sum over the cell's 32 or 64 pixels, using a running per-pixel minimum through the nested picks.
- The search is branch-and-bound: a subtree is skipped when Σ min(current, best colour still available) cannot beat the best set. The result is exactly the brute-force minimum, with the same tie order.
- A full multicolor solve evaluates 16 backgrounds × 1000 cells in that search, then dithers and refines three finalists.
- **Timings** on the CI macOS runner (`macos-latest`, Apple M1 Virtual, 3 cores), release, `tests/solve_bench.rs`, portrait (multicolor) and lineart (hi-res) fixtures at 4×:

| Case | Full, `parallel` | Full, single-threaded | Interactive after a 0.25 px nudge, `parallel` |
|---|---|---|---|
| Multicolor, Floyd–Steinberg | 82.2 ms | 93.8 ms | 9.3 ms |
| Multicolor, Bayer 4 | 28.4 ms | 54.5 ms | 7.2 ms |
| Hi-res, Floyd–Steinberg | 23.6 ms | 24.8 ms | 10.2 ms |
| Hi-res, Bayer 4 | 11.1 ms | 12.2 ms | 9.8–18.0 ms (noisy runner) |

**Quality bar as E5 shipped it** (superseded by #142 below): OKLab MSE against the unpreprocessed source, ours vs png2c64:

| Fixture | Ours | png2c64 |
|---|---|---|
| portrait | 0.0035 | 0.0106 |
| terrain | 0.0044 | 0.0180 |
| title | 0.0044 | 0.0232 |
| gradient | 0.0137 | 0.0189 |
| photo | 0.0098 | 0.0285 |
| lineart | 0.0030 | 0.0167 |

**Deviations from §3.1 and ADR-002**
- **The background search has two stages.** Every background is scored by the search alone (sum of the cells' best set errors), and only the best three are dithered and refined. §3.1 dithers every background; the search score ranks backgrounds the same way at a fifth of the cost.
- **The solver works on the target grid** (the `TargetFrame`), with coverage as each pixel's weight, not on the super grid. That is the pipeline order in §2: target resample, then solve.
- **Error diffusion runs in the pixel's metric coordinates**, not linear RGB.
  - Error flows only between pixels of the same metric and from diffusion pixels to diffusion pixels.
  - It is scaled by the sender's strength.
  - It is dropped when it would enter a cell whose set lacks the colour it pulls towards: the nearest of all 16 to the receiver's target plus the error.
- **Ordered dithering is positional.** A pixel picks between the two nearest colours of its cell's set by projection onto the segment between them. Strength pulls the threshold towards ½, so strength 0 is exactly nearest-in-set.
- **Refinement keeps its best round.** It re-picks sets from colour plus received error, at most twice, stops when nothing changes, and keeps the best-scoring round.
- **Force** counts 8× in the set search, and a colour outside the layer's four most-wanted (its own histogram) costs it a fixed 1.0 more. Dithering and the reported error use true distances.
- **`allowed_set` is a per-cell filter**: the intersection over the cell's covered pixels. The multicolor background is exempt, being global.
- **The same-luma pair bias is signed.** Positive values penalise pairs (the brief's flicker concern); negative values reward them (§1.1's "blend cleanly"). The default is 0.
- **Locks keep the previous background**, because a locked multicolor cell may show it. Interactive mode is implemented as "lock every clean cell".
- **`Orientation` gains `TwoToOne`** next to `VerticalBiased` (the line variant).
- **The writers came early.** The Koala and Art Studio writers are E6 scope (#6), pulled forward for `convert64 convert`.

As built in #142 (2026-10-12): calm flat areas, linear-light mixes and a fair quality bar.

- **Why.** Side by side, png2c64 looked better on `portrait` and `photo` although E5 scored 1.4–5× lower MSE. E5's per-pixel ratio followed the source's micro-texture, so flat areas dithered in blotches. Its ratio was taken in OKLab, while the eye averages a pattern in linear light, so mixes with dark colours came out too bright. And the bar compared png2c64's output, solved from a gamma-1.5 source, with the unpreprocessed one.
- **Mix levels** (`solve::mix`, recipe `mix_levels`, default 4; ordered and noise methods). A pixel mixes its nearest colour of the cell's set with the partner whose linear-RGB mix reproduces it best (not simply the second nearest: dark olive next to brown mixes with black, not the nearer dark grey), and its ratio is that mix's share. Per cell and pair, pixels within half a level step of the median are flat: they take one ratio, the cell's mean, snapped to what the eight neighbouring cells show of the same two colours when within half a step (hysteresis), and quantised to `N + 1` levels. Other pixels (edges, texture) keep their own continuous ratio. A pixel mixes only if the mix plus a spread penalty (`SPREAD_PENALTY` 0.25 × t(1 − t) × the pair's distance, after Yliluoma) is closer than its nearest colour alone, so pale areas next to a dark background colour are not peppered with it. `mix_levels = 0` keeps E5's continuous OKLab ratio.
- **Ranking by what the eye sees.** `PassResult::seen` measures a mixed pixel against the linear-light mix it is part of, at the share its pattern actually shows (`ordered::shown_share`). Finalists and refinement rounds are ranked by it; `Solution::error` and the clash report stay per pixel.
- **Pre-smoothing** (recipe `smooth`, default off). The ratio is taken from an edge-aware 5 × 5 Gaussian (σ = 1 target pixel; neighbours ≥ 0.08 OKLab ΔE away are left out), while colours are still chosen from the unblurred composite. It is off by default because, with mix levels doing the calming, it raised the blurred error on every fixture (title past png2c64's).
- **Levels.** `Adjustments` gains `gamma` (png2c64's `--gamma`: a per-channel power in linear light, applied first) and `match_range` (png2c64's `--match-range`: the image's 1st–99th-percentile OKLab extent mapped onto a palette's, applied last over the whole image). `suggest_range_match` is the "Auto levels" suggestion for sources whose L range covers under 60 % of the palette's. `convert64 convert` prints it as a hint and never applies it unasked (`--gamma`, `--match-range`, `--mix-levels`, `--smooth`).
- **Fair comparison.** The golden harness solves every fixture from png2c64's preprocessed source (`Gamma::PNG2C64` as a layer adjustment) and scores both converters against that source.
- **Blurred quality bar.** Both renderings are blurred in linear light (binomial, σ ≈ 1.4 display pixels) before the OKLab MSE. Ours must not be worse than png2c64's (`png2c64_blur_mse_vs_source`). Per-pixel MSE is still reported but is no longer a bar: against a colour the palette cannot reach, it is lowest for a flat wrong colour, which is what this issue removes. `portrait`'s olive background now dithers brown and black to the right tone, so its per-pixel MSE is 1.9× png2c64's while its blurred error is 10× lower.
- **Fixture fix.** `gradient`'s swatch band held Colodore values while the solve uses Pepto. It now holds the 16 Pepto colours, as its README says. Its png2c64 baseline was regenerated (VICE check 100 %), and `flat_areas::gradient_swatches_quantise_exactly` checks every swatch pixel, solved from the raw source.

| Fixture | Blurred MSE, ours | Blurred MSE, png2c64 | Per-pixel MSE, ours | Per-pixel MSE, png2c64 |
|---|---|---|---|---|
| portrait | 0.00144 | 0.01446 | 0.0331 | 0.0171 |
| terrain | 0.00116 | 0.00314 | 0.0088 | 0.0082 |
| title | 0.00226 | 0.00291 | 0.0082 | 0.0074 |
| gradient | 0.01052 | 0.01161 | 0.0163 | 0.0135 |
| photo | 0.00255 | 0.00376 | 0.0118 | 0.0157 |
| lineart | 0.00031 | 0.00045 | 0.0032 | 0.0151 |

As built in #114, part 1 (2026-10-12): every ordered, noise and effect method of [reference/dither-methods.md](reference/dither-methods.md) §1–§2.

- **Methods.** `DitherKind` has 39 kinds: 1 none, 4 diffusion, 27 ordered (`custom` included), 4 noise and 3 effects. `DitherFamily` gains `Effects` and `PaletteAware`, and `is_threshold()` covers ordered, noise and effects.
- **png2c64's matrices and analytical patterns** (`ign`, `r2`, `white-noise`, `crosshatch`, `radial`, `value-noise`) are ported from its `dither.cpp` as written, in f32 with basic IEEE operations only. Its names are accepted as serde aliases, as are the reference's `fs`, `jarvis` and `sierra3`.
- **Generated matrices.** `bayer16`, `clustered8` and the ArcPaint64 shapes (`dot` 6 × 6, `diamond`, `star`, `diag-down`/`diag-up` 8 × 8) are generated from integer metrics, so they are the same on every platform. `line-v*` are the transposed `line-h*`.
- **`custom`** carries a user 8 × 8 or 16 × 16 matrix in `Method::custom`; only its order matters. Without a matrix it is Bayer 8.
- **`white-noise`** is png2c64's per-pixel hash, the same everywhere. The reference's per-recipe seed is not built: the hash already keeps goldens stable.
- **Dither-aware set search.** As in png2c64, a threshold pixel's OKLab L is nudged by `(threshold − ½) × strength × 0.15` before the cell-set search scores it (`Px::search`). This uses the screen-locked pattern position. Without it, a cell in a smooth ramp keeps only the colour its pixels are nearest to, and the pattern has nothing to mix with. It lowered the blurred error on every golden fixture.
- **Golden.** `methods::gradient_with_every_method` renders `gradient` with every kind (`fixtures/gradient/expected_m-<id>.png`).

As built in #114, part 2 (2026-10-12): the diffusion kernels of §3.

- **Kernels.** 13 more diffusion kinds, 17 in all:
  - `false-fs`, `fs-wide`, `line-fs` (png2c64's), `stucki`, `burkes`, `sierra2`, `sierra-lite`, `stevenson-arce`, `shiau-fan1`/`shiau-fan2`;
  - `ostromoukhov`: png2c64's, Floyd–Steinberg weights × `0.6 + 0.8 × uncertainty`;
  - `zhou-fang`: the same, plus the choice between the two nearest colours thresholded at `½ + (blue noise − ½) × 0.5`;
  - `riemersma`.

  `DitherKind` has 52 kinds: 1 none, 17 diffusion, 27 ordered, 4 noise, 3 effects.
- **Riemersma** walks a Hilbert curve over the target before the raster walk. It keeps Riemersma's 16-sample history (weights `16^(i/15)` over 16, each sample the pixel's error against its own colour) and is clamped at cells like the others. Riemersma pixels neither send error to the raster walk nor receive it.
- **Diffusion-aware set search.** As png2c64 does with its global Floyd–Steinberg pass, a diffusion pixel is searched as the colour an unconstrained diffusion of the whole frame (all 16 colours, its own kernel) gives it. A cell then keeps the colours its dither mixes; before, a cell in a ramp kept only its nearest colour and the diffused error had nowhere to go. Refinement still re-picks from colour plus received error.

As built in #115 (2026-10-12): the palette-aware methods of §4 (`solve::aware`), 58 kinds in all (6 palette-aware).

- **All six are positional.** They are placed by Bayer 8 with phase, orientation and strength (thresholds pulled towards ½; strength 0 is nearest-in-set) and searched with the dither-aware lightness nudge. They mix in linear RGB and score in the pixel's metric.
- **Algorithms.**
  - `yliluoma1`: the best pair and ratio in 1/64 steps (the linear projection and its two neighbouring steps), with Yliluoma's 0.1 spread penalty.
  - `yliluoma2`: a 16-entry running-average candidate list.
  - `yliluoma3`: Knoll's error-accumulating list in linear light.
  - `knoll`: the same list in the metric's space, error × 0.75.

  Lists are sorted by luminance and indexed by the threshold.
- **`luma`** mixes the nearest colour with the set colour on the other side of the pixel's OKLab L that is closest to it in hue. The ratio is taken on luminance, so there is no colour speckle.
- **`pair`** takes the two set colours whose even mix is nearest. A same-luma pair scores `pair_bias − 0.002` lower (the solve's signed bias). The pair is used only if it beats the nearest colour alone, and below strength ½ the nearest colour is kept. The weave alternates by scanline on multicolor (PAL's delay line blends successive lines) and is a checker on hi-res.
- **Ranking.** Finalists are ranked by the colour each pattern averages to, like mix levels.

As built in #120 (2026-10-12): cell metrics for the set search (`solve::cellscore`, recipe `cell_metric`, CLI `--cell-metric`; `--metric mse|blur|ssim` is accepted as png2c64 spells it).

- **`mse`** (the default) is the squared-distance search as before.
- **`blur`** renders the cell with each pixel's nearest set colour, blurs it with a 3 × 3 binomial (taking the target outside the cell), and compares it with the target blurred the same way.
- **`ssim`** adds `0.05 × pixels × (1 − SSIM)` on lightness (Wang et al., K₁ 0.01, K₂ 0.03) to the squared distances, png2c64's hybrid.
- **Cost.** Scoring all 455 sets × 16 backgrounds this way is far slower than the min-and-sum search.
  - The background search stays on `mse`.
  - Only the finalists' cells re-pick, among the `mse` winner's colours and the cell's five most wanted colours: at most 56 multicolor sets or 28 hi-res pairs.
  - Refinement re-picks with the same metric.
- **Timings.** Full solve, release, local M-series, median of 5, single-threaded / `parallel`:

| Case | `mse` | `blur` | `ssim` |
|---|---|---|---|
| `title` multicolor, Bayer 2 | 57.0 / 10.9 ms | 86.1 / 20.0 ms | 79.6 / 18.5 ms |
| `lineart` hi-res, none | 7.3 / 7.4 ms | 34.8 / 23.1 ms | 25.3 / 17.7 ms |

As built in #150 (2026-10-12): `error_clamp`. The #116 comparison sheet showed Floyd–Steinberg and Sierra Lite smearing colour across the test card's saturated bands. An out-of-gamut colour, which no palette colour reaches, accumulated diffused error row after row, both in the unconstrained diffusion the set search uses and in the cell-clamped pass. Recipes now carry `error_clamp`, png2c64's per-channel limit on received error: default 0.1 (its default), 0 for off, `--error-clamp` on the CLI. It applies in every diffusion pass, Riemersma included.

As built in #156 (2026-10-12): png2c64's ordered semantics, the chroma guard and rendered set search. The sheet, not a metric, judged this work.

- **Semantics.** `OrderedSemantics::Png2c64` (the default) ports png2c64's `apply_ordered_matrix` exactly:
  - `a` and `b` are the nearest and second-nearest colours of the cell set in the pixel's metric;
  - `t = √dA / (√dA + √dB)`;
  - the pixel shows `b` where `threshold < t × strength`.

  `Legacy` keeps E5's pick and #142's mix levels (`mix_levels` and `smooth` only act there).
- **Chroma guard** (`chroma_guard`, default on).
  - A neutral pixel (OKLab chroma ≤ 0.02) only uses colours within 0.03 of grey, and its pair's mean chroma may exceed its own by at most 0.03.
  - A tinted pixel only uses colours within about 53° of its hue (`cos ≥ 0.6`; greys always pass).
  - Grey ramps render with greys only (`the_grey_ramp_has_no_chromatic_pixels`), and amber no longer mixes light red with green.
- **Rendered set search.** Cells with png2c64-ordered pixels re-pick their sets for the finalists. The score is:
  - each pixel's distance to the linear-light mix its pair shows (`t × strength`);
  - plus 0.01 × the squared width of the OKLab-L bracket the set gives the pixel (½ more when unbracketed). Same-hue colours bracket chromatic pixels.

  The short list holds the pixels' own guarded pairs over all allowed colours (png2c64's unconstrained choice), their lightness brackets, the squared-distance winner and the most wanted colours, six colours at most.
  - Effects: ramps keep their intermediate greys (`no_grey_ramp_cell_skips_an_intermediate_grey`), and dark olive gets brown with black instead of a black background (#154: `a_two_level_pattern_does_not_turn_dark_olive_black`; `checker` on portrait is a brown/black checker).
  - Cost: refinement skips cells that received no error, and the interactive fast path keeps the squared-distance sets. A full multicolor Bayer 4 solve is 14.8 ms with `parallel` and 98 ms single-threaded (local); it was 7.7 ms and about 55 ms before.
- **Golden harness.** It uses png2c64's own baseline recipe (`checker` 0.7) and reports per-pixel and blurred MSE without gating; a human reviews the sheet (§6, CONTRIBUTING.md).
- **Known differences on the sheet.**
  - The dark end of our grey ramps is too bright. png2c64's `t` is a ratio of OKLab distances, which overstates a dark mix in linear light, and our sets offer dark grey where png2c64's offer brown or nothing.
  - Colour ramps split into cell-wide bands where neighbouring cells pick different sets.
  - The radial blob has a pale ring.

As built in #181 (2026-10-12): png2c64's pipeline, ported, as the base of the bitmap solver ([ADR-010](adr/ADR-010-png2c64-reference-solver.md)).

- **Why.** Two rounds (#142, #157) reasoned about what png2c64 does and reimplemented it inside E5's solver; each time the sheet showed png2c64 winning by eye (20 of 30 ids after #157, every one an ordered method, every "equal" a diffusion method). Two lessons. The metrics misled: per-pixel and blurred MSE both said we were winning. And ordered dithers expose set-search errors that diffusion hides, so the fault was in the cell colour-set search, not in the threshold formula. The decisive fact was in png2c64's `main.cpp`: its command line never runs the ordered "picker" for ordered methods; the threshold only biases the search, and the picture is the search's own nearest colours.
- **The port** (`solve::png2c64_ref`, credited in `THIRD_PARTY.md`): colour conversions, preprocessing and bicubic scaling (#182), the cell search with its global Floyd–Steinberg pass (#183), every dither (#184), operation for operation in `f32`. `cbrt` and `pow` are computed in `f64` and rounded once.
- **The bar** (#185): `tests/png2c64_parity.rs`, png2c64's 30 ids on the test card and the portrait and all six fixtures, ≥ 99 % identical to png2c64's renders, as its own CI step. The reference is png2c64 built with strict IEEE arithmetic (`-ffp-contract=off`): FMA contraction changes png2c64's own diffusion output by up to 15 %, and #113's unrecorded build could not be reproduced on nine renders, which were regenerated (`cargo xtask png2c64-previews`). 65 of 66 cases are identical; the `title` baseline is at 99.997 %.
- **Default** (#186): `SolverKind::Reference` for the bitmap modes. Each pixel's recipe from the attribute map gives its method, strength (alpha and feather included), error clamp, `adaptive` and pattern anchor. The native solver keeps its goldens (`expected_native.png`) and the variant tests of its knobs. Frames the reference cannot solve fall back to it, and `Solution::fallback` says why.
- **Hooks** (#181 step 6): pins, allow/deny and recipes' allowed colours constrain a cell's search; locked cells, and on the interactive fast path every clean cell, are copied through with the previous background. Each is tested to leave the output png2c64's when neutral.
- **Divergences** (#181 step 7): `chroma_guard`, `max_luma_diff`, `pair_bias`, `cell_clamp`, `mix_levels` and `bridging` as `solve::Divergences`, all off. Each has a row on `sheet-divergences.png` for a human to judge.
- **Speed.** Full multicolor solve 25–27 ms sequential, 5–8 ms with `parallel` (native: 49–95 and 14–16 ms); a drag 6–8 ms sequential (§7).
- **Front end.** The compositor's frame is not png2c64's prepared image. We resample at the supersample factor with box filtering and apply gamma per layer before resampling; png2c64 runs a Mitchell bicubic straight to the target and applies gamma after it. Through the compositor, the fixtures come out 66–99.6 % identical to png2c64's baselines. The sheet compares solvers on png2c64's own front end.

As built in E6 (2026-10-12): the bitmap exports of `convert64-formats` ([reference/file-formats.md](reference/file-formats.md) has every byte). #6 tracks the PRs.

- **Modules.** `BitmapFormat` reads and writes Koala (`.kla`, `.koa`) and Art Studio (`.art`, `.aas`) at any load address (#158); `bitpairs` chooses which bit pair each cell colour takes on export, `Stable` (png2prg's neighbour rule), `Frequency` or `BruteForcePacked`, through `BitmapFormat::write_with` (#124); `raw` writes and reads the raw bitmap set with its JSON manifest (#159); `png` renders a palette-exact preview at zoom 1–8 with the 2:1 multicolor aspect (#160); `asm::emit` writes KickAssembler, ACME, 64tass, ca65 or a C header whose data is the writers' bytes (#161); `prg` (E5) wraps a picture in a display PRG.
- **No I/O.** Every writer returns `(file name, bytes)` pairs and every reader takes bytes or a lookup by file name; the CLI and MCP server do the file system.
- **Dependencies.** `serde` / `serde_json` for the manifest and `image` for the PNG encoder, all in the list above. `image` cannot write an indexed PNG, so the preview is 8-bit RGB. No compression crate: the bitpair policies measure packed size as fixed-Huffman DEFLATE, computed in the crate.
- **Tests.** Round trips of every format on every fixture, both our solves in both modes and the png2c64 baselines, in `crates/convert64-formats/tests/`; `insta` golden text of each emitter and of each bitpair policy's output; real assemblers and `cc` where installed.
- **VICE.** `cargo xtask vice-check` now also runs every export of our own solves (§6), with VICE 3.10.

As built in #163 (2026-10-12): the `.convert64` project file ([ADR-009](adr/ADR-009-project-file.md), [reference/project-file.md](reference/project-file.md)).

- **Where.** `convert64_cli::project`: serde types for the file (`ProjectFile`, `LayerEntry`, `SourceSpec`, `MaskSpec`, `RegionEntry`, `SteeringSpec`, `PaletteSpec`), path helpers and the PNG and base64 codecs for embedded data. `convert64_cli::recipe::Session` loads a project or a version-1 recipe (`Session::open`), and saves one (`Session::save`, `to_project_file`). The core is unchanged: `Project` stays layers, regions and steering; the target mode, palette and solve options are the session's.
- **State, not history.** A saved project holds every layer's mask raster, sources and modifiers, the regions and the steering. Optional `commands` run after loading, as in a recipe; saving writes their result and no commands.
- **Sources.** Linked (path relative to the project file), embedded (the original PNG bytes in base64) or shared with an earlier layer, per layer. `Embed::{Keep, All, None}` chooses on save; `recipe apply --save PATH [--embed | --link]` exposes it.
- **Masks on disk.** A uniform raster is one exact value; anything else an embedded 16-bit greyscale PNG (ADR-008's u16 on disk). The first save of a painted mask rounds it to 1/65535; after that load → save → load is byte-identical, and so is the solve (`tests/project.rs`).
- **Palette.** `pepto-pal` (default), `pepto-ntsc`, `colodore`, 16 custom colours or a VICE `.vpl` file. `Session` builds its `Solver` from it, so the palette a project names is the one it is solved with.
- **Solve options** gain `refinements` and `finalists` (defaults 2 and 3), checked on load with `hysteresis` and `luma_pair_bias`.
- **Base64** is a 60-line encoder/decoder in the CLI crate rather than a new dependency.

As built in #164 (2026-10-12): `convert64 convert` over projects, every M1 output, exit codes and `--json` ([reference/cli.md](reference/cli.md)).

- **One path.** A PNG becomes a one-layer project (`ProjectFile::from_png`, scaled to fill the canvas); a `.convert64` project or version-1 recipe is read as it is. Command-line options are applied to the project file's data before it is loaded (`Overrides::apply`): recipe options as a JSON patch on every layer's recipe, so the core's validating constructors check them; regions keep their own recipes. Then `Session` composites and solves, as `recipe apply` and the MCP server do; `convert` no longer has its own pipeline.
- **Outputs.** Repeatable `-o`, format from the extension or `--format`: `kla`, `art`, `prg` (`convert64_formats::prg::display_prg`) `raw` (`convert64_formats::raw::write`, named by a `.json` manifest path) and `png` (`convert64_formats::png::preview`, `--zoom`). One solve feeds every output. For a PNG input a `.kla` or `.art` output picks the mode.
- **Exit codes.** 0 ok, 1 not legal or more clashes than `--max-clashes` (files still written), 2 input or option error (clap usage errors included), 3 write error.
- **`--json`** is global. Every command prints one object: `command`, `ok`, `exit_code`, `error`, `timing_ms` (`total` and phases), `files_written`, `notes` (since #236: what loading changed, such as an older project's migration), then its own fields (`report.rs`). `convert`'s solve fields are `background`, `clashes`, `legal`, `violations`, `solve_error`, `summary`, `worst`; the total solve error moved from `error` to `solve_error`, since `error` is now the message.
- **`--save-project`** writes the project `convert` solved, overrides applied, so `convert64 convert title.png --dither bayer4 --save-project art/title.convert64` starts a project from the command line.

As built in #166 (2026-10-12): GUI/CLI parity, enforced by a test.

- **`convert64 apply`** runs any registry command on a project (`--commands FILE`, repeatable `--command JSON`) and saves it (`-o`, `--in-place`), through `Session::run_at`, so a command's file paths resolve against the file it came from or the working directory.
- **`convert64 commands`** lists the registry with each command's argument schema; `--json` adds the JSON Schema and a working example. Both live in `convert64_cli::schema`, which the MCP server now uses for its tools' input schemas too (it had hand-written schemas for three commands).
- **The parity test** (`crates/convert64-cli/tests/parity.rs`) enumerates `REGISTRY`, takes each command's example from `convert64 commands --json`, runs it through `convert64 apply`, and compares the saved project with the same command run on the core in-process. A registry command with no JSON form, schema, example or CLI path fails it.
- **What parity covers so far.** The registry holds the 14 mask and cell commands (E4, E5). Layer, recipe, mode and palette changes are not commands yet: the CLI sets them through the project file and `convert`'s options. They join the registry, and so this test, when the app (E8) needs them as undoable actions.

As built in #165 (2026-10-12): `export` to every assembler, `validate` for every format read, `batch` with a manifest.

- **`export <project | dir> --asm …`** solves each project and writes `convert64_formats::asm::emit`'s source for every target (`all` = KickAssembler, ACME, 64tass, ca65, C) with the binaries it includes, once per project; `--data` picks byte lines or includes of the raw set or picture file.
- **`validate`** reads Koala, Art Studio, raw sets (manifest and blocks via `convert64_formats::raw::read`), convert64's display PRGs (header and loader compared byte for byte with `convert64_formats::prg`'s, then the embedded picture validated) and projects (loaded, solved, the solution validated, `clashes` reported).
- **`batch <dir>`** converts every PNG and project in a directory with `std::thread::scope` workers pulling from a shared index (`-j`, default one per CPU); each file runs `convert`'s code with a non-printing report, and the batch report holds them all. Per-file settings come from a TOML manifest (`toml`, already approved in §4): `out`, `[defaults]`, `[files."…"]`, each `convert`'s options in kebab-case (the same `Overrides` struct, deserialised with unknown keys refused) plus `outputs`, `max-clashes` and `skip`. Precedence: command line, then the file's table, then `[defaults]`. A PNG with a same-named project beside it is skipped. Output is the same with one worker or many.

As built in #192 (2026-10-13): the project model moves out of the CLI into `crates/convert64-project` (ADR-009, amended), so the wasm app can load and save projects.

- **What moved.** `convert64_cli::project` and `convert64_cli::recipe` become `convert64_project`: `file` (the `ProjectFile` schema, `SolveSpec`, `PaletteSpec`, masks on disk), `recipe` (the version-1 `RecipeFile` and its migration), `command` (`JsonCommand`, `LayerRef`), `codec` (PNG decode/encode), `paths` (lexical `absolute`/`relative`), the base64 codec and `session::Session`. The CLI library keeps only `schema` (argument schemas and examples, which the MCP server shares). The core does not depend on the new crate.
- **`Store`.** `trait Store: Send + Sync { fn read(&self, &Path) -> Result<Vec<u8>, ProjectError>; fn write(&self, &Path, &[u8]) -> Result<(), ProjectError>; }`. A session owns a `Box<dyn Store>` and reads every linked source, mask, `.vpl` and `mask_load` PNG through it; `Session::open_in(store, path)`, `Session::load(file, base, store)` and `save_to_store` are the store-generic entry points. `MemoryStore` (paths normalised, shareable as `Arc<MemoryStore>`) is the browser's and the tests'. `FsStore`, `Session::open`/`from_project`/`save`, `ProjectFile::read` and the PNG writers (`write_mask`, `write_composite`, `write_attributes`) are behind the `fs` feature, enabled by the CLI and MCP server only.
- **`Session` lives here** because nothing in it needs the file system once files come through a `Store`, and the app needs the same thing the CLI has: a project loaded into the `Editor`, `Compositor` and `Solver`, with the composite, the evaluated masks and the attribute map as RGBA (`composite_rgba`, `evaluated_mask`, `attributes_rgba`).
- **Features.** The crate depends on the core without `parallel`; the CLI and MCP server turn it on, so native solves stay parallel and `cargo tree -p convert64-app --target wasm32-unknown-unknown -e features` shows neither `rayon` nor `fs`. No new external crates: `image`, `serde` and `serde_json` as before.
- **Unchanged.** Saved bytes, schema version 2, CLI output and exit codes. The on-disk round-trip tests stay in `convert64-cli/tests/project.rs`; the module tests moved with the code, and `convert64-project/tests/memory.rs` repeats the round trip with a `MemoryStore`. The app has a load/save smoke test.

As built in #194 (2026-10-13): the solve scheduler, `convert64_app::scheduler` (the app crate's library half, so `cargo test -p convert64-app` tests it without the GUI). The app hands it every edit and draws the newest result; no frame waits for a solve (UX §2, §11).

- **API.** `Scheduler::submit(Snapshot, SolveMode) -> u64` (a ticket), `cancel()`, `poll() -> Option<SolveResult>` once per frame, `tick() -> bool` (this frame's share of the work; `true`: request another frame), `is_busy()`, `set_waker(Arc<dyn Fn() + Send + Sync>)` (e.g. `request_repaint`). A `Snapshot` is core types only: layers, regions, steering, `SuperGrid`, `Palette`, `SolveOptions` and the previous solution (`Arc<Solution>`); it does not depend on the project crate. A `SolveResult` carries the ticket, the solution, the `SolveMode` that ran (for the fast path, with every cell it re-solved), `provisional`, and `Timing { composite_ms, solve_ms }`; `solver()` and `fallback()` read `Solution::solver` and `fallback`.
- **Supersession.** A newer submission replaces every older one: the older one's remaining stages are dropped, and a result of it that is computed but not yet taken is discarded. `cancel` does the same with no successor.
- **One compositor, one solver.** The scheduler owns them, so the compositor's caches survive between submissions and `Compositor::update` recomposites only what changed. Every solve is one `Solver::solve` call on the compositor's frame, with `layer_origins` derived as the CLI does; the tests compare each driver's output with the core called directly. The core is unchanged.
- **The fast path's dirty cells** are the submitted ones plus every cell whose input changed since `previous` was solved: the compositor's changed cells, steering changes (found by comparing pins, locks and allow/deny per cell), and every cell after a grid, palette or option change. The scheduler logs this per composite, so it stays right when the submissions in between were superseded after compositing. A `previous` the scheduler did not make (or no longer remembers; it keeps 16 solutions and 64 composites) counts as stale everywhere.
- **Native: one worker thread** (`Worker`; `std::thread`, a mutex-guarded latest-result slot and condition variables), with the core's `parallel` feature on (a target-specific dependency, so the web build stays single-threaded). Between stages the worker checks for a newer submission. A panic in the core is caught: the worker starts again with fresh caches, that submission delivers nothing, and `take_error` reports it.
- **wasm: stepping, no threads** (`Stepper`, also usable natively through `Scheduler::stepping`, which is how the tests run it). Since #265 the browser runs the solve in a Web Worker (below); stepping remains its fallback when the worker cannot start, and what export and the tests use. A submission is cut at stage boundaries: composite, then solve. A full solve with a previous solution first runs the fast path and delivers it as `provisional` (`Config::preview_full`, on in wasm, off natively). `tick` runs at least one stage, then more while the next stage's last measured duration fits in `Config::frame_budget_ms` (10 ms, leaving about 6 ms of a 60 Hz frame to the UI). A stage is one core call and cannot be split without changing the core (the parity test pins its output), so a full solve is one long frame in wasm; it comes after the provisional result is on screen.

As built in #265 (2026-10-14): in the browser the solve runs in a dedicated Web Worker, so no frame waits for it (it was one 50 ms frame for the fast path and one 160–180 ms frame for the full search, §7). The `Scheduler` API is unchanged; `Scheduler::new` picks `Remote` in wasm.

- **Where the work runs.** The page keeps the compositor (`Client` in `scheduler/remote.rs`): `submit` runs the composite stage at once (a few ms, and the compositor's caches and the fast path's change log stay with the edits) and posts one request. The worker (`Serve`) keeps only the solver, whose palette tables it rebuilds when the palette changes, and runs the request's solve stages, the provisional fast-path preview of a full solve and then the submitted solve, through the same `engine::solve` as the other drivers. Results come back to the page, which drops any for a superseded ticket and notes the solutions it was given for the fast path's dirty cells.
- **One module, two entry points.** `solve-worker.js` (copied into the bundle by `index.html`) imports the app's own `convert64-app.js` and initialises the same wasm in a module worker; `main` finds no `window` and calls `scheduler::serve_worker` instead of starting eframe. There is no second trunk target or crate; the browser fetches the wasm once and compiles it again for the worker (ready 80–110 ms after the worker is created, in parallel with the app's start).
- **Messages** (`scheduler/protocol.rs`): bytes in one `ArrayBuffer`, transferred, so no `SharedArrayBuffer` and no COOP/COEP headers. A header (magic, version, kind, ticket), then for a request a small JSON part for the core's serde types (grid, recipes, pins, solver, divergences, background) and a binary body: colour, coverage and attribute map of the `TargetFrame`, locks, allow/deny lists, palette, the other options, the modes and the previous solution; a result carries one `Solution`. Floats travel as their bits; in the JSON part an `f32` passes through `serde_json::Value` as the exact `f64` it widens to, so it reads back bit for bit (tested over 430 000 bit patterns). The core gained `TargetFrame::from_parts` and `AttributeMap::from_parts` (with `layer_recipes`/`region_recipes`) for this; nothing else in the core changed.
- **Cancellation.** The worker returns to its event loop between stages (`setTimeout` 0), so a newer request or a cancel is seen there; a superseded request is dropped before its next stage, and one superseded before it started is never decoded. A stage is one core call and cannot stop part-way without slicing the core's cell loop, so instead the page keeps a second, idle worker warm: a full submission or a cancel that finds the worker busy terminates it (which stops its stage at once) and goes to the spare, and a new spare starts. Drag frames (interactive) never terminate a worker; they wait for at most one fast-path stage. The worker says when it is idle (`Kind::Idle` with the newest ticket it saw), which is how the page knows it is busy.
- **Failure.** If the worker cannot load (no module workers, the script missing) or reports an error, the scheduler moves the submission that was in it to the stepping driver on the page, keeping its ticket, and logs why. A panic in the worker is logged to the console by a panic hook.
- **Tests** (`scheduler/tests/remote.rs`, native: the two halves connected in-process): protocol round trips (multicolor and hi-res, steering, another palette, options, a previous solution), malformed messages, the JSON float round trip, output identical to the core and to the stepping driver bit for bit, a superseded request dropped between stages and never decoded, cancel, and the fallback's ticket. The browser glue (`scheduler/web.rs`) is measured in §7.

As built in #193 (2026-10-13): every edit the app makes is a registry command.

- **The registry** grows from 14 to 30 commands: `layer_add`, `layer_remove`, `layer_reorder`, `layer_set_visible`, `layer_set_opacity`, `layer_set_blend`, `layer_set_transform`, `layer_nudge`, `layer_rename`, `layer_duplicate`, `adjustments_set`, `recipe_set`, `mode_set`, `palette_set`, `background_pin` and `solve_set`, beside the mask and cell commands and `layer_via_mask`. Each is undoable (a memento of what it replaced: the old property value, the removed layer, the old position, the old setting) and has a JSON form (`convert64_project::JsonCommand`), an argument schema and a working example (`convert64 commands --json`).
- **Partial edits.** `adjustments_set`, `recipe_set` and `solve_set` set the fields they name and keep the rest (`AdjustmentsChange`, `RecipeChange`, `SolveSet` in `convert64_core::command`); `null` clears an optional field (`match_range`, `hysteresis`). A recipe change goes through the recipe's validating constructors; its allowed colours are `allowed` (or the current set) plus `allow` minus `deny`.
- **The document** (`convert64_core::document::Project`) now holds `target`, `palette` and `solve` (`SolveSettings`, moved from the project crate's `SolveSpec`, same serde form) besides layers, regions and steering. The project file is unchanged. `Session` rebuilds the compositor's grid when the target changes and the solver when the palette changes, and solves in full after any settings change; `Dirty::Settings` says so to the app.
- **Divergences are in `solve_set`, not `recipe_set`.** png2c64's divergences (ADR-010) are options of the whole reference solve (`SolveOptions::divergences`), stored in the file's `solve`, not per layer; a per-layer command would promise a scope the solver does not have. The per-pixel knobs that do live on the recipe (`cell_clamp`, `max_luma_diff`, `mix_levels`, `chroma_guard`, …) are in `recipe_set`. The background has its own command, `background_pin`.
- **`mode_set`** accepts the bitmap modes the solver solves (multicolor and hi-res).
- **MCP** gains `project_open` (alias of `recipe_open`) and `project_save`; every registry command is a tool with the CLI's schema.
- **Parity** (`crates/convert64-cli/tests/parity.rs`) enumerates `REGISTRY` and, for each command's example, compares the project saved by `convert64 apply`, by the MCP tool (`convert64_mcp::Server` in-process, a dev-dependency) and by the core in-process; through the MCP server it also checks that undo gives back the original file and redo the edited one. The core's tests undo and redo each new command and a whole session of them.

As built in #195 (2026-10-13): the app's theme is generated from the Figma variables ([figma.md](figma.md), [ADR-007](adr/ADR-007-light-dark-theme-and-material-icons.md)).

- **`cargo xtask tokens`** reads `design/tokens.json` and writes `crates/convert64-app/src/theme/generated.rs` (rustfmt-clean, "do not edit"): `apply_light` / `apply_dark` (`egui::Visuals`), `apply_spacing` (`egui::style::Spacing`), `text_styles()`, `fonts::*` (the bundled font files named by `egui / Text`), `Theme::{LIGHT, DARK}` (`Convert64 / Semantic`; aliases resolved in the same mode) and `layout::*`. Each variable's `rust` path decides where it goes, and every egui path is checked against egui 0.36's fields and value types (`Color32`, `f32`, `u8` blur, `CornerRadius::same`, `Margin::same`); an unknown path, a wrong type or a missing mode is an error. Colours keep Figma's unmultiplied alpha (`Color32::from_rgba_unmultiplied_const`). `--check` writes nothing and fails if the committed file is stale; CI runs it on Linux, and an xtask test does the same comparison everywhere.
- **`C64 / Palette`** is not generated. An xtask test compares its 16 colours with `Palette::colodore` at colodore.com's default sliders (`DEFAULT_BRIGHTNESS`, `DEFAULT_CONTRAST`, `DEFAULT_SATURATION`), exactly.
- **App.** `convert64_app::theme` (the library half, beside the scheduler). `theme::install(ctx)` sets the fonts (Inter Regular first in `Proportional`, JetBrains Mono Regular first in `Monospace`, Inter Medium as the named family `medium`; egui's fonts stay as fallbacks), then `egui_material_icons::initialize` (after `set_fonts`, which would drop it), then both styles (egui's default style for the theme with the generated visuals, spacing and text styles), and follows the system theme. `theme::set_theme(ctx, ThemePreference)` switches; the View → Theme menu and persisting the choice are later E8 work. `Theme::current(ctx)` gives the semantic colours; the central panel already uses `canvas_bg`.
- **Values.** The tokens are not quite egui 0.36's defaults, and the app now uses the tokens: body and button text 12.5 pt and monospace 12 pt (egui 0.36: 13, 13, 13), `faint_bg_color` opaque `#f2f2f2` / `#202020` (egui: additive luminance 5, which a Figma variable cannot express), `scroll.bar_width` 6 (egui: 10). Fix them in Figma if egui's defaults are wanted.
- **Dependencies.** `egui_material_icons` 0.8.0 (egui 0.36; listed above) with only its `outline` feature: the unfilled cut, about 1 MB. The crate ships Material Symbols *Rounded*, not the Outlined cut ADR-007 names, so the app's glyphs have rounded corners where the Figma SVGs are square. `serde_json` in the xtask. The three text fonts add about 1.1 MB (`crates/convert64-app/assets/fonts/README.md` has versions and hashes); with the icon font about 2.1 MB of the release wasm (6.9 MB, 3.0 MB gzipped, at #195).

As built in #203 (2026-10-13): the app shell, `convert64_app::shell` (UX §3, Figma `App Shell / Light` and `/ Dark`).

- **Layout.** Menu bar (File, Edit, View, Layer, Recipe, Export, Help); a `layout::TOOLBAR_HEIGHT` toolbar with the four tools, zoom and the Source/Result and compare buttons in three groups and the working-view hint on the right; a resizable right dock (`layout::DOCK_WIDTH`, 260–420 px) with the result header, the result (`layout::RESULT_HEIGHT`) and the Recipe, Layers and Export tabs; a `layout::STATUS_HEIGHT` status bar; the working view fills the rest. The dock follows Figma's tabs, not UX §3's stacked Recipe/Layers groups, so UX §3 now describes tabs and there is no narrow-window collapse to do.
- **State and actions.** `Shell` holds only chrome state (view mode, tool, dock tab, what is hidden, grid, zoom, theme choice); `Shell::show` draws everything and returns `Action`s the app carries out (`Quit`, `SetTheme`). Menu items of features still to come are disabled, not hidden (UX §11). Shortcuts are `shell::shortcuts` constants, shown in menus with `ctx.format_shortcut`; `Q` and the tool keys are ignored while a text field has focus; `Tab` is left to egui's focus traversal and `Cmd \` hides the chrome (#240). `convert64_app::icons` names every glyph (ADR-007).
- **Theme.** View › Theme (System, Light, Dark) is stored in eframe's app storage (a file natively, `localStorage` on the web): the workspace's `eframe` gains its `persistence` feature, which brings in `ron` (eframe's own storage format).
- **Screenshots.** `convert64-app --screenshot <file.png> [--theme light|dark] [--size <w>x<h>]` (native) draws a few frames, saves the window via `ViewportCommand::Screenshot` and quits, without touching the stored theme; PR screenshots come from it.

As built in #204 (2026-10-13): the result panel and the working view (UX §4).

- **Document and workspace.** `convert64_app::document::Document` wraps a `convert64_project::Session` (edits go through registry commands; a revision counter tells the views what changed) and builds the scheduler's `Snapshot`. `Workspace` adds the newest solution and two textures: the source composite (the session's supersampled canvas, `Session::canvas_rgba`, linear with mipmaps) and the result (target grid, `TextureOptions::NEAREST`, drawn 2:1 for multicolor). The working view and the result panel share the result texture. Both are rebuilt only when the document's revision or the solution changes. The session's compositor serves the Source view and the scheduler's serves the solves, so an edit composites twice (in wasm on one thread); a shared canvas is a later optimisation. Since ADR-012 the solve's grid is 1×, so the Source view gets its own compositor at `Session::PREVIEW_N` (4×) for a sharp zoomed-in view: an edit composites at 1× twice and at 4× once.
- **Camera.** `convert64_app::view`: positions in screen units (hi-res pixels, 320 × 200 in every mode), one centre shared by both modes so `Q` keeps the place; Source zoom fractional 25–800 %, Result zoom 1×, 2×, 3×, 4×, 6×, 8× with the picture's corner on whole points; fit, 100 %, step and zoom-about-cursor. Pan with `Space` + drag, middle drag or scroll; zoom with pinch or `Cmd` + wheel.
- **Compare and overlays.** Hold `\` flips to the other image; `,` cycles split (draggable divider), A/B flicker at 2 Hz and side by side (linked cursor). Every compare mode draws both images with the view mode's geometry, so they line up. Overlays: cell grid (`Cmd '`, from 6 pt cells up), clash heatmap (`Shift C`: cell error as `clash` alpha, colour-starved cells outlined), pins and locks, the other image as a 30 % ghost, and the border (`Cmd B`). The border is the solution's background colour (black in hi-res): the project has no border colour yet.
- **Result panel.** The result inside its border at most 1× (the Figma frame's 16/12 px inset), clash dots, the navigator rectangle in Result mode; a click centres the working view there and switches it to Result, a drag pans. The header has the mode dropdown (multicolor and hi-res bitmap, `mode_set` then a full solve), the clash count (a button to the worst clash) and the compare and fit buttons. The solve time is in the status bar, with the cursor's cell, pixel, colours, error and pin/lock state.
- **Opening (interim).** `convert64-app picture.png` opens a PNG natively; File › Open, saving and the browser's file handling are #205. `--screenshot` takes `--view result`, `--overlay grid|clashes|ghost` and `--compare split|flicker|side` for PR screenshots.
- **Check.** The app's result is pixel-identical to `convert64 convert` on the same PNG (portrait: 64 000 of 64 000 pixels), and a test solves a document through the scheduler.

As built in #198 (2026-10-13): rotation and an anchor in the layer transform ([ADR-011](adr/ADR-011-layer-transform-affine-subpixel.md), amended 2026-10-13).

- **`Transform { translate, scale, rotate, anchor, flip_h, flip_v }`.** It maps as `canvas = translate + c + R(rotate) · (S · F(p) − c)` with `c = S · (anchor ⊙ size)`. Positive `rotate` is clockwise on screen (y down). `translate` stays the unrotated box's top-left corner and the anchor (0–1 across the box, default the centre) is only the pivot, so every existing project means what it meant (ADR-011's amendment says why this differs from its Decision §1). `rotate` and `anchor` are written to files only when they are not the defaults, and unknown transform keys are refused. `Transform::new` is unchanged (no rotation, centred anchor); `with_rotation`, `with_anchor`, `reanchored` (move the pivot, keep the picture), `scaled_about_anchor`, `canvas_quad` and `is_axis_aligned` are new. `layer_to_canvas`, `canvas_to_layer` and `canvas_bounds` (the rotated quad's bounding box) handle the rotation. Sine and cosine come from an f64 series, as `sin_pi` does for Lanczos, so every OS gets the same composite; multiples of 90° are exact.
- **Two paths.** `rotate == 0` takes the separable path of E4, #98 and #139 unchanged: `cargo xtask regen-goldens --report` reports no change, and the axis-aligned benchmark rows run the same code. A rotated layer goes through `composite::affine`:
  - Each super pixel's centre is mapped to layer space and the premultiplied source sampled there with a **Catmull-Rom** 4 × 4 kernel, edges clamped, colour clamped at 0 and alpha to 0–1 as on the separable path.
  - **Prefilter.** A layer scaled down samples a box-filtered mip of its source, one level per layer and per layer axis (a rip-map), `round(log2(footprint))` where the footprint is the source pixels one super pixel spans along that axis. An odd last pixel pairs with itself. The colour's mip and the mask's are cached with the layer.
  - **Coverage** at the rotated edge is exact: the super pixel's footprint, a parallelogram in layer space, is clipped against the layer's box (Sutherland–Hodgman, f64) and its area taken. Footprints wholly inside or outside skip the clip.
  - **Masks** take the same map and kernel, then their modifiers (ADR-008). Like the separable path they carry no coverage. Under a feather blurred at half resolution (σ ≥ 4 target px) the separable path averages each 2 × 2 block of samples (#139); a rotated mask is sampled once per block, at its centre, which takes a quarter of the samples. On a hard edge at 4×, against the full-resolution reference, it was measured within 0.0052 at σ = 4 target px, 0.0034 at 6 and 0.0023 at 9.5. The separable path's half-resolution feather on the same layer, unrotated, was within 0.0020, 0.0015 and 0.0010. The test's bound is 0.006, and one 8-bit step from σ = 6.
  - **Rectangle and dirty region.** A rotated layer's raster rectangle is the bounding box of its quad on the grid, so `Compositor::update`'s union of old and new rectangles is ADR-011 §5's dirty region with no change to the update. `mask_change_cells` maps a mask edit's four corners and grows by the kernel's and the mip's reach.
- **`layer::Snap { Off, C64Pixel, Cell }`** and `Snap::apply(t, size, mode)` (also `snap_transform`). The function rounds `translate` and the far edges of the scaled, unrotated box to whole target pixels (multicolor: 2 canvas px across) or to cells (8 × 8 canvas px in both modes). It rounds `rotate` to 0.5° or 15° and keeps the anchor and flips. `Transform` never records a snap.
- **Commands, CLI, MCP and the project file** (on top of #193's registry):
  - `layer_set_transform` takes the whole transform, `rotate` and `anchor` included, and an optional `snap`.
  - `layer_rotate { layer, degrees, relative?, anchor?, snap? }` turns a layer to or by an angle about its anchor. Given an `anchor`, it first moves the pivot there, keeping the picture in place.
  - `snap_set { snap }` sets `Project::snap`. Its dirty set is an empty `Dirty::Cells`, because no pixel changes.
  - The two transform commands write `Snap::apply` of what they are given, under the project's snap or their own `snap` argument (the app's Cmd-drag). `layer_nudge` moves by exactly its offset, unsnapped.
  - All three undo through the existing mementos (`Prop::Transform`, `Setting::Snap`).
  - The project file has a top-level `snap`, read as `"off"` when absent and written only when not `"off"`. `snap`, `rotate` and `anchor` leave every existing file byte for byte as it was, and schema version 2 stays.
  - `convert` and `batch` take `--rotate DEG` and `--anchor X,Y`, which append a `layer_rotate` per layer to the project's commands.
  - The MCP server lists the new commands with the CLI's schemas. `parity.rs` runs their examples through the CLI, the MCP server and the core.
- **Tests.** `composite::tests` checks that an unrotated layer ignores its anchor bit for bit, and that a full turn equals no turn: exactly on a 1× hi-res grid, and within 0.006 per channel of the frame on a smooth source at 4× and in multicolor, where Catmull-Rom meets bilinear or Lanczos. A quarter turn equals the source's pixels turned and placed axis-aligned, again exactly at 1× (colour, evaluated mask with and without feather, attribute map) and within 0.006 at 4×. Other tests check that a rotated downscale keeps a flat field flat and its coverage sums to the quad's area, and that rotation edits in `update` report exactly the changed cells, inside the old and new rectangles, with the same result as a full recomposite. `mask_change_cells` gets rotated cases, one through a mip. `layer` tests cover clockwise turns about the anchor, round trips, re-anchoring, validation, defaults in files and the series against `f64::sin`. `layer::snap` checks that edges land on pixels and cells in both modes, and `composite::affine` checks the kernel, the mips and the coverage. `tests/rotate.rs` turns the `gradient` fixture by 0.25°, which changes 765 of 1000 cells' statistics in multicolor and 725 in hi-res. `command::tests` covers snapping by the project's setting and by argument, `layer_rotate` about a moved anchor, and the undo and redo of the three commands. `convert64-project` checks that `snap`, `rotate` and `anchor` are backward compatible in files, and `tests/convert.rs` checks `--rotate` and `--anchor`.
- **Benchmark.** `tests/bench.rs` repeats its four cases with every layer turned `3 + 7·i` degrees. Its nudge row there is a rotation drag: layer 4 turned a further 0.25° per update. Six runs on the macOS runner class (Apple M1 Virtual, 3 cores), 2026-10-13, in two batches of three started at once. The runners were slower than during #139: the axis-aligned rows, the same code as before, are up to 1.2× "After #139". Each figure is the median of the run medians, with their range in brackets, multicolor / hi-res:

  | case | `parallel` | single-threaded |
  |---|---|---|
  | cold composite | 20.2 [14.5–74.1] / 32.1 [25.1–120] ms | 32.0 [24.1–55.1] / 72.9 [45.6–77.6] ms |
  | cold composite, rotated | 46.6 [34.1–129] / 88.5 [67.0–161] ms | 132 [106–183] / 240 [201–330] ms |
  | warm recomposite | 6.05 [2.90–22.5] / 10.1 [6.37–22.5] ms | 7.02 [6.01–14.0] / 14.4 [11.7–21.5] ms |
  | warm recomposite, rotated | 4.10 [2.95–5.89] / 8.23 [5.91–9.46] ms | 6.55 [5.75–9.42] / 13.7 [9.57–17.0] ms |
  | nudge one layer | 6.55 [4.33–15.8] / 9.38 [7.47–24.8] ms | 8.02 [5.96–14.5] / 14.8 [12.7–24.4] ms |
  | rotation drag | 6.53 [5.39–14.9] / 12.8 [11.0–26.7] ms | 15.8 [13.4–24.1] / 36.3 [25.0–41.3] ms |
  | feather slider | 3.13 [2.17–8.54] / 6.18 [3.65–10.8] ms | 4.38 [3.71–7.30] / 10.0 [6.87–14.6] ms |
  | feather slider, rotated | 3.17 [2.14–9.03] / 5.85 [4.21–12.7] ms | 6.45 [5.11–9.20] / 13.3 [12.5–19.2] ms |

  On an Apple M-series host (local, one run each), single-threaded: cold 13.7 / 26.4 ms, rotated 51.9 / 103 ms; nudge 3.6 / 7.3 ms, rotation drag 7.1 / 14.6 ms.

  **The rotated path is over the #98/#139 budgets**:
  - Ten rotated layers composite cold in 47 / 89 ms with `parallel` (budget 60: hi-res over) and 132 / 240 ms single-threaded (budget 60). That is about 4× the axis-aligned path, single-threaded.
  - The rotation drag takes 6.5 / 12.8 ms with `parallel`, at the nudge budget of 10 / 12 ms (hi-res just over), and 15.8 / 36.3 ms single-threaded. The wasm build is single-threaded; §7's budget for a drag there is 50 ms.
  - The warm recomposite only re-blends, so it is unaffected.

  Where the time goes, hi-res on the M-series host, per full-screen layer: colour 7.2 ms, against about 1.5 ms for colour and mask together on the separable path. A full-resolution mask costs 5.8 ms more; under a feather of at least 4 target px it is sampled at half resolution (above), which brings it to about 1.5 ms. A Catmull-Rom tap is a gather that does not vectorise across pixels. Three changes were measured and did not help: restructuring the sum, a pixel-batched gather, and one bounds check per block. Fused multiply-add would differ between platforms, and wasm has none. Possible follow-ups, none done:
  - sample the colour and the mask in one pass, sharing coordinates and weights;
  - skip the mask where neither the quad nor the feather's reach can use it;
  - pre-upsample the source with the separable Catmull-Rom and sample that bilinearly.

As built in #216 (2026-10-13): region commands for the Region tool (UX §7), and commands for a layer mask's sources list (UX §6).

- **The registry** grows from 32 to 43 commands: `region_add` (name, optional recipe and position; an empty 320 × 200 mask), `region_remove`, `region_rename`, `region_reorder` (0 = applied first; later regions win where masks overlap), `region_set_recipe` (a `RecipeChange`, as `recipe_set`), and painting a region's canvas-space mask with the layer mask shapes: `region_stroke`, `region_fill` (polygon), `region_gradient`, `region_clear`. For the Mask group's Sources list: `mask_source_remove` and `mask_source_set_combine` (`max`, `min`, `multiply`). Each has a JSON form (`convert64_project::JsonCommand`), a schema and an example in `convert64 commands --json`, runs through `convert64 apply` and as an MCP tool, and is covered by the parity test, whose base project now has a mask source on layer 1 and two regions. Regions are named by index or name (`RegionRef`, as `LayerRef`; `Session::resolve_region`).
- **Shared code.** The region paint commands run the layer mask's brush and lasso (`mask::paint`) with the same argument checks on the region's raster; the undo tiles moved from `Mask` to `MaskRaster` so both use them.
- **Undo.** Paint keeps the touched tiles (stroke, fill) or the tiles of the rectangle where values changed (gradient, clear); a removed region is kept whole; rename and recipe keep the old value. The source list commands keep the whole mask, as `mask_gradient` does.
- **`Dirty::CanvasRect`** (canvas pixels): paint reports the touched rectangle, gradient and clear the rectangle that changed, add, remove, reorder and recipe the region's non-zero extent (empty for a new region and for a rename); undo reports what the command did. `composite::canvas_change_cells` grows it by the one canvas pixel that bilinear sampling reaches and maps it to target cells, so the app re-solves only those; a test checks that it covers every cell `Compositor::update` reports, undo included. The source list commands report `Dirty::Layer`.
- **`region_gradient` is baked.** A region's mask is a raster with no procedural sources (a layer mask has them, ADR-008), so the gradient is combined into the raster at pixel centres with its combine mode and cannot be edited afterwards. Keeping regions raster-only leaves the compositor and the project file as they were.
- **The project file** is unchanged, schema version 2: `regions` already holds the name, the mask raster and the recipe. A test saves regions made by commands and reloads them (masks within 1/65535 after the first save, as layer masks) and checks that load → save is byte-identical.
- **Unchanged:** the solver and the compositor's output (no golden or parity change).

As built in #205 (2026-10-13): open, save and export in the app.

- **Open.** File › Open… (`Cmd O`) takes a PNG or a `.convert64` project; a picture dropped on the window opens too, or, on an open project, becomes a new layer on top (`layer_add`, fitted inside the screen). Natively `rfd` shows the system dialog (its default `xdg-portal` backend on Linux, so no GTK) and the app links files in place: `convert64-project`'s `fs` feature is on for the native build only. In the browser `rfd` shows the file picker and the bytes go into the session's `MemoryStore`, so a project opened there must embed its sources.
- **Save.** `Cmd S` writes the project back to its file (natively; links relative to it, as the CLI's `--save-project`), `Shift Cmd S` asks where. In the browser both download the project with every source embedded. The title shows `*` while there are unsaved edits (UX §9).
- **Export.** `convert64_app::export::files` makes the E6 outputs from the result on screen with the same `convert64-formats` calls as the CLI: Koala or Art Studio, the self-displaying PRG, the PNG preview (zoom 1–8), the raw set and source for KickAssembler, ACME, 64tass, ca65 and C (inline bytes or includes). The dialog (`Cmd E`, or the dock's Export tab) remembers its settings for Export Again (`Shift Cmd E`). One file gets a Save dialog natively, several a folder; in the browser each is a download (`rfd`'s wasm backend). Export waits for the full solve. A toast reports `Exported title.kla (10 003 B)`; failures name cause and remedy.
- **Dependency.** `rfd` 0.17 (TAD §4's file dialogs), default features.

As built in #206 (2026-10-13): the Layers tab (UX §5).

- **Rows.** Top layer first: a drag handle (`egui_dnd` 0.17, egui 0.36; reordering ends in one `layer_reorder`), the eye (`layer_set_visible`), a thumbnail (box-filtered to 64 px from the layer's source, cached per source), a chain icon when the layer has a mask, the name (click selects, double click renames with `layer_rename`), the recipe badge (`FS 100%`; the tooltip has method, metric and fitting mode) and the opacity. Blend mode (`layer_set_blend`) and opacity (`layer_set_opacity`) of the selected layer sit above the list; the strip below has New from file, Duplicate, Layer via mask and Add mask (disabled until the mask tools, #78) and Delete. With no layers the tab shows UX §5's empty state.
- **Undo.** `Cmd Z`, `Shift Cmd Z` and `Cmd Y` run the editor's history. A drag (the opacity slider now, nudges and transforms later) is one undo step: `Document::run_in_gesture` undoes the gesture's previous command before running the next, and the drag's frames take the scheduler's fast path, its release a full solve.
- **Shortcuts and menus.** `Shift Cmd N` new layer from file, `Cmd J` duplicate, `Cmd ,` show or hide, `Delete`/`Backspace` delete; Layer and Edit menus match. A picture dropped on an open project becomes a layer (#205).
- **Screenshot runs** neither restore nor save egui's memory (panel sizes), so they always draw the defaults; extra pictures on the command line are added as layers, and `--tab layers|export` picks the dock tab.
- **Dependency.** `egui_dnd` 0.17.0 (MIT; brings `egui_animation` and `simple-easing`), approved for E8.

As built in #207 (2026-10-13): the Recipe tab (UX §6), for the selected layer.

- **Adjustments** (`adjustments_set`): brightness, contrast, saturation, lightness (−100…100), hue (±180°), png2c64's gamma (0.1–8), levels (black, mid-tones, white), curves (master, red, green, blue: drag a point, click to add, right-click to remove; the end points keep x = 0 and 1) and range matching onto the palette (png2c64's `--match-range`).
- **Dither recipe** (`recipe_set`): the method by family (None, Ordered, Diffusion, Noise, Effects, Palette-aware), then a dropdown within it; strength; pattern phase; orientation; the error clamp for diffusion.
- **Colour fitting** (`recipe_set`): metric, Adaptive or Force with UX's one-line help, and the 16 allowed colours as toggles (a disallowed colour is hatched; `allow`/`deny` change one colour).
- **Advanced**, collapsed (`solve_set`): ADR-010's divergences from png2c64 — chroma guard, cell clamp, bridging, max luma diff, mix levels, pair bias — all off by default, then the solver (reference or native), refinements and finalists; with the native solver also its luma-pair bias and the layer's smooth and adaptive settings.
- **Conventions.** Label, slider and value field on one row; a dot before a changed label; a double click on a label resets that value; each group has Reset and its title takes the accent colour when anything in it changed. A drag is one undo step and takes the fast path until release (#206's gestures). The Transform group is #197 and the Mask group #78.

As built in #208 (2026-10-13): the steering tools (UX §7).

- **Pin** (`P`): a click pins the cell's current colour set (`cell_pin`, `full`); a drag pins a rectangle, each cell to its own set, as one undo step (`Document::run_all`); `Alt`-click pins only the colour under the cursor. **Lock** (`L`): click or rectangle `cell_lock`; on cells that are all locked it unlocks. **Nudge** (`V`): a click selects the topmost visible layer under the cursor, a drag moves it sub-pixel (`layer_nudge` as one gesture: fast path while dragging, full solve on release); the arrow keys nudge the selected layer 1 px, 10 px with `Shift`, 0.25 px with `Alt`.
- **Feedback.** Pinned and locked cells are outlined in the `pin` and `lock` colours with the glyph at the top right once a cell is 16 pt; a drag shows its rectangle; a toast says "Pinned 7 cells".
- **Right-click** a cell: Pin colours, Lock/Unlock, Clear pin/lock (`cell_clear`), Jump to worst neighbour, Copy cell colours (hex). `Shift Cmd ]` / `[` walk the clash report's worst-first order.
- **Undo steps.** `Document` keeps its own undo steps over the editor's history, so a group of commands (a rectangle of pins) undoes and redoes as one.
- **Not here.** The Region tool (`R`) paints a mask carrying its own recipe: the registry has no region commands yet, so it arrives with the mask tools (#78), which add them with CLI and MCP parity.
- **Screenshots** take `--command <json>` (registry commands run after opening), `--zoom` and `--centre`.

As built in #78 (2026-10-13): the mask tools (UX §7a, ADR-008).

- **Mask mode** (`M`, Edit mask in the Recipe tab's Mask group, or a click on a layer's mask thumbnail; `Esc` or `M` leaves) swaps the toolbar to Brush `B`, Lasso `W`, Gradient `G` and Key `K` and shows the selected layer's evaluated mask (`Session::evaluated_mask`, modifiers applied) as a red quick-mask overlay at 50 % where the mask hides the layer; `Alt`-click on the thumbnail, or Alone in the options bar, shows the mask in grey.
- **Tools.** Brush: one `mask_stroke` per drag (or a dab per click), radius in C64 pixels converted to layer pixels, hardness and flow, `X` swaps Add/Subtract, `Alt` subtracts while held, `[`/`]` size. Lasso: a freehand drag or vertices clicked and closed with a double click or `Enter`, one `mask_fill`. Gradient: drag from→to, `Shift` to 45°, `mask_gradient` with linear/radial, softness and combine. Key: a click samples the layer's pixel, a drag widens the range, `mask_key` with luma/chroma/colour. Points go from screen to layer pixels through `Transform::canvas_to_layer`.
- **Options bar** under the toolbar: the tool's options, then Snap to cells, Feather (0–32 px, one gesture per drag), Invert, Clear, Fill, Load PNG… (`mask_load`: linked natively, through the session's store in the browser), From alpha, Alone.
- **Selection.** With mask mode off the same tools (except the brush) build a `Selection` (lasso polygons, gradients, keys); Layer via mask (`Cmd J`, the strip's button) and via cut (`Shift Cmd J`) run `layer_via_mask`; Deselect drops it.
- **Panels.** The Layers rows show the mask thumbnail (raster with invert) beside the chain icon; the strip's Add mask runs `mask_from_alpha` when the picture has transparency, else `mask_clear` to 1. The Recipe tab's Mask group has Edit mask, feather, invert, snap and levels (Reset is one undo step), and lists the sources.
- **Sources list.** Each gradient or key has its combine mode (`mask_source_set_combine`) and a delete (`mask_source_remove`), both from #216.
- **Not yet.** "Re-solved N cells" and the feathered-edge clash hint wait for the scheduler to report re-solved cells.

As built in #209 (2026-10-13): palette, shortcuts, states and accessibility basics (UX §8, §11, §12).

- **Picture group** at the top of the Recipe tab (Figma `Recipe tab`; since #288 the result's picture row): mode, palette (Pepto PAL, Pepto NTSC, Colodore, or a VICE `.vpl` file: `palette_set`, linked natively, through the session's store in the browser) and the background (`background_pin`: Auto or one of the 16 swatches; the solver's pick is ringed in the text colour, a pinned one thicker in the accent, so not by colour alone). A palette change re-solves in full.
- **Shortcuts.** Every UX §8 shortcut outside the Transform tool (#197) is wired and shown in the menus; single keys are ignored while a text field has focus. New here: `H` (hand: a primary drag pans) and bare `1` (100 %). A table-driven test checks that each shortcut asks for its action.
- **States** (§11): the empty state (#203); a full solve running over 200 ms shows "Solving full search… n s" with **Pause** in the status bar and fades the result slightly; Pause cancels the solve and **Resume** restarts it. When clashes exist and the overlay is off, the status bar suggests "Show clashes (Shift C)" once, dismissible. File errors (open, mask, palette, save) sit inline in the status bar in the error colour; a click shows the details and Dismiss — no modal. Export success is a toast (#205).
- **Accessibility** (§12): the working view and the result panel carry `WidgetInfo` names for screen readers; the tab order is egui's; pins, locks, clashes and the chosen background are marked by glyphs, outlines, rings and text as well as colour. Glyphs missing from Inter (`✕`, `⇄`) were replaced by the icon font or words.

As built in #197 (2026-10-13): position, scale and rotation in the app (UX §6 item 1, §7, ADR-011), on #198's transform.

- **Transform group** first in the Recipe tab: X and Y (C64 pixels, 0.01 steps), W and H % chained by default (`scaled_about_anchor`, so the pivot stays put), rotation (±180°, 0.01°), a 3 × 3 anchor picker (`reanchored`: the picture stays, the pivot moves), Snap, flips, and presets (fit width, fit height, fit screen, fill screen, 1 and 2 source pixels per C64 pixel, 100 %; since #289 in screen pixels with Fit · Fill · 1:1 · 2:1). Every field is `layer_set_transform`; a drag is one undo step.
- **Transform tool** (`S`, `Cmd T`): the selected layer's rotated box (`canvas_quad`) with four corner and four edge handles and the anchor. Inside moves, a corner scales uniformly (`Shift`: free aspect), an edge scales one axis, just outside a corner rotates about the anchor (`Shift`: 15° steps), the anchor drags the pivot, the wheel over the box scales ±1 %. Holding `Cmd` inverts the project's Snap for that drag (`snap` on the command). `Esc` during a drag reverts it (`Document::cancel_gesture`). The status bar reads X, Y, W %, H % and the angle.
- **Snap** (`snap_set`) is in the toolbar beside the tools and in the Transform group; `Shift Cmd ;` cycles Off → C64 pixel → Cell.
- **Keys.** `Cmd Alt [` / `]` rotate ∓0.1° (`Shift`: 1°, `layer_rotate` relative); `Cmd [` / `]` scale ∓1 %. UX §8 gives `Shift Cmd [` / `]` both to ±10 % scale and to worst-cell navigation: with the Transform tool they scale, otherwise they walk the worst cells.

As built for the Region tool (2026-10-13; #208's `R`, on #216's commands): a drag paints the selected region's canvas-space mask with the brush (`region_stroke`; `Alt` or Subtract erases; size, hardness and flow in its options bar), and the first stroke with no region makes one (`region_add` and the stroke as one undo step). While the tool is active the regions show as a tint in the `region` colour and the Recipe tab becomes the region panel: the list (select, add, remove) and the selected region's dither recipe and colour fitting (`region_set_recipe`), UX §7's "pseudo-layer". Toasts are one line.

As built in #210 (2026-10-13): solve scheduling in the app and the E8 exit.

- **Scheduling.** Drags (sliders, nudges, transforms, opacity) submit the scheduler's fast path with the previous solution, their release a full solve; nothing waits for a solve natively (worker thread) or in the browser (a Web Worker, #265). A full search past 200 ms fades the result slightly in the working view and the result panel, shows "Solving…" there and "Solving full search… n s" with Pause in the status bar (#209).
- **Exit.** The *No Glory* workflow — two layered sources, a mask, a pin, export, VICE — ran natively and in the browser in both themes; `docs/screenshots/210/README.md` has the steps, screenshots and the VICE check (64 000 / 64 000 pixels against the exported Koala in both). Native screenshots come from `convert64-app --screenshot` with `--command` and `--export` (`native-workflow.sh`); the browser run drove the release bundle in Chromium through Playwright.
- **Found on the way.** In the browser a second `source.png` replaced the first in the session's store (fixed in #205); a slider value merely rounded for display re-sent itself every frame and kept the solver busy (fixed in #207).

As built in #228 (2026-10-13): the Sharpen adjustment per layer (UX §6).

- **What.** `Adjustments::sharpen`, a `Sharpen { amount, radius, threshold }`: an unsharp mask on OKLab L only (a and b are kept, so sharpening adds no colour clashes). `amount` −1…3 is png2c64's `--sharpen`: positive adds `amount × (L − blur)`, negative blends by `−amount` (at most 1) toward the blur; `|amount| < 0.01` is off, as in png2c64. `radius` is `Radius::Png2c64`, png2c64's 3 × 3 box (three passes of it when softening), or `Radius::Gaussian(σ)`, σ 0.3–3 target pixels, separable and cut at 3σ. `threshold` (0–0.1 in L) leaves a pixel alone where `|L − blur|` is below it, when sharpening; png2c64 has none. Serde default off, written only when not the default, so older projects load unchanged.
- **Where.** In the compositor's adjustment step after the resample (`adjust_planes`, ADR-012), at png2c64's position: gamma (`preprocess::apply_gamma`) → sharpen → png2c64's OKLab round trip and clamp → levels → brightness/contrast → HSL → curves → range matching. The radius is in target pixels (fat pixels in multicolor, like feather σ): on the default 1× grid png2c64's taps are one pixel apart, exactly png2c64; on an `n ×` grid (`--supersample`) they are `n` super pixels apart, which box-filters to png2c64's 3 × 3 at the target grid, and a Gaussian's σ is `n × σ`. `Adjustments::apply_image` (used by `AdjustCache`) runs the same order without the round trip; there gamma gets its own lookup table when a sharpen follows it, and amount 0 leaves the plan as before #228.
- **One kernel.** `solve::png2c64_ref::preprocess::unsharp_box` is the body of png2c64's `apply_sharpen`, shared: png2c64's preprocessing calls it with every weight 1 and a step of 1, the layer adjustment (`Sharpen::apply_rgb`) with alpha as the weight and the grid's step; with alpha 1 and step 1 the two are bit-identical (`png2c64_kernel_equals_png2c64_s_apply_sharpen`). Each pixel the kernel changes goes through png2c64's OKLab round trip and clamp, as in png2c64; a pixel the threshold skips is left as it was.
- **Alpha.** The blur is weighted by alpha (the premultiplied colour's lightness over its alpha), so a transparent surround pulls no halo into a layer's edge, and pixels of alpha 0 are neither used nor changed.
- **Dirty region.** The sharpen runs over the layer's resampled raster and never spreads outside its rectangle (alpha 0 stays 0), and any change to the sharpen, the source or the transform rebuilds the whole raster, whose rectangle the compositor already recomposites in whole cells. So the cells an edit dirties are the layer's footprint, not grown by the kernel radius; `sharpen_updates_equal_a_full_recomposite` checks the reported cells are exactly the changed ones and inside the footprint.
- **png2c64's front end.** A plain picture whose only non-gamma adjustment is a png2c64-radius sharpen with threshold 0 (range matching aside) keeps png2c64's front end, the amount passed to `prepare` as `preprocess::Settings::sharpen`; the general path gives the same frame bit for bit (`the_general_path_is_png2c64_s_front_end_for_a_plain_layer` covers sharpen ±0.5). A Gaussian radius or a threshold, which png2c64 does not have, takes the general path only.
- **Front-ends.** `adjustments_set` takes `sharpen` (`AdjustmentsChange::sharpen`; the MCP tool has the same schema); the CLI has `--sharpen <amount>`, `--sharpen-radius <px|png2c64>` and `--sharpen-threshold <L>`, also as batch keys. The app's Adjustments group has a Sharpen block: Amount −100…300 %, Radius (*png2c64 3×3* or *Gaussian* with a σ drag value 0.3–3.0 px), Threshold 0–10 %, each label resetting on a double click, with the slider guard against values merely rounded for display (#207, #210). The Source view shows the composite, so it shows the sharpened frame.
- **Parity.** png2c64's reference build rendered the portrait at its baseline settings with `--sharpen 0.5` and `--sharpen -0.5` (`fixtures/portrait/baseline_sharpen_0.5.png`, `baseline_sharpen_-0.5.png`, 95.7 % and 94.9 % of pixels the same as the unsharpened baseline); `convert64 convert … --sharpen ±0.5` reproduces both at 100 % (bar 99.9 %, CI's `png2c64 parity` step).
- **Sheet.** `cargo xtask sharpen-sheet` writes `docs/reference/dither-previews/convert64-sheets/sheet-sharpen.png`: per fixture, amount 0, 0.5 and 1.0 with png2c64's kernel and 1.0 with a Gaussian of σ 0.8, prepared above solved, with the share of each solve equal to the unsharpened one. Defaults stay off; the goldens are unchanged.

As built in #273 (2026-10-18): png2amiga's C64 path as a comparison solver; png2c64 stays the reference ([ADR-014](adr/ADR-014-png2c64-stays-reference.md), reversing [ADR-013](adr/ADR-013-png2amiga-c64-reference.md)).

- **What users see.** Nothing changes by default: the goldens, previews and formats snapshots are main's, and png2c64's parity is the CI gate again. New opt-ins: `--solver png2amiga`, `resample = lanczos` per layer, `black_point`, `--allowed`, the line-art recipe; experimental and png2amiga-solver-only: Refine, the Background rule; experimental on every solver: the hull range match and its Amount.
- **The port.** `solve::png2amiga_ref` ports png2amiga's C64 bitmap path at `4a87776` (v1.111.2): its widened Lanczos-3 (`scale`), preprocessing and per-(L, hue) range matching (`preprocess`), OKLab with the fast cube root and explicit FMAs (`colour`), all 68 dither methods its C64 path accepts (`dither`), the multicolor and hi-res encoders (`c64`) and `--cell-refine` (`refine`). `solve::reference_png2amiga` runs it on a frame with the same hooks as the reference (pins, locks, allow/deny, per-pixel recipes, a fixed background, the fast path); the cell metric is one per frame.
- **Solver kinds.** `SolverKind::Reference` (png2c64's port; serde and the CLI also take `png2c64`), `Png2amiga`, `Native`. Fallbacks: the reference → native, as before; png2amiga → the reference → native. `SolverKind::pipeline()` names the compositor's front end for a solver.
- **Compositor.** `composite::Pipeline` (§2); `Kernel::{Mitchell, Lanczos}` in `composite::resample`, chosen per layer by `Pipeline::colour_kernel`; masks are always Mitchell. The raster cache keys on the layer's `resample` and the pipeline. ADR-012's invariant test and `a_fitted_layer_is_png2c64_s_bicubic_bit_for_bit` hold as on main; `a_fitted_layer_is_png2amiga_s_resample_bit_for_bit` holds for the Lanczos opt-in.
- **New settings,** each a registry command or field and in the CLI, MCP and app: `layer_set_resample` / `resample` (project layer field written only when `lanczos`) / `--resample` / the Transform group's *Resample: Mitchell · Lanczos* (next to *Downscale: Auto · Exact*, renamed from "png2c64 exact"); `adjustments.black_point` / `--black-point` / Adjustments' *Black point*; `--allowed` (every layer's recipe `allowed`). Range matching: *Method: Box · Hull (experimental)* in the Range row, the hull's Amount under it; `--match-range` is png2c64's box, `--match-range-method hull` png2amiga's.
- **png2amiga-only, off by default:** `solve.refine` / `--cell-refine` (needs `--solver png2amiga`; the app neither offers it nor runs it in its full solve); `solve.background_rule` / `--bg-rule` (needs `--solver png2amiga`; the app shows it only with that solver). The reference keeps png2c64's 16-way background search and its box.
- **Fixtures.** png2c64's baselines are the gated ones again (`fixtures/<name>/`); png2amiga's, with their commands, are in `fixtures/png2amiga/<name>/` for its port's parity tests. New goldens only: the line-art recipe and png2amiga's 32 methods on the gradient.
- **Sheets.** `cargo xtask cause-sheet` (the swaps), `cargo xtask lineart-sheet` (the line-art recipe against tile 13, and on a synthetic coloured logo), `cargo xtask metric-check`; `cargo xtask reference-sheet` renders #261's review sheets on the png2amiga solver.

As built in #261 (2026-10-14): SSIMULACRA2 in `convert64-core::quality`, information only since ADR-014.

- **What.** `quality::ssimulacra2(reference, distorted, width, height) -> f64` is a port of png2amiga's `src/ssimulacra2.cpp` at the pin, which ports Cloudinary's SSIMULACRA2 (BSD-3, libjxl's XYB; [THIRD_PARTY.md](../THIRD_PARTY.md)). A port, not a crate: no TAD §4 dependency. Pure Rust, the same code in wasm; bit-exact against png2amiga's scalar build (`tests/ssimulacra2.rs`).
- **The pairing.** `quality::score_bitmap(frame, image, palette)` scores a solve as png2amiga scores its C64 output: the prepared frame against the render at the mode's logical raster, in linear light. `Session::ssimulacra2()` scores the last solve.
- **Front ends.** `convert`, `export`, `recipe apply --report` and `validate` on a project report `ssimulacra2` in `--json` (timed as the `ssimulacra2` phase), and the MCP `solve` tool returns it. Not in the human summary line or the app (ADR-014 §7).

As built in #241 (2026-10-18): autosave and restore, the unsaved-changes guard and saving straight to disk in the browser (usability finding C3; layout in [reference/autosave.md](reference/autosave.md)).

- **Storage layout.** `convert64_app::autosave`: a flat set of entries behind a callback-based `Storage` trait (`load`, `read`, `apply(Batch)`), so the browser's asynchronous stores and the native and in-memory ones look alike to the logic. `session-<id>.json` is a record: the project file as JSON with every layer's own source linked as `blobs/<key>` (`Session::to_recovery_file`, new in `convert64-project`; a `.vpl` palette is written as its colours), its content key, the key at the last Save, the blob keys and (natively) the files sources were linked from. `blob-<key>` is a source's bytes, stored once by content (a 128-bit FNV-1a + multiply–xorshift key; the app caches each source's key by pointer, so a source is hashed once per session). Natively the entries are files in `eframe::storage_dir("Convert64")/autosave/` (temporary name, then rename); in the browser OPFS (`navigator.storage.getDirectory()`, writable streams commit on close), or IndexedDB (one transaction per batch) where OPFS has no `createWritable` or is refused. The last session and one previous are kept (plus the one the restore bar offers); blobs no kept record links are deleted after each write.
- **Debounce.** A change counts once committed: one command, or a drag on its release (`Document::in_gesture` while the pointer or an arrow key is held). The save runs 1 s after the last committed change, at most 5 s after the first unsaved one; one batch at a time, a change during a write saving again after it. Blobs are written before the record that links them.
- **Quota.** A failed batch (`QuotaExceededError`, `StorageFull`, anything else) leaves one line in the status bar in the warning colour and work goes on; nothing retries until the next committed change, and a successful write clears it.
- **Restore.** On start the newest record whose project key differs from its saved key (undoing back to the saved state counts as saved) is offered in a non-modal bar under the toolbar; Restore opens it with `Document::restored` (dirty, same session id), natively relinking a source whose file still holds the same bytes and embedding the rest; Discard deletes the record and its blobs.
- **Unsaved changes.** The title's `*` (UX §9) now reaches the browser tab: eframe's web backend ignores `ViewportCommand::Title`, so the app sets `document.title`. A `beforeunload` listener asks while there are changes since the last Save. Natively a close request with unsaved changes is cancelled (`ViewportCommand::CancelClose`) and an `egui::Modal` asks Save · Don't save · Cancel; Don't save deletes the session's record.
- **Saving in the browser.** `convert64_app::web::save_files` replaces rfd's wasm save (its overlay with a "click here to download" link): `showSaveFilePicker` for one file and `showDirectoryPicker` for several where the browser has them (asked for in the frame that handles the key press or click; eframe runs a click's frame inside the click), else, or if the picker is refused, a `Blob` behind an `<a download>` clicked at once. Export goes the same way. `FileEvent::Saved` (and so the toast, now in human units: "Saved title.convert64 (1.3 MB)", and the cleared `*`, for the revision that was saved) comes only after the write, or the hand-over to the browser's download, has happened. rfd still opens files.
- **Browser API.** Everything is reached by name through `js_sys::Reflect` and `Function` calls on eframe's re-exports, as #265 does: OPFS, IndexedDB, the pickers, `Blob`, `URL.createObjectURL`, `document.title`, `beforeunload`. No new crates or `web-sys` features; `serde` (already in this list) became a direct dependency of the app for the record.
- **Tests.** `autosave::tests` against `MemoryStorage` (with a quota) and a temporary `DirStorage`: one save per drag on release, bursts capped at 5 s, sources stored once, last + previous kept and blobs collected, a full storage warning once without retrying, restore detection (unsaved, saved, undone back to saved), restore round trips (memory and the native folder, relinked or embedded), Discard, quitting without saving, the offer surviving other sessions; the title marker; human sizes; `Session::to_recovery_file` in `convert64-project/tests/memory.rs`. `tests/web/restore-smoke.mjs` drives the release bundle in headless Chrome over the DevTools protocol (Node 22, no packages): nudge, autosave, reload (the guard asks), Restore, the nudge still there, Save as a real download.

As built in #287 (2026-10-24): the export set and the Export panel (UX §6b; usability review M4).

- **Model.** `convert64_core::export`: `ExportEntry { format, name?, zoom, data }` and `ExportFormat` (`picture`, `prg`, `raw`, `kickass`, `acme`, `64tass`, `ca65`, `c`, `png`) in three `ExportGroup`s (C64 files, Source code, Preview). The project holds the rows as `Project::exports`, in order. Pure data: the core still does no I/O.
- **Command.** `export_set { exports }` replaces the whole list and checks each row (zoom 1–8; a name that is not empty, a path or hidden). Like `snap_set` it is a document setting (`Setting::Exports`), so it undoes, and its dirty set is an empty `Dirty::Cells`.
- **Project file.** A top-level `exports`, written only when not empty, with each row writing only what differs from its defaults; schema version 3 stays ([project-file.md](reference/project-file.md#export-set)).
- **Writers.** `convert64_formats::export::files` makes one row's files from a solved `BitmapImage`, with the calls `convert` and `export --asm` already made. `set_files` makes the whole set, writes a file two rows share once, and refuses two rows writing different bytes to one name. Three front-ends call them:
  - the app's Export panel and `Shift Cmd E`;
  - `convert64 export` without `--asm` (`convert64_cli::export_set`, the CLI library);
  - the MCP server's `export_run { dir, name? }`, through that library.
- **App.** The Export tab is the panel (`shell::export_panel`), and the export dialog is gone. Every row edit is an `export_set` action. Each row's files and sizes are computed once per solution and set.
  - Natively, Export writes into a folder picked with `rfd`, and Export Again writes into the same folder.
  - In the browser it goes through #241's path (`web::export_files`): `showDirectoryPicker` (its handle kept for Export Again while the page is open), else one `<a download>` per file.
  - `Cmd E` shows the panel.
- **Dialogs.** `Shell::dialog_open` is true while the quit question is up or any egui modal was drawn last frame (`Memory::top_modal_layer`). The shell's shortcuts then return at once, so `Esc` reaches the modal's own close and nothing acts behind it.
- **Tests.**
  - Core: ids and groups, row checks, `export_set` with undo.
  - Formats: every format in both modes, a set's shared and clashing files.
  - Project: `exports` optional and round-tripping, unknown options refused.
  - CLI: `tests/export.rs`, the export set from a project, against `convert`'s Koala.
  - MCP: `export_set` and `export_run`.
  - App: the panel's rows, + and Export; `Cmd E` and `Shift Cmd E`; a dialog blocking `Q`, `Shift C` and the rest until `Esc` closes it.
  - `parity.rs` runs `export_set`'s example through the CLI and the MCP server.

As built in #288 (2026-10-24): the Recipe tab in the order of use and the picture-wide settings with the result (usability review M5, UX §4.3, §6).

- **Picture row.** Mode, Palette and Background moved from the top of the Recipe tab (#209's picture group) to a row under the result header (`shell::dock`): the mode dropdown (the only one now), the palette dropdown and a compact background dropdown (a 12 px swatch with `Auto` or the pinned colour's name; inside, Auto and the 16 swatches, open until a click outside). The commands are unchanged: `mode_set`, `palette_set`, `background_pin`.
- **Recipe tab.** A layer header (32 px thumbnail, name, `Layer 1 of 2 · w × h px`), then the groups in `recipe::GROUP_ORDER`: Dither recipe, Colour fitting, Adjustments, Sharpen, Transform, Mask, Advanced; a region's recipe keeps the same order. Which groups are open is `shell::RecipeGroups`: Transform, Mask and Advanced start collapsed, a click on a heading opens or closes the group for every layer and region, and the user's choices are kept in eframe's storage under `recipe_groups` (JSON), not in the project. Screenshot runs start from the defaults and take `--open-group` / `--close-group <title>`.

As built in #289 (2026-10-24): the Transform group's units and controls (usability review M6, UX §6 item 1).

- **Units.** X, Y and W × H are screen pixels, the 320 × 200 grid every mode is composited on (ADR-011's C64 target pixels; a multicolor pixel is 2 wide). Under W × H the scale as a % of the source and, in multicolor, the width in multicolor pixels; the Transform tool's status-bar readout uses the same units. The chain keeps H / W as it is. Conversions and presets are `convert64_app::presets`.
- **Presets** are four buttons, the one the transform is at selected: Fit (uniform, `min(320 / w, 200 / h)`, centred), Fill (uniform, `max(…)`, centred), 1:1 (scale 1 × 1) and 2:1 (scale 2 × 1, one source pixel per multicolor pixel); the pixel presets are centred and then moved left and up onto whole C64 pixels of the mode (an even x in multicolor). Rotation, anchor and flips are kept. Every preset is `layer_set_transform`.
- **Controls.** The anchor is a 3 × 3 control of 16 px cells; the flips are two icon toggles (Material `flip`, turned a quarter for vertical).
- **New pictures.** A picture opened in the app starts with Fill (`document::picture_file`), not `ProjectFile::from_png`'s stretch to 320 × 200, which `convert64 convert` keeps as png2c64's default. For a 16:10 picture the two are the same transform, so it keeps png2c64's front end (`composite::plain_layer`); any other aspect is uniform, covers the screen and goes through the composite. A layer added on top is fitted inside and never enlarged, as before. Projects store their transforms: opening one, a version-2 file migrated by #251 included, is unchanged.

As built in #290 (2026-10-25): the Region flow (usability review M7, UX §7).

- **Per-tool options.** `shell::MaskState` holds a `BrushOptions` (mode, size, hardness, flow) for the mask Brush and another for the Region brush, and the Lasso's own `PaintMode`; `X`, `[` and `]` go through `Shell::paint_mode_mut` / `brush_mut`, which pick the current tool's. Nothing is shared between tools any more.
- **First stroke.** `mask::region_interact` sends `region_stroke` with `add` when the target region is new (with `region_add` in the same step) or empty (`mask::region_is_empty`: no value above 0), and resets the Region brush to Add. The commands are unchanged.
- **Outline.** `view::mask_outline` turns a region's 320 × 200 mask into the pixel-edge segments where it crosses 50 %, runs joined; `Workspace::region_overlay` makes them with the tint, per revision, and the working view draws them in the theme's `region` colour, with the stroke in progress as a swath.
- **Breadcrumb.** `recipe::breadcrumb` heads the Region tool's Recipe tab: `Region 1 ‹ <layer>`, the layer's name a link that picks the Nudge tool, so the tab shows that layer's recipe again.

### 4.1 Image model (`convert64-core::model`)

The public data model every front-end, solver, reader and writer shares. Constructors enforce sizes and the 0–15 colour range; everything else is the validator's job (§1.3). Deserialising goes through the same constructors.

| Type | What it is |
|---|---|
| [`ColourIndex`](../crates/convert64-core/src/colour.rs) | One of the 16 VIC-II colours, 0–15; used wherever a colour is stored. `ColourSet` is a 16-bit set of them. |
| [`IndexedBitmap`](../crates/convert64-core/src/model/indexed.rs) | Pixel level: a multicolor (160×200, plus background) or hi-res (320×200) picture, one colour per logical pixel. The solver's output and the preview's input. Only this layer can break rule 1, because the encoded formats cannot express more colours per cell than the hardware shows. |
| [`BitmapImage`](../crates/convert64-core/src/model/bitmap.rs) | Encoded bitmap: 8000 bitmap bytes, 1000 `ScreenCell`s, 1000 Colour RAM colours and the background (multicolor). Decodes to `IndexedBitmap`. |
| [`CharsetImage`](../crates/convert64-core/src/model/charset.rs) | Encoded charset picture: chars (8 bytes each), a `width × height` map of char codes of any size, one `CharColour` per cell, `CharsetGlobals` (hi-res, multicolor or ECM registers, which decide the mode) and a `TileSize`. |
| [`SpriteSet`](../crates/convert64-core/src/model/sprite.rs) | 64-byte `Sprite` blocks (colour and multicolor flag in the attribute byte), one MC1/MC2 pair for the whole set, a `SpritePos` per sprite and `OverlayPair`s. |
| [`CellPos`](../crates/convert64-core/src/model/mod.rs), `CellPalette` | Cell coordinates, and the colours a cell can show in bit-pattern order. |
| [`Mode`, `ModeSpec`](../crates/convert64-core/src/mode.rs) | The modes and their geometry (§1.2). |
| [`Rule`](../crates/convert64-core/src/rule.rs) | Validator rules 1–5, numbered as in the reference. |

## 5. Stack rationale (summary of ADR-001, ADR-005, ADR-006)
- **Rust core:** first-class WASM, `rayon`, safety, one crate graph for app + CLI + MCP.
- **Command registry (ADR-005):** every action is a named, serialisable command on `Project`; app, CLI and MCP are thin front-ends; undo/redo is command history.
- **egui/eframe (ADR-006):** immediate mode fits "slider → re-solve → redraw"; one codebase for native and web; no IPC, no state mirroring, no second toolchain. CPU-only compositing.
- **CLI parity:** the Makefile integration (`convert64 convert title.convert64 --mode koala -o build/title.kla`) is what makes this a pipeline tool.
- Rejected: Tauri + Svelte (polish not needed for a tool UI; dual execution paths), Python app (perf/packaging), editor plugins (data model mismatch), Electron (size).

## 6. Testing
- Unit tests per solver stage; property tests that every solver output passes the validator.
- **png2c64 parity (#181, #190), the standing bar of the default solver and of `convert`.** `crates/convert64-core/tests/png2c64_parity.rs`: the reference solver on png2c64's 30 ids on the test card and the portrait, and on all six fixtures, must be ≥ 99 % pixel-identical to png2c64's renders from its reference build ([reference/png2c64-reference.md](reference/png2c64-reference.md)). `crates/convert64-cli/tests/png2c64_convert.rs`: `convert64 convert` on every fixture source with its baseline's settings must be ≥ 99 % identical to `baseline.png` (all 100 % but `title`, 99.997 %), and with png2c64's `--sharpen 0.5` and `--sharpen -0.5` on the portrait ≥ 99.9 % identical to png2c64's renders of those (#228). CI runs both as its own step and prints every percentage. The default solver's golden MSE scores are reported, not gated; the native solver's goldens (`expected_native.png`) keep their gate.
- **png2amiga parity (#261, kept by ADR-014 as a test of the port, not of quality).** `crates/convert64-core/tests/png2amiga_parity.rs`: the png2amiga port on png2amiga's 68 C64 method ids on the test card (multicolor and hi-res) and the portrait, and on all six fixtures (also with `--match-range`, `--cell-refine`, and the portrait with `--sharpen ±0.5`) against png2amiga's committed renders (`fixtures/png2amiga/<name>/`, `docs/reference/dither-previews/png2amiga-*`); all 224 cases are 100 %. `crates/convert64-cli/tests/png2amiga_convert.rs` holds `convert64 convert --solver png2amiga` to the fixture renders (20 cases, 100 %). CI runs both in its own `png2amiga parity` step; nothing in CI builds png2amiga (the renders are made by `cargo xtask png2amiga build` / `renders` on a developer machine, GCC 15).
- **SSIMULACRA2 parity (#261).** `crates/convert64-core/tests/ssimulacra2.rs` scores 16 pairs built from the fixtures and compares each with png2amiga's score in `tests/data/ssimulacra2/reference.toml` (tolerance 1e-4; bit-exact).
- **No score decides quality (ADR-014 §7).** SSIMULACRA2 is reported in `--json` and by the MCP `solve` tool as information only; it is not a gate and not in the UI. A metric may become one only if it agrees with the eye on **≥ 90 %** of `fixtures/eye-rankings.toml`, the 28 pairs Jaakko ranked on #273's sheets (3 `confidence = "low"`). `cargo xtask metric-check` re-renders both pictures of every pair exactly as the sheet showed them (42 of 42 identical to the committed tiles), scores both against one reference per fixture (the source area-averaged in linear light) and writes `docs/reference/dither-previews/convert64-sheets/metric-check.md`. On 18 October 2026: `s2` agrees on 12 of 28 (43 %), `s2-blur` (σ = 1 display px) and `s2-crt` (a PAL model) on 16 (57 %), `oklab-blur` (#142's blurred OKLab MSE) on 15 (54 %); without the low-confidence pairs 44–60 %. None comes near 90 %. All four still prefer Refine where the eye rejected it and png2amiga's Lanczos on the zone plate; they miss cell blocks, colour casts and lost fine detail.
- **png2amiga's 32 methods on png2c64's pipeline (#275).** `solve::solver` tests hold a frame of each, in both modes, on the reference solver to `png2c64_ref::amiga::convert` (the hook #280's sheets were judged on) bit for bit, and check that locks and the interactive fast path keep working with them. Goldens: the `gradient` suite's `expected_m-<id>.png` for the 32 are the reference's renders (the other 58 stay native-solver goldens), and `portrait/expected_p2a-<id>.png` hold each at png2c64's baseline settings (strength 0.7, gamma 1.5).
- Golden fixtures: `fixtures/<name>/source.png` + expected `.kla/.art/.png`; compared bit-exact for formats, perceptually (OKLab MSE threshold) for solver output.
- Baseline comparison against png2c64 output on the same fixtures: both are scored against the source with png2c64's preprocessing (gamma 1.5), from which ours is solved too, with png2c64's own settings (`checker` 0.7). Per-pixel and blurred OKLab MSE (`png2c64_mse_vs_source`, `png2c64_blur_mse_vs_source` in `thresholds.toml`) are reported, not gated (#156).
- **Human review gate for solver changes (#156).** Both numeric bars have said we beat png2c64 while the side-by-side sheet showed png2c64 winning by eye. A PR that changes solver output carries the regenerated `sheet-vs-png2c64.png` in its description, with the author's per-id verdict, and merges only after a human (not the authoring agent) says it is at least as good as png2c64 on every shared id (CONTRIBUTING.md). The sheet's gradient-domain error (lightness gradients after a 1-px blur) is printed for reading, not as a bar.
- Composite scenes (`fixtures/composite/<scene>/scene.toml`): layered scenes rendered by the compositor and compared with `expected_1x.png` (target-grid composite, alpha = coverage) and `expected_attr.png` (attribute-map debug render), within ±2 per channel; regenerated through `cargo xtask regen-goldens` like every golden.
- Golden updates: `cargo xtask regen-goldens --accept --reason "…" --pr pending` before the first push, then `--set-pr N` once the pull request exists (#140). `fixtures/CHANGELOG.md` cites pull requests, not commit hashes, and no golden regeneration needs a red CI run. Details are in CONTRIBUTING.md.
- Dither previews: `cargo xtask dither-sheet --reason "…"` renders every method id on the dither-preview test card and the portrait, the card/zoom/portrait sheets in the png2c64 sheets' layout, and `sheet-vs-png2c64.png` with both converters' OKLab MSE for the 30 shared ids (`docs/reference/dither-previews/convert64-*/`, #116). Committed and regenerated with a reason, like the goldens.
- Acceptance: exported files load and display in VICE `x64sc`. `cargo xtask vice-check` also solves every fixture in both bitmap modes and shows each export (the writer, the three bitpair policies and the raw set read back from its manifest) in VICE with the same bar; see [file-formats.md](reference/file-formats.md#vice-acceptance-cargo-xtask-vice-check).
- **Pipeline example (#167).** `examples/no-glory` builds a title screen from a `.convert64` project with `make` and the CLI alone (Koala, PRG, PNG preview, KickAssembler source). CI's Linux and macOS jobs run `make check` there after `cargo test`, with the `convert64` that `cargo test` built.
- **VICE check (local, not in CI).** `cargo xtask vice-check` shows every png2c64 baseline in VICE `x64sc`. It builds a display PRG from the committed `baseline.kla`/`.art` with `convert64-formats::prg` ([file-formats.md](reference/file-formats.md#koala--art-studio-display-prg-convert64_formatsprgdisplay_prg)) and checks the PRG carries the baseline byte for byte, so it needs only VICE and a clone. VICE runs PAL, with the Pepto palette at neutral colour settings and no CRT filter. The task crops the 320×200 window from the 384×272 exit screenshot and maps every pixel to its nearest palette index. It requires **≥ 99.5 %** agreement with the decoded baseline and every cell showing only the baseline cell's colours. It writes `fixtures/<name>/vice.png` and a line in the fixture README. Tests in `xtask/src/vice.rs` run the comparison in CI on the committed captures, with negative controls: another fixture's baseline and a capture shifted one pixel right or down must fail. VICE is not installed on the CI runners, so run it locally whenever baselines change: `cargo xtask png2c64-baselines --png2c64 <bin>`, then `cargo xtask vice-check`. Never lower the bar to make a fixture pass.

## 7. Performance budgets
- Full re-solve (MC bitmap, 16 bg search): < 100 ms native, < 300 ms WASM. Measured (`tests/solve_bench.rs`, portrait at 4×, Apple M-series, release), sequential / `parallel`: reference solver Floyd–Steinberg 26.8 / 8.4 ms, Bayer 4 24.4 / 5.1 ms; native solver 49.0 / 15.8 ms and 95.4 / 13.9 ms (#181). Interactive re-solve after a 0.25 px nudge on the reference path: 8.0 / 5.5 ms and 5.7 / 3.3 ms.
- Interactive single pass while dragging: < 16 ms native, < 50 ms WASM.
- SSIMULACRA2 (#261, `tests/ssimulacra2.rs` `ssimulacra2_timing`, Apple M-series, release, one thread): 2.3 ms for 320 × 200, 1.3 ms for 160 × 200; in wasm (Node 22) 8.4 and 4.1 ms. It runs once per full solve in the CLI and MCP.
- The png2amiga solver (#261, `tests/solve_bench.rs`, portrait multicolor / lineart hi-res at 4×, sequential, Apple M-series, load ~35): full solve 6.7 / 16.6 ms (Floyd–Steinberg), 4.9 / 13.4 ms (Bayer 4), its interactive fast path about the same; Refine adds about 0.6 s (portrait 593 ms against 11 ms, release CLI).
- The Lanczos opt-in (#261's measurements of the same code, `tests/bench.rs`, Apple M-series, release, load 20–35): axis-aligned layers within the budgets; rotated layers cost more (the widened kernel has 6–14 taps per axis against Mitchell's 4: ten rotated layers all on Lanczos composite cold in 53–57 / 79–90 ms sequential, over #139's 60 ms in hi-res, 7–11 / 10–14 ms with `parallel`). A 4096 × 2560 Lanczos layer: a drag frame 0.98 / 1.61 ms under Auto, 34.8 / 33.6 ms under Exact sequential. The defaults (Mitchell) are as below.
- Rotated layers (#198, `tests/bench.rs`, macOS runner class, multicolor / hi-res): ten rotated masked layers composite cold in 47 / 89 ms with `parallel` and 132 / 240 ms single-threaded. A rotation drag of one layer takes 6.5 / 12.8 ms and 15.8 / 36.3 ms. That is over #98's and #139's budgets (60 ms cold, 10 / 12 ms per nudge); axis-aligned layers are unchanged. See "As built in #198".
- Compositor after ADR-012 (#227; `tests/bench.rs` at the 1× default, ten full-screen masked layers, Apple M-series, release, multicolor / hi-res, sequential then `parallel`). Axis-aligned: cold composite 15.2 / 29.2 ms and 4.3 / 6.2 ms, a 0.25 px nudge 1.7 / 3.3 ms and 1.6 / 2.3 ms (before, at 4×: 15.0 / 27.7 and 7.5 / 8.3 ms cold, 4.1 / 7.5 and 2.0 / 3.1 ms nudge). Rotated: cold 25.2 / 49.3 ms and 5.0 / 8.1 ms, a rotation drag 2.7 / 5.4 ms and 1.7 / 2.3 ms (before: 52.0 / 102.1 and 9.0 / 14.0 ms cold, 7.1 / 14.9 and 2.4 / 3.5 ms drag). Every row is within #98's and #139's budgets; the rotated path no longer exceeds them. Most of what is left is the masks' 4× grid (without it the rotated cold composite is 11 / 21 ms), which ADR-012 measured as needed.
- Mip chain (#235; `tests/bench.rs` `mip_build_4096`, Apple M-series, release): a 4096 × 2560 16-bit source fitted to the screen builds its 512 px level (three 2 × 2 reductions) in 5.8 ms single-threaded and 2.2 ms with `parallel`; the cold composite of that layer, the build included, is 8.8 / 9.5 ms (multicolor / hi-res) single-threaded, and a 0.25 px nudge after it 0.8 / 1.5 ms with no rebuild. In the browser build (headless Chrome, same machine) the three levels take 9–23 ms, the first page load slowest, logged to the console. The ten-layer benchmark above, whose sources are not shrunk past 4×, is unchanged (sequential cold 15.6 / 29.2 ms, `parallel` 4.1 / 5.3 ms; before #235, on the same machine, 17.4 / 31.8 and 4.2 / 6.4 ms).
- **Browser build after ADR-012 (2026-10-13; #233 + #234, `trunk build --release`, headless Google Chrome on Apple M-series, 1440 × 900, the `title` fixture, multicolor).** Measured per frame inside the app's `ui()` with `performance.now()` probes (a local patch, not committed), the input driven over the DevTools protocol, one pointer move per animation frame. Each edit composites three times: the scheduler's 1× composite, the session's 1× composite and the Source view's 4× composite (`Session::PREVIEW_N`).

  | Interaction | Frame with the edit (mean / p95) | Scheduler 1× | Session 1× | Source view 4× | Solve frames |
  |---|---|---|---|---|---|
  | One edit (arrow-key nudge) | 91 ms | 7 ms | 7 ms | 69 ms | fast path 58 ms, then the full solve 182 ms, each in one frame |
  | Nudge drag (Move tool) | 78 / 83 ms | 4 ms | 4 ms | 62 ms | none during the drag; 55 + 180 ms after release |
  | Slider drag (Brightness) | 82 / 86 ms | 5 ms | 5 ms | 66 ms | a 47–52 ms fast-path frame after most edit frames; 171 ms after release |

  **Every drag exceeds 33 ms per frame** (the target for a 30 fps drag), by about 2.5×. The 4× Source-view composite is 60–80 % of each edit frame; without it an edit frame would be about 12–16 ms. Second, the solver's work is not sliced in wasm: the fast path (~50 ms) and the full solve (~180 ms) each run inside a single `tick`, against the 10 ms per tick that §7's scheduler line aims at. Proposed fixes, not implemented: skip the 4× Source-view composite while a drag or slider is active (draw the session's 1× canvas, or keep the last 4× texture under the live transform) and composite it once on release; then slice the reference solver's cell loop across ticks, or move the solve to a Web Worker.
- **Browser solve in a Web Worker (#265, 2026-10-14;** `trunk build --release`, headless Google Chrome on Apple M-series, 1440 × 900, the `title` fixture, multicolor, the #250 method and driver with `performance.now()` probes in `App::ui`, a local patch). Medians of 8 runs before and 4 after, alternated, on a machine loaded by parallel builds (load average 15–70, so the absolute times are high; the comparison is paired). Time from `submit` to the result:

  | Interaction | Preview (provisional), before → after | Full result, before → after | Main-thread frames until the full result |
  |---|---|---|---|
  | One edit (arrow-key nudge) | 146 → 90 ms | 310 → 240 ms (−23 %) | before: a 48–63 ms and a 156–199 ms frame; after: ≤ 1.4 ms each |
  | Release of a nudge drag | 74 → 66 ms | 240 → 229 ms (−4 %) | same |
  | Release of a slider drag | 77 → 57 ms | 242 → 224 ms (−8 %) | same |

  The frame that submits still pays the composite on the page (4–5 ms with the request's encoding; 1.4 ms mean during drags) and, unchanged, the Source view's 4× composite (60–80 ms, #264). Without the spare worker the releases were 6–8 % slower than before, because the release waited for a superseded fast-path stage to finish (up to ~50 ms); terminating the busy worker removed that. During a drag the fast path runs in the worker but its results are almost never delivered (1–7 of ~115 drag frames), because each drag frame supersedes the previous submission before its ~50 ms solve returns; before #265 none were. The wasm grew by 0.16 MB (9.57 → 9.73 MB), plus the 0.5 KB worker script.
- **No 4× Source-view composite while dragging (#264).** While a drag runs, the Source view (`crate::drag`, `Workspace::prepare`) no longer composites at 4×:
  - A **transform drag** (Nudge and Transform tools, the Transform fields, held arrow keys) draws cached 4× textures moved on the GPU: the layers below the dragged one and the layers above it (`Session::layers_canvas_rgba`), and the dragged layer by itself at its transform from before the drag (`Session::layer_canvas_rgba_at`), drawn as an egui mesh through the affine map from that transform to the current one. Each texture is composited once when the drag starts. If the dragged layer is the only visible one, nothing is composited: the 4× texture from before the drag is its texture. Laying the textures over each other is the compositor's Normal blend, which is associative, so the stack is the composite (a test compares them pixel for pixel where the upper textures are opaque or empty). Where it is not, the drag shows the 1× composite instead: a dragged layer or a visible layer above it that is translucent or not Normal, a mask on a layer above, a mask on the dragged layer over visible layers below (the GPU blends soft edges in sRGB, the compositor in linear light), or a mask snapped to cells (it does not move rigidly). When the dragged layer reached past the screen when the drag started, its texture lacks what lay outside, so the current 1× composite is drawn underneath and shows there.
  - A **slider or field drag** shows the session's own composite, the 1× the solve reads (`Session::solve_canvas_rgba`), upscaled with nearest filtering.
  - On release (or when neither a pointer button nor an arrow key is down) the 4× view is composited exactly once. The solve and the result panel are unchanged.

  Measured as in the #250 bullet above, but with the frame times of every frame between the press and the release (the drags are 40 pointer moves, one per animation frame; the rotation drag uses the Transform tool's rotate handle on the picture after the nudge, so the layer reaches past the screen and the 1× backdrop is composited every frame). Before is `origin/main` at 3cfe72e; two runs each, with the load average (other builds were running) noted. Finals on a quieter machine are in the bullet after next.

  | Drag | Before: p50 / p95 / max | After: p50 / p95 / max | Load (before; after) |
  |---|---|---|---|
  | Nudge (Move tool) | 6.6 / 50.9 / 54.8 ms; 7.5 / 60.2 / 67.7 ms | 1.2 / 7.1 / 7.5 ms; 1.3 / 6.9 / 7.6 ms | 16–17, 25–38; 27–34, 31–38 |
  | Slider (Brightness) | 41.1 / 59.9 / 62.8 ms; 49.1 / 71.6 / 73.7 ms | 7.4 / 44.1 / 45.8 ms; 6.9 / 40.7 / 41.0 ms | as above |
  | Rotation (rotate handle) | 6.2 / 94.4 / 138.5 ms; 6.4 / 96.7 / 141.4 ms | 1.2 / 12.2 / 46.7 ms; 1.2 / 11.5 / 56.6 ms | as above |

  The Source view's share of a drag frame is now 3–7 ms (the 1× composite), against 50–90 ms. Nudge and rotation drags are within 33 ms at p95. **The slider drag is not:** its slow frames are the Interactive fast-path solve, 40–44 ms inside one `tick` every other frame, which this change leaves alone; slicing the solve or moving it to a Web Worker (#265) is what brings it down. The release frame composites at 4× once (42–82 ms), and the full solve follows (150–160 ms), as before.
- **Finals on a quieter machine (2026-10-14; load average 3–5).** The same method and driver (40-move drags, one pointer move per frame; the rotation drag on the nudged picture), release builds of `main` at 138259c (with #260's picker), with #264, with #265, and with both; two runs each, alternated; p95 of the frames between press and release, in ms. "Full solve" is the longest main-thread frame from the release (or the edit) until the full result, and the time from the full submit to the full result.

  | Build | Nudge drag p95 | Slider drag p95 | Rotation drag p95 | Longest frame until the full result | Submit → full result |
  |---|---|---|---|---|---|
  | `main` | 68.2–69.0 | 72.3 | 87.6–88.0 | 153–159 (the full solve's tick) | 203–297 ms |
  | + #264 | 9.3–11.0 | 43.9–44.7 (the fast path's tick) | 11.1–12.8 | 148–159 | 227–309 ms |
  | + #265 | 65.5–65.7 | 67.5–68.7 | 83.9–84.2 | 1.0–6.4 after a drag; 70–83 for a single edit (its 4× composite) | 161–232 ms |
  | **+ #264 and #265** | **5.5–5.6** | **5.2–5.4** | **6.5–6.6** | 54–79 (the release frame's one 4× composite); every frame while the worker solves ≤ 16 | 209–232 ms |

  With both, every drag is within 33 ms at p95 (max 9.7 ms), and the main thread never waits for a solve. What remains over 16 ms is one frame per gesture: the 4× Source-view composite on release (54–79 ms) or after a single edit (70–84 ms).
- Solve scheduler stages (#194; `stage_timings` in `convert64-app`, release, native with `parallel`, Apple M-series, multicolor at 4×, a full-screen layer and a 24 px square): cold composite 2.7 ms and full solve 8.9 ms; a 0.25 px nudge of the square 0.7 ms composite and 4.2 ms fast path; its release 7.6 ms full solve. In wasm each `tick` aims at 10 ms: a drag frame composites, and the next frame (or the same one, once both fit) solves the fast path; single-threaded the fast path is 6–8 ms native (above), an estimated 15–25 ms in wasm (not measured), within the 50 ms budget.
- **Method-picker thumbnails, all 90 methods (#275; png2amiga's 32 on the reference since then).** Natively (`preview::tests::thumbnail_timing`, release, both sources, so 180 thumbnails per mode; Apple M-series, load average 21–38 from parallel builds): single-threaded 369–581 ms multicolor (2.0–3.2 ms each) and 691–1202 ms hi-res (3.8–6.7 ms each); with `parallel` 444–2316 ms and 706–3899 ms, the spread being the load. In the browser (`tests/web/picker-timing.mjs`: release bundle, headless Google Chrome, 1440 × 900, the `title` fixture, multicolor, the Gradient source; load average 21–28), three opens: every visible card had its thumbnail 385–614 ms after the click (277–499 ms after the first render was queued; the first open also renders the cards above the current method that it scrolls past) and all 89 missing thumbnails were rendered 743–774 ms after it (628–657 ms after the queue started, 410–421 ms of it solving). Again after #288's Recipe-tab reorder, at load average 5–6: 386–625 ms to the visible cards and 729–754 ms to all (618–633 ms after the queue started, 372–380 ms solving); the wasm driver's 8 ms per frame budget, not the load, sets the pace. A full open is under 1 s; visible cards come first (#260). The app logs both times to the console.
- UI uploads the 320×200 preview as an egui texture with nearest-neighbour filtering and draws it at integer zoom with 2:1 pixel aspect where applicable; overlays (cell grid, pins/locks, clash heatmap) are painter calls over the same rect.
