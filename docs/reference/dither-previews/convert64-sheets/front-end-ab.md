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
