//! The argument schema of every registry command (JSON Schema), and a working example
//! of each, for `convert64 commands` and the MCP server's tool list (ADR-005).
//!
//! The arguments are those of a command in a recipe or project file
//! ([`crate::recipe::JsonCommand`]), without `"command"`. The parity test
//! (`tests/parity.rs`) runs every example through `convert64 apply`, so a registry
//! command without a schema, an example or a CLI path fails the build.

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
        _ => return None,
    })
}

/// A working example of a registry command, as a recipe or project file's `commands`
/// entry. It assumes layer 0 is at least 32 × 32 pixels and, for `mask_load`, a
/// greyscale `mask.png` of layer 0's size next to it.
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
    use crate::recipe::JsonCommand;
    use convert64_core::command::REGISTRY;

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
