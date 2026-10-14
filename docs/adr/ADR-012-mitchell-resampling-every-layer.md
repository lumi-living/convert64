# ADR-012: png2c64's Mitchell bicubic for every layer, on the target grid

**Status:** Accepted; superseded in part by ADR-013 (png2amiga's widened Lanczos-3 replaces Mitchell for every layer's colour, #261)
**Date:** 2026-10-13
**Deciders:** Jaakko Haaparanta

## Context

#190 gave a *plain* project (one opaque 8-bit layer filling the screen, nothing else)
png2c64's front end: its Mitchell bicubic straight to the target grid, then gamma and
its preprocessing. Anything more (a nudge, a mask, a second layer, a rotation) fell
back to the compositor: a 4× super grid, bilinear up and Lanczos-3 down
(Catmull-Rom for a rotated layer), box-filtered to the target. The #201 A/B sheet put
the two side by side and the human review chose png2c64's (#227): the 4× composite greys
thin lines (lineart) and softens photos, so adding a mask to a picture visibly
softened it.

The 4× grid was there for two reasons (TAD §3.3): sub-pixel placement must move the
picture continuously, and edges (layer, mask, feather) need antialiasing. The first
does not need a finer grid; a resampling kernel's phase moves the picture by any
fraction. The second had not been measured.

## Decision

1. **One resampler for every layer: png2c64's Mitchell bicubic** (B = C = ⅓,
   `solve::png2c64_ref::scale::mitchell`). Axis-aligned layers run it separably, rotated
   layers (ADR-011) inverse-mapped through the same kernel. A pixel `x` of the grid
   samples the source at `pos = (x + 0.5 − offset)·ratio − 0.5`, four taps at
   `floor(pos) − 1 … + 2`, indices clamped to the source and the weights renormalised:
   png2c64's `scale::bicubic` operation for operation. Sub-pixel placement and scale
   come from the kernel's phase (`offset`, `ratio`).
2. **The default grid is the target grid** (`SuperGrid::DEFAULT_N = 1`). The factor
   stays a setting (1–8); above 1 the colour is resampled onto the finer grid and
   box-filtered down, which is softer.
3. **Every layer gets png2c64's front end.** After resampling, a layer's colour goes
   through png2c64's gamma and preprocessing round trip (OKLab and back, clamped),
   then our other adjustments, then png2c64's range matching over the pixels the layer
   covers. The 8-bit decode uses png2c64's sRGB table. Every frame is therefore
   "prepared", and the reference solver does not repeat the round trip.
4. **Invariant:** a plain project through the general path is png2c64's prepared image
   bit for bit, at 1×, in both modes, with and without gamma and range matching (test
   `the_general_path_is_png2c64_s_front_end_for_a_plain_layer`). The compatibility
   branch (`set_png2c64_front_end`) stays until the invariant has stood for a while.
5. **Masks are supersampled, and only masks.** A mask's layer-space values are
   resampled at 4× (`MASK_N`), inverted and levelled there, box-reduced to the grid, then
   snapped and feathered on the grid (σ in target px, ADR-008 unchanged). The 4× grid is
   sampled with Mitchell, except on a rotated layer, where it is sampled bilinearly:
   Mitchell's sixteen taps per fine pixel cost the rotated benchmark 60 / 130 ms
   (multicolor / hi-res, single-threaded) and bought 0.006 / 0.011 of coverage. A turn
   by a multiple of 90° is separable and keeps Mitchell. Measured on a
   white masked layer over a dark backdrop against an 8× composite, the largest
   per-pixel error in linear light, multicolor / hi-res:

   | Mask | mask at 1× | mask at 4× (this ADR) |
   |---|---|---|
   | hard edge, 1:1 | 0.047 / 0.097 | 0.002 / 0.002 |
   | hard edge, +0.25 px | 0.107 / 0.035 | 0.003 / 0.002 |
   | diagonal edge, +0.25 px | 0.151 / 0.137 | 0.004 / 0.003 |
   | diagonal, feather 1 | 0.037 / 0.020 | 0.005 / 0.004 |
   | diagonal, feather 3 | 0.010 / 0.019 | 0.012 / 0.018 |
   | diagonal, 1024 px raster fitted to 320 | 0.315 / 0.245 | 0.014 / 0.006 |
   | diagonal, rotated 3° | 0.174 / 0.087 | 0.012 / 0.014 (bilinear at 4×) |
   | linear gradient | 0.000 / 0.000 | 0.000 / 0.000 |

   A mask edge at 1× is off by up to a third of the coverage on the pixels along it.
   At 4× and box-reduced it is within 0.02 everywhere; what is left under a feather of
   3 is the blur itself sampled on the coarser grid. `tests/mask_edges.rs` keeps the
   1× composite with its 4× mask equal to the 4× composite.
