# png2amiga's C64 path as the reference solver (#261, ADR-013)

`convert64_core::solve::png2amiga_ref` ports png2amiga's C64 bitmap path (multicolor and
hi-res) so that our output is pixel-identical to png2amiga's. It is the reference solver
(`SolverKind::Reference`) since [ADR-013](../adr/ADR-013-png2amiga-c64-reference.md) (in PR #262), which moved the reference from
png2c64 ([png2c64-reference.md](png2c64-reference.md), kept behind `--solver png2c64`).
This page records which png2amiga build is "the reference", why, what differs from the
png2c64 port, and how the test data was produced.

## The reference build

png2amiga commit `4a8777653536ac07074a650ee86860347c8755fb` (v1.111.2, 2026-10-05; the
newest tag when #261 started, the GitHub "latest release" being the older v1.110.0),
built by `cargo xtask png2amiga build`:

- a blob-less clone into `target/xtask/png2amiga-src`, the pinned commit checked out,
  and only the three submodules its CMake build checks for (`libwebp`, `ssimulacra2`,
  `constixel`; the Amiga debugger submodule is not needed and did not clone);
- Homebrew **GCC 15.3.0**, `CMAKE_BUILD_TYPE=Release`, C++26 (png2amiga's own setting; no
  `<print>` or other shims were needed with GCC 15);
- **strict IEEE arithmetic**: png2amiga adds `-ffp-contract=fast` as a target option,
  after any `CMAKE_CXX_FLAGS`, so a compiler launcher (`strict-launcher.sh`, written by
  the task) appends `-ffp-contract=off` to every compile, where it is the last word;
- a **maths shim**, [`png2amiga-reference/crmath.c`](png2amiga-reference/crmath.c),
  linked in through `CMAKE_EXE_LINKER_FLAGS`: `powf`, `sinf`, `cosf`, `atan2f` and `expf`
  computed in double and rounded once, which is what the Rust port does.

```sh
cargo xtask png2amiga build            # → target/xtask/png2amiga-build/reference/png2amiga
cargo xtask png2amiga renders          # every baseline and preview below
```

**Why strict.** png2amiga calls `std::fma` explicitly in its OKLab conversions and squared
distances (an explicit FMA is fused in any build, and the port uses `f32::mul_add`), but
elsewhere GCC's default `-ffp-contract=fast` fuses `a·b + c` wherever it likes, which no
portable port can follow. **Why the shim.** macOS's `atan2f` differs from the correctly
rounded value on 5.8 % of random inputs and its `powf` on some subnormal ones (checked on
200 million and on every float in `0..2`); `sinf`/`cosf` agree on every float in
`−64..64`. png2amiga's range matching takes `atan2f`, its scaler `sinf`. The shim makes the
reference and the port evaluate the same function on every platform.

Pixels identical to the reference build (strict + shim), same commit, same input (`card`
is the dither-preview test card at Convert64's settings, `portrait` the portrait at
png2amiga's own C64 defaults, `fixture` a fixture at its baseline's settings):

| Render | GCC strict, platform libm | GCC default flags | Apple clang 21, default flags |
|---|---|---|---|
| card `floyd-steinberg`, `jarvis`, `riemersma`, `checker`, `yliluoma1` | 100 % | 100 % | 100 % |
| portrait `floyd-steinberg`, `floyd-steinberg-oklab`, `bayer8x8`, `none` (blur metric) | 100 % | 100 % | 100 % |
| portrait `floyd-steinberg --match-range` | 100 % | **83.487 %** | **83.487 %** |
| fixture portrait, photo `--match-range`, portrait `--cell-refine` | 100 % | 100 % | 100 % |
| gradient `floyd-steinberg`, photo `atkinson`, title `structure-fs`, terrain `sierra-lite` | 100 % | 100 % | 100 % |

png2amiga's C64 path is far less sensitive to the float model than png2c64's was (its
global Floyd–Steinberg picks colours by histogram, and its diffusion runs in sRGB), but
not insensitive: one of the 17 renders changes on 16.5 % of its pixels with FMA
contraction. The libm differences changed none of them; the shim stays as insurance.

## What differs from the png2c64 port

png2amiga's C64 encoder began as a merge of png2c64 (`c64.hpp`). File by file, against
`solve::png2c64_ref` (png2c64 `1265329a`):

| png2amiga | png2c64 (our port until #261) | png2amiga (ported in `png2amiga_ref`) |
|---|---|---|
| `scale.cpp` | Mitchell–Netravali (B = C = ⅓), 4 taps, edge pixel repeated; weights summed, the sum divided by them; negative lobes kept | **Lanczos-3 widened by the shrink**: `filter_scale = max(1, src/dst)`, `⌈3·filter_scale⌉` taps each side, taps outside the image **cut** and the rest renormalised; weights normalised first, `Σ src·w`; **each pass clamped to 0..1** |
| `preprocess.cpp` `apply` | gamma (default **1.5**), 3 × 3 sharpen/blur on L, levels, brightness/contrast/saturation/hue in OKLab, every pixel round-tripped and clamped | the same steps operation for operation; default gamma **1.0**; png2amiga's OKLab conversions (below) |
| `preprocess.cpp` `match_palette_range` | per OKLab axis: the source's 1st–99th percentile range stretched onto the palette's bounding box shrunk 5 %; L remapped, a/b scaled around 0 | **per (lightness, hue)** on a 64 × 64 grid: the palette's reachable chroma (its colours within one bin of L, every pairwise mix crossing L, and grey; the furthest projection on the hue) over the source's 95th-percentile chroma (twice box-diffused across hue and L), bilinear; each pixel's chroma × `c_pal/c_src`, **clipped at `c_pal`**, **no upper cap on the scale**; L and hue kept; grey pixels and empty bins untouched; `percentile` and `margin` ignored |
| `color_space.hpp` | correctly rounded `cbrt`; plain multiply-adds; plain squared distances | **`fast_cbrt`**: a bit-trick seed and two Newton steps; **explicit `std::fma`** in the OKLab matrix rows (`fma_dot3`), in `oklab_to_linear` and in every squared distance (`fma_dist_sq`); the 8-bit sRGB table is the same |
| `c64.cpp` multicolor | background **searched** over all 16 (least brute-force error); ordered methods bias the 455-triple search (`L + 0.15·t`, `a, b + 0.03·t`, `t × strength²`) and the picture is the search's own nearest colours; diffusion and `none` replace each cell's colours by the top 3 of a global Floyd–Steinberg (±0.12) | background **always colour 0** (the one shared register; never searched); **any method but `none`**: each cell's 3 colours are the most used of the global Floyd–Steinberg (ordered methods included), unfilled slots padded with colour 0; **`none`**: the 455-triple brute force by the cell metric; then **every method dithers** within the cell's colours |
| `c64.cpp` hi-res | the 120-pair brute force (with the ordered bias); diffusion/`none` take the global FS top 2 | **dithered**: the pair (`i ≤ j`, so a colour may pair with itself) minimising Σ `mix_segment_score` (distance to the OKLab segment + `λ·u(1−u)·|a−b|²`, λ = 0.09375) over the **3 × 3-blurred** cell; **`none`**: the 120-pair brute force by the cell metric |
| `c64.hpp` cell metric | `mse` only | **`blur` by default** (Pappas–Neuhoff: the rendered cell blurred 3 × 3 with its edge replicated, against the globally blurred source, in OKLab); `mse` too |
| `dither.cpp` / `dither.hpp` | 30 methods; diffusion in an OKLab error buffer clamped on read and write; Ostromoukhov; `--adaptive` | **68 methods**, all through `diffuse_raw_buffer`: the target is the pixel's **sRGB** value plus its received error (clamped per channel **on read only**), back to OKLab, plus a structure bias (`structure-fs`, `contrast-fs`, `zhoufang`) and the ordered offset `thr × strength × (0.15, 0.03, 0.03)` (strength once); the pick is the nearest cell colour or a palette-aware picker (Yliluoma family, Knoll, tri-tone, the optimal pairs and plans); the residual `(target − chosen) × strength` spreads **in sRGB**; `floyd-steinberg-oklab` keeps an OKLab residual (a/b clamped at half); `riemersma` is a 16-deep exponential history **in scan order** (no curve); Atkinson, Stucki and Jarvis use png2amiga's OKLab-tuned weights; **no Ostromoukhov, no adaptive**; `gilbert` and `dbs` have no kernel there, so on the C64 path they are a **plain nearest pick**; `radial` is centred at (160, 128) |
| `main.cpp` defaults | `checker`, strength 1.0, error clamp 0.1, gamma 1.5, Pepto | **`floyd-steinberg`**, strength from `dither_tuning` (FS: 0.76), **error clamp 0.35**, **gamma 1.0**, `blur`, **Colodore** |
| `cell_graph_refine.hpp` (`--cell-refine`) | — | opt-in Refine: coordinate descent on the 3 × 3-blurred image difference (every register: background, screen-RAM nibbles, colour RAM; then every bitmap bit unless the method is ordered, which instead re-runs its picker), four passes in linear RGB and four in OKLab, a full-image SSIMULACRA2 after each, the best kept only if it beats the start; it may change the multicolor background |

Convert64's own defaults (`checker` 0.7, error clamp 0.1, gamma 1.5, Pepto, cell metric
`mse`) do not change with ADR-013: they need a separate decision. The fixture baselines
are made with them, spelled out; the portrait previews use png2amiga's own defaults.

**Background rule (#273).** `solve.background_rule` (`--bg-rule`, `solve_set`'s
`background_rule`): `black` is png2amiga's colour 0 and the default, which the parity
tests use; `search` runs png2c64's outer loop over all 16 backgrounds on png2amiga's
encoder (`png2amiga_ref::c64::search_background`): per background, the sum over the free
cells of each cell's least `mse` error over the 455 triples of the other colours
(unbiased; png2c64 biases it for ordered methods), the lowest total winning. It acts only
on the reference in multicolor when the background is not pinned.

**Which differences cost the quality (#273).** `cargo xtask cause-sheet` swaps one
difference at a time at png2c64's baseline flags
([sheet-cause.md](dither-previews/convert64-sheets/sheet-cause.md)). Three account for
png2amiga's output differing from png2c64's: the scaler (widened Lanczos-3 against
4-tap Mitchell), the cell colours (the global Floyd–Steinberg top 3 against png2c64's
threshold-biased brute force) and the ordered amplitude (strength once, against
png2c64's strength²). With all three swapped, png2amiga's port reproduces png2c64's
baselines on 99.886–100 % of pixels. The background search, the colour maths and the
preprocessing (identical steps at equal gamma) each move at most 1 point of identical pixels.

## The parity test

`crates/convert64-core/tests/png2amiga_parity.rs` runs the port on:

- every one of png2amiga's 68 C64 method ids on the test card in multicolor and in hi-res
  (Convert64's settings, strength 1.0, metric `mse`) and on the portrait (png2amiga's
  defaults: gamma 1, error clamp 0.35, metric `blur`, strength 1.0);
- every fixture with its baseline's settings, with `--match-range`, with `--cell-refine`,
  and the portrait with `--sharpen 0.5` and `--sharpen -0.5`;

and requires at least 99 % of pixels identical to png2amiga's render in each case. **All
224 cases are 100 %.** `crates/convert64-cli/tests/png2amiga_convert.rs` holds
`convert64 convert` (the product: compositor front end, reference solver, Refine) to the
fixture renders. CI runs both in the `png2amiga parity` step; png2c64's parity test is
kept but ignored (`cargo test --test png2c64_parity -- --ignored`).

The renders are png2amiga's own `.koa` / `.hir` output (their bytes are Convert64's Koala
and Art Studio layouts), drawn at 320 × 200 through Pepto:

| Where | What |
|---|---|
| `fixtures/<name>/baseline.kla` / `.art`, `baseline.png` | the fixture at its baseline's settings |
| `fixtures/<name>/baseline_match_range.png`, `baseline_refine.png` | with `--match-range`, `--cell-refine` |
| `fixtures/portrait/baseline_sharpen_±0.5.png` | with `--sharpen ±0.5` |
| `docs/reference/dither-previews/png2amiga-card/`, `png2amiga-card-hires/`, `png2amiga-portrait/` | every method |

png2c64's baselines moved to `fixtures/archive/png2c64/` and are no longer gated.

## SSIMULACRA2

The score on the sheets and in `--json` is png2amiga's own SSIMULACRA2
(`convert64_core::quality`, bit-identical to png2amiga's scalar build), taken as png2amiga
takes it for C64: the prepared input against the render, at the mode's logical raster,
in linear light. The Refine step keeps its result only if that score improves, so the port
of Refine depends on it being png2amiga's, bit for bit.
