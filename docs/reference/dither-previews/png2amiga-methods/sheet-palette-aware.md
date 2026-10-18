<!-- Written by `cargo xtask png2amiga-methods-sheet`; do not edit by hand. -->

# png2amiga's palette-aware methods on png2c64's pipeline (#275)

[sheet-palette-aware.png](sheet-palette-aware.png): each method on png2c64's front end (4-tap Mitchell, gamma 1.5) and cell stage, with png2amiga's threshold pattern or per-pixel pass (`png2c64_ref::amiga::convert`), at png2c64's baseline flags: multicolor, strength 0.7, error clamp 0.1, Pepto, cell metric `mse`; lineart in hi-res, as its baseline is. Share of each tile's pixels identical to the png2c64 baseline (checker 0.7; lineart hi-res `none`), at 320 × 200 through Pepto. No SSIMULACRA2.

| Input | `yliluoma` | `opt-checker` | `opt-line` | `opt-line-checker` | `opt-vline` | `opt-vline-checker` | `tri-tone` |
|---|---|---|---|---|---|---|---|
| portrait | 56.944 % | 57.288 % | 54.394 % | 52.619 % | 54.531 % | 52.634 % | 55.212 % |
| terrain | 56.572 % | 69.456 % | 58.169 % | 60.494 % | 57.888 % | 60.306 % | 69.987 % |
| title | 53.303 % | 60.234 % | 51.416 % | 49.619 % | 51.541 % | 49.816 % | 57.397 % |
| gradient | 80.034 % | 83.144 % | 78.250 % | 76.862 % | 78.053 % | 77.219 % | 81.878 % |
| photo | 56.500 % | 63.519 % | 56.781 % | 55.594 % | 56.788 % | 55.591 % | 61.625 % |
| lineart | 98.103 % | 97.781 % | 97.862 % | 98.097 % | 97.862 % | 98.064 % | 98.014 % |
| testcard | 80.356 % | 83.053 % | 75.719 % | 75.584 % | 75.572 % | 75.684 % | 82.150 % |

| Id | On png2amiga's C64 path | Here, on png2c64's pipeline | Faithful | png2amiga's own render |
|---|---|---|---|---|
| `yliluoma` | 64-entry greedy mixing plan on Bayer 8 (Yliluoma's alg. 2, plain); within the cell colours png2amiga chose (global Floyd–Steinberg top 3, background black; hi-res the best-mixing pair) | png2c64's cells for a method without a threshold (unbiased search, then the global Floyd–Steinberg replacement), then png2amiga's picker at strength 0.7 on each pixel's own colour, within the cell's colours | picker exact; cells png2c64's | [card](../png2amiga-card/yliluoma.png) |
| `opt-checker` | nearest colour and its best partner, alternated by checker; within the cell colours png2amiga chose (global Floyd–Steinberg top 3, background black; hi-res the best-mixing pair) | png2c64's cells for a method without a threshold (unbiased search, then the global Floyd–Steinberg replacement), then png2amiga's picker at strength 0.7 on each pixel's own colour, within the cell's colours | picker exact; cells png2c64's | [card](../png2amiga-card/opt-checker.png) |
| `opt-line` | nearest colour and its best partner, alternated by row; within the cell colours png2amiga chose (global Floyd–Steinberg top 3, background black; hi-res the best-mixing pair) | png2c64's cells for a method without a threshold (unbiased search, then the global Floyd–Steinberg replacement), then png2amiga's picker at strength 0.7 on each pixel's own colour, within the cell's colours | picker exact; cells png2c64's | [card](../png2amiga-card/opt-line.png) |
| `opt-line-checker` | 4-entry plan on the line-checker matrix; within the cell colours png2amiga chose (global Floyd–Steinberg top 3, background black; hi-res the best-mixing pair) | png2c64's cells for a method without a threshold (unbiased search, then the global Floyd–Steinberg replacement), then png2amiga's picker at strength 0.7 on each pixel's own colour, within the cell's colours | picker exact; cells png2c64's | [card](../png2amiga-card/opt-line-checker.png) |
| `opt-vline` | nearest colour and its best partner, alternated by column; within the cell colours png2amiga chose (global Floyd–Steinberg top 3, background black; hi-res the best-mixing pair) | png2c64's cells for a method without a threshold (unbiased search, then the global Floyd–Steinberg replacement), then png2amiga's picker at strength 0.7 on each pixel's own colour, within the cell's colours | picker exact; cells png2c64's | [card](../png2amiga-card/opt-vline.png) |
| `opt-vline-checker` | 4-entry plan on the column checker; within the cell colours png2amiga chose (global Floyd–Steinberg top 3, background black; hi-res the best-mixing pair) | png2c64's cells for a method without a threshold (unbiased search, then the global Floyd–Steinberg replacement), then png2amiga's picker at strength 0.7 on each pixel's own colour, within the cell's colours | picker exact; cells png2c64's | [card](../png2amiga-card/opt-vline-checker.png) |
| `tri-tone` | Yliluoma's tri-tone: 4-entry plan on Bayer 2; within the cell colours png2amiga chose (global Floyd–Steinberg top 3, background black; hi-res the best-mixing pair) | png2c64's cells for a method without a threshold (unbiased search, then the global Floyd–Steinberg replacement), then png2amiga's picker at strength 0.7 on each pixel's own colour, within the cell's colours | picker exact; cells png2c64's | [card](../png2amiga-card/tri-tone.png) |

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
