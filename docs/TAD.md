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
│  ├─ convert64-cli/            `convert64` binary (clap): convert / validate / export / batch
│  ├─ convert64-mcp/            MCP server (rmcp, stdio) over the command registry
│  └─ convert64-app/            eframe/egui UI; native via `cargo run`, web via `trunk build`
├─ fixtures/                    source images + golden outputs for regression
└─ docs/                        PDD, TAD, adr/, reference/, TOOLING, github/
```

Crates: `image` (decode), own OKLab code, `rayon` behind a `parallel` feature, `serde` (project file + command args), `clap`, `serde_json` (CLI/MCP JSON output, Figma token import), `toml` (fixture thresholds, project config), `rmcp`, `eframe`/`egui`, `egui_material_icons` (icons, ADR-007), `rfd` (file dialogs), `insta` + `proptest` (tests). No `std::fs` in core.

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
- **Front-ends.** `convert64-cli` is also a library, holding the recipe-file JSON and `Session`. `convert64 recipe apply` and `convert64-mcp` run the same `Session`.

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

**Benchmark (#87, #98)**
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
- **Builds.** `convert64-cli` and `convert64-mcp` build the core with `parallel`. The app does not, because it also targets wasm.

As built in E5 (2026-10-12): the bitmap solver (§3.1, §3.4–§3.5), in `convert64-core::solve` (#126–#136; #5 tracks them). The full dither scope in [reference/dither-methods.md](reference/dither-methods.md) continues in #114–#116 and #120.

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
- **Mix levels** (`solve::mix`, recipe `mix_levels`, default 4; ordered and noise methods). A pixel's ratio is the share of the second of its two nearest colours whose linear-RGB average reproduces it. Per cell and pair, pixels within half a level step of the median are flat: they take one ratio, the cell's mean, snapped to what the eight neighbouring cells show of the same two colours when within half a step (hysteresis), and quantised to `N + 1` levels. Other pixels (edges, texture) keep their own continuous ratio. A pixel mixes only if the mix plus a spread penalty (`SPREAD_PENALTY` 0.25 × t(1 − t) × the pair's distance, after Yliluoma) is closer than its nearest colour, so pale areas next to a dark background colour are not peppered with it. `mix_levels = 0` keeps E5's continuous OKLab ratio.
- **Ranking by what the eye sees.** `PassResult::seen` measures a mixed pixel against the linear-light mix it is part of, at the share its pattern actually shows (`ordered::shown_share`). Finalists and refinement rounds are ranked by it; `Solution::error` and the clash report stay per pixel.
- **Pre-smoothing** (recipe `smooth`, default off). The ratio is taken from an edge-aware 5 × 5 Gaussian (σ = 1 target pixel; neighbours ≥ 0.08 OKLab ΔE away are left out), while colours are still chosen from the unblurred composite. It is off by default because, with mix levels doing the calming, it raised the blurred error on every fixture (title past png2c64's).
- **Levels.** `Adjustments` gains `gamma` (png2c64's `--gamma`: a per-channel power in linear light, applied first) and `match_range` (png2c64's `--match-range`: the image's 1st–99th-percentile OKLab extent mapped onto a palette's, applied last over the whole image). `suggest_range_match` is the "Auto levels" suggestion for sources whose L range covers under 60 % of the palette's. `convert64 convert` prints it as a hint and never applies it unasked (`--gamma`, `--match-range`, `--mix-levels`, `--smooth`).
- **Fair comparison.** The golden harness solves every fixture from png2c64's preprocessed source (`Gamma::PNG2C64` as a layer adjustment) and scores both converters against that source.
- **Blurred quality bar.** Both renderings are blurred in linear light (binomial, σ ≈ 1.4 display pixels) before the OKLab MSE. Ours must not be worse than png2c64's (`png2c64_blur_mse_vs_source`). Per-pixel MSE is still reported but is no longer a bar: against a colour the palette cannot reach, it is lowest for a flat wrong colour, which is what this issue removes. `portrait`'s olive background now dithers brown and black to the right tone, so its per-pixel MSE is 1.9× png2c64's while its blurred error is 7.6× lower.
- **Fixture fix.** `gradient`'s swatch band held Colodore values while the solve uses Pepto. It now holds the 16 Pepto colours, as its README says. Its png2c64 baseline was regenerated (VICE check 100 %), and `flat_areas::gradient_swatches_quantise_exactly` checks every swatch pixel, solved from the raw source.

| Fixture | Blurred MSE, ours | Blurred MSE, png2c64 | Per-pixel MSE, ours | Per-pixel MSE, png2c64 |
|---|---|---|---|---|
| portrait | 0.00191 | 0.01446 | 0.0317 | 0.0171 |
| terrain | 0.00182 | 0.00314 | 0.0067 | 0.0082 |
| title | 0.00290 | 0.00291 | 0.0064 | 0.0074 |
| gradient | 0.01059 | 0.01161 | 0.0158 | 0.0135 |
| photo | 0.00336 | 0.00376 | 0.0104 | 0.0157 |
| lineart | 0.00031 | 0.00045 | 0.0032 | 0.0151 |

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
- Golden fixtures: `fixtures/<name>/source.png` + expected `.kla/.art/.png`; compared bit-exact for formats, perceptually (OKLab MSE threshold) for solver output.
- Baseline comparison against png2c64 output on the same fixtures: both are scored against the source with png2c64's preprocessing (gamma 1.5), from which ours is solved too. Each solver golden requires our OKLab MSE after a blur in linear light to be no worse than png2c64's (`png2c64_blur_mse_vs_source` in `thresholds.toml`, #142). Per-pixel MSE (`png2c64_mse_vs_source`) is reported, not gated.
- Composite scenes (`fixtures/composite/<scene>/scene.toml`): layered scenes rendered by the compositor and compared with `expected_1x.png` (target-grid composite, alpha = coverage) and `expected_attr.png` (attribute-map debug render), within ±2 per channel; regenerated through `cargo xtask regen-goldens` like every golden.
- Golden updates: `cargo xtask regen-goldens --accept --reason "…" --pr pending` before the first push, then `--set-pr N` once the pull request exists (#140). `fixtures/CHANGELOG.md` cites pull requests, not commit hashes, and no golden regeneration needs a red CI run. Details are in CONTRIBUTING.md.
- Acceptance: exported files load and display in VICE `x64sc`.
- **VICE check (local, not in CI).** `cargo xtask vice-check` shows every png2c64 baseline in VICE `x64sc`. It builds a display PRG from the committed `baseline.kla`/`.art` with `convert64-formats::prg` ([file-formats.md](reference/file-formats.md#koala--art-studio-display-prg-convert64_formatsprgdisplay_prg)) and checks the PRG carries the baseline byte for byte, so it needs only VICE and a clone. VICE runs PAL, with the Pepto palette at neutral colour settings and no CRT filter. The task crops the 320×200 window from the 384×272 exit screenshot and maps every pixel to its nearest palette index. It requires **≥ 99.5 %** agreement with the decoded baseline and every cell showing only the baseline cell's colours. It writes `fixtures/<name>/vice.png` and a line in the fixture README. Tests in `xtask/src/vice.rs` run the comparison in CI on the committed captures, with negative controls: another fixture's baseline and a capture shifted one pixel right or down must fail. VICE is not installed on the CI runners, so run it locally whenever baselines change: `cargo xtask png2c64-baselines --png2c64 <bin>`, then `cargo xtask vice-check`. Never lower the bar to make a fixture pass.

## 7. Performance budgets
- Full re-solve (MC bitmap, 16 bg search): < 100 ms native, < 300 ms WASM.
- Interactive single pass while dragging: < 16 ms native, < 50 ms WASM.
- UI uploads the 320×200 preview as an egui texture with nearest-neighbour filtering and draws it at integer zoom with 2:1 pixel aspect where applicable; overlays (cell grid, pins/locks, clash heatmap) are painter calls over the same rect.
