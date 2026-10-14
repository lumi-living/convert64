# Convert64 dither previews: changelog

Every regeneration of `convert64-*/` with `cargo xtask dither-sheet --reason`.

- 2026-10-12: first render of every method (#116)
- 2026-10-12: #156: png2c64's ordered semantics, chroma guard, rendered set search; gradient-domain error under each pair
- 2026-10-12: #181: png2c64's pipeline is the default solver; Convert64 next to png2c64 is the reference on png2c64's input; divergence rows; portrait at strength 1.0
- 2026-10-14: png2amiga's C64 path as the reference (ADR-013, #261): every render through its front end and solver
