<!-- Written by `cargo xtask png2amiga-methods-sheet`; do not edit by hand. -->

# png2amiga's ordered methods on png2c64's pipeline (#275)

[sheet-ordered.png](sheet-ordered.png): each method on png2c64's front end (4-tap Mitchell, gamma 1.5) and cell stage, with png2amiga's threshold pattern or per-pixel pass (`png2c64_ref::amiga::convert`), at png2c64's baseline flags: multicolor, strength 0.7, error clamp 0.1, Pepto, cell metric `mse`; lineart in hi-res, as its baseline is. Share of each tile's pixels identical to the png2c64 baseline (checker 0.7; lineart hi-res `none`), at 320 × 200 through Pepto. No SSIMULACRA2.

| Input | `bayer3` | `bayer5` | `bayer6` | `bayer7` | `v4x2` | `bayer4x2` | `bayer2x4` | `line-v-checker` | `fractal16` | `aseprite-old` | `libcaca3` | `libcaca6` | `pegasus` |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| portrait | 95.081 % | 95.019 % | 96.803 % | 94.919 % | 96.634 % | 97.869 % | 97.994 % | 95.156 % | 94.941 % | 97.787 % | 95.037 % | 97.872 % | 96.756 % |
| terrain | 79.703 % | 80.022 % | 86.753 % | 79.862 % | 88.759 % | 89.159 % | 89.550 % | 81.222 % | 78.850 % | 89.909 % | 79.775 % | 89.456 % | 85.047 % |
| title | 79.638 % | 79.431 % | 88.188 % | 79.519 % | 86.509 % | 91.684 % | 91.737 % | 80.200 % | 79.819 % | 91.753 % | 79.716 % | 91.875 % | 86.103 % |
| gradient | 90.716 % | 90.559 % | 94.431 % | 90.559 % | 94.397 % | 96.416 % | 96.244 % | 89.806 % | 90.425 % | 96.006 % | 90.741 % | 96.013 % | 93.372 % |
| photo | 81.200 % | 81.747 % | 88.772 % | 81.737 % | 87.713 % | 92.209 % | 92.084 % | 82.122 % | 81.053 % | 92.309 % | 81.444 % | 92.250 % | 87.112 % |
| lineart | 91.830 % | 91.808 % | 91.789 % | 91.812 % | 91.722 % | 91.713 % | 91.700 % | 91.789 % | 91.759 % | 91.706 % | 91.844 % | 91.695 % | 91.772 % |
| testcard | 88.412 % | 88.594 % | 93.772 % | 88.362 % | 92.166 % | 94.950 % | 95.444 % | 88.475 % | 88.344 % | 95.116 % | 88.609 % | 95.122 % | 92.091 % |

