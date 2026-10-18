<!-- Written by `cargo xtask lineart-sheet`; do not edit by hand. -->

# Line-art recipe sheet (#276)

[sheet-lineart-recipe.png](sheet-lineart-recipe.png): the source, the cause sheet's tile 13 (Jaakko's choice), the new line-art recipe (`examples/recipes/lineart.toml`: hi-res `none`, gamma 1.5, black point 0.5, cells limited to Pepto's greys 0, 11, 12, 15, 1, on `--solver png2amiga`) and the old one (#273: the same flags on the reference path), each labelled with its share of the 320 × 200 pixels identical to tile 13. The logo row is the line-art sheet's synthetic logo through the same: the recipe is for black-and-white line art only, since it fills coloured shapes black. No SSIMULACRA2.

| Picture | new recipe vs tile 13 | old recipe vs tile 13 |
|---|---|---|
| lineart | 100.000 % | 87.614 % |
| logo (tile 13's settings on the port) | 100.000 % | 94.402 % |

Checks: tile 13 rebuilt here from the ports is 100.000 % identical to the cause sheet's; the recipe's golden (`fixtures/lineart/expected_lineart-recipe.png`) is 100.000 % identical to tile 13.

## What tile 13 does that #273's recipe did not

Each row puts #273's choice into tile 13 alone, on lineart, and gives the result's share of pixels identical to tile 13. Rows marked *for scale* change something the two share, to show how much it weighs. Only the cell pair matters: png2c64's global Floyd–Steinberg replacement is the whole gap.

| Difference | Tile 13 (png2amiga's port) | #273's recipe (png2c64 path) | Tile 13 with #273's, identical to tile 13 |
|---|---|---|---|
| Solver: cell pair (hi-res `none`) | png2amiga's port: brute force of the 120 pairs by the cell metric, each pixel the nearer of the two | png2c64's port: the same 120-pair search, then its global Floyd–Steinberg replacement (each cell's two most used colours, within the greys, of an FS dither of the whole picture against all 16) | 87.541 % |
| Solver: png2c64's search alone | (as above) | png2c64's 120-pair search without the FS replacement: not a png2c64 option, measured to locate the difference | 100.000 % |
| Front end: scaler and preprocessing | png2amiga's `prepare`: its widened Lanczos-3, then gamma, black point and the OKLab round trip in png2amiga's maths (fast cube root, fused multiply-adds) | the compositor on the png2c64 pipeline: the same widened Lanczos-3 (`Resample::Lanczos`), then png2c64's gamma, black point and round trip (correctly rounded cube root, no fma) | 100.000 % |
| Front end: preprocessing maths alone | png2amiga's `preprocess::apply` after png2amiga's scaler | png2c64's `preprocess::apply` (the same steps in the same order) after png2amiga's scaler | 100.000 % |
| Colour maths everywhere | png2amiga's OKLab in the front end and the encoder | png2c64's OKLab in both (`PNG2C64_MATHS`) | 100.000 % |
| Black point (for scale: off) | 0.5, after the gamma and before the round trip: OKLab L ← clamp((L − 0.5) / (1 − 0.5), 0, 1), png2amiga's `apply_levels` | 0.5, same place, same formula: png2c64's `apply_levels` (`levels_px`); only the maths differ (row above) | 82.513 % |
| Grey limit (for scale: off) | every cell limited to 0, 11, 12, 15, 1 (`CellRule`) | the same indices (the recipe's allowed colours, as `CellRule`s) | 95.691 % |
| Cell metric (for scale: `blur`) | `mse`: each pixel's squared OKLab distance to the nearer of the pair | `mse` (png2c64 has no other); png2amiga's own default is `blur` | 92.086 % |
| Gamma (for scale: 1) | 1.5 (png2c64's default) | 1.5; png2amiga's own default is 1 | 86.714 % |
| Error clamp | 0.35 (png2amiga's default) | none (a recipe's 0); `none` diffuses no error | 100.000 % |
| Front end and solver together | (tile 13) | #273's front end and png2c64's cell stage: #273's recipe rebuilt from the ports | 87.614 % |
| #273's front end, png2c64's search alone | (tile 13) | what the png2c64 path would need to come close: a new divergence that skips the FS replacement | 100.000 % |
| The new recipe's frame, png2amiga's encoder | (tile 13) | the compositor on `Pipeline::Png2amiga` (what `--solver png2amiga` solves) | 100.000 % |
