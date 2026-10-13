//! Registry commands as JSON: `{"command": "<name>", ...arguments}`, as a project or
//! recipe file's `commands` lists them and `convert64 apply` and the MCP server take
//! them (ADR-005). [`Session::to_command`](crate::Session::to_command) turns one into
//! the core's [`Command`](convert64_core::command::Command), resolving layer and region
//! names and reading any file it names.

use crate::file::{PaletteSpec, SourceSpec};
use convert64_core::command::{
    AdjustmentsChange, BackgroundPin, CellAllow, CellClear, CellDeny, CellLock, CellPin, ModeSet,
    ModifierChange, RecipeChange, Selection, SnapSet, SolveSet,
};
use convert64_core::geom::Vec2;
use convert64_core::layer::{Adjustments, Blend, Recipe, Snap, Transform};
use convert64_core::mask::{Combine, Gradient, Key, PaintMode};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const fn one() -> f32 {
    1.0
}

const fn yes() -> bool {
    true
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

/// A region by position (0 = applied first) or by name, as [`LayerRef`] names a layer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RegionRef {
    /// Position, 0 = applied first.
    Index(usize),
    /// Region name.
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
    /// New layer from a PNG (linked or embedded) or an earlier layer's pixels.
    LayerAdd {
        /// Display name.
        name: String,
        /// The pixels: a path relative to the file the command is in (or the working
        /// directory), `{"png_base64": …}` or `{"layer": n}`.
        source: SourceSpec,
        /// Stack position, 0 = bottom; absent puts it on top.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        index: Option<usize>,
        /// Whether it takes part in the composite.
        #[serde(default = "yes")]
        visible: bool,
        /// Placement in canvas pixels.
        #[serde(default)]
        transform: Transform,
        /// Opacity, `0..=1`.
        #[serde(default = "one")]
        opacity: f32,
        /// Blend mode.
        #[serde(default)]
        blend: Blend,
        /// Dither recipe.
        #[serde(default)]
        recipe: Box<Recipe>,
        /// Colour adjustments.
        #[serde(default)]
        adjustments: Box<Adjustments>,
    },
    /// Remove a layer.
    LayerRemove {
        /// Layer.
        layer: LayerRef,
    },
    /// Move a layer to stack position `to` (0 = bottom).
    LayerReorder {
        /// Layer.
        layer: LayerRef,
        /// Its new position.
        to: usize,
    },
    /// Show or hide a layer.
    LayerSetVisible {
        /// Layer.
        layer: LayerRef,
        /// Visible.
        visible: bool,
    },
    /// Set a layer's opacity.
    LayerSetOpacity {
        /// Layer.
        layer: LayerRef,
        /// `0..=1`.
        opacity: f32,
    },
    /// Set a layer's blend mode.
    LayerSetBlend {
        /// Layer.
        layer: LayerRef,
        /// `normal`, `multiply`, `screen` or `overlay`.
        blend: Blend,
    },
    /// Place a layer, snapped by the project's setting or `snap`.
    LayerSetTransform {
        /// Layer.
        layer: LayerRef,
        /// Translate, scale, rotate, anchor, flips.
        transform: Transform,
        /// Snap this way instead of by the project's setting.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        snap: Option<Snap>,
    },
    /// Turn a layer about its anchor, to or by `degrees` (clockwise on screen).
    LayerRotate {
        /// Layer.
        layer: LayerRef,
        /// The angle, or the change with `relative`.
        degrees: f32,
        /// Add `degrees` to the rotation instead of setting it.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        relative: bool,
        /// Move the pivot here first (0–1 across the box), keeping the picture in place.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        anchor: Option<Vec2>,
        /// Snap this way instead of by the project's setting.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        snap: Option<Snap>,
    },
    /// Move a layer by an offset, canvas pixels.
    LayerNudge {
        /// Layer.
        layer: LayerRef,
        /// The offset.
        by: Vec2,
    },
    /// Rename a layer.
    LayerRename {
        /// Layer.
        layer: LayerRef,
        /// The new name.
        name: String,
    },
    /// Copy a layer just above itself, sharing its pixels.
    LayerDuplicate {
        /// Layer.
        layer: LayerRef,
    },
    /// Set some of a layer's adjustments.
    AdjustmentsSet {
        /// Layer.
        layer: LayerRef,
        /// The adjustments that change.
        adjustments: AdjustmentsChange,
    },
    /// Set some of a layer's recipe.
    RecipeSet {
        /// Layer.
        layer: LayerRef,
        /// The recipe fields that change.
        recipe: RecipeChange,
    },
    /// Set the target mode.
    ModeSet(ModeSet),
    /// Set the palette (a `.vpl` path is relative to the file the command is in, or the
    /// working directory).
    PaletteSet {
        /// As a project file's `palette`.
        palette: PaletteSpec,
    },
    /// Pin the background or let the solver search.
    BackgroundPin(BackgroundPin),
    /// Set some of the solve settings.
    SolveSet(SolveSet),
    /// Set what transform commands snap to.
    SnapSet(SnapSet),
    /// Remove one entry of a layer mask's sources list.
    MaskSourceRemove {
        /// Layer.
        layer: LayerRef,
        /// Position in the sources list, 0 = first applied.
        index: usize,
    },
    /// Set how one of a layer mask's sources combines.
    MaskSourceSetCombine {
        /// Layer.
        layer: LayerRef,
        /// Position in the sources list, 0 = first applied.
        index: usize,
        /// `max`, `min` or `multiply`.
        combine: Combine,
    },
    /// New region with an empty 320 × 200 mask.
    RegionAdd {
        /// Display name.
        name: String,
        /// The recipe it imposes; absent is the default recipe.
        #[serde(default)]
        recipe: Box<Recipe>,
        /// Position, 0 = applied first; absent puts it last (on top).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        index: Option<usize>,
    },
    /// Remove a region.
    RegionRemove {
        /// Region.
        region: RegionRef,
    },
    /// Rename a region.
    RegionRename {
        /// Region.
        region: RegionRef,
        /// The new name.
        name: String,
    },
    /// Move a region to position `to` (0 = applied first).
    RegionReorder {
        /// Region.
        region: RegionRef,
        /// Its new position.
        to: usize,
    },
    /// Set some of a region's recipe.
    RegionSetRecipe {
        /// Region.
        region: RegionRef,
        /// The recipe fields that change.
        recipe: RecipeChange,
    },
    /// Soft round brush along a polyline on a region's mask, canvas pixels.
    RegionStroke {
        /// Region.
        region: RegionRef,
        /// Polyline, canvas pixels.
        points: Vec<Vec2>,
        /// Radius, canvas pixels.
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
    /// Polygon (lasso) fill on a region's mask, canvas pixels.
    RegionFill {
        /// Region.
        region: RegionRef,
        /// Polygon, canvas pixels.
        polygon: Vec<Vec2>,
        /// `0..=1`.
        #[serde(default = "one")]
        value: f32,
        /// `add` or `subtract`.
        #[serde(default)]
        mode: PaintMode,
    },
    /// Gradient combined into a region's mask (baked), canvas pixels.
    RegionGradient {
        /// Region.
        region: RegionRef,
        /// The gradient.
        gradient: Gradient,
        /// `max`, `min` or `multiply`.
        #[serde(default)]
        combine: Combine,
    },
    /// A region's whole mask to one value (0 clears, 1 fills).
    RegionClear {
        /// Region.
        region: RegionRef,
        /// `0..=1`.
        #[serde(default)]
        value: f32,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use convert64_core::ColourIndex;
    use convert64_core::layer::{DitherKind, Gamma, Levels, Sharpen};
    use convert64_core::mask::{
        GradientKind, KeyChannel, MaskSource, Modifiers, SnapToCells, SourceEntry,
    };
    use convert64_core::mode::Mode;
    use convert64_core::model::CellPos;
    use convert64_core::solve::{Background, SolverKind};

    fn every_command() -> Vec<JsonCommand> {
        let l = LayerRef::Name("photo".into());
        let r = RegionRef::Name("sky".into());
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
                source_layer: l.clone(),
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
            JsonCommand::LayerAdd {
                name: "logo".into(),
                source: SourceSpec::Path("logo.png".into()),
                index: Some(1),
                visible: false,
                transform: Transform::translated(Vec2::new(8.0, 4.0)).unwrap(),
                opacity: 0.5,
                blend: Blend::Screen,
                recipe: Box::new(Recipe::with_method(DitherKind::Bayer4, 0.75).unwrap()),
                adjustments: Box::default(),
            },
            JsonCommand::LayerRemove {
                layer: LayerRef::Index(1),
            },
            JsonCommand::LayerReorder {
                layer: l.clone(),
                to: 0,
            },
            JsonCommand::LayerSetVisible {
                layer: l.clone(),
                visible: false,
            },
            JsonCommand::LayerSetOpacity {
                layer: l.clone(),
                opacity: 0.25,
            },
            JsonCommand::LayerSetBlend {
                layer: l.clone(),
                blend: Blend::Multiply,
            },
            JsonCommand::LayerSetTransform {
                layer: l.clone(),
                transform: Transform::new(Vec2::new(1.5, 2.0), Vec2::new(2.0, 2.0), true, false)
                    .unwrap(),
                snap: None,
            },
            JsonCommand::LayerSetTransform {
                layer: l.clone(),
                transform: Transform::new(Vec2::new(1.5, 2.0), Vec2::new(2.0, 2.0), true, false)
                    .unwrap()
                    .with_rotation(-12.5)
                    .unwrap()
                    .with_anchor(Vec2::new(0.0, 1.0))
                    .unwrap(),
                snap: Some(Snap::Cell),
            },
            JsonCommand::LayerRotate {
                layer: l.clone(),
                degrees: 2.5,
                relative: true,
                anchor: Some(Vec2::new(0.25, 0.75)),
                snap: Some(Snap::Off),
            },
            JsonCommand::LayerNudge {
                layer: l.clone(),
                by: Vec2::new(0.25, -1.0),
            },
            JsonCommand::LayerRename {
                layer: l.clone(),
                name: "background".into(),
            },
            JsonCommand::LayerDuplicate { layer: l.clone() },
            JsonCommand::AdjustmentsSet {
                layer: l.clone(),
                adjustments: AdjustmentsChange {
                    gamma: Some(Gamma::new(1.4).unwrap()),
                    sharpen: Some(Sharpen::png2c64(-0.5).unwrap()),
                    match_range: Some(None),
                    ..AdjustmentsChange::default()
                },
            },
            JsonCommand::RecipeSet {
                layer: l,
                recipe: RecipeChange {
                    strength: Some(0.5),
                    deny: Some(vec![ColourIndex::YELLOW]),
                    ..RecipeChange::default()
                },
            },
            JsonCommand::ModeSet(ModeSet {
                mode: Mode::HiresBitmap,
            }),
            JsonCommand::PaletteSet {
                palette: PaletteSpec::Colodore,
            },
            JsonCommand::BackgroundPin(BackgroundPin {
                background: Background::Fixed(ColourIndex::BLUE),
            }),
            JsonCommand::SolveSet(SolveSet {
                solver: Some(SolverKind::Native),
                hysteresis: Some(None),
                ..SolveSet::default()
            }),
            JsonCommand::SnapSet(SnapSet {
                snap: Snap::C64Pixel,
            }),
            JsonCommand::MaskSourceRemove {
                layer: LayerRef::Index(0),
                index: 1,
            },
            JsonCommand::MaskSourceSetCombine {
                layer: LayerRef::Name("photo".into()),
                index: 0,
                combine: Combine::Multiply,
            },
            JsonCommand::RegionAdd {
                name: "sky".into(),
                recipe: Box::new(Recipe::with_method(DitherKind::Bayer8, 0.5).unwrap()),
                index: Some(0),
            },
            JsonCommand::RegionRemove {
                region: RegionRef::Index(1),
            },
            JsonCommand::RegionRename {
                region: r.clone(),
                name: "clouds".into(),
            },
            JsonCommand::RegionReorder {
                region: r.clone(),
                to: 0,
            },
            JsonCommand::RegionSetRecipe {
                region: r.clone(),
                recipe: RecipeChange {
                    strength: Some(0.25),
                    allow: Some(vec![ColourIndex::CYAN]),
                    ..RecipeChange::default()
                },
            },
            JsonCommand::RegionStroke {
                region: r.clone(),
                points: vec![Vec2::new(10.0, 20.0), Vec2::new(30.5, 40.0)],
                radius: 8.0,
                hardness: 0.5,
                flow: 0.75,
                mode: PaintMode::Add,
            },
            JsonCommand::RegionFill {
                region: RegionRef::Index(0),
                polygon: vec![Vec2::ZERO, Vec2::new(50.0, 0.0), Vec2::new(0.0, 50.0)],
                value: 1.0,
                mode: PaintMode::Subtract,
            },
            JsonCommand::RegionGradient {
                region: r.clone(),
                gradient: Gradient::new(
                    GradientKind::Linear,
                    Vec2::new(0.0, 0.0),
                    Vec2::new(0.0, 200.0),
                    1.0,
                )
                .unwrap(),
                combine: Combine::Min,
            },
            JsonCommand::RegionClear {
                region: r,
                value: 1.0,
            },
        ]
    }

    #[test]
    fn partial_edits_keep_what_they_do_not_name() {
        let v: JsonCommand = serde_json::from_str(
            r#"{"command":"solve_set","hysteresis":null,"divergences":{"chroma_guard":true}}"#,
        )
        .unwrap();
        let JsonCommand::SolveSet(s) = v else {
            panic!("{v:?}")
        };
        assert_eq!(s.hysteresis, Some(None), "null clears");
        assert_eq!(s.refinements, None, "absent keeps");
        let v: JsonCommand =
            serde_json::from_str(r#"{"command":"adjustments_set","layer":0,"adjustments":{}}"#)
                .unwrap();
        assert!(matches!(
            v,
            JsonCommand::AdjustmentsSet { adjustments, .. } if adjustments == AdjustmentsChange::default()
        ));
        for bad in [
            r#"{"command":"solve_set","surprise":1}"#,
            r#"{"command":"recipe_set","layer":0,"recipe":{"strenght":0.5}}"#,
            r#"{"command":"mode_set","mode":"koala"}"#,
            r#"{"command":"background_pin","background":{"fixed":16}}"#,
        ] {
            assert!(serde_json::from_str::<JsonCommand>(bad).is_err(), "{bad}");
        }
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
        // A region by index or name; region_add's recipe and index are optional.
        let add = r#"{"command":"region_add","name":"sky"}"#;
        assert!(matches!(
            serde_json::from_str::<JsonCommand>(add),
            Ok(JsonCommand::RegionAdd { index: None, recipe, .. }) if *recipe == Recipe::default()
        ));
        let by_name = r#"{"command":"region_clear","region":"sky"}"#;
        assert!(matches!(
            serde_json::from_str::<JsonCommand>(by_name),
            Ok(JsonCommand::RegionClear { region: RegionRef::Name(_), value }) if value == 0.0
        ));
    }
}