| Id | On png2amiga's C64 path | Here, on png2c64's pipeline | Faithful | png2amiga's own render |
|---|---|---|---|---|
| `bayer3` | dispersed-dot 3 x 3 (magic-square permutation); offsets the target by `t × 0.7` (L 0.15, a and b 0.03) after the cell colours are chosen (global Floyd–Steinberg top 3, background black) | png2amiga's threshold as a png2c64 ordered method: `t × 0.7 × 0.7` biases png2c64's cell search (L by 0.15 t, a and b by 0.03 t), each pixel its cell's nearest colour to the biased target | pattern exact; role and amplitude png2c64's | [card](../png2amiga-card/bayer3x3.png) |
| `bayer5` | dispersed-dot 5 x 5 (magic-square permutation); offsets the target by `t × 0.7` (L 0.15, a and b 0.03) after the cell colours are chosen (global Floyd–Steinberg top 3, background black) | png2amiga's threshold as a png2c64 ordered method: `t × 0.7 × 0.7` biases png2c64's cell search (L by 0.15 t, a and b by 0.03 t), each pixel its cell's nearest colour to the biased target | pattern exact; role and amplitude png2c64's | [card](../png2amiga-card/bayer5x5.png) |
| `bayer6` | dispersed-dot 6 x 6 (magic-square permutation); offsets the target by `t × 0.7` (L 0.15, a and b 0.03) after the cell colours are chosen (global Floyd–Steinberg top 3, background black) | png2amiga's threshold as a png2c64 ordered method: `t × 0.7 × 0.7` biases png2c64's cell search (L by 0.15 t, a and b by 0.03 t), each pixel its cell's nearest colour to the biased target | pattern exact; role and amplitude png2c64's | [card](../png2amiga-card/bayer6x6.png) |
| `bayer7` | dispersed-dot 7 x 7 (magic-square permutation); offsets the target by `t × 0.7` (L 0.15, a and b 0.03) after the cell colours are chosen (global Floyd–Steinberg top 3, background black) | png2amiga's threshold as a png2c64 ordered method: `t × 0.7 × 0.7` biases png2c64's cell search (L by 0.15 t, a and b by 0.03 t), each pixel its cell's nearest colour to the biased target | pattern exact; role and amplitude png2c64's | [card](../png2amiga-card/bayer7x7.png) |
| `v4x2` | 4 x 2 literal matrix; offsets the target by `t × 0.7` (L 0.15, a and b 0.03) after the cell colours are chosen (global Floyd–Steinberg top 3, background black) | png2amiga's threshold as a png2c64 ordered method: `t × 0.7 × 0.7` biases png2c64's cell search (L by 0.15 t, a and b by 0.03 t), each pixel its cell's nearest colour to the biased target | pattern exact; role and amplitude png2c64's | [card](../png2amiga-card/v4x2.png) |
| `bayer4x2` | 4 x 2 Bayer; offsets the target by `t × 0.7` (L 0.15, a and b 0.03) after the cell colours are chosen (global Floyd–Steinberg top 3, background black) | png2amiga's threshold as a png2c64 ordered method: `t × 0.7 × 0.7` biases png2c64's cell search (L by 0.15 t, a and b by 0.03 t), each pixel its cell's nearest colour to the biased target | pattern exact; role and amplitude png2c64's | [card](../png2amiga-card/bayer4x2.png) |
| `bayer2x4` | 2 x 4 Bayer; offsets the target by `t × 0.7` (L 0.15, a and b 0.03) after the cell colours are chosen (global Floyd–Steinberg top 3, background black) | png2amiga's threshold as a png2c64 ordered method: `t × 0.7 × 0.7` biases png2c64's cell search (L by 0.15 t, a and b by 0.03 t), each pixel its cell's nearest colour to the biased target | pattern exact; role and amplitude png2c64's | [card](../png2amiga-card/bayer2x4.png) |
| `line-v-checker` | column-biased 2 x 2 (`vline-checker`); offsets the target by `t × 0.7` (L 0.15, a and b 0.03) after the cell colours are chosen (global Floyd–Steinberg top 3, background black) | png2amiga's threshold as a png2c64 ordered method: `t × 0.7 × 0.7` biases png2c64's cell search (L by 0.15 t, a and b by 0.03 t), each pixel its cell's nearest colour to the biased target | pattern exact; role and amplitude png2c64's | [card](../png2amiga-card/vline-checker.png) |
| `fractal16` | 16 x 16 self-nested Bayer (Obra Dinn); offsets the target by `t × 0.7` (L 0.15, a and b 0.03) after the cell colours are chosen (global Floyd–Steinberg top 3, background black) | png2amiga's threshold as a png2c64 ordered method: `t × 0.7 × 0.7` biases png2c64's cell search (L by 0.15 t, a and b by 0.03 t), each pixel its cell's nearest colour to the biased target | pattern exact; role and amplitude png2c64's | [card](../png2amiga-card/fractal16.png) |
| `aseprite-old` | Aseprite's old 4 x 4; offsets the target by `t × 0.7` (L 0.15, a and b 0.03) after the cell colours are chosen (global Floyd–Steinberg top 3, background black) | png2amiga's threshold as a png2c64 ordered method: `t × 0.7 × 0.7` biases png2c64's cell search (L by 0.15 t, a and b by 0.03 t), each pixel its cell's nearest colour to the biased target | pattern exact; role and amplitude png2c64's | [card](../png2amiga-card/aseprite-old.png) |
| `libcaca3` | libcaca's 3 x 3; offsets the target by `t × 0.7` (L 0.15, a and b 0.03) after the cell colours are chosen (global Floyd–Steinberg top 3, background black) | png2amiga's threshold as a png2c64 ordered method: `t × 0.7 × 0.7` biases png2c64's cell search (L by 0.15 t, a and b by 0.03 t), each pixel its cell's nearest colour to the biased target | pattern exact; role and amplitude png2c64's | [card](../png2amiga-card/libcaca3.png) |
| `libcaca6` | libcaca's 6 x 6; offsets the target by `t × 0.7` (L 0.15, a and b 0.03) after the cell colours are chosen (global Floyd–Steinberg top 3, background black) | png2amiga's threshold as a png2c64 ordered method: `t × 0.7 × 0.7` biases png2c64's cell search (L by 0.15 t, a and b by 0.03 t), each pixel its cell's nearest colour to the biased target | pattern exact; role and amplitude png2c64's | [card](../png2amiga-card/libcaca6.png) |
| `pegasus` | Pegasus' 8 x 8; offsets the target by `t × 0.7` (L 0.15, a and b 0.03) after the cell colours are chosen (global Floyd–Steinberg top 3, background black) | png2amiga's threshold as a png2c64 ordered method: `t × 0.7 × 0.7` biases png2c64's cell search (L by 0.15 t, a and b by 0.03 t), each pixel its cell's nearest colour to the biased target | pattern exact; role and amplitude png2c64's | [card](../png2amiga-card/pegasus.png) |

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
