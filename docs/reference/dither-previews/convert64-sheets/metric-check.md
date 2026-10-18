<!-- Written by `cargo xtask metric-check`; do not edit by hand. -->

# Metric check (#273)

Each candidate metric against Jaakko's eye-ranked pairs (`fixtures/eye-rankings.toml`), both pictures of a pair re-rendered as the reviewed sheet showed them and scored against the same reference (the source area-averaged in linear light). A metric agrees when it scores the eye's winner strictly better. No metric is a gate.

| Metric | agrees, all 28 pairs | agrees, 25 pairs without `low` | pairs it gets wrong |
|---|---|---|---|
| `s2` | 12 / 28 (43 %) | 11 / 25 (44 %) | baselines lineart png2amiga > png2c64; baselines gradient png2c64 > png2amiga; baselines terrain png2c64 > png2amiga (low); match-range portrait off > box; match-range portrait png2c64-baseline > box; match-range terrain off > hull (low); match-range title box > off; match-range title box > hull; match-range gradient png2c64-baseline > off; match-range gradient png2c64-baseline > box; refine portrait off > on; refine title off > on; refine lineart off > on; refine gradient off > on; large-sources zoneplate-4096 old-mitchell-mip > png2amiga-exact; large-sources zoneplate-4096 old-mitchell-mip > mip-accelerated |
| `s2-blur` | 16 / 28 (57 %) | 15 / 25 (60 %) | baselines terrain png2c64 > png2amiga (low); match-range portrait off > box; match-range portrait off > hull; match-range portrait png2c64-baseline > box; match-range terrain off > hull (low); match-range title box > off; refine portrait off > on; refine title off > on; refine lineart off > on; refine gradient off > on; large-sources zoneplate-4096 old-mitchell-mip > png2amiga-exact; large-sources zoneplate-4096 old-mitchell-mip > mip-accelerated |
| `s2-crt` | 16 / 28 (57 %) | 15 / 25 (60 %) | baselines terrain png2c64 > png2amiga (low); match-range portrait off > box; match-range portrait off > hull; match-range portrait png2c64-baseline > box; match-range terrain off > hull (low); match-range title box > off; refine portrait off > on; refine title off > on; refine lineart off > on; refine gradient off > on; large-sources zoneplate-4096 old-mitchell-mip > png2amiga-exact; large-sources zoneplate-4096 old-mitchell-mip > mip-accelerated |
| `oklab-blur` | 15 / 28 (54 %) | 12 / 25 (48 %) | baselines title png2c64 > png2amiga; baselines lineart png2amiga > png2c64; baselines gradient png2c64 > png2amiga; match-range portrait off > box; match-range portrait off > hull; match-range portrait png2c64-baseline > box; match-range title png2c64-baseline > box; match-range gradient png2c64-baseline > off; refine title off > on; refine lineart off > on; refine gradient off > on; large-sources zoneplate-4096 old-mitchell-mip > png2amiga-exact; large-sources zoneplate-4096 old-mitchell-mip > mip-accelerated |

Every pair's scores (winner / loser; S2 higher is better, `oklab-blur` lower):

