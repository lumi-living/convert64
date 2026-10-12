//! Version-1 recipe files: a layered scene plus a list of commands, in JSON (E4,
//! ADR-005). Every build keeps reading them: [`ProjectFile::parse`](crate::ProjectFile::parse)
//! migrates one onto a project ([`ProjectFile::from_recipe`](crate::ProjectFile::from_recipe)).
//!
//! ```json
//! {
//!   "version": 1,
//!   "target": "multicolor-bitmap",
//!   "supersample": 4,
//!   "layers": [{ "name": "photo", "source": "photo.png" }],
//!   "commands": [
//!     { "command": "mask_key", "layer": "photo",
//!       "key": { "channel": "chroma", "centre": [0.0, 1.0, 0.0], "range": 0.1, "softness": 0.05 },
//!       "combine": "max" },
//!     { "command": "mask_set_modifier", "layer": 0, "modifier": { "feather": 3.0 } }
//!   ]
//! }
//! ```

use crate::command::JsonCommand;
use crate::file::SolveSpec;
use convert64_core::SuperGrid;
use convert64_core::layer::{Adjustments, Blend, Recipe, Transform};
use convert64_core::mode::Mode;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// The recipe-file format version this build reads.
pub const VERSION: u32 = 1;

/// One layer of a recipe file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayerSpec {
    /// Display name, also usable as a [`LayerRef`](crate::LayerRef).
    pub name: String,
    /// PNG source, relative to the recipe file.
    pub source: PathBuf,
    /// Placement in canvas pixels.
    #[serde(default)]
    pub transform: Transform,
    /// Opacity, `0..=1`.
    #[serde(default = "one")]
    pub opacity: f32,
    /// Blend mode.
    #[serde(default)]
    pub blend: Blend,
    /// Dither recipe.
    #[serde(default)]
    pub recipe: Recipe,
    /// Colour adjustments.
    #[serde(default)]
    pub adjustments: Adjustments,
}

const fn one() -> f32 {
    1.0
}

const fn default_supersample() -> u8 {
    SuperGrid::DEFAULT_N
}

/// A recipe file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecipeFile {
    /// Must be [`VERSION`].
    pub version: u32,
    /// Target mode.
    pub target: Mode,
    /// Supersample factor, 1–8.
    #[serde(default = "default_supersample")]
    pub supersample: u8,
    /// Layers, bottom first.
    pub layers: Vec<LayerSpec>,
    /// Commands, run in order.
    #[serde(default)]
    pub commands: Vec<JsonCommand>,
    /// How the picture is solved.
    #[serde(default)]
    pub solve: SolveSpec,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recipe_files_carry_solver_knobs() {
        let text = r#"{
            "version": 1, "target": "multicolor-bitmap",
            "layers": [{ "name": "a", "source": "a.png",
                "recipe": { "method": { "kind": "floyd-steinberg" }, "strength": 1.0,
                            "cell_clamp": false, "max_luma_diff": 0, "adaptive": 0.5 } }],
            "solve": { "background": { "fixed": 6 }, "hysteresis": 0.97, "luma_pair_bias": -0.01 }
        }"#;
        let f: RecipeFile = serde_json::from_str(text).unwrap();
        let r = f.layers[0].recipe;
        assert!(!r.cell_clamp() && r.max_luma_diff() == 0 && r.adaptive() == 0.5);
        assert_eq!(f.solve.hysteresis, Some(0.97));
        let back: RecipeFile = serde_json::from_str(&serde_json::to_string(&f).unwrap()).unwrap();
        assert_eq!(back, f);
    }
}
