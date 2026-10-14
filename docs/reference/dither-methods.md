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

Reference renders of all 30 png2c64 methods are in [`dither-previews/`](dither-previews/README.md) (`sheet-card.png`, `sheet-zoom.png`, `sheet-portrait.png`). Convert64's renders of every id, and a sheet pairing them with png2c64's, are in `dither-previews/convert64-*/`, made by `cargo xtask dither-sheet` (#116).

Every method below honours per-pixel `strength` (0–100 %, scales the threshold
offset or the propagated error), `allowed_set` and cell clamping. Ordered
methods also honour `phase` (`ScreenLocked` keeps the matrix anchored to the
C64 screen so patterns tile across layers; `LayerLocked` anchors it to the
layer so a nudged layer carries its pattern with it).

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
reference (#261) `mse` and `blur` are png2amiga's and choose the colours only of cells
whose method is `none`; png2amiga's own default is `blur`, Convert64's stays `mse`.

## 7. png2amiga's methods (#261, ADR-013)

Since ADR-013 the reference solver is png2amiga's C64 path
([png2amiga-reference.md](png2amiga-reference.md)), and every method it offers for C64
is a kind, solved by the reference exactly as png2amiga solves it (the parity test covers
all 68 of its ids on the card, multicolor and hi-res, and the portrait). On png2amiga's
path **every method runs after the cell colours are chosen**: ordered and noise
thresholds offset the target (`L + 0.15·t·s`, `a, b + 0.03·t·s`), diffusion spreads the
residual in gamma-encoded sRGB, palette-aware methods pick from the cell's 2 or 4 colours.
Renders: `dither-previews/png2amiga-card/`, `png2amiga-card-hires/`,
`png2amiga-portrait/`.

Kinds Convert64 already had take png2amiga's algorithm on the reference path where the
ids match: `bayer2/4/8`, `checker`, `h2x4`, `clustered4`, `halftone8`, `diagonal8`,
`spiral5`, `hex5`, `hex8`, `line-h2/4/8`, `line-v2/4/8`, `line-checker`, `blue-noise`,
`ign`, `r2`, `white-noise`, `crosshatch`, `radial` (centred at (160, 128), as png2amiga),
`value-noise`, `floyd-steinberg`, `atkinson`, `jarvis-judice-ninke`, `stucki` (the last
three with png2amiga's OKLab-tuned weights), `sierra-lite`, `zhou-fang` (png2amiga's:
Floyd–Steinberg with hashed noise on L), `riemersma` (png2amiga's: a 16-deep exponential
history in scan order), `yliluoma1`, `yliluoma2`, `knoll`. Kinds png2amiga lacks
(`fs-wide`, `line-fs`, `ostromoukhov`) go to the png2c64 port; the rest (`bayer16`,
`clustered8`, `dot`, `diamond`, `star`, `diag-*`, `custom`, `false-fs`, `burkes`,
`sierra`, `sierra2`, `stevenson-arce`, `shiau-fan1/2`, `yliluoma3`, `luma`, `pair`) to the
native solver.

New kinds (png2amiga's spelling accepted as an alias where it differs):

| Id | Family | What |
|---|---|---|
| `bayer3`, `bayer5`, `bayer6`, `bayer7` (`bayer3x3`…) | Ordered | dispersed-dot magic-square permutations, non-power-of-2 |
| `v4x2`, `bayer4x2`, `bayer2x4` | Ordered | non-square matrices |
| `line-v-checker` (`vline-checker`) | Ordered | column-biased 2 × 2 |
| `fractal16` | Ordered | 16 × 16 self-nested Bayer (Obra Dinn) |
| `aseprite-old`, `libcaca3`, `libcaca6`, `pegasus` | Ordered | hand-tuned matrices |
| `void-cluster`, `cluster-noise` | Noise | 64 × 64 void-and-cluster blue noise, tight or clustered |
| `cranley-bayer` | Noise | Bayer 8 with a hashed per-tile offset |
| `ign-tri`, `r2-tri` | Noise | IGN / R2 remapped to a triangular distribution (less banding at the ends) |
| `quasicrystal`, `truchet` | Effects | sum of cosines; quarter-arc Truchet tiles |
| `floyd-steinberg-oklab` | Diffusion | the residual carried in OKLab (a/b clamped at half the limit) |
| `structure-fs`, `contrast-fs` | Diffusion | Floyd–Steinberg with a Laplacian / local-contrast bias on L, gated by the 5 × 5 deviation |
| `gilbert`, `dbs` | Diffusion | png2amiga's curve diffusion and direct binary search; **on its C64 path neither has a kernel, so both are a plain nearest pick**, ported as such |
| `yliluoma` | Palette-aware | 64-entry greedy plan on Bayer 8 (png2amiga's plain alg. 2) |
| `opt-checker`, `opt-line`, `opt-vline` | Palette-aware | the nearest colour and its best partner (a separation penalty growing as strength drops), alternated by checker / row / column |
| `opt-line-checker`, `opt-vline-checker` | Palette-aware | a 4-entry plan on the line / column checker |
| `tri-tone` | Palette-aware | Yliluoma's tri-tone: a 4-entry plan on Bayer 2 |

On the native solver (`--solver native`, or a frame it must take) the new threshold
kinds use png2amiga's thresholds (shifted into 0–1, Truchet clamped); the others stand in
for the nearest native method (`DitherKind::native`): the Floyd–Steinberg variants for
`floyd-steinberg`, `gilbert` and `dbs` for `none`, `yliluoma` for `yliluoma2`, the
optimal pairs and plans for `pair`, `tri-tone` for `knoll`.

## Implementation order (E5)

Family machinery first (ordered with phase and 2:1 forms, diffusion with
clamping, mixed pass, noise tiles), then the long tail is table entries:
matrices in a `const` table, kernels in a `const` table. Palette-aware methods
(§4) and `riemersma` are separate issues because they change the traversal or
the candidate search. Goldens: one fixture (`gradient`) rendered with every
method, so an accidental change to any matrix fails a test.
