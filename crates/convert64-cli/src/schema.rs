//! The argument schema of every registry command (JSON Schema), and a working example
//! of each, for `convert64 commands` and the MCP server's tool list (ADR-005).
//!
//! The arguments are those of a command in a recipe or project file
//! ([`convert64_project::JsonCommand`]), without `"command"`. The parity test
//! (`tests/parity.rs`) runs every example through `convert64 apply`, so a registry
//! command without a schema, an example or a CLI path fails the build.

use convert64_core::layer::DitherKind;
use serde_json::{Value, json};

/// An object schema with these properties, of which `required` must be present.
pub fn object_schema(properties: Value, required: &[&str]) -> Value {
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false
    })
}

/// A layer by stack index (0 = bottom) or name.
pub fn layer_schema() -> Value {
    json!({
        "description": "Layer index (0 = bottom) or name",
        "oneOf": [{ "type": "integer", "minimum": 0 }, { "type": "string" }]
    })
}

/// A region by position (0 = applied first) or name.
pub fn region_schema() -> Value {
    json!({
        "description": "Region index (0 = applied first) or name",
        "oneOf": [{ "type": "integer", "minimum": 0 }, { "type": "string" }]
    })
}

fn point_schema() -> Value {
    object_schema(
        json!({ "x": { "type": "number" }, "y": { "type": "number" } }),
        &["x", "y"],
    )
}

fn points_schema(min: usize, what: &str) -> Value {
    json!({ "type": "array", "items": point_schema(), "minItems": min, "description": what })
}

fn unit(description: &str, default: f32) -> Value {
    json!({ "type": "number", "minimum": 0, "maximum": 1, "default": default,
            "description": description })
}

fn combine_schema() -> Value {
    json!({ "type": "string", "enum": ["max", "min", "multiply"], "default": "max",
            "description": "How the source combines with the mask below it" })
}

fn paint_mode_schema() -> Value {
    json!({ "type": "string", "enum": ["add", "subtract"], "default": "add" })
}

fn cells_schema() -> Value {
    json!({
        "type": "array", "minItems": 1, "description": "Cells, x 0–39 and y 0–24",
        "items": object_schema(json!({
            "x": { "type": "integer", "minimum": 0, "maximum": 39 },
            "y": { "type": "integer", "minimum": 0, "maximum": 24 }
        }), &["x", "y"])
    })
}

fn colours_schema(min: usize, max: usize) -> Value {
    json!({ "type": "array", "minItems": min, "maxItems": max,
            "items": { "type": "integer", "minimum": 0, "maximum": 15 },
            "description": "VIC-II colour indices" })
}

fn gradient_schema() -> Value {
    object_schema(
        json!({
            "kind": { "type": "string", "enum": ["linear", "radial"] },
            "from": point_schema(),
            "to": point_schema(),
            "softness": unit("Fraction of the length that fades", 1.0)
        }),
        &["kind", "from", "to"],
    )
}

fn key_schema() -> Value {
    object_schema(
        json!({
            "channel": { "type": "string", "enum": ["luma", "chroma", "colour"] },
            "centre": { "type": "array", "items": { "type": "number", "minimum": 0, "maximum": 1 },
                        "minItems": 3, "maxItems": 3,
                        "description": "Keyed colour, linear sRGB 0–1" },
            "range": { "type": "number", "minimum": 0, "maximum": 2,
                       "description": "Full-strength OKLab distance" },
            "softness": { "type": "number", "minimum": 0, "maximum": 2, "default": 0,
                          "description": "Fall-off beyond the range" }
        }),
        &["channel", "centre", "range"],
    )
}

fn levels_schema() -> Value {
    object_schema(
        json!({
            "black": { "type": "number", "minimum": 0, "maximum": 1 },
            "white": { "type": "number", "minimum": 0, "maximum": 1 },
            "gamma": { "type": "number", "exclusiveMinimum": 0 }
        }),
        &["black", "white", "gamma"],
    )
}

