# Dither methods — implementation scope

The full set of dithering methods Convert64 implements, grouped by how they
plug into the attribute map (ADR-002): every target pixel carries
`(method, strength, phase, metric, allowed_set, fit_mode)`, and one pass over
the image handles all methods at once. "More is better" here: each method is a
few lines once the family machinery exists, and scene users have strong
opinions about which one is right for a given picture.

Reference points: png2c64 (30 methods), ArcPaint64's pattern grid, Ditherista,
Photoshop/GIMP/Aseprite indexed-colour dialogs, and the classic literature
(Ulichney, Ostromoukhov, Yliluoma).

Reference renders of all 30 png2c64 methods are in [`dither-previews/`](dither-previews/README.md) (`sheet-card.png`, `sheet-zoom.png`, `sheet-portrait.png`). Convert64's renders of all 90 ids on the default (reference) solver, and a sheet pairing them with png2c64's, are in `dither-previews/convert64-*/`, made by `cargo xtask dither-sheet` (#116).

Every method below honours per-pixel `strength` (0–100 %, scales the threshold
offset or the propagated error), `allowed_set` and cell clamping. Ordered
methods also honour `phase` (`ScreenLocked` keeps the matrix anchored to the
C64 screen so patterns tile across layers; `LayerLocked` anchors it to the
layer so a nudged layer carries its pattern with it).

## The catalogue: all 90 methods

Every method Convert64 offers, in the method picker's order (UX §6a: families None ·
Ordered · Ordered noise · Error diffusion · Palette-aware · Effects). The id is the CLI's
`--dither <id>`, a recipe's `method.kind` and the MCP schema's enum; the names under
*Also accepted* parse too (png2c64's and png2amiga's spellings where they differ). The
name and the description are the picker card's (`preview::catalogue`, whose tests keep
this table complete). The last three columns say how each solver runs it:

- **Reference** (the default, png2c64's pipeline, [ADR-014](../adr/ADR-014-png2c64-stays-reference.md)):
  png2c64's 30 methods as png2c64 runs them; png2amiga's 32 that png2c64 lacks on
  png2c64's pipeline (§7, #275); the other 28 hand the frame to the native solver.
- **`--solver png2amiga`** (a comparison solver): png2amiga's 68 C64 methods as png2amiga
  runs them; the rest go to the reference, then native.
- **`--solver native`**: Convert64's own solver; for 12 of png2amiga's kinds a stand-in.

### None (1)

| Id | Also accepted | Name | What it looks like | Reference (default) | `--solver png2amiga` | `--solver native` |
|---|---|---|---|---|---|---|
| `none` | — | None | Nearest colour in the cell, no dither: flat areas, hard steps. | png2c64's `none` | png2amiga's `none` | own |

### Ordered (40)

| Id | Also accepted | Name | What it looks like | Reference (default) | `--solver png2amiga` | `--solver native` |
|---|---|---|---|---|---|---|
| `bayer2` | `bayer2x2` | Bayer 2×2 | Ordered 2×2 matrix: five levels, a coarse regular grid. | native solver (png2c64 lacks it) | png2amiga's `bayer2x2` | own |
| `bayer4` | `bayer4x4` | Bayer 4×4 | Ordered 4×4 matrix: the classic crosshatch look. | png2c64's `bayer4` | png2amiga's `bayer4x4` | own |
| `bayer8` | `bayer8x8` | Bayer 8×8 | Ordered 8×8 matrix: finer steps, the pattern still visible. | png2c64's `bayer8` | png2amiga's `bayer8x8` | own |
| `bayer16` | — | Bayer 16×16 | Ordered 16×16 matrix: smooth gradients over large areas. | native solver (png2c64 lacks it) | native | own |
| `bayer2x2-mc` | — | Bayer 2×2 (2:1) | png2c64's 2×2 Bayer for 2:1 pixels. | png2c64's `bayer2x2` | png2amiga's `bayer2x2` | own |
| `checker` | — | Checker | 50 % checkerboard, two levels: png2c64's default and the scene's workhorse. | png2c64's `checker` | png2amiga's `checker` | own |
| `h2x4` | — | H 2×4 | png2c64's 2×4 Bayer: eight levels, square on 2:1 pixels. | png2c64's `h2x4` | png2amiga's `h2x4` | own |
| `clustered4` | `clustered-dot` | Clustered 4×4 | Clustered-dot halftone shaped for 2:1 pixels: stable on a CRT. | png2c64's `clustered` | png2amiga's `clustered-dot` | own |
| `clustered8` | — | Clustered 8×8 | One round dot per 8×8 tile: newspaper halftone. | native solver (png2c64 lacks it) | native | own |
| `halftone8` | `halftone8x8` | Halftone 8×8 | png2c64's 45° round-dot halftone. | png2c64's `halftone8` | png2amiga's `halftone8x8` | own |
| `dot` | — | Dot | ArcPaint64's 6×6 round dot. | native solver (png2c64 lacks it) | native | own |
| `diamond` | — | Diamond | ArcPaint64's 8×8 dot that grows as a diamond. | native solver (png2c64 lacks it) | native | own |
| `star` | — | Star | ArcPaint64's 8×8 dot that grows along its axes first. | native solver (png2c64 lacks it) | native | own |
| `line-h2` | `line2` | Lines 2 | Horizontal lines, period 2: cheap on 2:1 pixels. | png2c64's `line2` | png2amiga's `line2` | own |
| `line-h4` | `line4` | Lines 4 | Horizontal lines, period 4. | png2c64's `line4` | png2amiga's `line4` | own |
| `line-h8` | `line8` | Lines 8 | Horizontal lines, period 8. | png2c64's `line8` | png2amiga's `line8` | own |
| `line-v2` | `vline2` | Vertical lines 2 | Vertical lines, period 2: costly in multicolor, cheap in hi-res. | native solver (png2c64 lacks it) | png2amiga's `vline2` | own |
| `line-v4` | `vline4` | Vertical lines 4 | Vertical lines, period 4 (ArcPaint64 Vertical 4). | native solver (png2c64 lacks it) | png2amiga's `vline4` | own |
| `line-v8` | `vline8` | Vertical lines 8 | Vertical lines, period 8 (ArcPaint64 Vertical 8). | native solver (png2c64 lacks it) | png2amiga's `vline8` | own |
| `line-checker` | — | Line checker | Lines with a checker offset (ArcPaint64 Horizontal + Checker). | png2c64's `line-checker` | png2amiga's `line-checker` | own |
| `diag-down` | — | Diagonal ↘ | 45° lines falling to the right. | native solver (png2c64 lacks it) | native | own |
| `diag-up` | — | Diagonal ↗ | 45° lines rising to the right. | native solver (png2c64 lacks it) | native | own |
| `diagonal8` | `diagonal8x8` | Diagonal 8×8 | png2c64's 8×8 diagonal clustered dot. | png2c64's `diagonal8` | png2amiga's `diagonal8x8` | own |
| `spiral5` | `spiral5x5` | Spiral 5×5 | png2c64's 5×5 spiral. | png2c64's `spiral5` | png2amiga's `spiral5x5` | own |
| `hex5` | `hex5x5` | Hex 5×5 | png2c64's 5×5 slanted tiling. | png2c64's `hex5` | png2amiga's `hex5x5` | own |
| `hex8` | `hex8x8` | Hex 8×8 | png2c64's 8×8 hexagonal tiling. | png2c64's `hex8` | png2amiga's `hex8x8` | own |
| `bayer3` | `bayer3x3` | Bayer 3×3 | png2amiga's dispersed-dot 3×3: ten levels on a non-power-of-two grid. | png2amiga's threshold biasing png2c64's search | png2amiga's `bayer3x3` | png2amiga's threshold, shifted into 0–1 |
| `bayer5` | `bayer5x5` | Bayer 5×5 | png2amiga's dispersed-dot 5×5. | png2amiga's threshold biasing png2c64's search | png2amiga's `bayer5x5` | png2amiga's threshold, shifted into 0–1 |
| `bayer6` | `bayer6x6` | Bayer 6×6 | png2amiga's dispersed-dot 6×6. | png2amiga's threshold biasing png2c64's search | png2amiga's `bayer6x6` | png2amiga's threshold, shifted into 0–1 |
| `bayer7` | `bayer7x7` | Bayer 7×7 | png2amiga's dispersed-dot 7×7. | png2amiga's threshold biasing png2c64's search | png2amiga's `bayer7x7` | png2amiga's threshold, shifted into 0–1 |
| `v4x2` | — | V 4×2 | A 4×2 vertical-bias matrix, for tall pixels. | png2amiga's threshold biasing png2c64's search | png2amiga's `v4x2` | png2amiga's threshold, shifted into 0–1 |
| `bayer4x2` | — | Bayer 4×2 | Bayer ordering 4 wide, 2 tall. | png2amiga's threshold biasing png2c64's search | png2amiga's `bayer4x2` | png2amiga's threshold, shifted into 0–1 |
| `bayer2x4` | — | Bayer 2×4 | Bayer ordering 2 wide, 4 tall. | png2amiga's threshold biasing png2c64's search | png2amiga's `bayer2x4` | png2amiga's threshold, shifted into 0–1 |
| `line-v-checker` | `vline-checker` | Vertical line checker | Column-biased 2×2: vertical lines with a row shift. | png2amiga's threshold biasing png2c64's search | png2amiga's `vline-checker` | png2amiga's threshold, shifted into 0–1 |
| `fractal16` | — | Fractal 16×16 | Self-nested Bayer recursion: reads at several zoom levels (Obra Dinn). | png2amiga's threshold biasing png2c64's search | png2amiga's `fractal16` | png2amiga's threshold, shifted into 0–1 |
| `aseprite-old` | — | Aseprite old 4×4 | Aseprite's hand-tuned 4×4, checker-biased. | png2amiga's threshold biasing png2c64's search | png2amiga's `aseprite-old` | png2amiga's threshold, shifted into 0–1 |
| `libcaca3` | — | libcaca 3×3 | libcaca's hand-tuned 3×3 dispersed dot. | png2amiga's threshold biasing png2c64's search | png2amiga's `libcaca3` | png2amiga's threshold, shifted into 0–1 |
| `libcaca6` | — | libcaca 6×6 | libcaca's hand-tuned 6×6 dispersed dot. | png2amiga's threshold biasing png2c64's search | png2amiga's `libcaca6` | png2amiga's threshold, shifted into 0–1 |
| `pegasus` | — | Pegasus 8×8 | The Pegasus/REXPaint 8×8 mosaic, centre-biased. | png2amiga's threshold biasing png2c64's search | png2amiga's `pegasus` | png2amiga's threshold, shifted into 0–1 |
| `custom` | — | Custom matrix | Your own 8×8 or 16×16 matrix from a recipe file; Bayer 8×8 without one. | native solver (png2c64 lacks it) | native | own |

### Ordered noise (9)

| Id | Also accepted | Name | What it looks like | Reference (default) | `--solver png2amiga` | `--solver native` |
|---|---|---|---|---|---|---|
| `blue-noise` | — | Blue noise | 64×64 blue-noise tile: the best general-purpose ordered dither for photos. | png2c64's `blue-noise` | png2amiga's `blue-noise` | own |
| `ign` | — | Interleaved gradient | Interleaved gradient noise: cheap, no tile. | png2c64's `ign` | png2amiga's `ign` | own |
| `r2` | `r2-sequence` | R2 sequence | The R2 low-discrepancy sequence: even, no visible tile. | png2c64's `r2` | png2amiga's `r2` | own |
| `white-noise` | — | White noise | Seeded random thresholds: grainy, for when it must look random. | png2c64's `white-noise` | png2amiga's `white-noise` | own |
| `void-cluster` | — | Void and cluster | png2amiga's 64×64 void-and-cluster blue noise: per-rank optimal. | png2amiga's threshold biasing png2c64's search | png2amiga's `void-cluster` | png2amiga's threshold, shifted into 0–1 |
| `cluster-noise` | — | Cluster noise | Blue noise with wider clusters: a coarser, film-grain texture. | png2amiga's threshold biasing png2c64's search | png2amiga's `cluster-noise` | png2amiga's threshold, shifted into 0–1 |
| `cranley-bayer` | — | Cranley–Patterson Bayer | Bayer 8×8 with a hashed offset per tile: a patchwork of phases. | png2amiga's threshold biasing png2c64's search | png2amiga's `cranley-bayer` | png2amiga's threshold, shifted into 0–1 |
| `ign-tri` | — | IGN triangular | Interleaved gradient noise remapped to a triangle: less banding at the ends. | png2amiga's threshold biasing png2c64's search | png2amiga's `ign-tri` | png2amiga's threshold, shifted into 0–1 |
| `r2-tri` | — | R2 triangular | R2 remapped to a triangle (Wronski): even darks and lights. | png2amiga's threshold biasing png2c64's search | png2amiga's `r2-tri` | png2amiga's threshold, shifted into 0–1 |

### Error diffusion (22)

| Id | Also accepted | Name | What it looks like | Reference (default) | `--solver png2amiga` | `--solver native` |
|---|---|---|---|---|---|---|
| `floyd-steinberg` | `fs` | Floyd–Steinberg | The classic 7/3/5/1 error diffusion. | png2c64's `fs` | png2amiga's `floyd-steinberg` | own |
| `false-fs` | — | False Floyd–Steinberg | 3/3/2 diffusion: lighter and faster than Floyd–Steinberg. | native solver (png2c64 lacks it) | native | own |
| `fs-wide` | — | Floyd–Steinberg wide | png2c64's Floyd–Steinberg for 2:1 pixels. | png2c64's `fs-wide` | reference (png2amiga lacks it) | own |
| `line-fs` | — | Line Floyd–Steinberg | png2c64's line-biased diffusion: all error goes down. | png2c64's `line-fs` | reference (png2amiga lacks it) | own |
| `jarvis-judice-ninke` | `jarvis` | Jarvis–Judice–Ninke | Wide 12-tap diffusion: smooth, slightly soft. | png2c64's `jarvis` | png2amiga's `jarvis` | own |
| `stucki` | — | Stucki | Jarvis-like 12-tap diffusion, a little sharper. | native solver (png2c64 lacks it) | png2amiga's `stucki` | own |
| `burkes` | — | Burkes | Two-row Stucki: fast and clean. | native solver (png2c64 lacks it) | native | own |
| `sierra` | `sierra3` | Sierra | Three-row Sierra diffusion. | native solver (png2c64 lacks it) | native | own |
| `sierra2` | — | Sierra two-row | Two-row Sierra diffusion. | native solver (png2c64 lacks it) | native | own |
| `sierra-lite` | — | Sierra Lite | Three-tap Sierra: png2c64's `sierra`. | png2c64's `sierra` | png2amiga's `sierra-lite` | own |
| `atkinson` | — | Atkinson | Diffuses 3/4 of the error: the Mac look, high contrast, good on the C64. | png2c64's `atkinson` | png2amiga's `atkinson` | own |
| `stevenson-arce` | — | Stevenson–Arce | Hexagonal-grid weights: fewer directional artefacts. | native solver (png2c64 lacks it) | native | own |
| `shiau-fan1` | — | Shiau–Fan | Four taps: fewer worms than Floyd–Steinberg. | native solver (png2c64 lacks it) | native | own |
| `shiau-fan2` | — | Shiau–Fan 2 | Five taps: fewer worms still. | native solver (png2c64 lacks it) | native | own |
| `ostromoukhov` | — | Ostromoukhov | Coefficients by input level: the best neutral diffusion. | png2c64's `ostromoukhov` | reference (png2amiga lacks it) | own |
| `zhou-fang` | `zhoufang` | Zhou–Fang | Ostromoukhov plus threshold modulation: removes the remaining worms. | native solver (png2c64 lacks it) | png2amiga's `zhoufang` | own |
| `riemersma` | — | Riemersma | Error carried along a Hilbert curve: no directional artefacts. | native solver (png2c64 lacks it) | png2amiga's `riemersma` | own |
| `floyd-steinberg-oklab` | — | Floyd–Steinberg (OKLab) | Floyd–Steinberg with the residual carried in OKLab: often better on photos, worse on logos. | png2amiga's pass in png2c64's cells | png2amiga's `floyd-steinberg-oklab` | stand-in: `floyd-steinberg` |
| `structure-fs` | — | Structure-aware FS | Floyd–Steinberg with a Laplacian bias on lightness: keeps edges. | png2amiga's pass in png2c64's cells | png2amiga's `structure-fs` | stand-in: `floyd-steinberg` |
| `contrast-fs` | — | Contrast-aware FS | Floyd–Steinberg with a local-contrast bias on lightness. | png2amiga's pass in png2c64's cells | png2amiga's `contrast-fs` | stand-in: `floyd-steinberg` |
| `gilbert` | — | Gilbert curve | png2amiga's Gilbert-curve diffusion. On C64, png2amiga runs this as a nearest pick, so it looks like None. | png2amiga's nearest pick in png2c64's cells (as `none`) | png2amiga's `gilbert` | stand-in: `none` |
| `dbs` | — | Direct binary search | png2amiga's direct binary search. On C64, png2amiga runs this as a nearest pick, so it looks like None. | png2amiga's nearest pick in png2c64's cells (as `none`) | png2amiga's `dbs` | stand-in: `none` |

### Palette-aware (13)

| Id | Also accepted | Name | What it looks like | Reference (default) | `--solver png2amiga` | `--solver native` |
|---|---|---|---|---|---|---|
| `yliluoma1` | — | Yliluoma 1 | The best two-colour mix per pixel and its ratio, placed by Bayer 8×8. | native solver (png2c64 lacks it) | png2amiga's `yliluoma1` | own |
| `yliluoma2` | — | Yliluoma 2 | A 16-entry candidate list per pixel whose running average approaches it, indexed by Bayer 8×8. *Slow* (over 200 ms). | native solver (png2c64 lacks it) | png2amiga's `yliluoma2` | own |
| `yliluoma3` | — | Yliluoma 3 | Knoll-style candidate list in linear light. | native solver (png2c64 lacks it) | native | own |
| `knoll` | — | Knoll pattern | Thomas Knoll's pattern dither (Photoshop's Pattern). | native solver (png2c64 lacks it) | png2amiga's `knoll` | own |
| `luma` | — | Luma | Dithers lightness only, hue from the nearest colour: no colour speckle. | native solver (png2c64 lacks it) | native | own |
| `pair` | — | Luma pair | Weaves two near-luma colours into a third, as a PAL screen blends them. | native solver (png2c64 lacks it) | native | own |
| `yliluoma` | — | Yliluoma (png2amiga) | png2amiga's Yliluoma: a 64-entry mixing plan on Bayer 8. | png2amiga's pass in png2c64's cells | png2amiga's `yliluoma` | stand-in: `yliluoma2` |
| `opt-checker` | — | Optimal checker | The nearest colour and its best partner, as a 2×2 checker. | png2amiga's pass in png2c64's cells | png2amiga's `opt-checker` | stand-in: `pair` |
| `opt-line` | — | Optimal line | The optimal pair alternated by row: CRT-scanline friendly. | png2amiga's pass in png2c64's cells | png2amiga's `opt-line` | stand-in: `pair` |
| `opt-line-checker` | — | Optimal line checker | A 4-colour plan on the line-checker 2×2. | png2amiga's pass in png2c64's cells | png2amiga's `opt-line-checker` | stand-in: `pair` |
| `opt-vline` | — | Optimal vertical line | The optimal pair alternated by column. | png2amiga's pass in png2c64's cells | png2amiga's `opt-vline` | stand-in: `pair` |
| `opt-vline-checker` | — | Optimal vertical line checker | A 4-colour plan on the column-checker 2×2. | png2amiga's pass in png2c64's cells | png2amiga's `opt-vline-checker` | stand-in: `pair` |
| `tri-tone` | — | Tri-tone | Yliluoma's tri-tone: one colour at 50 %, two at 25 %, on Bayer 2×2. | png2amiga's pass in png2c64's cells | png2amiga's `tri-tone` | stand-in: `knoll` |

