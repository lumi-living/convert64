<!-- Written by `cargo xtask png2amiga-methods-sheet`; do not edit by hand. -->

# png2amiga's ordered noise methods on png2c64's pipeline (#275)

[sheet-noise.png](sheet-noise.png): each method on png2c64's front end (4-tap Mitchell, gamma 1.5) and cell stage, with png2amiga's threshold pattern or per-pixel pass (`png2c64_ref::amiga::convert`), at png2c64's baseline flags: multicolor, strength 0.7, error clamp 0.1, Pepto, cell metric `mse`; lineart in hi-res, as its baseline is. Share of each tile's pixels identical to the png2c64 baseline (checker 0.7; lineart hi-res `none`), at 320 × 200 through Pepto. No SSIMULACRA2.

| Input | `void-cluster` | `cluster-noise` | `cranley-bayer` | `ign-tri` | `r2-tri` |
|---|---|---|---|---|---|
| portrait | 94.975 % | 94.850 % | 94.950 % | 95.544 % | 95.666 % |
| terrain | 79.953 % | 78.822 % | 79.463 % | 83.484 % | 83.931 % |
| title | 79.372 % | 78.766 % | 79.713 % | 81.978 % | 81.956 % |
| gradient | 90.309 % | 90.094 % | 90.888 % | 91.834 % | 91.812 % |
| photo | 81.544 % | 80.516 % | 81.678 % | 83.588 % | 83.300 % |
| lineart | 91.781 % | 91.936 % | 91.728 % | 91.862 % | 91.828 % |
| testcard | 88.231 % | 87.844 % | 88.091 % | 89.716 % | 89.675 % |

| Id | On png2amiga's C64 path | Here, on png2c64's pipeline | Faithful | png2amiga's own render |
|---|---|---|---|---|
| `void-cluster` | 64 x 64 void-and-cluster blue noise; offsets the target by `t × 0.7` (L 0.15, a and b 0.03) after the cell colours are chosen (global Floyd–Steinberg top 3, background black) | png2amiga's threshold as a png2c64 ordered method: `t × 0.7 × 0.7` biases png2c64's cell search (L by 0.15 t, a and b by 0.03 t), each pixel its cell's nearest colour to the biased target | pattern exact; role and amplitude png2c64's | [card](../png2amiga-card/void-cluster.png) |
| `cluster-noise` | 64 x 64 void-and-cluster, clustered; offsets the target by `t × 0.7` (L 0.15, a and b 0.03) after the cell colours are chosen (global Floyd–Steinberg top 3, background black) | png2amiga's threshold as a png2c64 ordered method: `t × 0.7 × 0.7` biases png2c64's cell search (L by 0.15 t, a and b by 0.03 t), each pixel its cell's nearest colour to the biased target | pattern exact; role and amplitude png2c64's | [card](../png2amiga-card/cluster-noise.png) |
| `cranley-bayer` | Bayer 8 with a hashed offset per 8 x 8 tile; offsets the target by `t × 0.7` (L 0.15, a and b 0.03) after the cell colours are chosen (global Floyd–Steinberg top 3, background black) | png2amiga's threshold as a png2c64 ordered method: `t × 0.7 × 0.7` biases png2c64's cell search (L by 0.15 t, a and b by 0.03 t), each pixel its cell's nearest colour to the biased target | pattern exact; role and amplitude png2c64's | [card](../png2amiga-card/cranley-bayer.png) |
| `ign-tri` | interleaved gradient noise, triangular distribution; offsets the target by `t × 0.7` (L 0.15, a and b 0.03) after the cell colours are chosen (global Floyd–Steinberg top 3, background black) | png2amiga's threshold as a png2c64 ordered method: `t × 0.7 × 0.7` biases png2c64's cell search (L by 0.15 t, a and b by 0.03 t), each pixel its cell's nearest colour to the biased target | pattern exact; role and amplitude png2c64's | [card](../png2amiga-card/ign-tri.png) |
| `r2-tri` | R2 sequence, triangular distribution; offsets the target by `t × 0.7` (L 0.15, a and b 0.03) after the cell colours are chosen (global Floyd–Steinberg top 3, background black) | png2amiga's threshold as a png2c64 ordered method: `t × 0.7 × 0.7` biases png2c64's cell search (L by 0.15 t, a and b by 0.03 t), each pixel its cell's nearest colour to the biased target | pattern exact; role and amplitude png2c64's | [card](../png2amiga-card/r2-tri.png) |

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
