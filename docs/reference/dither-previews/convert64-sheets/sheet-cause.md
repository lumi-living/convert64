<!-- Written by `cargo xtask cause-sheet`; do not edit by hand. -->

# Cause sheet (#273)

Share of each tile's pixels identical to the png2c64 baseline (tile 1), at 320 × 200 through Pepto ([sheet-cause.png](sheet-cause.png)). Every tile at png2c64's baseline flags unless named: multicolor checker 0.7, error clamp 0.1, gamma 1.5, Pepto, cell metric `mse`; lineart hi-res `none`, gamma 1.5. In brackets, the multicolor background the tile used. No SSIMULACRA2.

| Fixture | 2. png2amiga as shipped (#273) | 3. png2amiga port, equal flags | 4. 3 + background search | 5. 4 + png2c64 mitchell | 6. png2c64 port + png2amiga lanczos | 7. 4 + png2c64 colour maths | 8. 4 + png2c64 cell colours | 9. 4 + png2c64 ordered amplitude | 10. 8 + png2c64 ordered amplitude | 11. 5 + 8 + 9: all three swaps | 12. 4 + greys only (allowed colours) | 13. 12 + black point 0.5 |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| portrait | 94.394 % | 94.394 % (0) | 94.659 % (9) | 97.000 % (9) | 95.688 % (9) | 94.647 % (9) | 95.194 % (9) | 95.172 % (9) | 95.688 % (9) | 100.000 % (9) | n/a | n/a |
| terrain | 63.263 % | 63.263 % (0) | 63.719 % (15) | 82.866 % (9) | 69.569 % (15) | 63.709 % (15) | 67.628 % (15) | 65.966 % (15) | 69.569 % (15) | 100.000 % (9) | n/a | n/a |
| title | 73.856 % | 73.856 % (0) | 74.853 % (11) | 81.597 % (11) | 87.600 % (11) | 75.016 % (11) | 85.144 % (11) | 75.791 % (11) | 87.600 % (11) | 99.997 % (11) | n/a | n/a |
| gradient | 91.316 % | 91.316 % (0) | 91.388 % (9) | 92.359 % (11) | 98.091 % (9) | 91.400 % (9) | 96.531 % (9) | 93.256 % (9) | 98.091 % (9) | 100.000 % (12) | n/a | n/a |
| photo | 55.697 % | 55.697 % (0) | 56.500 % (9) | 73.569 % (15) | 66.181 % (9) | 56.303 % (9) | 65.094 % (9) | 56.634 % (9) | 66.181 % (9) | 100.000 % (15) | n/a | n/a |
| lineart | 87.862 % | 87.862 % | 87.862 % | 91.930 % | 87.916 % | 87.862 % | 87.914 % | 87.862 % | 87.914 % | 99.886 % | 88.050 % | 85.792 % |

Checks: tile 3 against tile 2 (png2amiga's port against png2amiga's own render), png2c64's port as it is (Mitchell, its front end and encoder) against tile 1, and the background png2c64's search chose:

| Fixture | tile 3 vs tile 2 | png2c64 port vs tile 1 | png2c64's background |
|---|---|---|---|
| portrait | 100.000 % | 100.000 % | 9 |
| terrain | 100.000 % | 100.000 % | 9 |
| title | 100.000 % | 99.997 % | 11 |
| gradient | 100.000 % | 100.000 % | 12 |
| photo | 100.000 % | 100.000 % | 15 |
| lineart | 100.000 % | 100.000 % | none (hi-res) |

## Lineart

[sheet-cause-lineart.png](sheet-cause-lineart.png): pixels (24, 16)–(159, 111) of 320 × 200, magnified 3×.

Prepared image (OKLab L of the 320 × 200 picture the encoder sees), pixels per tenth of L:

| Front end | 0.0 | 0.1 | 0.2 | 0.3 | 0.4 | 0.5 | 0.6 | 0.7 | 0.8 | 0.9 | 1.0 |
|---|---|---|---|---|---|---|---|---|---|---|---|
| png2amiga Lanczos | 319 | 79 | 140 | 303 | 567 | 1016 | 2228 | 3421 | 3148 | 3721 | 49058 |
| png2c64 Mitchell | 939 | 426 | 771 | 1068 | 983 | 906 | 878 | 1070 | 1316 | 2035 | 53608 |

Pixels per colour in each tile (Pepto's greys by lightness: 0 black L 0.00, 11 L 0.39, 12 L 0.53, 15 L 0.67, 1 white L 1.00; the others that appear: 3 L 0.69 chroma 0.059, 7 L 0.80 chroma 0.114, 9 L 0.34 chroma 0.071, 13 L 0.81 chroma 0.120):

| Tile | 0 | 11 | 12 | 15 | 1 | other |
|---|---|---|---|---|---|---|
| 1. png2c64 baseline | 2050 | 1244 | 557 | 2651 | 57159 | 3: 61, 4: 8, 6: 111, 7: 90, 9: 68, 10: 1 |
| 2. png2amiga as shipped (#273) | 136 | 768 | 1941 | 6999 | 53780 | 3: 20, 7: 223, 9: 133 |
| 3. png2amiga port, equal flags | 136 | 768 | 1941 | 6999 | 53780 | 3: 20, 7: 223, 9: 133 |
| 4. 3 + background search | 136 | 768 | 1941 | 6999 | 53780 | 3: 20, 7: 223, 9: 133 |
| 5. 4 + png2c64 mitchell | 734 | 3299 | 1097 | 135 | 57456 | 7: 8, 9: 1271 |
| 6. png2c64 port + png2amiga lanczos | 263 | 148 | 348 | 9645 | 53518 | 3: 10, 4: 2, 7: 60, 10: 6 |
| 7. 4 + png2c64 colour maths | 136 | 768 | 1941 | 6999 | 53780 | 3: 20, 7: 223, 9: 133 |
| 8. 4 + png2c64 cell colours | 272 | 98 | 354 | 9700 | 53503 | 3: 10, 4: 2, 7: 58, 10: 3 |
| 9. 4 + png2c64 ordered amplitude | 136 | 768 | 1941 | 6999 | 53780 | 3: 20, 7: 223, 9: 133 |
| 10. 8 + png2c64 ordered amplitude | 272 | 98 | 354 | 9700 | 53503 | 3: 10, 4: 2, 7: 58, 10: 3 |
| 11. 5 + 8 + 9: all three swaps | 2068 | 1252 | 549 | 2625 | 57167 | 3: 60, 4: 8, 6: 112, 7: 90, 9: 68, 10: 1 |
| 12. 4 + greys only (allowed colours) | 143 | 877 | 2005 | 7088 | 53887 | none |
| 13. 12 + black point 0.5 | 1823 | 7399 | 1495 | 732 | 52551 | none |

Every cell png2amiga (tile 3) gives a colour that is not a grey, with its pair, png2c64's and tile 5's, and the prepared cell's darkest L / mean L of its line pixels (L < 0.97; their count) under each scaler:

| Cell | tile 3 pair | tile 1 (png2c64) pair | tile 5 (Mitchell) pair | Lanczos L min / line mean (n) | Mitchell L min / line mean (n) |
|---|---|---|---|---|---|
| (17, 2) | [1, 7] | [1, 0] | [0, 1] | 0.83 / 0.83 (1) | 0.96 / 0.96 (1) |
| (5, 3) | [1, 7] | [1, 0] | [0, 1] | 0.82 / 0.87 (2) | 0.96 / 0.96 (1) |
| (12, 3) | [1, 7] | [1, 15] | [1, 15] | 0.78 / 0.88 (14) | 0.61 / 0.80 (7) |
| (28, 3) | [1, 7] | [1, 0] | [0, 1] | 0.84 / 0.84 (1) | 0.97 / 1.00 (0) |
| (8, 4) | [1, 7] | [1, 15] | [1, 15] | 0.73 / 0.84 (29) | 0.62 / 0.88 (14) |
| (10, 4) | [1, 3] | [1, 12] | [1, 11] | 0.71 / 0.84 (35) | 0.32 / 0.72 (16) |
| (21, 8) | [1, 9] | [1, 0] | [0, 1] | 0.00 / 0.59 (34) | 0.00 / 0.52 (29) |
| (19, 9) | [1, 9] | [1, 0] | [0, 1] | 0.00 / 0.51 (46) | 0.00 / 0.35 (38) |
| (22, 9) | [1, 9] | [1, 0] | [0, 1] | 0.00 / 0.56 (46) | 0.00 / 0.42 (39) |
| (35, 9) | [1, 7] | [1, 15] | [1, 15] | 0.84 / 0.90 (2) | 0.71 / 0.71 (1) |
| (26, 10) | [0, 7] | [1, 0] | [1, 9] | 0.00 / 0.42 (43) | 0.00 / 0.34 (40) |
| (28, 10) | [1, 7] | [1, 0] | [0, 1] | 0.86 / 0.86 (1) | 0.98 / 1.00 (0) |
| (27, 11) | [0, 7] | [1, 0] | [0, 1] | 0.00 / 0.46 (44) | 0.00 / 0.36 (40) |
| (6, 12) | [1, 9] | [1, 0] | [0, 1] | 0.00 / 0.43 (21) | 0.00 / 0.14 (16) |
| (7, 12) | [1, 9] | [1, 0] | [0, 1] | 0.00 / 0.47 (27) | 0.00 / 0.44 (27) |
| (9, 12) | [1, 7] | [1, 0] | [0, 1] | 0.84 / 0.90 (3) | 0.95 / 0.95 (1) |
| (10, 12) | [1, 7] | [1, 0] | [0, 1] | 0.81 / 0.88 (6) | 0.94 / 0.94 (1) |
| (10, 14) | [7, 11] | [1, 0] | [1, 9] | 0.00 / 0.66 (58) | 0.00 / 0.46 (42) |
| (17, 20) | [7, 11] | [1, 0] | [1, 11] | 0.07 / 0.70 (56) | 0.00 / 0.47 (35) |
| (23, 21) | [1, 9] | [1, 0] | [0, 1] | 0.00 / 0.46 (12) | 0.00 / 0.34 (11) |
| (7, 22) | [1, 9] | [1, 0] | [0, 1] | 0.00 / 0.43 (25) | 0.00 / 0.14 (19) |
| (28, 22) | [1, 9] | [1, 0] | [0, 1] | 0.00 / 0.50 (20) | 0.00 / 0.30 (16) |