### Effects (5)

| Id | Also accepted | Name | What it looks like | Reference (default) | `--solver png2amiga` | `--solver native` |
|---|---|---|---|---|---|---|
| `quasicrystal` | — | Quasicrystal | A sum of five cosine waves: an aperiodic shimmer. | png2amiga's threshold biasing png2c64's search | png2amiga's `quasicrystal` | png2amiga's threshold, shifted into 0–1 |
| `truchet` | — | Truchet | Quarter-arc Truchet tiles: a woven texture. | png2amiga's threshold biasing png2c64's search | png2amiga's `truchet` | png2amiga's threshold, shifted into 0–1 and clamped |
| `crosshatch` | — | Crosshatch | png2c64's overlaid angled lines: pen and ink. | png2c64's `crosshatch` | png2amiga's `crosshatch` | own |
| `radial` | — | Radial | Concentric circles around the screen centre: engraving. | png2c64's `radial` | png2amiga's `radial` | own |
| `value-noise` | — | Value noise | Smooth hash noise: organic clumps. | png2c64's `value-noise` | png2amiga's `value-noise` | own |

## 1. Ordered — threshold matrices

Deterministic, parallel, pattern visible. Multicolor pixels are 2:1, so every
matrix exists in a square-pixel form and a 2:1 form (sampled in target space),
and the 2:1 forms are the ones that look right on a real display.

