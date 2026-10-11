# ADR-008: Layer masks — one raster plus parametric modifiers, edited in Convert64

**Status:** Accepted
**Date:** 2026-10-11
**Deciders:** Jaakko Haaparanta

## Context

The headline feature (ADR-002) is per-layer dithering with masks, feathering and
sub-pixel placement. PDD lists "raster mask per layer, paintable" as v1 and
vector/gradient masks as later; TAD and ADR-002 define feather as a Gaussian in
target pixels that fades both colour alpha and dither strength. Nothing yet says
how a mask is represented, what "paintable" means next to ADR-003's "no pixel
painting", or why masking must happen inside Convert64 rather than in the
image editor that produced the layer.

The reason it must be in-app: a mask edge is tuned *against the cell grid*. An
edge that looks right in Photoshop lands on a 4×8 cell boundary and either
clashes (five colours in the cell) or fades into mud. The user needs to move
the edge, change the feather and watch the solver react, cell by cell. A
pre-masked PNG is a fine starting point and nothing more.

## Decision

### Representation

1. **One raster mask per layer**, f32 in memory, u16 on disk, in **layer
   space at the layer's source resolution**. It is transformed with the layer
   (translate, scale, flip, sub-pixel nudge). The PNG's own alpha is a separate
   channel; effective alpha = `png_alpha × mask`. A layer without a mask
   behaves as mask = 1.
2. **Parametric modifiers** applied at composite time, in this fixed order,
   never baked into the raster:
   `Invert → Levels(black, white, gamma) → SnapToCells(mode) → Feather(σ_target_px)`.
   The slider the user drags is a modifier parameter; dragging it from 6 px to
   4 px loses nothing.
3. **Procedural sources** that contribute to the raster non-destructively and
   stay editable: `Gradient { Linear | Radial, from, to, softness }` and
   `Key { Luma | Chroma | Colour, centre, range, softness }` sampled from the
   layer's own pixels. They are combined with painted strokes by `Max`,
   `Min` or `Multiply` as the user chooses per source.
4. **Vector (bezier) masks are deferred.** A lasso/polygon tool rasterises into
   the mask; an editable path is a lot of UI for little conversion benefit.
   Revisit when users redraw the same shape repeatedly.

### Editing — all through the command engine (ADR-005)

ADR-003 is unchanged: Convert64 paints no *output* pixels. Painting a *mask*
steers conversion and is in scope; the distinction is that a mask stroke never
produces a colour, only a weight.

Tools, each a command with a dirty-rect memento for undo:

| Command | Parameters | Notes |
|---|---|---|
| `MaskStroke` | layer, polyline (layer px), radius, hardness, flow, `Add | Subtract` | soft round brush; the only freehand tool in the app |
| `MaskFill` | layer, polygon (layer px), value, `Add | Subtract` | lasso / polygon |
| `MaskGradient` | layer, kind, from, to, softness, combine | adds a procedural source |
| `MaskKey` | layer, channel, centre, range, softness, combine | adds a procedural source |
| `MaskSetModifier` | layer, `Invert | Levels | SnapToCells | Feather`, params | the sliders |
| `MaskLoad` / `MaskClear` / `MaskFromAlpha` | layer, path | import a greyscale PNG; reset; copy PNG alpha into the mask |
| `LayerViaMask` | source layer, selection, `cut: bool` | new layer sharing the source pixels with the selection as mask; `cut` inverts it on the original. How an image is split into layers without copying pixels (UX §5a) |

Because every tool is a command, masks are scriptable: the CLI and MCP server
can key out a backdrop or feather a logo from a recipe file without the UI.

### Feather semantics

`Feather(σ)` is a Gaussian blur of the effective mask with σ in **target
pixels** (320×200 or 160×200 space), computed on the supersampled grid so a
half-pixel nudge moves the fade coherently. The compositor consumes the
feathered mask twice, as ADR-002 requires: as alpha when blending colour, and
as the `strength` multiplier in the attribute map, so dither fades across the
feather instead of stopping at an edge. Feather is clamped to 0–32 target px.

*Amended 2026-10-11 (#99):* in the attribute map, strength is multiplied by the
three-factor product `png_alpha × mask × opacity`, so a 30 %-opaque layer fades its
dither the way a feathered edge does instead of imposing its full recipe. Which
layer governs a pixel is unchanged: the top-most one whose `png_alpha × mask` is
above zero (opacity 0 hides the layer altogether). Colour compositing is unchanged;
there opacity is applied by the blend as before.

### SnapToCells

Unique to this product: `SnapToCells { Off | Expand | Contract | Nearest }`
quantises the mask edge to whole 8×8 (hi-res) or 4×8 (multicolor) cells after
levels and before feather. A snapped hard edge can never cause a cell clash by
itself. The UX hints it when a clash sits under a feathered edge.

### Performance

A mask edit invalidates only the cells under its dirty rect; the cell-local
solver re-solves those. `Feather` is cached per modifier change on a
half-resolution mask and re-blurred on stroke end, not per mouse move.

## Options considered

| Option | Verdict |
|---|---|
| Raster + parametric modifiers (this ADR) | **Chosen** — non-destructive where it matters, cheap, scriptable |
| Pure raster, feather baked by a "Feather…" action | Rejected — cannot tune feather against the result |
| Vector-only masks (bezier) | Rejected for v1 — heavy UI, poor fit for photo cut-outs and keys |
| Masks edited externally (transparent PNG only) | Rejected — the edge must be tuned against the cell grid; kept as `MaskFromAlpha` import |
| Masks in *target* space (320×200) | Rejected — would not follow sub-pixel nudges and would alias on scaled layers |

## Consequences

- Easier: one mask type to render; feather/snap are two lines in the
  compositor; MCP gets masking for free.
- Harder: a brush tool in egui (cursor, pressure later, dirty-rect tiling);
  mask thumbnails and a quick-mask overlay in the UI.
- Interaction with ADR-003: the brush is the one freehand tool and it is
  confined to the mask channel. If a user asks why the brush will not paint
  colour, the answer is this ADR.
- Revisit when: a layer-specific dither *pattern* is wanted (ADR-002 already
  anticipates using the mask as the threshold source), or when bezier masks
  are requested repeatedly.

## Action items

- [ ] E4 #76: mask raster, procedural sources and modifiers in the compositor and attribute map
- [ ] E4 #77: mask commands in the command registry, with undo mementos and JSON/MCP exposure
- [ ] E8 #78: mask tools in the app — brush, lasso, gradient, key, modifier sliders, quick-mask view
