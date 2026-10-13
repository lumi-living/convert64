# The `.convert64` project file

A project file is the whole document the app and the CLI share ([ADR-009](../adr/ADR-009-project-file.md)): target mode, palette, layers, regions, cell steering and solve options, as UTF-8 JSON. `convert64 recipe apply in.convert64 --save out.convert64` reads and writes one (and reads a version-1 recipe the same way). The serde types are in `crates/convert64-project/src/file.rs` (`convert64_project`, #192), which the app, the CLI and the MCP server share.

Unknown fields are errors, so a typo never silently does nothing. Values are checked by the same constructors the app uses: a strength of 1.5, a scale of 0 or a mask of the wrong size is an error naming the layer.

## Versions

| `schema_version` | What it is | This build |
|---|---|---|
| (none, `"version": 1`) | The recipe file of E4 (`crates/convert64-project/src/recipe.rs`): layers with linked sources, commands, solve options | reads it as a project: sources linked, the default palette, its commands run after loading |
| `2` | This document as saved before #236, when the default `supersample` was 4 (and every save wrote it) | reads it and migrates it to 3: `supersample` becomes 1, whatever it was |
| `3` | This document (#236) | reads and writes |

A file with `schema_version` above 3 or below 2 is refused with a message naming the version. A change that old readers would misread (a renamed field, a new meaning) increments `schema_version`, with a migration from the old version; adding an optional field with a default does not.

**Migration from 2 (#236).** ADR-012 made 1× the default composite factor, png2c64's look; a version-2 project stores the 4× it was saved with and would keep the softer composite. Loading one sets `supersample` to 1 (the field stays and is written, `"supersample": 1`), so every project looks the same, and reports it once with the note **"Updated project from v2: now uses 1× resampling"**: a `note:` line on stderr and the `notes` array of `--json` reports ([cli.md](cli.md#--json)), a message in the app's status bar, a `note:` line in the MCP server's `project_open` reply. Saving writes version 3, which loads as written and without a note. Version-1 recipe files are not migrated: a recipe states `supersample` only when it means it. (`ProjectFile::migrate`, `ProjectFile::migration_note`, `Session::notes`.)

## Top level

```json
{
  "schema_version": 3,
  "target": "multicolor-bitmap",
  "supersample": 1,
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
| `schema_version` | yes | | `3` (`2` is read and migrated, above) |
| `target` | yes | | `"multicolor-bitmap"` or `"hires-bitmap"` (the M1 modes; the other names in `convert64_core::mode::Mode` parse but cannot be solved yet) |
| `supersample` | no | `1` | Composite factor, 1–8. Since ADR-012 (#227) 1: colour is resampled straight to the target grid with png2c64's Mitchell bicubic, masks at 4× and box-reduced. A higher factor resamples colour onto the finer grid and box-filters it down, which is softer. Projects saved before #227 (version 2) store `4`; loading migrates them to `1` (above) |
| `palette` | no | `"pepto-pal"` | See [Palette](#palette) |
| `layers` | yes | | Bottom first; see [Layers](#layers) |
| `regions` | no | none | Recipe overrides over part of the canvas; see [Regions](#regions) |
| `steering` | no | none | Pins, locks, allow and deny lists; see [Steering](#steering) |
| `solve` | no | defaults | See [Solve options](#solve-options) |
| `snap` | no | `"off"` | What `layer_set_transform` and `layer_rotate` round a transform to ([ADR-011](../adr/ADR-011-layer-transform-affine-subpixel.md)): `"off"`, `"c64-pixel"` (translate and the unrotated box's edges on whole target pixels, multicolor 2 canvas px across; rotation in 0.5° steps) or `"cell"` (cell boundaries, 8 × 8 canvas px; 15° steps). Written only when not `"off"`; set by `snap_set` (#198) |
| `commands` | no | none | Registry commands run in order after loading, as in a recipe file (`convert64 commands` lists them; since #193 they cover every field above: layers, adjustments, recipes, `target` (`mode_set`), `palette` (`palette_set`), `solve.background` (`background_pin`), the other solve options (`solve_set`) and, since #198, `snap` (`snap_set`)). A saved project has none: their result is in the fields. |

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

A recipe override over part of the picture (UX §7). `mask` is a raster as above, in canvas space: 320 × 200. Regions are applied in order, later ones winning where masks overlap. The `region_*` commands (`region_add`, `region_stroke`, `region_set_recipe`, …; [cli.md](cli.md#apply)) make and edit them; a saved region is exactly what they made, with the same 1/65535 rounding of a painted mask on the first save.

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