fn snap_schema() -> Value {
    json!({ "type": "string", "enum": ["off", "expand", "contract", "nearest"] })
}

fn modifiers_schema() -> Value {
    object_schema(
        json!({
            "invert": { "type": "boolean", "default": false },
            "levels": levels_schema(),
            "snap": snap_schema(),
            "feather": { "type": "number", "minimum": 0, "maximum": 32, "default": 0,
                         "description": "Gaussian σ in target pixels" }
        }),
        &[],
    )
}

fn source_entry_schema() -> Value {
    object_schema(
        json!({
            "source": {
                "description": "A gradient or key with \"type\": \"gradient\" or \"key\"",
                "oneOf": [gradient_schema(), key_schema()]
            },
            "combine": combine_schema()
        }),
        &["source"],
    )
}

fn enum_schema(values: &[&str], description: &str) -> Value {
    json!({ "type": "string", "enum": values, "description": description })
}

fn layer_index_schema(description: &str) -> Value {
    json!({ "type": "integer", "minimum": 0, "description": description })
}

fn transform_schema() -> Value {
    object_schema(
        json!({
            "translate": point_schema(),
            "scale": object_schema(json!({
                "x": { "type": "number", "exclusiveMinimum": 0 },
                "y": { "type": "number", "exclusiveMinimum": 0 }
            }), &["x", "y"]),
            "rotate": { "type": "number", "default": 0,
                        "description": "Degrees about the anchor, clockwise on screen (ADR-011)" },
            "anchor": anchor_schema(),
            "flip_h": { "type": "boolean", "default": false },
            "flip_v": { "type": "boolean", "default": false }
        }),
        &[],
    )
}

/// A pivot: fractions of the layer's box, `(0, 0)` its top-left.
fn anchor_schema() -> Value {
    let mut s = object_schema(
        json!({
            "x": { "type": "number", "minimum": 0, "maximum": 1 },
            "y": { "type": "number", "minimum": 0, "maximum": 1 }
        }),
        &["x", "y"],
    );
    s["description"] = json!(
        "The pivot of the rotation, 0–1 across the layer's box ({x: 0.5, y: 0.5} the centre)"
    );
    s
}

/// A transform snap (ADR-011), not a mask's `snap_to_cells`.
fn transform_snap_schema(description: &str) -> Value {
    enum_schema(&["off", "c64-pixel", "cell"], description)
}

fn blend_schema() -> Value {
    json!({ "type": "string", "enum": ["normal", "multiply", "screen", "overlay"], "default": "normal" })
}

fn source_schema() -> Value {
    json!({
        "description": "The pixels: a PNG path relative to the file the command is in (or the working directory), {\"path\": …}, {\"png_base64\": …} or {\"layer\": n} (an existing layer's pixels, shared)",
        "oneOf": [
            { "type": "string" },
            object_schema(json!({ "path": { "type": "string" } }), &["path"]),
            object_schema(json!({ "png_base64": { "type": "string" } }), &["png_base64"]),
            object_schema(json!({ "layer": { "type": "integer", "minimum": 0 } }), &["layer"])
        ]
    })
}

fn method_schema() -> Value {
    let kinds: Vec<Value> = DitherKind::ALL
        .iter()
        .filter_map(|k| serde_json::to_value(k).ok())
        .collect();
    object_schema(
        json!({
            "kind": { "type": "string", "enum": kinds,
                      "description": "Dither method (docs/reference/dither-methods.md)" },
            "orientation": enum_schema(&["auto", "vertical-biased", "two-to-one"], "Orientation preference"),
            "custom": { "type": "array", "items": { "type": "integer", "minimum": 0 },
                        "description": "Threshold ranks of a custom matrix (kind \"custom\")" }
        }),
        &["kind"],
    )
}

