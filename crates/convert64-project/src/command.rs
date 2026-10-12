//! Registry commands as JSON: `{"command": "<name>", ...arguments}`, as a project or
//! recipe file's `commands` lists them and `convert64 apply` and the MCP server take
//! them (ADR-005). [`Session::to_command`](crate::Session::to_command) turns one into
//! the core's [`Command`](convert64_core::command::Command), resolving layer names and
//! reading any file it names.

use convert64_core::command::{
    CellAllow, CellClear, CellDeny, CellLock, CellPin, ModifierChange, Selection,
};
use convert64_core::geom::Vec2;
use convert64_core::mask::{Combine, Gradient, Key, PaintMode};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const fn one() -> f32 {
    1.0
}

/// A layer by stack index (0 = bottom) or by name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum LayerRef {
    /// Stack index, bottom first.
    Index(usize),
    /// Layer name.
    Name(String),
}

/// A command as JSON: `{"command": "<name>", ...arguments}`. Names are the core
/// registry's (`convert64_core::command::REGISTRY`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
pub enum JsonCommand {
    /// Soft round brush along a polyline.
    MaskStroke {
        /// Layer.
        layer: LayerRef,
        /// Polyline, layer pixels.
        points: Vec<Vec2>,
        /// Radius, layer pixels.
        radius: f32,
        /// `0..=1`.
        #[serde(default)]
        hardness: f32,
        /// `0..=1`.
        #[serde(default = "one")]
        flow: f32,
        /// `add` or `subtract`.
        #[serde(default)]
        mode: PaintMode,
    },
    /// Polygon (lasso) fill.
    MaskFill {
        /// Layer.
        layer: LayerRef,
        /// Polygon, layer pixels.
        polygon: Vec<Vec2>,
        /// `0..=1`.
        #[serde(default = "one")]
        value: f32,
        /// `add` or `subtract`.
        #[serde(default)]
        mode: PaintMode,
    },
    /// Gradient source.
    MaskGradient {
        /// Layer.
        layer: LayerRef,
        /// The gradient.
        gradient: Gradient,
        /// `max`, `min` or `multiply`.
        #[serde(default)]
        combine: Combine,
    },
    /// Key source.
    MaskKey {
        /// Layer.
        layer: LayerRef,
        /// The key.
        key: Key,
        /// `max`, `min` or `multiply`.
        #[serde(default)]
        combine: Combine,
    },
    /// One modifier: `{"invert": true}`, `{"levels": {...}}`, `{"snap": "expand"}` or
    /// `{"feather": 3.0}`.
    MaskSetModifier {
        /// Layer.
        layer: LayerRef,
        /// The change.
        modifier: ModifierChange,
    },
    /// Greyscale PNG (8 or 16 bit) of the layer's size, relative to the recipe file.
    MaskLoad {
        /// Layer.
        layer: LayerRef,
        /// PNG path.
        path: PathBuf,
    },
    /// Whole raster to one value (0 clears, 1 fills).
    MaskClear {
        /// Layer.
        layer: LayerRef,
        /// `0..=1`.
        #[serde(default)]
        value: f32,
    },
    /// PNG alpha into the mask.
    MaskFromAlpha {
        /// Layer.
        layer: LayerRef,
    },
    /// Pin colours on cells (`{"x": 3, "y": 4}` positions, colours 0–15).
    CellPin(CellPin),
    /// Lock cells (`"locked": false` unlocks).
    CellLock(CellLock),
    /// Clear all steering on cells.
    CellClear(CellClear),
    /// Restrict cells to colours.
    CellAllow(CellAllow),
    /// Forbid colours in cells.
    CellDeny(CellDeny),
    /// New layer sharing the source pixels with a selection as its mask.
    LayerViaMask {
        /// The layer to split.
        source_layer: LayerRef,
        /// The selection.
        selection: Selection,
        /// Remove the selection from the original too.
        #[serde(default)]
        cut: bool,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use convert64_core::ColourIndex;
    use convert64_core::layer::Levels;
    use convert64_core::mask::{
        GradientKind, KeyChannel, MaskSource, Modifiers, SnapToCells, SourceEntry,
    };
    use convert64_core::model::CellPos;

    fn every_command() -> Vec<JsonCommand> {
        let l = LayerRef::Name("photo".into());
        vec![
            JsonCommand::MaskStroke {
                layer: l.clone(),
                points: vec![Vec2::new(1.0, 2.0), Vec2::new(3.5, 4.0)],
                radius: 6.0,
                hardness: 0.25,
                flow: 0.5,
                mode: PaintMode::Subtract,
            },
            JsonCommand::MaskFill {
                layer: LayerRef::Index(0),
                polygon: vec![Vec2::ZERO, Vec2::new(5.0, 0.0), Vec2::new(0.0, 5.0)],
                value: 0.75,
                mode: PaintMode::Add,
            },
            JsonCommand::MaskGradient {
                layer: l.clone(),
                gradient: Gradient::new(
                    GradientKind::Radial,
                    Vec2::new(5.0, 5.0),
                    Vec2::new(9.0, 5.0),
                    0.5,
                )
                .unwrap(),
                combine: Combine::Multiply,
            },
            JsonCommand::MaskKey {
                layer: l.clone(),
                key: Key::new(KeyChannel::Chroma, [0.0, 1.0, 0.0], 0.1, 0.05).unwrap(),
                combine: Combine::Min,
            },
            JsonCommand::MaskSetModifier {
                layer: l.clone(),
                modifier: ModifierChange::Feather(3.0),
            },
            JsonCommand::MaskSetModifier {
                layer: l.clone(),
                modifier: ModifierChange::Invert(true),
            },
            JsonCommand::MaskSetModifier {
                layer: l.clone(),
                modifier: ModifierChange::Snap(SnapToCells::Expand),
            },
            JsonCommand::MaskSetModifier {
                layer: l.clone(),
                modifier: ModifierChange::Levels(Levels::new(0.1, 0.9, 1.2).unwrap()),
            },
            JsonCommand::MaskLoad {
                layer: l.clone(),
                path: "mask.png".into(),
            },
            JsonCommand::MaskClear {
                layer: l.clone(),
                value: 1.0,
            },
            JsonCommand::MaskFromAlpha { layer: l.clone() },
            JsonCommand::CellPin(CellPin {
                cells: vec![CellPos::new(3, 4)],
                colours: vec![ColourIndex::RED, ColourIndex::WHITE],
                full: true,
            }),
            JsonCommand::CellLock(CellLock {
                cells: vec![CellPos::new(0, 0), CellPos::new(39, 24)],
                locked: false,
            }),
            JsonCommand::CellClear(CellClear {
                cells: vec![CellPos::new(1, 1)],
            }),
            JsonCommand::CellAllow(CellAllow {
                cells: vec![CellPos::new(2, 2)],
                colours: vec![ColourIndex::BLACK],
            }),
            JsonCommand::CellDeny(CellDeny {
                cells: vec![CellPos::new(2, 2)],
                colours: vec![ColourIndex::YELLOW],
            }),
            JsonCommand::LayerViaMask {
                source_layer: l,
                selection: Selection {
                    polygons: vec![vec![Vec2::ZERO, Vec2::new(4.0, 0.0), Vec2::new(4.0, 4.0)]],
                    sources: vec![SourceEntry {
                        source: MaskSource::Gradient(
                            Gradient::new(GradientKind::Linear, Vec2::ZERO, Vec2::ONE, 1.0)
                                .unwrap(),
                        ),
                        combine: Combine::Max,
                    }],
                    modifiers: Modifiers::feathered(1.0).unwrap(),
                },
                cut: true,
            },
        ]
    }

    #[test]
    fn every_command_round_trips_through_json() {
        let all = every_command();
        let names: std::collections::BTreeSet<String> = all
            .iter()
            .map(|c| {
                serde_json::to_value(c).unwrap()["command"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            })
            .collect();
        let registry: std::collections::BTreeSet<String> = convert64_core::command::REGISTRY
            .iter()
            .map(|c| c.name.to_owned())
            .collect();
        assert_eq!(names, registry, "every registry command has a JSON form");
        for c in all {
            let text = serde_json::to_string(&c).unwrap();
            let back: JsonCommand = serde_json::from_str(&text).unwrap();
            assert_eq!(back, c, "{text}");
        }
    }

    #[test]
    fn json_goes_through_the_core_constructors() {
        let bad = r#"{"command":"mask_set_modifier","layer":0,"modifier":{"feather":-1}}"#;
        // Feather is validated when the command runs; the shape itself parses.
        assert!(serde_json::from_str::<JsonCommand>(bad).is_ok());
        let bad_gradient = r#"{"command":"mask_gradient","layer":0,"gradient":{"kind":"linear","from":{"x":1,"y":1},"to":{"x":1,"y":1}}}"#;
        assert!(serde_json::from_str::<JsonCommand>(bad_gradient).is_err());
        let pin = r#"{"command":"cell_pin","cells":[{"x":3,"y":4}],"colours":[2,1],"full":true}"#;
        assert!(matches!(
            serde_json::from_str::<JsonCommand>(pin),
            Ok(JsonCommand::CellPin(_))
        ));
        let unknown = r#"{"command":"mask_paint_colour","layer":0}"#;
        assert!(serde_json::from_str::<JsonCommand>(unknown).is_err());
    }
}