6. **Nothing else is supersampled.** The layer's own edge is its exact box coverage at
   any factor (an opaque source's alpha is exactly the coverage), and the attribute map
   reads the same mask and alpha, so neither needs a finer grid.

## Options considered

### Option A: keep the 4× composite (Lanczos-3 / bilinear, Catmull-Rom rotated)
| Dimension | Assessment |
|---|---|
| Complexity | None to add |
| Performance | Rotated cold composite over budget (#198: 132 ms against 60 ms) |
| Maintenance | Two front ends that disagree; plain projects differ from edited ones |
| Fit | Rejected by eye on the #201 sheet: greys lines, softens photos |

### Option B: Mitchell to the target grid, masks at 4× (chosen)
| Dimension | Assessment |
|---|---|
| Complexity | One kernel for both paths; a mask-only fine grid |
| Performance | See Consequences |
| Maintenance | The plain path becomes a special case of the general one, by test |
| Fit | png2c64-crisp for every project; nudges still steer the solve |

### Option C: Mitchell on the 4× grid
| Dimension | Assessment |
|---|---|
| Complexity | Smallest change |
| Performance | As today |
| Maintenance | Still not png2c64's picture for a plain project |
| Fit | The box filter still softens; does not answer #227 |

## Trade-off analysis

png2c64's scaler does not widen the kernel when it downscales (the issue assumed it
did), so a large source scaled down aliases a little, in colour as in png2c64's own
output. We follow png2c64, since its picture is the bar (ADR-010); masks, where
aliasing showed as a jagged edge, get the 4× grid. The rotated path loses its mip
prefilter for the same reason. Mitchell's negative lobes are kept, as png2c64 keeps
them; the preprocessing round trip clamps.

## Consequences
- Easier: one front end; a plain project is the general path's special case; the
  composite is 16× smaller, so composites and drags are faster, the rotated path
  included (TAD §7).
- Harder: the app's Source view needs its own finer compositor (`Session::PREVIEW_N`,
  4×) to stay sharp when zoomed in, so an edit composites three times in the app.
  Projects saved before #227 store `"supersample": 4`; since #236 they are migrated
  to 1× on load (project `schema_version` 3, ADR-009), so every project gets the
  same look.
- Revisit when: the sheet shows a downscaled layer aliasing (widen the kernel as an
  option); the invariant has held long enough to drop the compatibility branch.

