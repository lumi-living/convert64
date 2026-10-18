<!-- Written by `cargo xtask front-end-sheet`; do not edit by hand. -->

Share of png2c64's baseline pixels each front end's solve reproduces (#190), at the baselines' settings, reference solver both times. A is the general path, B the compatibility branch; since ADR-012 (#227) they are the same picture. Below, the edited scene (#227): the share of the old composite's solve the new one keeps.

| Fixture | A: compositor | B: png2c64's front end |
|---|---|---|
| portrait | 100.000 % | 100.000 % |
| terrain | 100.000 % | 100.000 % |
| title | 99.997 % | 99.997 % |
| gradient | 100.000 % | 100.000 % |
| photo | 100.000 % | 100.000 % |
| lineart | 100.000 % | 100.000 % |

| Edited (+0.25 px, 3°, masked) | old vs new solve, % same |
|---|---|
| portrait | 96.912 % |
| lineart | 91.314 % |

#235, large sources (4096 px, shrunk 12.8×; multicolor, checker 0.7, error clamp 0.1, gamma 1.5, pepto). Grain leakage: the share of output pixels that change between `title-grain-4096` and the same upscale without grain (gate: Auto ≤ 5 %). The reference is the source box-averaged to 320 × 200 in linear light, then the same front end and solver; the evidence (#238) averaged the 8-bit sRGB values instead, which is given beside it. Auto vs reference: the share of the reference's solve that Auto's reproduces.

| Source | exact (png2c64) | auto (mip level) | area-averaged reference | box on sRGB values |
|---|---|---|---|---|
| title-grain-4096, grain leakage | 24.4 % | 4.5 % | 4.2 % | 2.5 % |
| zoneplate-4096, auto vs reference | | 89.9 % same | | |
| title-grain-4096, auto vs reference | | 93.1 % same | | |
