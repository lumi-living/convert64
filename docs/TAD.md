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
- Baseline comparison against png2c64 output on the same fixtures.
- Acceptance: exported files load and display in VICE `x64sc`.

## 7. Performance budgets
- Full re-solve (MC bitmap, 16 bg search): < 100 ms native, < 300 ms WASM.
- Interactive single pass while dragging: < 16 ms native, < 50 ms WASM.
- UI uploads the 320×200 preview as an egui texture with nearest-neighbour filtering and draws it at integer zoom with 2:1 pixel aspect where applicable; overlays (cell grid, pins/locks, clash heatmap) are painter calls over the same rect.
