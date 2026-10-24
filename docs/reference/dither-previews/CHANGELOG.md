# Convert64 dither previews: changelog

Every regeneration of `convert64-*/` with `cargo xtask dither-sheet --reason`.

- 2026-10-12: first render of every method (#116)
- 2026-10-12: #156: png2c64's ordered semantics, chroma guard, rendered set search; gradient-domain error under each pair
- 2026-10-12: #181: png2c64's pipeline is the default solver; Convert64 next to png2c64 is the reference on png2c64's input; divergence rows; portrait at strength 1.0
- 2026-10-24: #280 (#275): all 90 methods; png2amiga's 32 on png2c64's pipeline, the reference solver's since #275 (not rendered before). Of the other 58, all on the card and 21 on the portrait differ from the renders last made in #181, by the solver changes since (#190's front end onwards); this PR does not change them (their goldens and png2c64's parity are unchanged). `scores.md`, `sheet-vs-png2c64.png` and `sheet-divergences.png` are byte-identical.
