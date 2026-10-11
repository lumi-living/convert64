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

Reference renders of all 30 png2c64 methods are in [`dither-previews/`](dither-previews/README.md) (`sheet-card.png`, `sheet-zoom.png`, `sheet-portrait.png`).

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
| `pair` | same-luma pair weaving | the C64 trick: alternate two near-luma colours to fake a third; needs the metric's pair bias |

## 5. Modifiers (orthogonal to the method)

| Parameter | Range | Meaning |
|---|---|---|
| `strength` | 0–100 % (png2c64 allows up to 200 %) | threshold / error scale, from the attribute map |
| `adaptive` | 0–1 | reduce dithering in high-detail areas (png2c64 `--adaptive`), measured by local gradient on the super grid |
| `gamma` | 0.1–8 | perceptual lift before thresholding (png2c64 default 1.5); we dither in linear light so the default here is 1.0 |
| `error_clamp` | per-channel | diffusion only |
| `serpentine` | bool | diffusion only |
| `phase` | Screen / Layer | ordered only |
| `orientation` | Auto / Vertical-biased | picks the 2:1 or square form of a matrix |

## 6. Cell metrics for the colour-set search (not dithers, listed because png2c64 pairs them)

`mse` (default), `blur` (Pappas–Neuhoff: compare after a small blur — models
the eye at viewing distance, good for charsets/PETSCII), `ssim` (Wang).
These choose cell colour sets; the dither method then places pixels.

## Implementation order (E5)

Family machinery first (ordered with phase and 2:1 forms, diffusion with
clamping, mixed pass, noise tiles), then the long tail is table entries:
matrices in a `const` table, kernels in a `const` table. Palette-aware methods
(§4) and `riemersma` are separate issues because they change the traversal or
the candidate search. Goldens: one fixture (`gradient`) rendered with every
method, so an accidental change to any matrix fails a test.