/// The recipe fields, every one optional (a `recipe_set` change; a whole recipe needs
/// `method` and `strength`).
fn recipe_properties() -> Value {
    json!({
        "method": method_schema(),
        "strength": unit("Dither strength", 1.0),
        "phase": enum_schema(&["screen", "layer"], "Pattern locked to the screen or to the layer"),
        "metric": enum_schema(&["oklab", "cielab", "pepto-yuv", "rgb"], "Colour metric"),
        "allowed": colours_schema(1, 16),
        "fit_mode": enum_schema(&["adaptive", "force"], "Adaptive or Force"),
        "cell_clamp": { "type": "boolean", "description": "Native solver: keep diffused error inside a cell's colours" },
        "max_luma_diff": { "type": "integer", "minimum": 0, "maximum": 8,
                           "description": "Native solver: luma levels a mix may span (8 = off)" },
        "adaptive": unit("Native solver: adaptive strength", 0.0),
        "mix_levels": { "type": "integer", "minimum": 1, "maximum": 16,
                        "description": "Native solver: mix ratios per pair" },
        "smooth": { "type": "boolean", "description": "Native solver: smooth gradients" },
        "cell_metric": enum_schema(&["mse", "blur", "ssim"], "Native solver: the cell score"),
        "error_clamp": { "type": "number", "exclusiveMinimum": 0,
                         "description": "Diffused error clamp per metric channel" },
        "ordered_semantics": enum_schema(&["png2c64", "legacy"], "Ordered threshold semantics"),
        "chroma_guard": { "type": "boolean", "description": "Native solver: the chroma guard" }
    })
}

fn recipe_schema() -> Value {
    object_schema(recipe_properties(), &["method", "strength"])
}

fn recipe_change_schema() -> Value {
    let mut props = recipe_properties();
    let p = props.as_object_mut().expect("object");
    p.insert(
        "allow".to_owned(),
        json!({ "type": "array", "items": { "type": "integer", "minimum": 0, "maximum": 15 },
                "description": "Colours added to the allowed set" }),
    );
    p.insert(
        "deny".to_owned(),
        json!({ "type": "array", "items": { "type": "integer", "minimum": 0, "maximum": 15 },
                "description": "Colours taken out of the allowed set" }),
    );
    let mut schema = object_schema(props, &[]);
    schema["description"] = json!(
        "The recipe fields that change; the others are kept. Allowed colours: `allowed` (or the current set), plus `allow`, minus `deny`."
    );
    schema
}

fn adjustments_properties() -> Value {
    let signed = |d: &str| json!({ "type": "number", "minimum": -1, "maximum": 1, "default": 0, "description": d });
    let curve = json!({ "type": "array", "items": { "type": "array", "items": { "type": "number", "minimum": 0, "maximum": 1 }, "minItems": 2, "maxItems": 2 },
                        "description": "Control points [x, y], 0–1" });
    json!({
        "gamma": { "type": "number", "minimum": 0.1, "maximum": 8, "default": 1 },
        "levels": levels_schema(),
        "brightness_contrast": object_schema(json!({
            "brightness": signed("Brightness, linear light"),
            "contrast": signed("Contrast")
        }), &[]),
        "hsl": object_schema(json!({
            "hue": { "type": "number", "default": 0, "description": "Hue rotation, degrees" },
            "saturation": signed("Saturation change"),
            "lightness": signed("Lightness change")
        }), &[]),
        "curves": object_schema(json!({
            "master": curve.clone(), "red": curve.clone(), "green": curve.clone(), "blue": curve
        }), &[]),
        "match_range": {
            "description": "OKLab range matching (a project file's form), or null to turn it off",
            "oneOf": [
                { "type": "null" },
                object_schema(json!({
                    "percentile": { "type": "number" },
                    "l": { "type": "array", "items": { "type": "number" }, "minItems": 2, "maxItems": 2 },
                    "a": { "type": "array", "items": { "type": "number" }, "minItems": 2, "maxItems": 2 },
                    "b": { "type": "array", "items": { "type": "number" }, "minItems": 2, "maxItems": 2 }
                }), &["percentile", "l", "a", "b"])
            ]
        }
    })
}

