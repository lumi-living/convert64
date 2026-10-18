<!-- Written by `cargo xtask png2amiga-methods-sheet`; do not edit by hand. -->

# png2amiga's error diffusion methods on png2c64's pipeline (#275)

[sheet-diffusion.png](sheet-diffusion.png): each method on png2c64's front end (4-tap Mitchell, gamma 1.5) and cell stage, with png2amiga's threshold pattern or per-pixel pass (`png2c64_ref::amiga::convert`), at png2c64's baseline flags: multicolor, strength 0.7, error clamp 0.1, Pepto, cell metric `mse`; lineart in hi-res, as its baseline is. Share of each tile's pixels identical to the png2c64 baseline (checker 0.7; lineart hi-res `none`), at 320 × 200 through Pepto. No SSIMULACRA2.

| Input | `floyd-steinberg-oklab` | `structure-fs` | `contrast-fs` | `gilbert` | `dbs` |
|---|---|---|---|---|---|
| portrait | 64.763 % | 54.016 % | 53.059 % | 95.144 % | 95.144 % |
| terrain | 59.419 % | 53.781 % | 54.169 % | 80.747 % | 80.747 % |
| title | 52.847 % | 51.356 % | 51.131 % | 77.206 % | 77.206 % |
| gradient | 78.466 % | 78.769 % | 78.744 % | 90.456 % | 90.456 % |
| photo | 59.428 % | 51.191 % | 49.606 % | 71.103 % | 71.103 % |
| lineart | 98.873 % | 98.452 % | 98.381 % | 100.000 % | 100.000 % |
| testcard | 76.156 % | 78.097 % | 78.475 % | 88.412 % | 88.412 % |

The same tiles against png2c64's nearest existing methods at the same flags (`fs`: its Floyd–Steinberg, residual in OKLab clamped on read and write; `none`):

| Input | `floyd-steinberg-oklab` vs `fs` | vs `none` | `structure-fs` vs `fs` | vs `none` | `contrast-fs` vs `fs` | vs `none` | `gilbert` vs `fs` | vs `none` | `dbs` vs `fs` | vs `none` |
|---|---|---|---|---|---|---|---|---|---|---|
| portrait | 64.719 % | 66.481 % | 54.862 % | 55.481 % | 53.897 % | 54.325 % | 66.056 % | 100.000 % | 66.056 % | 100.000 % |
| terrain | 85.825 % | 67.134 % | 59.578 % | 60.000 % | 58.484 % | 60.122 % | 66.753 % | 100.000 % | 66.753 % | 100.000 % |
| title | 67.266 % | 62.725 % | 55.800 % | 60.728 % | 54.694 % | 59.800 % | 62.253 % | 100.000 % | 62.253 % | 100.000 % |
| gradient | 86.644 % | 81.506 % | 78.803 % | 83.566 % | 78.522 % | 83.556 % | 81.347 % | 100.000 % | 81.347 % | 100.000 % |
| photo | 90.306 % | 76.509 % | 63.591 % | 64.675 % | 59.828 % | 61.322 % | 76.097 % | 100.000 % | 76.097 % | 100.000 % |
| lineart | 99.981 % | 98.873 % | 98.125 % | 98.452 % | 97.639 % | 98.381 % | 98.883 % | 100.000 % | 98.883 % | 100.000 % |
| testcard | 79.734 % | 79.084 % | 76.184 % | 81.150 % | 76.400 % | 81.200 % | 78.553 % | 100.000 % | 78.553 % | 100.000 % |

| Id | On png2amiga's C64 path | Here, on png2c64's pipeline | Faithful | png2amiga's own render |
|---|---|---|---|---|
| `floyd-steinberg-oklab` | Floyd-Steinberg with the residual in OKLab (a, b clamped at half the limit); within the cell colours png2amiga chose (global Floyd–Steinberg top 3, background black; hi-res the best-mixing pair) | png2c64's cells for a method without a threshold (unbiased search, then the global Floyd–Steinberg replacement), then png2amiga's pass in them: residual in OKLab, clamped on read (L 0.1, a and b 0.05), × 0.7 | pass exact; cells png2c64's | [card](../png2amiga-card/floyd-steinberg-oklab.png) |
| `structure-fs` | Floyd-Steinberg in sRGB with a Laplacian bias on L, gated by the 5 x 5 deviation; within the cell colours png2amiga chose (global Floyd–Steinberg top 3, background black; hi-res the best-mixing pair) | png2c64's cells for a method without a threshold (unbiased search, then the global Floyd–Steinberg replacement), then png2amiga's pass in them: its L bias, the residual in gamma-encoded sRGB clamped on read at 0.1, × 0.7 | pass exact; cells png2c64's | [card](../png2amiga-card/structure-fs.png) |
| `contrast-fs` | Floyd-Steinberg in sRGB with a local-contrast bias on L, gated by the 5 x 5 deviation; within the cell colours png2amiga chose (global Floyd–Steinberg top 3, background black; hi-res the best-mixing pair) | png2c64's cells for a method without a threshold (unbiased search, then the global Floyd–Steinberg replacement), then png2amiga's pass in them: its L bias, the residual in gamma-encoded sRGB clamped on read at 0.1, × 0.7 | pass exact; cells png2c64's | [card](../png2amiga-card/contrast-fs.png) |
| `gilbert` | no kernel on the C64 path: a plain nearest pick | **not a port of a real algorithm**: png2c64's cells as for `none`, then png2amiga's nearest pick, so it is png2c64's `none` (to within png2amiga's colour maths) | no: nothing to port on C64 | [card](../png2amiga-card/gilbert.png) |
| `dbs` | no kernel on the C64 path: a plain nearest pick | **not a port of a real algorithm**: png2c64's cells as for `none`, then png2amiga's nearest pick, so it is png2c64's `none` (to within png2amiga's colour maths) | no: nothing to port on C64 | [card](../png2amiga-card/dbs.png) |

Check: the baseline tile (png2c64's port) against the committed png2c64 baseline (`fixtures/<name>/baseline.png`; the test card has none at strength 0.7):

| Input | Identical |
|---|---|
| portrait | 100.000 % |
| terrain | 100.000 % |
| title | 99.997 % |
| gradient | 100.000 % |
| photo | 100.000 % |
| lineart | 100.000 % |
| testcard | n/a |
