# ADR-011: Layer transform — translate, scale and rotate, sub-pixel by default, snapping on request

**Status:** Accepted
**Date:** 2026-10-13
**Deciders:** Jaakko Haaparanta

## Context

E4 built `Transform { translate, scale: Vec2, flip_h, flip_v }`, deliberately
axis-aligned "to keep resampling separable" (layer/mod.rs). The app design then
gave scale only a number field. Two needs came up:

1. **Rotation.** Straightening a photographed source, tilting a logo, setting a
   scanned drawing level. Today the user must rotate in another editor and
   re-import, which breaks the non-destructive project.
2. **Sub-pixel placement for every transform, not only translate.** Layers are
   16-bit sources composited at N× supersample; the whole point (ADR-002, TAD
   §3.3) is that a fractional placement changes cell statistics and the solver
   legitimately re-picks colours. A scale that snaps the layer's edges to whole
   C64 pixels throws that away. Snapping is still wanted — for pixel-art
   sources, logos and anything that must land exactly on the grid — but as a
   choice, not a constraint.

## Decision

1. **Transform becomes a 2-D affine made of named parts**:
   `translate: Vec2, scale: Vec2, rotate: f32 (degrees), anchor: Vec2 (0–1 in
   layer space), flip_h, flip_v`. Composition order, fixed:
   `canvas = translate + R(rotate) · S(scale) · F(flip) · (p − anchor·size)`.
   Skew and perspective stay out (UX.md "Out of scope").
2. **Everything is sub-pixel by default.** Translate, scale and rotate are f32
   in C64 target pixels; the compositor samples the source through the inverse
   affine on the super grid, so any fractional result reaches the solver.
3. **Snapping is opt-in**, one setting with three values, stored per project
   and toggled in the toolbar (`Shift Cmd ;`):
   - **Off** (default) — fully continuous.
   - **C64 pixel** — translate and the scaled bounding-box edges land on whole
     target pixels (multicolor: whole 2:1 pixels); rotation snaps to 0.5°.
   - **Cell** — edges land on 4×8 / 8×8 cell boundaries; rotation snaps to
     15°.
   Holding `Cmd` while dragging inverts the current snap for that drag.
   Snapping changes the values written into the transform; the transform
   itself never stores a "snapped" flag.
4. **Resampling**: axis-aligned transforms (rotate = 0, the common case) keep
   the existing separable path (Lanczos-3 down, bilinear up). Rotated layers
   use a non-separable path: inverse-mapped sampling with a bicubic (Catmull-
   Rom) kernel and, when scale < 1, a prefilter at the scaled footprint
   (mip level or area pre-blur) to avoid aliasing. Masks transform with their
   layer through the same affine (ADR-008).
5. **Dirty region** for the incremental solve is the axis-aligned bounding
   box of the old ∪ new rotated layer quad, mapped to target cells.
6. **png2c64 parity** (ADR-010, #190) is unaffected: the compatibility path
   only applies to a single layer with the identity transform plus fit.

## Options considered

| Option | Verdict |
|---|---|
| Affine with named parts, sub-pixel default, opt-in snap (this ADR) | **Chosen** |
| Keep axis-aligned, add 90° steps only | Rejected — does not straighten photos, the actual need |
| Full matrix (incl. skew) | Rejected — no conversion use case, harder UI, harder snapping |
| Snap by default, sub-pixel on request | Rejected — inverts the product's core idea (fractional placement steers the solver) |

## Consequences

- Easier: one transform model for nudge, scale, rotate; snapping is a UI
  concern only.
- Harder: a second, non-separable resampler; it must be benchmarked against
  the #98/#139 budgets and must not regress the axis-aligned path.
- Revisit when: perspective correction for photographed artwork is requested.

## Action items

- [ ] Core: extend `Transform` (serde with migration in the project file, old
      files load with rotate = 0, anchor = centre), non-separable resampler,
      rotated dirty bounds — E4 follow-up issue.
- [ ] App: Position & scale & rotate fields, Transform tool rotate handle,
      snap setting — #197.
