<!-- Written by `cargo xtask png2amiga-methods-sheet`; do not edit by hand. -->

# png2amiga's effects methods on png2c64's pipeline (#275)

[sheet-effects.png](sheet-effects.png): each method on png2c64's front end (4-tap Mitchell, gamma 1.5) and cell stage, with png2amiga's threshold pattern or per-pixel pass (`png2c64_ref::amiga::convert`), at png2c64's baseline flags: multicolor, strength 0.7, error clamp 0.1, Pepto, cell metric `mse`; lineart in hi-res, as its baseline is. Share of each tile's pixels identical to the png2c64 baseline (checker 0.7; lineart hi-res `none`), at 320 × 200 through Pepto. No SSIMULACRA2.

| Input | `quasicrystal` | `truchet` |
|---|---|---|
| portrait | 95.653 % | 93.709 % |
| terrain | 84.669 % | 76.316 % |
| title | 82.859 % | 75.309 % |
| gradient | 92.169 % | 87.972 % |
| photo | 83.888 % | 76.756 % |
| lineart | 91.678 % | 91.622 % |
| testcard | 89.878 % | 85.581 % |

| Id | On png2amiga's C64 path | Here, on png2c64's pipeline | Faithful | png2amiga's own render |
|---|---|---|---|---|
| `quasicrystal` | sum of five cosines; offsets the target by `t × 0.7` (L 0.15, a and b 0.03) after the cell colours are chosen (global Floyd–Steinberg top 3, background black) | png2amiga's threshold as a png2c64 ordered method: `t × 0.7 × 0.7` biases png2c64's cell search (L by 0.15 t, a and b by 0.03 t), each pixel its cell's nearest colour to the biased target | pattern exact; role and amplitude png2c64's | [card](../png2amiga-card/quasicrystal.png) |
| `truchet` | quarter-arc Truchet tiles, 8 x 8 (threshold up to 0.915); offsets the target by `t × 0.7` (L 0.15, a and b 0.03) after the cell colours are chosen (global Floyd–Steinberg top 3, background black) | png2amiga's threshold as a png2c64 ordered method: `t × 0.7 × 0.7` biases png2c64's cell search (L by 0.15 t, a and b by 0.03 t), each pixel its cell's nearest colour to the biased target | pattern exact; role and amplitude png2c64's | [card](../png2amiga-card/truchet.png) |

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