fn palette_schema() -> Value {
    json!({
        "description": "A preset, 16 colours in VIC-II index order, or a VICE .vpl file relative to the file the command is in (or the working directory)",
        "oneOf": [
            { "type": "string", "enum": ["pepto", "pepto-pal", "pepto-ntsc", "colodore"] },
            object_schema(json!({ "custom": { "type": "array", "minItems": 16, "maxItems": 16,
                "items": { "type": "string", "pattern": "^#?[0-9A-Fa-f]{6}$" } } }), &["custom"]),
            object_schema(json!({ "vpl": { "type": "string" } }), &["vpl"])
        ]
    })
}

fn divergences_schema() -> Value {
    let mut schema = object_schema(
        json!({
            "chroma_guard": { "type": "boolean", "default": false },
            "max_luma_diff": { "type": ["integer", "null"], "minimum": 0, "maximum": 8, "default": null },
            "pair_bias": { "type": "number", "default": 0 },
            "cell_clamp": { "type": "boolean", "default": false },
            "mix_levels": { "type": ["integer", "null"], "minimum": 1, "default": null },
            "bridging": { "type": "boolean", "default": false }
        }),
        &[],
    );
    schema["description"] = json!(
        "The reference solver's named divergences from png2c64 (docs/reference/png2c64-reference.md), as a whole: those not named are off"
    );
    schema
}