| Id | Matrix | Notes |
|---|---|---|
| `none` | — | nearest-in-set |
| `bayer2` | 2×2 | |
| `bayer4` | 4×4 | the classic |
| `bayer8` | 8×8 | |
| `bayer16` | 16×16 | smooth gradients, large areas |
| `bayer2x2-mc` | 2×2 in 2:1 pixels | png2c64 `bayer2x2` |
| `checker` | 50 % checkerboard | png2c64 default; the scene's workhorse |
| `h2x4` | 2×4 horizontal | png2c64 |
| `clustered4` / `clustered8` | clustered-dot halftone | newspaper look; stable on CRT |
| `halftone8` | 8×8 round dot | png2c64 |
| `dot` | ArcPaint64 "Dot" | |
| `diamond` | ArcPaint64 "Diamond" | |
| `star` | ArcPaint64 "Star" | |
| `line-h2` / `line-h4` / `line-h8` | horizontal lines, period 2/4/8 | png2c64 `line2/4/8`; horizontal lines cost little on 2:1 pixels |
| `line-v2` / `line-v4` / `line-v8` | vertical lines | ArcPaint64 "Vertical 4/8"; expensive on multicolor, cheap on hi-res |
| `line-checker` | lines + checker | png2c64 `line-checker`, ArcPaint64 "Horizontal + Checker" |
| `diag-down` / `diag-up` | 45° lines | ArcPaint64 |
| `diagonal8` | 8×8 diagonal halftone | png2c64 |
| `crosshatch` | | png2c64 |
| `spiral5` | 5×5 spiral | png2c64 |
| `hex5` / `hex8` | hexagonal dot | png2c64 |
| `radial` | | png2c64; an *effect* rather than a quantiser, like `crosshatch` and `value-noise` — group under Effects |
| `custom` | user 8×8 (or 16×16) matrix | from a recipe file or a layer mask used as threshold source (ADR-002 "revisit when") |

