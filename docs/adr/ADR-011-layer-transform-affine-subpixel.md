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

## Amended 2026-10-13 (#198)

The core implementation (#198) changes Decision §1's formula and pins down the
details §3–§5 left open. The Decision section above is kept as written.

1. **Composition.** The transform is
   `canvas = translate + c + R(rotate) · (S · F(p) − c)` with
   `c = S · (anchor ⊙ size)`. `F` flips within the layer's box (`x → width − x`),
   `S = diag(scale)`, and `R` turns by `rotate` degrees, **positive clockwise on
   screen** (canvas y points down, so `R = [cos −sin; sin cos]`). `translate` stays
   the **top-left corner of the unrotated box**; the anchor is only the pivot of the
   rotation, a fraction of the box (after the flips) from its top-left corner. With
   `rotate = 0` this is exactly E4's `translate + S · F(p)`.

   Decision §1 makes `translate` the anchor's position instead. Every existing caller,
   test, golden and saved project uses top-left semantics, and moving to anchor
   semantics would need a migration that depends on each layer's source size. With
   this formula old files mean what they meant, with no migration.
2. **Fields.** `Transform { translate, scale, rotate, anchor, flip_h, flip_v }`:
   `rotate` is finite degrees, stored as given (360 is not folded to 0), and `anchor`
   is 0–1 on both axes, default `(0.5, 0.5)`. In files both are optional and written
   only when they differ from the default, so files and goldens without a rotation are
   byte for byte unchanged. Unknown transform keys are refused. The helpers
   `reanchored` (move the pivot, keep the picture in place) and `scaled_about_anchor`
   (scale with the pivot fixed, as UX §6–§7 scale) live on `Transform`.
3. **Two paths.** `rotate == 0` takes the separable path unchanged, bit for bit. A
   rotated layer is inverse-mapped: each super pixel's centre goes to layer space, and
   the premultiplied source is sampled there with a Catmull-Rom 4 × 4 kernel, edges
   clamped. Sine and cosine come from a series in f64, not the platform's, so the
   result is the same on every OS. A multiple of 90° is exact.
4. **Prefilter.** When the layer is scaled down, a box-filtered mip of the source is
   used, chosen once per layer and per layer axis (a rip-map). An affine map has the
   same footprint everywhere, so one level fits the whole layer: on each axis the
   level is `round(log2(footprint))`, the footprint being the source pixels one super
   pixel spans along that axis. The kernel then reads at most √2 mip pixels per super
   pixel. The mips are cached with the layer, the mask's alongside.
5. **Coverage.** The rotated edge is antialiased **exactly**, not approximated. Each
   super pixel's footprint is a parallelogram in layer space. It is clipped against
   the box `[0, w] × [0, h]` (Sutherland–Hodgman) and its area taken in f64. Footprints
   wholly inside or outside the box skip the clip.
6. **Masks** go through the same affine map and the same kernel, then their modifiers
   as before (ADR-008). Like the separable path, the mask carries no coverage: outside
   the quad it continues its edge values, and the colour's coverage hides it. The
   fused half-resolution feather of #139 stays on the separable path. A rotated mask
   is resampled at full resolution, then blurred at half resolution as usual.
7. **Extents and dirty region.** A rotated layer's raster rectangle is the bounding
   box of its rotated quad on the super grid. The compositor's existing union of old
   and new rectangles therefore gives §5's dirty region with no further change, and a
   test checks it. `mask_change_cells` maps a mask edit's four corners through the
   rotation, with the kernel's and the mip's reach.
8. **Snap** is `layer::Snap { Off, C64Pixel, Cell }` with `Snap::apply(transform,
   source size, mode)`, a pure function as §3 describes. It rounds `translate` and
   the far edges of the scaled, unrotated box, so the box spans a whole number of
   steps. C64 pixel steps are 1 canvas px (multicolor: 2 across), Cell steps are 8 × 8
   canvas px in both modes. It also rounds `rotate` to 0.5° or 15°. Anchor and flips
   are kept.
9. **png2c64 parity.** The compatibility path of #190 also requires `rotate == 0`.
   The fit transform alone does not exclude a full turn.
