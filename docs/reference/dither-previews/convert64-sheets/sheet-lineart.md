<!-- Written by `cargo xtask lineart-sheet`; do not edit by hand. -->

# Line-art sheet (#273, ADR-014 §4)

[sheet-lineart.png](sheet-lineart.png). #273's line-art recipe: hi-res `none`, gamma 1.5, Lanczos resampling, black point 0.5, cells limited to Pepto's greys (0, 11, 12, 15, 1), on the reference path (png2c64's port and front end). Tile 13 of the cause sheet is the same on png2amiga's port. Since #276 the recipe (`examples/recipes/lineart.toml`) runs on the png2amiga solver and is tile 13: [sheet-lineart-recipe.md](sheet-lineart-recipe.md). Shares are of the 320 × 200 pixels through Pepto; no SSIMULACRA2.

## lineart

| Pair | Identical |
|---|---|
| 4. #273 recipe vs 3. tile 13 | 87.614 % |
| 4. #273 recipe vs 1. png2c64 baseline | 87.064 % |
| 4. #273 recipe vs 2. png2amiga as shipped | 85.458 % |
| 3. tile 13 vs 1. png2c64 baseline | 85.792 % |
| 2. png2amiga as shipped vs 1. png2c64 baseline | 87.862 % |
| 5. lanczos only vs 1. png2c64 baseline | 87.916 % |

Checks: the product pipeline at hi-res `none`, gamma 1.5 reproduces png2c64's baseline on 100.000 % of pixels (the reference) and png2amiga's on 100.000 % (`--solver png2amiga`).

Colours used (index: pixels; `*` not a grey):

- 1. png2c64 baseline: 0: 2050, 1: 57159, 3*: 61, 4*: 8, 6*: 111, 7*: 90, 9*: 68, 10*: 1, 11: 1244, 12: 557, 15: 2651, non-grey 339
- 2. png2amiga as shipped: 0: 136, 1: 53780, 3*: 20, 7*: 223, 9*: 133, 11: 768, 12: 1941, 15: 6999, non-grey 376
- 3. tile 13: 0: 1823, 1: 52551, 11: 7399, 12: 1495, 15: 732, non-grey 0
- 4. #273 recipe: 0: 4203, 1: 52901, 11: 2458, 12: 893, 15: 3545, non-grey 0
- 5. lanczos only: 0: 263, 1: 53518, 3*: 10, 4*: 2, 7*: 60, 10*: 6, 11: 148, 12: 348, 15: 9645, non-grey 78

## logo

A synthetic logo made by the xtask ([lineart-logo-source.png](lineart-logo-source.png), 1024 × 640: a red disc with a black ring, a blue panel with a navy outline, a yellow triangle outlined in black, wavy strokes 2–6 px wide in green, orange, purple, cyan and black, a 3 px black cross). No fixture has coloured outline art.

| Pair | Identical |
|---|---|
| 3. #273 recipe vs 1. png2c64 | 67.244 % |
| 4. recipe, any colour vs 1. png2c64 | 67.384 % |
| 2. png2amiga vs 1. png2c64 | 93.734 % |
| 4. recipe, any colour vs 3. #273 recipe | 95.891 % |
| 5. lanczos only vs 1. png2c64 | 96.833 % |

Colours used (index: pixels; `*` not a grey):

- 1. png2c64: 0: 2307, 1: 41292, 2*: 539, 3*: 176, 4*: 398, 5*: 105, 6*: 8214, 7*: 2109, 8*: 2, 9*: 3, 10*: 8630, 11: 2, 12: 167, 13*: 20, 14*: 35, 15: 1, non-grey 20231
- 2. png2amiga: 0: 1395, 1: 41017, 2*: 1081, 3*: 222, 4*: 105, 6*: 8141, 7*: 2132, 9*: 836, 10*: 7830, 11: 163, 12: 329, 13*: 230, 14*: 128, 15: 391, non-grey 20705
- 3. #273 recipe: 0: 22518, 1: 40857, 11: 33, 12: 61, 15: 531, non-grey 0
- 4. recipe, any colour: 0: 20374, 1: 40792, 2*: 239, 3*: 83, 4*: 2, 5*: 21, 6*: 88, 7*: 18, 8*: 45, 9*: 45, 10*: 2025, 11: 8, 12: 44, 13*: 10, 15: 206, non-grey 2576
- 5. lanczos only: 0: 1925, 1: 41037, 2*: 948, 3*: 514, 4*: 342, 6*: 8117, 7*: 2233, 10*: 8334, 11: 50, 12: 20, 13*: 182, 14*: 16, 15: 282, non-grey 20686