/// The JSON Schema of a registry command's arguments, or `None` for a name that is not
/// a command.
pub fn command_schema(name: &str) -> Option<Value> {
    let layer = || layer_schema();
    Some(match name {
        "mask_stroke" => object_schema(
            json!({
                "layer": layer(),
                "points": points_schema(1, "The polyline, layer pixels"),
                "radius": { "type": "number", "minimum": 0.25, "maximum": 1000,
                            "description": "Brush radius, layer pixels" },
                "hardness": unit("Fraction of the radius at full strength", 0.0),
                "flow": unit("Strength per dab", 1.0),
                "mode": paint_mode_schema()
            }),
            &["layer", "points", "radius"],
        ),
        "mask_fill" => object_schema(
            json!({
                "layer": layer(),
                "polygon": points_schema(3, "The polygon, layer pixels"),
                "value": unit("Fill value", 1.0),
                "mode": paint_mode_schema()
            }),
            &["layer", "polygon"],
        ),
        "mask_gradient" => object_schema(
            json!({ "layer": layer(), "gradient": gradient_schema(), "combine": combine_schema() }),
            &["layer", "gradient"],
        ),
        "mask_key" => object_schema(
            json!({ "layer": layer(), "key": key_schema(), "combine": combine_schema() }),
            &["layer", "key"],
        ),
        "mask_set_modifier" => object_schema(
            json!({
                "layer": layer(),
                "modifier": {
                    "description": "Exactly one modifier",
                    "oneOf": [
                        object_schema(json!({ "invert": { "type": "boolean" } }), &["invert"]),
                        object_schema(json!({ "levels": levels_schema() }), &["levels"]),
                        object_schema(json!({ "snap": snap_schema() }), &["snap"]),
                        object_schema(json!({ "feather": { "type": "number", "minimum": 0, "maximum": 32 } }), &["feather"])
                    ]
                }
            }),
            &["layer", "modifier"],
        ),
        "mask_load" => object_schema(
            json!({
                "layer": layer(),
                "path": { "type": "string",
                          "description": "Greyscale PNG (8 or 16 bit) of the layer's size, relative to the file the command is in (or the working directory on the command line)" }
            }),
            &["layer", "path"],
        ),
        "mask_clear" => object_schema(
            json!({ "layer": layer(), "value": unit("The whole raster's value: 0 clears, 1 fills", 0.0) }),
            &["layer"],
        ),
        "mask_from_alpha" => object_schema(json!({ "layer": layer() }), &["layer"]),
        "layer_via_mask" => object_schema(
            json!({
                "source_layer": layer(),
                "selection": object_schema(json!({
                    "polygons": { "type": "array", "items": points_schema(3, "A lasso polygon, layer pixels") },
                    "sources": { "type": "array", "items": source_entry_schema() },
                    "modifiers": modifiers_schema()
                }), &[]),
                "cut": { "type": "boolean", "default": false,
                         "description": "Also remove the selection from the original" }
            }),
            &["source_layer", "selection"],
        ),
        "cell_pin" => object_schema(
            json!({
                "cells": cells_schema(),
                "colours": colours_schema(1, 4),
                "full": { "type": "boolean", "default": false,
                          "description": "The set is exactly these colours (else it must contain them)" }
            }),
            &["cells", "colours"],
        ),
        "cell_lock" => object_schema(
            json!({ "cells": cells_schema(), "locked": { "type": "boolean", "default": true } }),
            &["cells"],
        ),
        "cell_clear" => object_schema(json!({ "cells": cells_schema() }), &["cells"]),
        "cell_allow" | "cell_deny" => object_schema(
            json!({ "cells": cells_schema(), "colours": colours_schema(1, 16) }),
            &["cells", "colours"],
        ),
        "layer_add" => object_schema(
            json!({
                "name": { "type": "string" },
                "source": source_schema(),
                "index": layer_index_schema("Stack position, 0 = bottom; absent puts it on top"),
                "visible": { "type": "boolean", "default": true },
                "transform": transform_schema(),
                "opacity": unit("Opacity", 1.0),
                "blend": blend_schema(),
                "recipe": recipe_schema(),
                "adjustments": object_schema(adjustments_properties(), &[])
            }),
            &["name", "source"],
        ),
        "layer_remove" | "layer_duplicate" => {
            object_schema(json!({ "layer": layer() }), &["layer"])
        }
        "layer_reorder" => object_schema(
            json!({ "layer": layer(), "to": layer_index_schema("The new stack position, 0 = bottom") }),
            &["layer", "to"],
        ),
        "layer_set_visible" => object_schema(
            json!({ "layer": layer(), "visible": { "type": "boolean" } }),
            &["layer", "visible"],
        ),
        "layer_set_opacity" => object_schema(
            json!({ "layer": layer(), "opacity": { "type": "number", "minimum": 0, "maximum": 1 } }),
            &["layer", "opacity"],
        ),
        "layer_set_blend" => object_schema(
            json!({ "layer": layer(), "blend": blend_schema() }),
            &["layer", "blend"],
        ),
        "layer_set_transform" => object_schema(
            json!({ "layer": layer(), "transform": transform_schema(),
                    "snap": transform_snap_schema("Snap this way instead of by the project's setting") }),
            &["layer", "transform"],
        ),
        "layer_rotate" => object_schema(
            json!({ "layer": layer(),
                    "degrees": { "type": "number", "description": "The angle, clockwise on screen (or the change, with relative)" },
                    "relative": { "type": "boolean", "default": false, "description": "Add degrees to the rotation instead of setting it" },
                    "anchor": anchor_schema(),
                    "snap": transform_snap_schema("Snap this way instead of by the project's setting") }),
            &["layer", "degrees"],
        ),
        "layer_nudge" => object_schema(
            json!({ "layer": layer(), "by": point_schema() }),
            &["layer", "by"],
        ),
        "layer_rename" => object_schema(
            json!({ "layer": layer(), "name": { "type": "string", "minLength": 1 } }),
            &["layer", "name"],
        ),
        "adjustments_set" => object_schema(
            json!({
                "layer": layer(),
                "adjustments": {
                    "description": "The adjustments that change; the others are kept",
                    "type": "object",
                    "properties": adjustments_properties(),
                    "additionalProperties": false
                }
            }),
            &["layer", "adjustments"],
        ),
        "recipe_set" => object_schema(
            json!({ "layer": layer(), "recipe": recipe_change_schema() }),
            &["layer", "recipe"],
        ),
        "mode_set" => object_schema(
            json!({ "mode": enum_schema(&["multicolor-bitmap", "hires-bitmap"], "Target mode") }),
            &["mode"],
        ),
        "palette_set" => object_schema(json!({ "palette": palette_schema() }), &["palette"]),
        "background_pin" => object_schema(
            json!({
                "background": {
                    "description": "{\"fixed\": n} pins colour n (multicolor); \"auto\" searches all 16",
                    "oneOf": [
                        { "type": "string", "enum": ["auto"] },
                        object_schema(json!({ "fixed": { "type": "integer", "minimum": 0, "maximum": 15 } }), &["fixed"])
                    ]
                }
            }),
            &["background"],
        ),
        "snap_set" => object_schema(
            json!({ "snap": transform_snap_schema("What layer_set_transform and layer_rotate round to: off, whole target pixels and 0.5° (c64-pixel), or cells and 15° (cell)") }),
            &["snap"],
        ),
        "mask_source_remove" => object_schema(
            json!({
                "layer": layer(),
                "index": layer_index_schema("Position in the mask's sources list, 0 = first applied")
            }),
            &["layer", "index"],
        ),
        "mask_source_set_combine" => object_schema(
            json!({
                "layer": layer(),
                "index": layer_index_schema("Position in the mask's sources list, 0 = first applied"),
                "combine": combine_schema()
            }),
            &["layer", "index", "combine"],
        ),
        "region_add" => object_schema(
            json!({
                "name": { "type": "string", "minLength": 1 },
                "recipe": recipe_schema(),
                "index": layer_index_schema("Position, 0 = applied first; absent puts it last (on top)")
            }),
            &["name"],
        ),
        "region_remove" => object_schema(json!({ "region": region_schema() }), &["region"]),
        "region_rename" => object_schema(
            json!({ "region": region_schema(), "name": { "type": "string", "minLength": 1 } }),
            &["region", "name"],
        ),
        "region_reorder" => object_schema(
            json!({ "region": region_schema(),
                    "to": layer_index_schema("The new position, 0 = applied first (later regions win)") }),
            &["region", "to"],
        ),
        "region_set_recipe" => object_schema(
            json!({ "region": region_schema(), "recipe": recipe_change_schema() }),
            &["region", "recipe"],
        ),
        "region_stroke" => object_schema(
            json!({
                "region": region_schema(),
                "points": points_schema(1, "The polyline, canvas pixels (320 × 200)"),
                "radius": { "type": "number", "minimum": 0.25, "maximum": 1000,
                            "description": "Brush radius, canvas pixels" },
                "hardness": unit("Fraction of the radius at full strength", 0.0),
                "flow": unit("Strength per dab", 1.0),
                "mode": paint_mode_schema()
            }),
            &["region", "points", "radius"],
        ),
        "region_fill" => object_schema(
            json!({
                "region": region_schema(),
                "polygon": points_schema(3, "The polygon, canvas pixels (320 × 200)"),
                "value": unit("Fill value", 1.0),
                "mode": paint_mode_schema()
            }),
            &["region", "polygon"],
        ),
        "region_gradient" => object_schema(
            json!({
                "region": region_schema(),
                "gradient": gradient_schema(),
                "combine": {
                    "type": "string", "enum": ["max", "min", "multiply"], "default": "max",
                    "description": "How the gradient (canvas pixels) combines with the region's mask; it is baked into the mask"
                }
            }),
            &["region", "gradient"],
        ),
        "region_clear" => object_schema(
            json!({ "region": region_schema(), "value": unit("The whole mask's value: 0 clears, 1 fills", 0.0) }),
            &["region"],
        ),
        "solve_set" => object_schema(
            json!({
                "solver": enum_schema(&["reference", "native"], "png2c64's pipeline (reference) or Convert64's own (native)"),
                "divergences": divergences_schema(),
                "hysteresis": { "type": ["number", "null"], "exclusiveMinimum": 0, "maximum": 1,
                                "description": "Refinement hysteresis ratio; null turns it off" },
                "luma_pair_bias": { "type": "number", "description": "Same-luma pair bias per pair (native solver)" },
                "refinements": { "type": "integer", "minimum": 0, "maximum": 2 },
                "finalists": { "type": "integer", "minimum": 1, "maximum": 16 }
            }),
            &[],
        ),
        _ => return None,
    })
}