| Sheet | Fixture | Eye: winner > loser | `s2` | `s2-blur` | `s2-crt` | `oklab-blur` |
|---|---|---|---|---|---|---|
| baselines | portrait | png2c64 > png2amiga | -42.35 / -46.38 | -39.76 / -43.83 | -41.74 / -45.36 | 0.00488 / 0.00497 |
| baselines | title | png2c64 > png2amiga | -47.18 / -54.55 | -48.98 / -56.03 | -51.29 / -57.49 | 0.01625 / 0.01555 |
| baselines | photo | png2c64 > png2amiga | -48.88 / -54.88 | -54.72 / -58.00 | -57.62 / -59.77 | 0.01092 / 0.01197 |
| baselines | lineart | png2amiga > png2c64 | -4.15 / -3.81 | 11.96 / 10.00 | 4.75 / 4.64 | 0.00024 / 0.00024 |
| baselines | gradient | png2c64 > png2amiga | -50.47 / -50.05 | -53.70 / -54.31 | -55.16 / -55.78 | 0.01642 / 0.01627 |
| baselines | terrain | png2c64 > png2amiga (low) | -46.69 / -42.61 | -48.28 / -43.14 | -51.06 / -46.99 | 0.01113 / 0.01307 |
| match-range | portrait | off > box | -46.38 / -27.08 | -43.83 / -27.27 | -45.36 / -26.75 | 0.00497 / 0.00341 |
| match-range | portrait | off > hull | -46.38 / -46.88 | -43.83 / -43.66 | -45.36 / -44.52 | 0.00497 / 0.00491 |
| match-range | portrait | png2c64-baseline > off | -42.35 / -46.38 | -39.76 / -43.83 | -41.74 / -45.36 | 0.00488 / 0.00497 |
| match-range | portrait | png2c64-baseline > box | -42.35 / -27.08 | -39.76 / -27.27 | -41.74 / -26.75 | 0.00488 / 0.00341 |
| match-range | portrait | png2c64-baseline > hull | -42.35 / -46.88 | -39.76 / -43.66 | -41.74 / -44.52 | 0.00488 / 0.00491 |
| match-range | terrain | off > box (low) | -42.61 / -58.12 | -43.14 / -57.58 | -46.99 / -59.39 | 0.01307 / 0.02846 |
| match-range | terrain | off > hull (low) | -42.61 / -40.00 | -43.14 / -40.70 | -46.99 / -45.04 | 0.01307 / 0.01317 |
| match-range | title | box > off | -59.74 / -54.55 | -60.24 / -56.03 | -61.50 / -57.49 | 0.00819 / 0.01555 |
| match-range | title | box > hull | -59.74 / -59.39 | -60.24 / -60.37 | -61.50 / -61.78 | 0.00819 / 0.01958 |
| match-range | title | png2c64-baseline > box | -47.18 / -59.74 | -48.98 / -60.24 | -51.29 / -61.50 | 0.01625 / 0.00819 |
| match-range | gradient | png2c64-baseline > off | -50.47 / -50.05 | -53.70 / -54.31 | -55.16 / -55.78 | 0.01642 / 0.01627 |
| match-range | gradient | png2c64-baseline > box | -50.47 / -50.13 | -53.70 / -55.95 | -55.16 / -57.18 | 0.01642 / 0.01705 |
| match-range | gradient | png2c64-baseline > hull | -50.47 / -51.62 | -53.70 / -55.12 | -55.16 / -56.47 | 0.01642 / 0.01743 |
| match-range | photo | png2c64-baseline > off | -48.88 / -54.88 | -54.72 / -58.00 | -57.62 / -59.77 | 0.01092 / 0.01197 |
| match-range | photo | png2c64-baseline > box | -48.88 / -54.11 | -54.72 / -57.83 | -57.62 / -59.76 | 0.01092 / 0.01169 |
| match-range | photo | png2c64-baseline > hull | -48.88 / -56.21 | -54.72 / -59.47 | -57.62 / -60.96 | 0.01092 / 0.01233 |
| refine | portrait | off > on | -46.38 / -35.29 | -43.83 / -29.12 | -45.36 / -35.11 | 0.00497 / 0.00584 |
| refine | title | off > on | -54.55 / -47.25 | -56.03 / -46.26 | -57.49 / -49.81 | 0.01555 / 0.01450 |
| refine | lineart | off > on | -4.15 / 13.30 | 11.96 / 29.27 | 4.75 / 13.59 | 0.00024 / 0.00021 |
| refine | gradient | off > on | -50.05 / -48.56 | -54.31 / -52.48 | -55.78 / -53.54 | 0.01627 / 0.01556 |
| large-sources | zoneplate-4096 | old-mitchell-mip > png2amiga-exact | -43.47 / -12.30 | -42.07 / -2.89 | -50.31 / -12.89 | 0.01063 / 0.01028 |
| large-sources | zoneplate-4096 | old-mitchell-mip > mip-accelerated | -43.47 / -8.91 | -42.07 / 1.41 | -50.31 / -10.64 | 0.01063 / 0.01033 |

Re-renders identical to the reviewed sheets' tiles: 42 of 42.