## 2. Ordered — noise

| Id | Source | Notes |
|---|---|---|
| `blue-noise` | 64×64 void-and-cluster tile, committed with its seed | best general-purpose ordered dither for photos |
| `ign` | interleaved gradient noise | png2c64; cheap, no tile |
| `r2` | R2 low-discrepancy sequence | png2c64 |
| `white-noise` | seeded | for the "it must look random" cases; seed in the recipe so goldens hold |
| `value-noise` | smooth noise | png2c64 |

## 3. Error diffusion — kernels

Serpentine by default (`serpentine: bool` per recipe). All kernels run in the
one mixed pass with error **clamped at cell boundaries** (TAD §3.1): error that
would cross into a cell lacking the needed colour is dropped. `error_clamp`
(max error per OKLab channel, png2c64 default 0.1) is a recipe parameter so
blow-ups on hard edges are bounded.

| Id | Kernel | Notes |
|---|---|---|
| `fs` | Floyd–Steinberg 7/3/5/1 | |
| `false-fs` | 3/3/2 | lighter, faster |
| `fs-wide` | png2c64's wide FS | |
| `line-fs` | png2c64: FS along lines | |
| `jarvis` | Jarvis–Judice–Ninke | |
| `stucki` | | |
| `burkes` | | |
| `sierra3` / `sierra2` / `sierra-lite` | | |
| `atkinson` | 3/4 of the error propagated | the Mac look; high contrast, good on C64 |
| `stevenson-arce` | hex grid | |
| `shiau-fan1` / `shiau-fan2` | | fewer worms than FS |
| `ostromoukhov` | variable coefficients by input level | png2c64; best "neutral" diffusion |
| `zhou-fang` | Ostromoukhov + threshold modulation | removes the remaining worms |
| `riemersma` | Hilbert-curve path, 16-sample history | no directional artefacts; needs its own traversal in the mixed pass |