/// A working example of a registry command, as a recipe or project file's `commands`
/// entry. It assumes a project of at least two layers whose layer 0 is at least 32 × 32
/// pixels and whose layer 1 has a mask with at least one source, with two regions,
/// `sky` (painted) and `ground`, and with a greyscale `mask.png` of layer 0's size
/// (`mask_load`) and a PNG `photo.png` (`layer_add`) next to it.
pub fn example(name: &str) -> Option<Value> {
    let square =
        json!([{ "x": 4, "y": 4 }, { "x": 28, "y": 4 }, { "x": 28, "y": 28 }, { "x": 4, "y": 28 }]);
    let args = match name {
        "mask_stroke" => json!({ "layer": 0, "points": [{ "x": 4, "y": 4 }, { "x": 28, "y": 20 }],
                                 "radius": 3.0, "hardness": 0.5, "mode": "subtract" }),
        "mask_fill" => json!({ "layer": 0, "polygon": square, "value": 0.5 }),
        "mask_gradient" => json!({ "layer": 0, "combine": "multiply",
            "gradient": { "kind": "linear", "from": { "x": 0, "y": 0 }, "to": { "x": 32, "y": 0 } } }),
        "mask_key" => json!({ "layer": 0,
            "key": { "channel": "chroma", "centre": [0.0, 0.58, 0.0], "range": 0.12, "softness": 0.1 } }),
        "mask_set_modifier" => json!({ "layer": 0, "modifier": { "feather": 2.0 } }),
        "mask_load" => json!({ "layer": 0, "path": "mask.png" }),
        "mask_clear" => json!({ "layer": 0, "value": 1.0 }),
        "mask_from_alpha" => json!({ "layer": 0 }),
        "layer_via_mask" => {
            json!({ "source_layer": 0, "selection": { "polygons": [square] }, "cut": true })
        }
        "cell_pin" => {
            json!({ "cells": [{ "x": 3, "y": 4 }], "colours": [0, 1, 2, 5], "full": true })
        }
        "cell_lock" => json!({ "cells": [{ "x": 39, "y": 24 }] }),
        "cell_clear" => json!({ "cells": [{ "x": 3, "y": 4 }] }),
        "cell_allow" => json!({ "cells": [{ "x": 5, "y": 5 }], "colours": [0, 1, 2] }),
        "cell_deny" => json!({ "cells": [{ "x": 6, "y": 5 }], "colours": [7] }),
        "layer_add" => json!({ "name": "overlay", "source": "photo.png", "index": 1,
            "opacity": 0.5, "blend": "screen",
            "transform": { "translate": { "x": 16, "y": 8 }, "scale": { "x": 2, "y": 2 } } }),
        "layer_remove" => json!({ "layer": 1 }),
        "layer_reorder" => json!({ "layer": 0, "to": 1 }),
        "layer_set_visible" => json!({ "layer": 1, "visible": false }),
        "layer_set_opacity" => json!({ "layer": 0, "opacity": 0.5 }),
        "layer_set_blend" => json!({ "layer": 1, "blend": "multiply" }),
        "layer_set_transform" => json!({ "layer": 0, "transform": {
            "translate": { "x": 8, "y": 4 }, "scale": { "x": 2.5, "y": 2.5 }, "rotate": -12.5,
            "anchor": { "x": 0, "y": 1 }, "flip_h": true } }),
        "layer_nudge" => json!({ "layer": 0, "by": { "x": 0.5, "y": -1 } }),
        "layer_rotate" => json!({ "layer": 0, "degrees": 2.5, "anchor": { "x": 0.5, "y": 1 } }),
        "layer_rename" => json!({ "layer": 0, "name": "background" }),
        "layer_duplicate" => json!({ "layer": 0 }),
        "adjustments_set" => json!({ "layer": 0, "adjustments": { "gamma": 1.4,
            "brightness_contrast": { "brightness": 0.1, "contrast": 0.2 } } }),
        "recipe_set" => json!({ "layer": 0, "recipe": { "method": { "kind": "bayer4" },
            "strength": 0.75, "deny": [7] } }),
        "mode_set" => json!({ "mode": "hires-bitmap" }),
        "palette_set" => json!({ "palette": "colodore" }),
        "background_pin" => json!({ "background": { "fixed": 0 } }),
        "solve_set" => json!({ "solver": "native", "divergences": { "chroma_guard": true },
            "hysteresis": 0.97 }),
        "snap_set" => json!({ "snap": "c64-pixel" }),
        "mask_source_remove" => json!({ "layer": 1, "index": 0 }),
        "mask_source_set_combine" => json!({ "layer": 1, "index": 0, "combine": "min" }),
        "region_add" => json!({ "name": "faces", "index": 1,
            "recipe": { "method": { "kind": "atkinson" }, "strength": 0.8 } }),
        "region_remove" => json!({ "region": "ground" }),
        "region_rename" => json!({ "region": 0, "name": "clouds" }),
        "region_reorder" => json!({ "region": "sky", "to": 1 }),
        "region_set_recipe" => json!({ "region": "sky", "recipe": { "method": { "kind": "bayer8" },
            "strength": 0.5, "allowed": [0, 6, 14, 1] } }),
        "region_stroke" => json!({ "region": "sky",
            "points": [{ "x": 200, "y": 40 }, { "x": 300, "y": 120 }], "radius": 8.0, "hardness": 0.5 }),
        "region_fill" => json!({ "region": "ground",
            "polygon": [{ "x": 0, "y": 120 }, { "x": 320, "y": 120 }, { "x": 320, "y": 200 }, { "x": 0, "y": 200 }] }),
        "region_gradient" => json!({ "region": "sky", "combine": "multiply",
            "gradient": { "kind": "linear", "from": { "x": 0, "y": 0 }, "to": { "x": 0, "y": 200 } } }),
        "region_clear" => json!({ "region": "sky" }),
        _ => return None,
    };
    let mut v = json!({ "command": name });
    v.as_object_mut()
        .expect("object")
        .extend(args.as_object().expect("object").clone());
    Some(v)
}

