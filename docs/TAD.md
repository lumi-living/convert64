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
        │  per-layer: float transform, mask, feather, levels/curves/HSL, blend, opacity,
        │             dither recipe (method, strength, phase, metric, allowed set, force/adaptive)
        ▼
 Composite at N× supersample (default 4× → 1280×800 for a 320×200 target)
        │  + attribute map: per pixel, which recipe governs it
        │    (top-most non-transparent layer wins; strength blends through alpha/feather)
        ▼
 Target-mode resample (box/Lanczos → 320×200 or 160×200, honouring 2:1 aspect)
        ▼
 Constraint solver (global colours → per-cell colour sets → cell-clamped dithering → refine)
        ▼
 Validated C64 image model (bitmap/screen/colour/bg | charset+map | sprites)
        ├─► Preview renderer (palette object, border, aspect, optional PAL shader)
        └─► Exporters (formats crate)
```

**Dithering is a per-pixel attribute carried through compositing, not an operation per layer** ([ADR-002](adr/ADR-002-dither-as-per-pixel-attribute.md)). Palette indices cannot be blended; layers stay in float and control only the recipe.

## 3. Algorithms

### 3.1 Per-cell constraint solving (bitmap modes)
**Since #181 the base is png2c64's pipeline, ported** (`solve::png2c64_ref`, [ADR-010](adr/ADR-010-png2c64-reference-solver.md), [reference/png2c64-reference.md](reference/png2c64-reference.md)): per cell, a brute force over every set (455 multicolor triples per background, 120 hi-res pairs) on OKLab distances, an ordered method's threshold biasing each pixel (L by 0.15 t, a and b by 0.03 t) and the picture being the search's nearest colours; for diffusion and `none`, a global Floyd–Steinberg pass against the full palette chooses each cell's colours (most used first) and the diffusion runs within them; the background with the least search error wins. The parity test against png2c64's own output is the standing bar. Our extras are hooks on it (pins, locks, allow/deny, per-pixel recipes, the interactive fast path) and named divergences, off by default. The outline below is the native solver (`SolverKind::Native`), which frames the reference cannot solve fall back to (methods png2c64 lacks, non-OKLab metrics, Force, the native knobs).
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
Layers positioned in float; composite at N× supersample; box-filter to the target grid. A 0.25-px nudge changes target-pixel coverage and cell statistics, so the solver legitimately re-picks cell colours. Supersample factor is a quality setting.

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

Crates: `image` (decode; PNG preview export in `convert64-formats`), own OKLab code, `rayon` behind a `parallel` feature, `serde` (project file + command args), `clap`, `serde_json` (CLI/MCP JSON output, Figma token import, the raw bitmap set's manifest in `convert64-formats`), `toml` (fixture thresholds, project config), `rmcp`, `eframe`/`egui`, `egui_material_icons` (icons, ADR-007), `rfd` (file dialogs), `insta` + `proptest` (tests). No `std::fs` in core.

As built in E1 (2026-10-11): `eframe` runs on the **`glow`** (OpenGL / WebGL2) renderer, not its default `wgpu`, keeping ADR-006's no-wgpu rule; `rust-toolchain.toml` pins stable with `rustfmt`, `clippy` and the `wasm32-unknown-unknown` target; the app's web build uses the eframe web template with no extra JS-side crates (eframe re-exports what it needs). See [ADR-005](adr/ADR-005-command-engine-with-cli-json-mcp.md) and [ADR-006](adr/ADR-006-egui-single-rust-workspace.md).

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
- **Supersampling.** The default factor is 4× (1–8).
- **Resampling** is separable per axis because transforms are axis-aligned. It is bilinear where a source pixel covers at least one super pixel, and Lanczos-3 stretched to the footprint below that. A layer's edge is its exact box coverage, so a sub-pixel nudge moves target pixels continuously.
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
- **`--json`** is global. Every command prints one object: `command`, `ok`, `exit_code`, `error`, `timing_ms` (`total` and phases), `files_written`, then its own fields (`report.rs`). `convert`'s solve fields are `background`, `clashes`, `legal`, `violations`, `solve_error`, `summary`, `worst`; the total solve error moved from `error` to `solve_error`, since `error` is now the message.
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
- **wasm: stepping, no threads** (`Stepper`, also usable natively through `Scheduler::stepping`, which is how the tests run it). A submission is cut at stage boundaries: composite, then solve. A full solve with a previous solution first runs the fast path and delivers it as `provisional` (`Config::preview_full`, on in wasm, off natively). `tick` runs at least one stage, then more while the next stage's last measured duration fits in `Config::frame_budget_ms` (10 ms, leaving about 6 ms of a 60 Hz frame to the UI). A stage is one core call and cannot be split without changing the core (the parity test pins its output), so a full solve is one long frame in wasm; it comes after the provisional result is on screen.

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
- **State and actions.** `Shell` holds only chrome state (view mode, tool, dock tab, what is hidden, grid, zoom, theme choice); `Shell::show` draws everything and returns `Action`s the app carries out (`Quit`, `SetTheme`). Menu items of features still to come are disabled, not hidden (UX §11). Shortcuts are `shell::shortcuts` constants, shown in menus with `ctx.format_shortcut`; `Tab`, `Shift Tab`, `Q` and the tool keys are ignored while a text field has focus. `convert64_app::icons` names every glyph (ADR-007).
- **Theme.** View › Theme (System, Light, Dark) is stored in eframe's app storage (a file natively, `localStorage` on the web): the workspace's `eframe` gains its `persistence` feature, which brings in `ron` (eframe's own storage format).
- **Screenshots.** `convert64-app --screenshot <file.png> [--theme light|dark] [--size <w>x<h>]` (native) draws a few frames, saves the window via `ViewportCommand::Screenshot` and quits, without touching the stored theme; PR screenshots come from it.

As built in #204 (2026-10-13): the result panel and the working view (UX §4).

- **Document and workspace.** `convert64_app::document::Document` wraps a `convert64_project::Session` (edits go through registry commands; a revision counter tells the views what changed) and builds the scheduler's `Snapshot`. `Workspace` adds the newest solution and two textures: the source composite (the session's supersampled canvas, `Session::canvas_rgba`, linear with mipmaps) and the result (target grid, `TextureOptions::NEAREST`, drawn 2:1 for multicolor). The working view and the result panel share the result texture. Both are rebuilt only when the document's revision or the solution changes. The session's compositor serves the Source view and the scheduler's serves the solves, so an edit composites twice (in wasm on one thread); a shared canvas is a later optimisation.
- **Camera.** `convert64_app::view`: positions in screen units (hi-res pixels, 320 × 200 in every mode), one centre shared by both modes so `Q` keeps the place; Source zoom fractional 25–800 %, Result zoom 1×, 2×, 3×, 4×, 6×, 8× with the picture's corner on whole points; fit, 100 %, step and zoom-about-cursor. Pan with `Space` + drag, middle drag or scroll; zoom with pinch or `Cmd` + wheel.
- **Compare and overlays.** Hold `\` flips to the other image; `,` cycles split (draggable divider), A/B flicker at 2 Hz and side by side (linked cursor). Every compare mode draws both images with the view mode's geometry, so they line up. Overlays: cell grid (`Cmd '`, from 6 pt cells up), clash heatmap (`Shift C`: cell error as `clash` alpha, colour-starved cells outlined), pins and locks, the other image as a 30 % ghost, and the border (`Cmd B`). The border is the solution's background colour (black in hi-res): the project has no border colour yet.
- **Result panel.** The result inside its border at most 1× (the Figma frame's 16/12 px inset), clash dots, the navigator rectangle in Result mode; a click centres the working view there and switches it to Result, a drag pans. The header has the mode dropdown (multicolor and hi-res bitmap, `mode_set` then a full solve), the clash count (a button to the worst clash) and the compare and fit buttons. The solve time is in the status bar, with the cursor's cell, pixel, colours, error and pin/lock state.
- **Opening (interim).** `convert64-app picture.png` opens a PNG natively; File › Open, saving and the browser's file handling are #205. `--screenshot` takes `--view result`, `--overlay grid|clashes|ghost` and `--compare split|flicker|side` for PR screenshots.
- **Check.** The app's result is pixel-identical to `convert64 convert` on the same PNG (portrait: 64 000 of 64 000 pixels), and a test solves a document through the scheduler.

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
- **png2c64 parity (#181), the standing bar of the default solver.** `tests/png2c64_parity.rs`: the reference solver on png2c64's 30 ids on the test card and the portrait, and on all six fixtures, must be ≥ 99 % pixel-identical to png2c64's renders from its reference build ([reference/png2c64-reference.md](reference/png2c64-reference.md)). CI runs it as its own step and prints every percentage. The default solver's golden MSE scores are reported, not gated; the native solver's goldens (`expected_native.png`) keep their gate.
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
- Solve scheduler stages (#194; `stage_timings` in `convert64-app`, release, native with `parallel`, Apple M-series, multicolor at 4×, a full-screen layer and a 24 px square): cold composite 2.7 ms and full solve 8.9 ms; a 0.25 px nudge of the square 0.7 ms composite and 4.2 ms fast path; its release 7.6 ms full solve. In wasm each `tick` aims at 10 ms: a drag frame composites, and the next frame (or the same one, once both fit) solves the fast path; single-threaded the fast path is 6–8 ms native (above), an estimated 15–25 ms in wasm (not measured), within the 50 ms budget.
- UI uploads the 320×200 preview as an egui texture with nearest-neighbour filtering and draws it at integer zoom with 2:1 pixel aspect where applicable; overlays (cell grid, pins/locks, clash heatmap) are painter calls over the same rect.