## 4. Palette-aware — designed for small fixed palettes

These are the ones a 16-colour machine actually benefits from, and the ones
most converters lack.

| Id | Method | Notes |
|---|---|---|
| `yliluoma1` | Yliluoma's ordered dithering, algorithm 1 | brute-forces the best colour *mix* per threshold position; slow, exact |
| `yliluoma2` | algorithm 2 | faster, per-pixel candidate list |
| `yliluoma3` | algorithm 3 (gamma-aware, knoll-style) | |
| `knoll` | Thomas Knoll pattern dithering (Photoshop's "Pattern"; patent expired) | 8×8 Bayer with candidate list per pixel |
| `luma` | dither on L only, hue from nearest set colour | avoids the colour speckle 16 colours produce |
| `pair` | same-luma pair weaving | the C64 trick: alternate two near-luma colours to fake a third; needs the metric's pair bias; pairs in [`colour-pairs.md`](colour-pairs.md) |

## 5. Modifiers (orthogonal to the method)

| Parameter | Range | Meaning |
|---|---|---|
| `strength` | 0–100 % (png2c64 allows up to 200 %) | threshold / error scale, from the attribute map |
| `adaptive` | 0–1 | reduce dithering in high-detail areas (png2c64 `--adaptive`), measured by local gradient on the super grid |
| `gamma` | 0.1–8 | perceptual lift before thresholding (png2c64 default 1.5); we dither in linear light so the default here is 1.0. Built as the `gamma` layer adjustment (#142) |
| `error_clamp` | per-channel | diffusion only |
| `serpentine` | bool | diffusion only |
| `cell_clamp` | bool, default true | diffusion only: drop error at cell borders (TAD §3.1). `false` = png2c64 behaviour, error flows into neighbours |
| `max_luma_diff` | 0–8 luma levels | restrict any mix to colour pairs within N levels (`colour-pairs.md`); 0 = same-luma only, the PAL-blend-safe setting; 8 = off |
| `phase` | Screen / Layer | ordered only |
| `orientation` | Auto / Vertical-biased | picks the 2:1 or square form of a matrix |

## 6. Cell metrics for the colour-set search (not dithers, listed because png2c64 pairs them)

`mse` (default), `blur` (Pappas–Neuhoff: compare after a small blur — models
the eye at viewing distance, good for charsets/PETSCII), `ssim` (Wang).
These choose cell colour sets; the dither method then places pixels. On the png2amiga
solver (#261) `mse` and `blur` are png2amiga's and choose the colours only of cells
whose method is `none`; png2amiga's own default is `blur`, Convert64's stays `mse`.

## 7. png2amiga's methods on png2c64's pipeline (#261, #275, ADR-014)

png2amiga offers 68 methods for C64; 32 of them are kinds Convert64 did not have before
#261 (below). Jaakko judged each on #280's sheets on png2c64's pipeline
(`docs/reference/dither-previews/png2amiga-methods/`, `cargo xtask png2amiga-methods-sheet`)
and **kept all 32** (ADR-014 §6, #275). They are ordinary catalogue methods now, and the
defaults are unchanged.

**On the reference solver** (the default) they run on png2c64's pipeline
(`solve::png2c64_ref::amiga`): png2c64's front end and png2c64's cell stage, with only the
per-pixel step taken from png2amiga, exactly as the sheets showed them (a test holds a
frame of each to the sheets' hook bit for bit, in both modes). They mix with png2c64's
methods in one frame and keep every reference hook (pins, locks, allowed colours, the
interactive fast path, the divergences). Three ports:

- **Threshold** (the 13 ordered, 5 noise and 2 effect kinds): png2amiga's threshold
  value, exact and unclamped (Truchet's reaches 0.915), used with png2c64's ordered
  semantics as png2c64 uses its own matrices: `t × strength × strength` biases the cell
  search (L by 0.15 t, a and b by 0.03 t) and each pixel is its cell's nearest colour
  to the biased target. Through this port png2amiga's `checker` is png2c64's `checker`
  bit for bit.
- **png2amiga's pass in png2c64's cells** (`floyd-steinberg-oklab`, `structure-fs`,
  `contrast-fs` and the 7 palette-aware kinds): png2c64's cells for a method without a
  threshold (the unbiased search, png2c64's background, then the global Floyd–Steinberg
  replacement), then png2amiga's own per-pixel pass within those cells: its residual in
  gamma-encoded sRGB (OKLab for `floyd-steinberg-oklab`), its error clamp on read only,
  its structure biases, its pickers and plans at the recipe's strength.
- **Nearest pick** (`gilbert`, `dbs`): **on C64, png2amiga runs these as a nearest
  pick**, with no curve diffusion or direct binary search, so there is nothing more to
  port. On png2c64's pipeline that is png2c64's `none` (identical on all seven sheet
  inputs). They are kept because Jaakko kept all 32; their cards say so.

**With `--solver png2amiga`** every method png2amiga offers runs exactly as png2amiga runs
it (its parity test covers all 68 ids on the card, multicolor and hi-res, and the
portrait): png2amiga's cells first (a global Floyd–Steinberg top 3, background black),
then ordered and noise thresholds offset the target (`L + 0.15·t·s`, `a, b + 0.03·t·s`),
diffusion spreads the residual in gamma-encoded sRGB and the palette-aware methods pick
from the cell's 2 or 4 colours. Renders: `dither-previews/png2amiga-card/`,
`png2amiga-card-hires/`, `png2amiga-portrait/`. Kinds Convert64 already had take
png2amiga's algorithm on that solver where the ids match (on the reference they keep
png2c64's or ours): `bayer2/4/8`, `checker`, `h2x4`, `clustered4`, `halftone8`,
`diagonal8`, `spiral5`, `hex5`, `hex8`, `line-h2/4/8`, `line-v2/4/8`, `line-checker`,
`blue-noise`, `ign`, `r2`, `white-noise`, `crosshatch`, `radial` (centred at (160, 128),
as png2amiga), `value-noise`, `floyd-steinberg`, `atkinson`, `jarvis-judice-ninke`,
`stucki` (the last three with png2amiga's OKLab-tuned weights), `sierra-lite`,
`zhou-fang` (png2amiga's: Floyd–Steinberg with hashed noise on L), `riemersma`
(png2amiga's: a 16-deep exponential history in scan order), `yliluoma1`, `yliluoma2`,
`knoll`. Under the png2amiga solver, kinds png2amiga lacks (`fs-wide`, `line-fs`,
`ostromoukhov`) go to the reference; the rest (`bayer16`, `clustered8`, `dot`,
`diamond`, `star`, `diag-*`, `custom`, `false-fs`, `burkes`, `sierra`, `sierra2`,
`stevenson-arce`, `shiau-fan1/2`, `yliluoma3`, `luma`, `pair`) to the native solver.

The 32 kinds (png2amiga's spelling accepted as an alias where it differs):

| Id | Family | Port on the reference | What |
|---|---|---|---|
| `bayer3`, `bayer5`, `bayer6`, `bayer7` (`bayer3x3`…) | Ordered | threshold | dispersed-dot magic-square permutations, non-power-of-2 |
| `v4x2`, `bayer4x2`, `bayer2x4` | Ordered | threshold | non-square matrices |
| `line-v-checker` (`vline-checker`) | Ordered | threshold | column-biased 2 × 2 |
| `fractal16` | Ordered | threshold | 16 × 16 self-nested Bayer (Obra Dinn) |
| `aseprite-old`, `libcaca3`, `libcaca6`, `pegasus` | Ordered | threshold | hand-tuned matrices |
| `void-cluster`, `cluster-noise` | Ordered noise | threshold | 64 × 64 void-and-cluster blue noise, tight or clustered |
| `cranley-bayer` | Ordered noise | threshold | Bayer 8 with a hashed per-tile offset |
| `ign-tri`, `r2-tri` | Ordered noise | threshold | IGN / R2 remapped to a triangular distribution (less banding at the ends) |
| `quasicrystal`, `truchet` | Effects | threshold | sum of cosines; quarter-arc Truchet tiles |
| `floyd-steinberg-oklab` | Error diffusion | pass | the residual carried in OKLab (a/b clamped at half the limit) |
| `structure-fs`, `contrast-fs` | Error diffusion | pass | Floyd–Steinberg with a Laplacian / local-contrast bias on L, gated by the 5 × 5 deviation |
| `gilbert`, `dbs` | Error diffusion | nearest pick | png2amiga's curve diffusion and direct binary search; **on C64, png2amiga runs both as a nearest pick** |
| `yliluoma` | Palette-aware | pass | 64-entry greedy plan on Bayer 8 (png2amiga's plain alg. 2) |
| `opt-checker`, `opt-line`, `opt-vline` | Palette-aware | pass | the nearest colour and its best partner (a separation penalty growing as strength drops), alternated by checker / row / column |
| `opt-line-checker`, `opt-vline-checker` | Palette-aware | pass | a 4-entry plan on the line / column checker |
| `tri-tone` | Palette-aware | pass | Yliluoma's tri-tone: a 4-entry plan on Bayer 2 |

**On the native solver** (`--solver native`, or a frame the reference hands it, e.g. for
a non-OKLab metric or a native knob) the new threshold kinds use png2amiga's thresholds
(shifted into 0–1, Truchet clamped); the others stand in for the nearest native method
(`DitherKind::native`): the Floyd–Steinberg variants for `floyd-steinberg`, `gilbert` and
`dbs` for `none`, `yliluoma` for `yliluoma2`, the optimal pairs and plans for `pair`,
`tri-tone` for `knoll`. Before #275 the reference handed every frame of these kinds to
the native solver, so the stand-ins were what the default showed.

## Implementation order (E5)

Family machinery first (ordered with phase and 2:1 forms, diffusion with
clamping, mixed pass, noise tiles), then the long tail is table entries:
matrices in a `const` table, kernels in a `const` table. Palette-aware methods
(§4) and `riemersma` are separate issues because they change the traversal or
the candidate search. Goldens: one fixture (`gradient`) rendered with every
method, so an accidental change to any matrix fails a test.
