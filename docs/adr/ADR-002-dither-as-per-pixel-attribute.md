# ADR-002: Dithering as a per-pixel attribute carried through compositing

**Status:** Accepted
**Date:** 2026-10-11
**Deciders:** Jaakko Haaparanta

## Context

The headline feature is "different dithering per layer" with masks, feathering and sub-pixel placement, while the final image must be a single legal VIC-II picture whose cell colour sets are chosen globally.

## Decision

Layers are composited in f32 linear RGBA at N× supersample. Alongside the colour composite, an **attribute map** is composited: for each target pixel, the recipe `(method, strength, phase, metric, allowed_set, fit_mode)` of the top-most non-transparent layer, with `strength` blended through alpha/feather. One constraint-solver pass then quantises the whole image against that map.

## Options considered

### A. Quantise each layer separately, composite palette indices
Rejected: palette indices cannot be blended; overlapping layers and soft edges break; cell colour sets cannot be chosen globally.

### B. Composite in float, single global recipe (what every existing tool does)
Rejected: no per-region control — the problem Convert64 exists to solve.

### C. Composite in float + attribute map, single solver pass (chosen)
Per-region control, global legality, sub-pixel nudging for free (supersampled composite), feathered strength gradients.

## Consequences
- Easier: one solver, one validator; every steering feature (pins, locks, Force/Adaptive, allow-lists) is just another attribute channel.
- Harder: the solver must handle mixed methods in one pass (ordered pixels and diffusion pixels interleaved); diffusion must be clamped at cell boundaries.
- Revisit when: a layer-specific *pattern* (e.g. a hand-drawn dither texture) is wanted — that can be a method whose threshold source is the layer's own mask.