## Action items
- [x] Mitchell for both paths, the 1× default, the per-layer front end, mask
      supersampling (#227)
- [x] `cargo xtask front-end-sheet` row: nudged, masked and rotated, old against new
- [x] Older projects' `"supersample": 4` migrated to 1× on load (#236)
- [ ] Drop the compatibility branch (the rotated path's mip machinery is used again
      since #235, below)

## Amended 2026-10-13 (#235): large shrinks sample a cached mip chain

The "revisit when" above came true. png2c64's four taps per output pixel read a small
part of a source shrunk 12–25×: a 4096 × 2560 zone plate fills the frame with
aliasing rings where the right answer is flat grey, and the grain of a 4096 px photo
leaks into the dither (24.5 % of output pixels change with the grain, 2.8 % when the
source is area-averaged first; evidence in `docs/research/aliasing-2026-10-13/`, #238).
Decision (Jaakko, 2026-10-13):

1. **A per-layer option, `Downscale`: `auto` (default) or `exact`.** Exact is
   png2c64's sampling at every shrink (this ADR as first written). Auto is the same
   up to a 4× shrink and, past it, samples a box-reduced **mip level** of the source
   with the same Mitchell taps. It is a layer field (`downscale`, written only when
   `exact`), a registry command (`layer_set_downscale`), `convert64 convert
   --downscale auto|exact` (and the batch key), and a control in the app's Transform
   group.
2. **The shrink** is source pixels per target pixel along each layer axis, measured
   on the hi-res grid (320 · n across in either mode, n the supersample factor): the
   source pixels one such pixel's footprint spans, `1 / (scale · n)` for an unrotated
   layer, `(|cos| + |sin|) / (scale · n)` turned. Up to 4 the original is sampled bit
   for bit as before; above, level `L = ⌈log2(shrink / 2)⌉` on that axis (the
   remaining shrink is 1–2). 4096 → 320 samples the 512 px level. A plain project past
   4× under Auto leaves png2c64's compatibility branch, which has no levels.
3. **The chain** is repeated 2 × 2 box reductions of the premultiplied colour in
   linear light, kept per layer axis (a rip-map, ADR-011's #198 amendment §4) and
   built lazily: a level is made the first time a composite needs it and kept for as
   long as the source lives. It lives in the `SourceImage` (keyed by its identity, so
   every layer and every compositor holding the source shares it, the app's three
   included). Its values do not depend on the order levels were asked for: level
   `(lx, ly)` is always the diagonal `(k, k)` by 2 × 2 boxes up to `min(lx, ly)`, then
   1-D halvings, started from the furthest level on that path already built. Levels
   with an axis at 1 are only intermediates (L jumps from 0 to 2) and are not kept.
4. **Both paths, the same rule.** The separable path's axis maps index the level
   (positions divided by `2^L`, exact in f32); the rotated path's inverse map samples
   it (#198's `inv_mip`). Placement, the fitted-span positions and the edge coverage
   stay the original's, so coverage is still the exact box coverage of the layer.
5. **Masks** follow the rule on the grid they are resampled on (§5: 4× the target):
   the shrink is source pixels per mask pixel, so a mask switches to a level only past
   a 4× shrink there (16× on the target grid, 8× across in multicolor). A mask's
   levels are kept with its layer and rebuilt only when its content changes.
6. **Never saved, never destructive.** The levels are a cache; the project stores the
   original and the option, and Exact or a later enlargement samples the original
   again. A scale, rotation or nudge drag reuses the levels it has (a test counts the
   builds: none after the first composite while the level stays the same).

**Why 4×.** Up to a 4× shrink png2c64's four taps cover the whole footprint of an
output pixel, so it does not alias much, and every source up to 1280 px wide fitted to
the 320 grid stays bit-identical to png2c64 (the parity suite, this ADR's invariant and
a test that Auto equals Exact up to 4×). Past it, levels leave a shrink of 1–2 for the
kernel, the range its taps were made for.

**Why a cache, not a baked resize.** Resizing the source once when it is placed would
make the picture depend on the scale at that moment and lose detail for a later
enlargement or rotation; it would also have to be saved, or redone on load. A cache
of power-of-two levels is independent of the transform, so drags reuse it, and the
project keeps the original (ADR-003's non-destructive editing).

**Measured** (`cargo xtask front-end-sheet`, multicolor, checker 0.7, error clamp 0.1,
gamma 1.5, pepto; the grain one value per pixel on all channels, as in the evidence):
grain leakage on `title-grain-4096` is 24.4 % exact, **4.5 % auto**, 4.2 % with the
source area-averaged in linear light first (2.5 % averaged on the sRGB values, as the
evidence did). On the zone plate Auto's frame beyond the centre is grey within a
standard deviation of 0.027 against Exact's 0.262 (unit test). Building the 512 px level
of a 4096 × 2560 source (three reductions) takes 5.8 ms single-threaded and 2.2 ms with
`parallel` natively (Apple M-series, `tests/bench.rs`), and 9–23 ms in the browser build
(headless Chrome, the first page load slowest); the app logs it to the console. The cold composite of that layer is 8.8 ms single-threaded, since the
source is no longer premultiplied whole.