/// The arguments of a schema in a line: the required ones in order, then the optional
/// ones marked `?` (`layer, points, radius, flow?, hardness?, mode?`).
pub fn signature(schema: &Value) -> String {
    let required: Vec<&str> = schema["required"]
        .as_array()
        .map(|r| r.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let optional = schema["properties"]
        .as_object()
        .into_iter()
        .flat_map(|p| p.keys())
        .filter(|k| !required.contains(&k.as_str()))
        .map(|k| format!("{k}?"));
    required
        .iter()
        .map(|k| (*k).to_owned())
        .chain(optional)
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use convert64_core::command::REGISTRY;
    use convert64_project::JsonCommand;

    #[test]
    fn every_registry_command_has_a_schema_and_an_example_that_parses() {
        for c in REGISTRY {
            let schema = command_schema(c.name).unwrap_or_else(|| panic!("{}: no schema", c.name));
            let ex = example(c.name).unwrap_or_else(|| panic!("{}: no example", c.name));
            serde_json::from_value::<JsonCommand>(ex.clone())
                .unwrap_or_else(|e| panic!("{}: {e}", c.name));
            // Every argument the example gives is in the schema, and every required one
            // is given.
            let props = schema["properties"].as_object().unwrap();
            for k in ex.as_object().unwrap().keys().filter(|k| *k != "command") {
                assert!(props.contains_key(k), "{}: {k} not in the schema", c.name);
            }
            for r in schema["required"].as_array().unwrap() {
                assert!(
                    ex.get(r.as_str().unwrap()).is_some(),
                    "{}: {r} missing",
                    c.name
                );
            }
        }
        assert!(command_schema("paint_pixel").is_none());
        assert!(example("paint_pixel").is_none());
    }

    #[test]
    fn signatures_mark_optional_arguments() {
        let s = signature(&command_schema("mask_clear").unwrap());
        assert_eq!(s, "layer, value?");
    }
}
