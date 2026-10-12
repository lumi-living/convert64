# The `.convert64` project file

A project file is the whole document the app and the CLI share ([ADR-009](../adr/ADR-009-project-file.md)): target mode, palette, layers, regions, cell steering and solve options, as UTF-8 JSON. `convert64 recipe apply in.convert64 --save out.convert64` reads and writes one (and reads a version-1 recipe the same way). The serde types are in `crates/convert64-project/src/file.rs` (`convert64_project`, #192), which the app, the CLI and the MCP server share.

Unknown fields are errors, so a typo never silently does nothing. Values are checked by the same constructors the app uses: a strength of 1.5, a scale of 0 or a mask of the wrong size is an error naming the layer.

## Versions

| `schema_version` | What it is | This build |
|---|---|---|
| (none, `"version": 1`) | The recipe file of E4 (`crates/convert64-project/src/recipe.rs`): layers with linked sources, commands, solve options | reads it as a project: sources linked, the default palette, its commands run after loading |
| `2` | This document | reads and writes |

A file with `schema_version` above 2 is refused with a message naming the version. A change that old readers would misread (a renamed field, a new meaning) increments `schema_version`; adding an optional field with a default does not.

## Top level

```json
{
  "schema_version": 2,
  "target": "multicolor-bitmap",
  "supersample": 4,
  "palette": "pepto-pal",
  "layers": [ … ],
  "regions": [ … ],
  "steering": { … },
  "solve": { … },
  "commands": [ … ]
}
```

| Field | Required | Default | Meaning |
|---|---|---|---|
| `schema_version` | yes | | `2` |
| `target` | yes | | `"multicolor-bitmap"` or `"hires-bitmap"` (the M1 modes; the other names in `convert64_core::mode::Mode` parse but cannot be solved yet) |
| `supersample` | no | `4` | Composite factor, 1–8 |
| `palette` | no | `"pepto-pal"` | See [Palette](#palette) |
| `layers` | yes | | Bottom first; see [Layers](#layers) |
| `regions` | no | none | Recipe overrides over part of the canvas; see [Regions](#regions) |
| `steering` | no | none | Pins, locks, allow and deny lists; see [Steering](#steering) |
| `solve` | no | defaults | See [Solve options](#solve-options) |
| `commands` | no | none | Registry commands run in order after loading, as in a recipe file (`convert64 commands` lists them). A saved project has none: their result is in the layers, masks and steering. |

## Palette

| Value | Palette |
|---|---|
| `"pepto-pal"` (also `"pepto"`) | Pepto, PAL. The default. |
| `"pepto-ntsc"` | Pepto, NTSC |
| `"colodore"` | Colodore at colodore.com's default sliders |
| `{"custom": ["#000000", "#FFFFFF", …]}` | 16 colours in VIC-II index order |
| `{"vpl": "palettes/mine.vpl"}` | A VICE palette file, relative to the project file |

The palette is the one the picture is solved and previewed with (CLAUDE.md non-negotiable 5).

## Layers

```json
{
  "name": "title",
  "source": { "path": "title.png" },
  "visible": true,
  "transform": { "translate": { "x": 0.0, "y": 0.0 }, "scale": { "x": 1.0, "y": 1.0 }, "flip_h": false, "flip_v": false },
  "opacity": 1.0,
  "blend": "normal",
  "recipe": { "method": { "kind": "bayer4" }, "strength": 0.8 },
  "adjustments": { "gamma": 1.5 },
  "mask": { "raster": { "fill": 1.0 }, "sources": [ … ], "modifiers": { "feather": 2.0 } }
}
```

| Field | Default | Meaning |
|---|---|---|
| `name` | | Display name; commands can name a layer by it |
| `source` | | Where the pixels come from (below) |
| `visible` | `true` | Hidden layers take no part in the composite. Written only when `false`. |
| `transform` | identity | Placement in the 320 × 200 canvas: `translate` (canvas pixels, the unrotated box's top-left), `scale` (> 0 on both axes), `rotate` (degrees, clockwise on screen; default 0, written only when not 0), `anchor` (the pivot of the rotation, `{x, y}` 0–1 across the box; default the centre, written only when not the centre), `flip_h`, `flip_v`. [ADR-011](../adr/ADR-011-layer-transform-affine-subpixel.md) has the formula. Builds before #198 ignored `rotate` and `anchor`; unknown keys are now refused. |
| `opacity` | `1.0` | 0–1 |
| `blend` | `"normal"` | `normal`, `multiply`, `screen`, `overlay` |
| `recipe` | Floyd–Steinberg 100 % | The dither recipe (ADR-002): `method` (`kind`, `orientation`, `custom`), `strength`, `phase`, `metric`, `allowed`, `fit_mode`, `cell_clamp`, `max_luma_diff`, `adaptive`, `mix_levels`, `smooth`, `cell_metric`, `error_clamp`. Method ids are in [dither-methods.md](dither-methods.md); defaults are in TAD §4 "E5 in summary". |
| `adjustments` | identity | `gamma`, `levels`, `brightness_contrast`, `hsl`, `curves`, `match_range` |
| `mask` | none (opaque) | The layer mask (ADR-008), below |

### Sources

Each layer chooses how its pixels are stored:

| Form | Meaning |
|---|---|
| `{"path": "art/title.png"}` | **Linked**: a PNG, relative to the project file's directory, `/`-separated. |
| `"art/title.png"` | The same, as recipe files write it. |
| `{"png_base64": "iVBORw0KGgo…"}` | **Embedded**: the PNG file's bytes, standard base64 with padding. The project then stands alone. |
| `{"layer": 0}` | The pixels of an earlier (lower) layer, shared rather than copied. `layer_via_mask` creates such layers. |

Saving keeps each layer as it was loaded unless told otherwise: `--embed` embeds every source (the original PNG bytes, not a re-encoding), `--link` links every one and fails for a source that has no file. Linked paths are rewritten relative to the directory the project is saved in. Two layers linking the same file share one copy of its pixels.

### Masks

| Field | Default | Meaning |
|---|---|---|
| `raster` | | The painted raster, the layer source's size: `{"fill": v}` (every value `v`, 0–1), `{"path": "mask.png"}` (a greyscale PNG, 8 or 16 bit, relative to the project) or `{"png_base64": "…"}` (an embedded 16-bit greyscale PNG) |
| `sources` | none | Gradients and keys, applied in order: `{"source": {"type": "gradient", …}, "combine": "max"}`, `{"source": {"type": "key", …}, "combine": "multiply"}` (the `mask_gradient` and `mask_key` arguments) |
| `modifiers` | none | `invert`, `levels`, `snap` (`off`, `expand`, `contract`, `nearest`), `feather` (σ in target pixels, 0–32) |

A saved raster is `{"fill": v}` when every value is the same (stored exactly) and an embedded 16-bit PNG otherwise. Masks are f32 in memory and 16-bit on disk (ADR-008), so the first save rounds a painted mask to 1/65535; from then on load → save → load is exact.

## Regions

```json
{ "name": "sky", "mask": { "fill": 1.0 }, "recipe": { "method": { "kind": "none" }, "strength": 1.0 } }
```

A recipe override over part of the picture (UX §7). `mask` is a raster as above, in canvas space: 320 × 200.

## Steering

```json
{
  "pins":  [{ "x": 3, "y": 4, "colours": [0, 1, 2, 5], "full": true }],
  "locks": [{ "x": 39, "y": 24 }],
  "allow": [{ "x": 5, "y": 5, "colours": [0, 1, 2] }],
  "deny":  [{ "x": 6, "y": 5, "colours": [7] }]
}
```

Cells are `x` 0–39, `y` 0–24; colours are indices 0–15. A pin names 1–4 colours: `full` fixes the cell's set to exactly them, otherwise the set must contain them. What each entry does is the `cell_pin`, `cell_lock`, `cell_allow` and `cell_deny` commands'. Saved in cell order.

## Solve options

| Field | Default | Meaning |
|---|---|---|
| `solver` | `"reference"` | `"reference"` (png2c64's pipeline, ported, #181) or `"native"` (Convert64's own solver). The reference falls back to native for frames it cannot solve yet; `refinements`, `finalists`, `hysteresis` and `luma_pair_bias` and the recipes' `cell_clamp`, `max_luma_diff`, `mix_levels`, `smooth`, `ordered_semantics` and `chroma_guard` are the native solver's |
| `divergences` | all off (omitted) | The reference solver's named divergences: `{"chroma_guard": bool, "max_luma_diff": n or null, "pair_bias": x, "cell_clamp": bool, "mix_levels": n or null, "bridging": bool}`, any subset ([png2c64-reference.md](png2c64-reference.md#divergences)) |
| `background` | `"auto"` | `"auto"` (search all 16) or `{"fixed": n}` (multicolor) |
| `hysteresis` | none | Refinement hysteresis ratio in (0, 1], e.g. `0.97` |
| `luma_pair_bias` | `0.0` | Same-luma pair bias per pair (positive penalises, negative rewards) |
| `refinements` | `2` | Refinement rounds, 0–2 |
| `finalists` | `3` | Backgrounds carried from the search into dithering, 1–16 |

## Round trip

A project saved by the CLI reloads identically and solves to the same picture: load → save gives the same bytes again, and the reloaded project's solve is byte-identical (`crates/convert64-cli/tests/project.rs` on disk, `crates/convert64-project/tests/memory.rs` in memory, as the browser build loads them). JSON is written pretty-printed with a trailing newline, fields in the order above.
