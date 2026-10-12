//! The command engine (ADR-005): every user-visible action is a named command with a
//! serde-serialisable argument struct, applied to a [`Project`] through an [`Editor`]
//! that keeps undo/redo history as command + memento pairs.
//!
//! The registry holds the mask commands of ADR-008 (#77), the cell steering commands
//! (E5) and, since #193, every other edit the app makes: layers (add, remove, reorder,
//! visibility, opacity, blend, transform, nudge, rename, duplicate), adjustments,
//! recipes, the target mode, the palette, the background and the solve settings. The
//! app, `convert64 apply` and the MCP server all execute these same values; the JSON
//! forms and file paths live in `convert64-project`, since this crate does no I/O.
//!
//! Undo mementos are the smallest state that restores the project: for paint, the
//! [`TILE`](crate::mask::TILE)-sized raster tiles the command touched (taken before it
//! ran), so undo is O(touched tiles); for modifier and source edits, the previous mask;
//! for a property, its previous value; for a removed layer, the layer.

use crate::colour::ColourIndex;
use crate::document::SolveSettings;
use crate::geom::{PxRect, Vec2};
use crate::layer::{
    Adjustments, Blend, BrightnessContrast, CellMetric, Curves, FitMode, Gamma, Hsl, Layer, Levels,
    Method, Metric, OrderedSemantics, PatternPhase, RangeMatch, Recipe, RecipeError, Transform,
};
use crate::mask::paint::{self, PaintMode};
use crate::mask::{
    Combine, Gradient, Key, Mask, MaskError, MaskRaster, MaskSource, Modifiers, SnapToCells,
    SourceEntry, Tile,
};
use crate::mode::Mode;
use crate::model::CellPos;
use crate::palette::Palette;
use crate::solve::png2c64_ref::Divergences;
use crate::solve::steering::{Pin, Steering, colour_set};
use crate::solve::{Background, SolverKind};
use serde::{Deserialize, Deserializer, Serialize};
use std::fmt;

pub use crate::document::Project;

/// `Option<Option<T>>` from JSON: absent is `None` (keep), `null` is `Some(None)`
/// (clear), a value is `Some(Some(v))`.
fn present<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<Option<T>>, D::Error> {
    Option::<T>::deserialize(d).map(Some)
}

/// Why a command could not run.
#[derive(Clone, Debug, PartialEq)]
pub enum CommandError {
    /// No layer at this index.
    NoLayer(usize),
    /// An argument is out of range.
    Param {
        /// Which argument.
        what: &'static str,
        /// Why.
        why: String,
    },
    /// A mask value was rejected.
    Mask(MaskError),
    /// A recipe value was rejected.
    Recipe(RecipeError),
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoLayer(i) => write!(f, "there is no layer {i}"),
            Self::Param { what, why } => write!(f, "{what}: {why}"),
            Self::Mask(e) => write!(f, "mask: {e}"),
            Self::Recipe(e) => write!(f, "recipe: {e}"),
        }
    }
}

impl std::error::Error for CommandError {}

impl From<MaskError> for CommandError {
    fn from(e: MaskError) -> Self {
        Self::Mask(e)
    }
}

impl From<RecipeError> for CommandError {
    fn from(e: RecipeError) -> Self {
        Self::Recipe(e)
    }
}

fn param(what: &'static str, why: impl Into<String>) -> CommandError {
    CommandError::Param {
        what,
        why: why.into(),
    }
}

fn check_range(what: &'static str, v: f32, lo: f32, hi: f32) -> Result<(), CommandError> {
    if v.is_finite() && (lo..=hi).contains(&v) {
        Ok(())
    } else {
        Err(param(what, format!("{v} is outside {lo}–{hi}")))
    }
}

fn check_points(what: &'static str, pts: &[Vec2], min: usize) -> Result<(), CommandError> {
    if pts.len() < min {
        return Err(param(what, format!("needs at least {min} points")));
    }
    if pts.iter().any(|p| !p.is_finite()) {
        return Err(param(what, "points must be finite"));
    }
    Ok(())
}

/// Paints a soft round brush along a polyline (ADR-008 `MaskStroke`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MaskStroke {
    /// Layer index.
    pub layer: usize,
    /// The polyline, layer pixels.
    pub points: Vec<Vec2>,
    /// Brush radius, layer pixels, `0.25..=1000`.
    pub radius: f32,
    /// Fraction of the radius at full strength, `0..=1`.
    pub hardness: f32,
    /// Strength per dab, `0..=1`; accumulates where dabs overlap.
    pub flow: f32,
    /// Add or subtract.
    pub mode: PaintMode,
}

/// Fills a polygon (lasso) into the mask (ADR-008 `MaskFill`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MaskFill {
    /// Layer index.
    pub layer: usize,
    /// Polygon, layer pixels, at least 3 points.
    pub polygon: Vec<Vec2>,
    /// Fill value, `0..=1`.
    pub value: f32,
    /// Add or subtract.
    pub mode: PaintMode,
}

/// Adds a gradient source (ADR-008 `MaskGradient`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MaskGradient {
    /// Layer index.
    pub layer: usize,
    /// The gradient.
    pub gradient: Gradient,
    /// How it combines.
    pub combine: Combine,
}

/// Adds a key source (ADR-008 `MaskKey`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MaskKey {
    /// Layer index.
    pub layer: usize,
    /// The key.
    pub key: Key,
    /// How it combines.
    pub combine: Combine,
}

/// One modifier slider (ADR-008 `MaskSetModifier`).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ModifierChange {
    /// Invert on or off.
    Invert(bool),
    /// Levels.
    Levels(Levels),
    /// Snap to cells.
    Snap(SnapToCells),
    /// Feather σ in target pixels, `0..=32`.
    Feather(f32),
}

/// Sets one modifier of a layer's mask.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MaskSetModifier {
    /// Layer index.
    pub layer: usize,
    /// The change.
    pub modifier: ModifierChange,
}

/// Replaces the mask raster with an imported greyscale image of the layer's size
/// (ADR-008 `MaskLoad`; the CLI and MCP server read the PNG).
#[derive(Clone, Debug, PartialEq)]
pub struct MaskLoad {
    /// Layer index.
    pub layer: usize,
    /// The raster.
    pub raster: MaskRaster,
}

/// Sets the whole raster to one value (ADR-008 `MaskClear`): 0 is the options bar's
/// **Clear**, 1 its **Fill**. Sources and modifiers stay.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MaskClear {
    /// Layer index.
    pub layer: usize,
    /// The value, `0..=1`.
    #[serde(default)]
    pub value: f32,
}

/// Copies the layer's PNG alpha into its mask raster (ADR-008 `MaskFromAlpha`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MaskFromAlpha {
    /// Layer index.
    pub layer: usize,
}

/// A selection made with the mask tools while no mask is being edited (UX §5a): lasso
/// polygons filled at 1, then procedural sources, then modifiers, all in the source
/// layer's space.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Selection {
    /// Lasso polygons, layer pixels.
    #[serde(default)]
    pub polygons: Vec<Vec<Vec2>>,
    /// Gradients and keys.
    #[serde(default)]
    pub sources: Vec<SourceEntry>,
    /// Modifiers for the new layer's mask.
    #[serde(default)]
    pub modifiers: Modifiers,
}

/// A new layer above `source_layer` sharing its pixels (`Arc`) with the selection as its
/// mask; with `cut`, the original keeps only the inverse (ADR-008 `LayerViaMask`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LayerViaMask {
    /// The layer to split.
    pub source_layer: usize,
    /// The selection.
    pub selection: Selection,
    /// Also remove the selection from the original.
    #[serde(default)]
    pub cut: bool,
}

/// Pins colours on cells (UX §7): `full` fixes each cell's set to exactly `colours`;
/// otherwise the set must contain them and the solver picks the rest.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CellPin {
    /// The cells.
    pub cells: Vec<CellPos>,
    /// 1–4 colours.
    pub colours: Vec<ColourIndex>,
    /// Pin the whole set rather than these colours only.
    #[serde(default)]
    pub full: bool,
}

/// Locks cells: the next solve copies them from the previous result untouched.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CellLock {
    /// The cells.
    pub cells: Vec<CellPos>,
    /// `false` unlocks.
    #[serde(default = "yes")]
    pub locked: bool,
}

const fn yes() -> bool {
    true
}

/// Removes every pin, lock, allow and deny on cells.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CellClear {
    /// The cells.
    pub cells: Vec<CellPos>,
}

/// Restricts cells to these colours (a filter on top of the recipes' allowed colours).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CellAllow {
    /// The cells.
    pub cells: Vec<CellPos>,
    /// The colours they may use.
    pub colours: Vec<ColourIndex>,
}

/// Forbids colours in cells.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CellDeny {
    /// The cells.
    pub cells: Vec<CellPos>,
    /// The colours they must not use.
    pub colours: Vec<ColourIndex>,
}

/// Adds a layer (`layer_add`). The front-ends build it from a source they read (the
/// JSON form names a PNG or an earlier layer's pixels); the core takes it whole.
#[derive(Clone, Debug, PartialEq)]
pub struct LayerAdd {
    /// Stack position, 0 = bottom; the layer count puts it on top.
    pub index: usize,
    /// The layer.
    pub layer: Layer,
}

/// Removes a layer (`layer_remove`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LayerRemove {
    /// Layer index.
    pub layer: usize,
}

/// Moves a layer so it ends up at stack position `to`, 0 = bottom (`layer_reorder`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LayerReorder {
    /// Layer index.
    pub layer: usize,
    /// Its new position.
    pub to: usize,
}

/// Shows or hides a layer (`layer_set_visible`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LayerSetVisible {
    /// Layer index.
    pub layer: usize,
    /// Whether it takes part in the composite.
    pub visible: bool,
}

/// Sets a layer's opacity (`layer_set_opacity`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LayerSetOpacity {
    /// Layer index.
    pub layer: usize,
    /// `0..=1`.
    pub opacity: f32,
}

/// Sets a layer's blend mode (`layer_set_blend`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LayerSetBlend {
    /// Layer index.
    pub layer: usize,
    /// The blend mode.
    pub blend: Blend,
}

/// Places a layer on the canvas (`layer_set_transform`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LayerSetTransform {
    /// Layer index.
    pub layer: usize,
    /// The whole transform: translate, scale, flips.
    pub transform: Transform,
}

/// Moves a layer by a (sub-pixel) offset in canvas pixels (`layer_nudge`, TAD §3.3).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LayerNudge {
    /// Layer index.
    pub layer: usize,
    /// The offset, canvas pixels.
    pub by: Vec2,
}

/// Renames a layer (`layer_rename`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LayerRename {
    /// Layer index.
    pub layer: usize,
    /// The new name, not empty.
    pub name: String,
}

/// Copies a layer, sharing its pixels, just above it (`layer_duplicate`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LayerDuplicate {
    /// Layer index.
    pub layer: usize,
}

/// The adjustments `adjustments_set` changes; those not named are kept.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdjustmentsChange {
    /// Gamma.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gamma: Option<Gamma>,
    /// Levels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub levels: Option<Levels>,
    /// Brightness and contrast.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brightness_contrast: Option<BrightnessContrast>,
    /// Hue, saturation, lightness.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hsl: Option<Hsl>,
    /// Tone curves.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub curves: Option<Curves>,
    /// OKLab range matching; `null` turns it off.
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub match_range: Option<Option<RangeMatch>>,
}

impl AdjustmentsChange {
    /// `a` with these changes.
    pub fn apply(&self, a: &Adjustments) -> Adjustments {
        Adjustments {
            gamma: self.gamma.unwrap_or(a.gamma),
            levels: self.levels.unwrap_or(a.levels),
            brightness_contrast: self.brightness_contrast.unwrap_or(a.brightness_contrast),
            hsl: self.hsl.unwrap_or(a.hsl),
            curves: self.curves.clone().unwrap_or_else(|| a.curves.clone()),
            match_range: self.match_range.unwrap_or(a.match_range),
        }
    }
}

/// Sets some of a layer's colour adjustments (`adjustments_set`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AdjustmentsSet {
    /// Layer index.
    pub layer: usize,
    /// What changes.
    pub adjustments: AdjustmentsChange,
}

/// The recipe fields `recipe_set` changes; those not named are kept. The allowed colours
/// are `allowed` (or the current set), plus `allow`, minus `deny`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecipeChange {
    /// Dither method.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<Method>,
    /// Strength, `0..=1`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strength: Option<f32>,
    /// Screen- or layer-locked pattern phase.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<PatternPhase>,
    /// Colour metric.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metric: Option<Metric>,
    /// The allowed colours, exactly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed: Option<Vec<ColourIndex>>,
    /// Colours added to the allowed set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow: Option<Vec<ColourIndex>>,
    /// Colours taken out of the allowed set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deny: Option<Vec<ColourIndex>>,
    /// Adaptive or Force.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fit_mode: Option<FitMode>,
    /// Native solver: keep diffused error inside a cell's colours.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cell_clamp: Option<bool>,
    /// Native solver: luma levels a mix may span (8 = off).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_luma_diff: Option<u8>,
    /// Native solver: adaptive strength, `0..=1`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adaptive: Option<f32>,
    /// Native solver: mix ratios per pair.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mix_levels: Option<u8>,
    /// Native solver: smooth gradients.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub smooth: Option<bool>,
    /// Native solver: the cell score.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cell_metric: Option<CellMetric>,
    /// Diffused error clamp per metric channel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_clamp: Option<f32>,
    /// Ordered threshold semantics.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ordered_semantics: Option<OrderedSemantics>,
    /// Native solver: the chroma guard.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chroma_guard: Option<bool>,
}

impl RecipeChange {
    /// `r` with these changes, through the recipe's validating constructors.
    pub fn apply(&self, r: &Recipe) -> Result<Recipe, RecipeError> {
        let mut allowed = self.allowed.as_deref().map_or(r.allowed(), colour_set);
        if let Some(a) = &self.allow {
            allowed = allowed.union(colour_set(a));
        }
        if let Some(d) = &self.deny {
            allowed = allowed.difference(colour_set(d));
        }
        Recipe::new(
            self.method.unwrap_or(r.method()),
            self.strength.unwrap_or(r.strength()),
            self.phase.unwrap_or(r.phase()),
            self.metric.unwrap_or(r.metric()),
            allowed,
            self.fit_mode.unwrap_or(r.fit_mode()),
        )?
        .with_cell_clamp(self.cell_clamp.unwrap_or(r.cell_clamp()))
        .with_max_luma_diff(self.max_luma_diff.unwrap_or(r.max_luma_diff()))?
        .with_adaptive(self.adaptive.unwrap_or(r.adaptive()))?
        .with_mix_levels(self.mix_levels.unwrap_or(r.mix_levels()))?
        .with_smooth(self.smooth.unwrap_or(r.smooth()))
        .with_cell_metric(self.cell_metric.unwrap_or(r.cell_metric()))
        .with_error_clamp(self.error_clamp.unwrap_or(r.error_clamp()))
        .map(|n| {
            n.with_ordered_semantics(self.ordered_semantics.unwrap_or(r.ordered_semantics()))
                .with_chroma_guard(self.chroma_guard.unwrap_or(r.chroma_guard()))
        })
    }
}

/// Sets some of a layer's dither recipe (`recipe_set`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RecipeSet {
    /// Layer index.
    pub layer: usize,
    /// What changes.
    pub recipe: RecipeChange,
}

/// Sets the target mode (`mode_set`): one the compositor and solver support.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModeSet {
    /// The mode.
    pub mode: Mode,
}

/// Sets the palette the picture is solved and previewed with (`palette_set`). The
/// front-ends resolve the JSON form (a preset, 16 colours or a `.vpl` file).
#[derive(Clone, Debug, PartialEq)]
pub struct PaletteSet {
    /// The palette.
    pub palette: Palette,
}

/// Pins the multicolor background colour, or lets the solver search (`background_pin`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackgroundPin {
    /// `{"fixed": n}` pins colour `n`; `"auto"` searches all 16.
    pub background: Background,
}

/// Sets some of the project-wide solve settings (`solve_set`); those not named are
/// kept. The background has its own command, [`BackgroundPin`].
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SolveSet {
    /// `reference` or `native`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub solver: Option<SolverKind>,
    /// The reference solver's divergences from png2c64, as a whole: those not named are
    /// off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub divergences: Option<Divergences>,
    /// Refinement hysteresis in (0, 1]; `null` turns it off.
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub hysteresis: Option<Option<f32>>,
    /// Same-luma pair bias (native solver).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub luma_pair_bias: Option<f32>,
    /// Refinement rounds, 0–2.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refinements: Option<u8>,
    /// Backgrounds carried into dithering, 1–16.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finalists: Option<u8>,
}

impl SolveSet {
    /// `s` with these changes (unchecked; see [`SolveSettings::check`]).
    pub fn apply(&self, s: &SolveSettings) -> SolveSettings {
        SolveSettings {
            solver: self.solver.unwrap_or(s.solver),
            divergences: self.divergences.unwrap_or(s.divergences),
            background: s.background,
            hysteresis: self.hysteresis.unwrap_or(s.hysteresis),
            luma_pair_bias: self.luma_pair_bias.unwrap_or(s.luma_pair_bias),
            refinements: self.refinements.unwrap_or(s.refinements),
            finalists: self.finalists.unwrap_or(s.finalists),
        }
    }
}

/// A command (ADR-005). See [`REGISTRY`] for the names.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    /// `mask_stroke`.
    MaskStroke(MaskStroke),
    /// `mask_fill`.
    MaskFill(MaskFill),
    /// `mask_gradient`.
    MaskGradient(MaskGradient),
    /// `mask_key`.
    MaskKey(MaskKey),
    /// `mask_set_modifier`.
    MaskSetModifier(MaskSetModifier),
    /// `mask_load`.
    MaskLoad(MaskLoad),
    /// `mask_clear`.
    MaskClear(MaskClear),
    /// `mask_from_alpha`.
    MaskFromAlpha(MaskFromAlpha),
    /// `layer_via_mask`.
    LayerViaMask(LayerViaMask),
    /// `cell_pin`.
    CellPin(CellPin),
    /// `cell_lock`.
    CellLock(CellLock),
    /// `cell_clear`.
    CellClear(CellClear),
    /// `cell_allow`.
    CellAllow(CellAllow),
    /// `cell_deny`.
    CellDeny(CellDeny),
    /// `layer_add`.
    LayerAdd(Box<LayerAdd>),
    /// `layer_remove`.
    LayerRemove(LayerRemove),
    /// `layer_reorder`.
    LayerReorder(LayerReorder),
    /// `layer_set_visible`.
    LayerSetVisible(LayerSetVisible),
    /// `layer_set_opacity`.
    LayerSetOpacity(LayerSetOpacity),
    /// `layer_set_blend`.
    LayerSetBlend(LayerSetBlend),
    /// `layer_set_transform`.
    LayerSetTransform(LayerSetTransform),
    /// `layer_nudge`.
    LayerNudge(LayerNudge),
    /// `layer_rename`.
    LayerRename(LayerRename),
    /// `layer_duplicate`.
    LayerDuplicate(LayerDuplicate),
    /// `adjustments_set`.
    AdjustmentsSet(Box<AdjustmentsSet>),
    /// `recipe_set`.
    RecipeSet(Box<RecipeSet>),
    /// `mode_set`.
    ModeSet(ModeSet),
    /// `palette_set`.
    PaletteSet(Box<PaletteSet>),
    /// `background_pin`.
    BackgroundPin(BackgroundPin),
    /// `solve_set`.
    SolveSet(SolveSet),
}

/// A command's name and a one-line description, for front-ends (CLI help, MCP tool list).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommandInfo {
    /// The command's name in JSON and MCP.
    pub name: &'static str,
    /// What it does.
    pub summary: &'static str,
}

/// Every command, by name.
pub const REGISTRY: &[CommandInfo] = &[
    CommandInfo {
        name: "mask_stroke",
        summary: "Paint the mask with a soft round brush along a polyline (add or subtract).",
    },
    CommandInfo {
        name: "mask_fill",
        summary: "Fill a polygon (lasso) into the mask (add or subtract).",
    },
    CommandInfo {
        name: "mask_gradient",
        summary: "Add a linear or radial gradient source to the mask.",
    },
    CommandInfo {
        name: "mask_key",
        summary: "Add a luma, chroma or colour key on the layer's own pixels to the mask.",
    },
    CommandInfo {
        name: "mask_set_modifier",
        summary: "Set one mask modifier: invert, levels, snap to cells or feather.",
    },
    CommandInfo {
        name: "mask_load",
        summary: "Replace the mask raster with a greyscale PNG of the layer's size.",
    },
    CommandInfo {
        name: "mask_clear",
        summary: "Set the whole mask raster to one value (0 clears, 1 fills).",
    },
    CommandInfo {
        name: "mask_from_alpha",
        summary: "Copy the layer's PNG alpha into its mask raster.",
    },
    CommandInfo {
        name: "layer_via_mask",
        summary: "New layer sharing the source pixels, with a selection as its mask; cut \
                  removes the selection from the original.",
    },
    CommandInfo {
        name: "cell_pin",
        summary: "Pin colours on cells: the whole set (full) or colours the set must contain.",
    },
    CommandInfo {
        name: "cell_lock",
        summary: "Lock cells so a solve copies them from the previous result (locked: false unlocks).",
    },
    CommandInfo {
        name: "cell_clear",
        summary: "Remove every pin, lock, allow and deny on cells.",
    },
    CommandInfo {
        name: "cell_allow",
        summary: "Restrict cells to a list of colours.",
    },
    CommandInfo {
        name: "cell_deny",
        summary: "Forbid colours in cells.",
    },
    CommandInfo {
        name: "layer_add",
        summary: "Add a layer from a PNG (or an earlier layer's pixels) at a stack position.",
    },
    CommandInfo {
        name: "layer_remove",
        summary: "Remove a layer.",
    },
    CommandInfo {
        name: "layer_reorder",
        summary: "Move a layer to another stack position (0 = bottom).",
    },
    CommandInfo {
        name: "layer_set_visible",
        summary: "Show or hide a layer.",
    },
    CommandInfo {
        name: "layer_set_opacity",
        summary: "Set a layer's opacity (0–1).",
    },
    CommandInfo {
        name: "layer_set_blend",
        summary: "Set a layer's blend mode: normal, multiply, screen or overlay.",
    },
    CommandInfo {
        name: "layer_set_transform",
        summary: "Place a layer: translate, scale and flips, in canvas pixels.",
    },
    CommandInfo {
        name: "layer_nudge",
        summary: "Move a layer by a (sub-pixel) offset in canvas pixels.",
    },
    CommandInfo {
        name: "layer_rename",
        summary: "Rename a layer.",
    },
    CommandInfo {
        name: "layer_duplicate",
        summary: "Copy a layer just above itself, sharing its pixels.",
    },
    CommandInfo {
        name: "adjustments_set",
        summary: "Set some of a layer's colour adjustments; the others are kept.",
    },
    CommandInfo {
        name: "recipe_set",
        summary: "Set some of a layer's dither recipe (method, strength, phase, metric, \
                  allowed colours, fit mode, native knobs); the others are kept.",
    },
    CommandInfo {
        name: "mode_set",
        summary: "Set the target mode (multicolor-bitmap or hires-bitmap).",
    },
    CommandInfo {
        name: "palette_set",
        summary: "Set the palette: pepto-pal, pepto-ntsc, colodore, 16 colours or a .vpl file.",
    },
    CommandInfo {
        name: "background_pin",
        summary: "Pin the multicolor background colour ({\"fixed\": n}) or search all 16 (\"auto\").",
    },
    CommandInfo {
        name: "solve_set",
        summary: "Set some of the solve settings: solver, png2c64 divergences, hysteresis, \
                  luma pair bias, refinements, finalists; the others are kept.",
    },
];

/// What a command changed, for the compositor and solver: `LayerRect` maps to target
/// cells through `composite::mask_change_cells`; the others re-solve the layer's
/// footprint or the whole picture (`Compositor::update` finds the exact cells).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dirty {
    /// Part of a layer's mask raster, layer pixels.
    LayerRect {
        /// Layer index.
        layer: usize,
        /// Touched rectangle.
        rect: PxRect,
    },
    /// A whole layer.
    Layer(usize),
    /// The stack itself (a layer added or removed).
    Stack,
    /// Steering on these cells changed: re-solve them (the composite is unchanged).
    Cells(crate::geom::CellSet),
    /// The document's settings changed (target mode, palette, background or solve
    /// settings): re-solve the whole picture; a new mode also needs a new grid.
    Settings,
}

/// What undo needs.
#[derive(Clone, Debug, PartialEq)]
enum Memento {
    /// Raster tiles before a paint command; `created` when the command added the mask.
    Tiles {
        layer: usize,
        created: bool,
        tiles: Vec<Tile>,
    },
    /// The whole mask before the command (`None`: the layer had no mask).
    Mask { layer: usize, before: Option<Mask> },
    /// A layer inserted at `index`; the original's mask before a cut.
    Inserted {
        index: usize,
        original: usize,
        original_mask: Option<Option<Mask>>,
    },
    /// Steering before a cell command, and the cells it touched.
    Steering(Box<Steering>, crate::geom::CellSet),
    /// A layer property before the command.
    Prop { layer: usize, before: Prop },
    /// A layer removed from `index`.
    Removed { index: usize, layer: Box<Layer> },
    /// A layer moved from `from` to `to`.
    Moved { from: usize, to: usize },
    /// A document setting before the command.
    Setting(Setting),
}

/// A layer property's value, for undo.
#[derive(Clone, Debug, PartialEq)]
enum Prop {
    Visible(bool),
    Opacity(f32),
    Blend(Blend),
    Transform(Transform),
    Name(String),
    Adjustments(Box<Adjustments>),
    Recipe(Box<Recipe>),
}

/// A document setting's value, for undo.
#[derive(Clone, Debug, PartialEq)]
enum Setting {
    Target(Mode),
    Palette(Box<Palette>),
    Solve(Box<SolveSettings>),
}

/// The layer's mask, creating a fully opaque one if it has none. Returns whether it was
/// created.
fn mask_of(layer: &mut Layer) -> Result<(bool, &mut Mask), CommandError> {
    let created = layer.mask().is_none();
    if created {
        let (w, h) = layer.source().size();
        layer
            .set_mask(Some(Mask::opaque(w, h)?))
            .expect("built at the source's size");
    }
    Ok((created, layer.mask_mut().expect("present")))
}

fn layer_mut(p: &mut Project, i: usize) -> Result<&mut Layer, CommandError> {
    p.layers.get_mut(i).ok_or(CommandError::NoLayer(i))
}

impl Command {
    /// The command's registry name.
    pub const fn name(&self) -> &'static str {
        match self {
            Self::MaskStroke(_) => "mask_stroke",
            Self::MaskFill(_) => "mask_fill",
            Self::MaskGradient(_) => "mask_gradient",
            Self::MaskKey(_) => "mask_key",
            Self::MaskSetModifier(_) => "mask_set_modifier",
            Self::MaskLoad(_) => "mask_load",
            Self::MaskClear(_) => "mask_clear",
            Self::MaskFromAlpha(_) => "mask_from_alpha",
            Self::LayerViaMask(_) => "layer_via_mask",
            Self::CellPin(_) => "cell_pin",
            Self::CellLock(_) => "cell_lock",
            Self::CellClear(_) => "cell_clear",
            Self::CellAllow(_) => "cell_allow",
            Self::CellDeny(_) => "cell_deny",
            Self::LayerAdd(_) => "layer_add",
            Self::LayerRemove(_) => "layer_remove",
            Self::LayerReorder(_) => "layer_reorder",
            Self::LayerSetVisible(_) => "layer_set_visible",
            Self::LayerSetOpacity(_) => "layer_set_opacity",
            Self::LayerSetBlend(_) => "layer_set_blend",
            Self::LayerSetTransform(_) => "layer_set_transform",
            Self::LayerNudge(_) => "layer_nudge",
            Self::LayerRename(_) => "layer_rename",
            Self::LayerDuplicate(_) => "layer_duplicate",
            Self::AdjustmentsSet(_) => "adjustments_set",
            Self::RecipeSet(_) => "recipe_set",
            Self::ModeSet(_) => "mode_set",
            Self::PaletteSet(_) => "palette_set",
            Self::BackgroundPin(_) => "background_pin",
            Self::SolveSet(_) => "solve_set",
        }
    }

    /// Checks the arguments without touching a project.
    fn validate(&self) -> Result<(), CommandError> {
        match self {
            Self::MaskStroke(c) => {
                check_points("points", &c.points, 1)?;
                check_range("radius", c.radius, 0.25, 1000.0)?;
                check_range("hardness", c.hardness, 0.0, 1.0)?;
                check_range("flow", c.flow, 0.0, 1.0)
            }
            Self::MaskFill(c) => {
                check_points("polygon", &c.polygon, 3)?;
                check_range("value", c.value, 0.0, 1.0)
            }
            Self::MaskClear(c) => check_range("value", c.value, 0.0, 1.0),
            Self::MaskSetModifier(c) => match c.modifier {
                ModifierChange::Feather(f) => {
                    Modifiers::default().with_feather(f)?;
                    Ok(())
                }
                _ => Ok(()),
            },
            Self::LayerViaMask(c) => {
                if c.selection.polygons.is_empty() && c.selection.sources.is_empty() {
                    return Err(param("selection", "is empty"));
                }
                for poly in &c.selection.polygons {
                    check_points("selection polygon", poly, 3)?;
                }
                Ok(())
            }
            Self::CellPin(c) => {
                check_cells(&c.cells)?;
                if c.colours.is_empty() || colour_set(&c.colours).len() > 4 {
                    return Err(param("colours", "pin 1–4 different colours"));
                }
                Ok(())
            }
            Self::CellLock(CellLock { cells, .. })
            | Self::CellClear(CellClear { cells })
            | Self::CellAllow(CellAllow { cells, .. })
            | Self::CellDeny(CellDeny { cells, .. }) => check_cells(cells),
            Self::LayerSetOpacity(c) => check_range("opacity", c.opacity, 0.0, 1.0),
            Self::LayerNudge(c) => {
                if c.by.is_finite() {
                    Ok(())
                } else {
                    Err(param("by", "must be finite"))
                }
            }
            Self::LayerRename(c) => {
                if c.name.is_empty() {
                    Err(param("name", "must not be empty"))
                } else {
                    Ok(())
                }
            }
            // The bitmap modes are the ones the solver solves (M1).
            Self::ModeSet(c) => match c.mode {
                Mode::MulticolorBitmap | Mode::HiresBitmap => Ok(()),
                other => Err(param(
                    "mode",
                    format!(
                        "{other:?} cannot be solved yet; use multicolor-bitmap or hires-bitmap"
                    ),
                )),
            },
            Self::SolveSet(c) => c
                .apply(&SolveSettings::default())
                .check()
                .map_err(|e| param("solve", e)),
            Self::MaskGradient(_)
            | Self::MaskKey(_)
            | Self::MaskLoad(_)
            | Self::MaskFromAlpha(_)
            | Self::LayerAdd(_)
            | Self::LayerRemove(_)
            | Self::LayerReorder(_)
            | Self::LayerSetVisible(_)
            | Self::LayerSetBlend(_)
            | Self::LayerSetTransform(_)
            | Self::LayerDuplicate(_)
            | Self::AdjustmentsSet(_)
            | Self::RecipeSet(_)
            | Self::PaletteSet(_)
            | Self::BackgroundPin(_) => Ok(()),
        }
    }

    fn apply(&self, p: &mut Project) -> Result<(Memento, Dirty), CommandError> {
        self.validate()?;
        match self {
            Self::MaskStroke(c) => {
                let layer = layer_mut(p, c.layer)?;
                let (created, mask) = mask_of(layer)?;
                let bounds = paint::stroke_bounds(mask.raster(), &c.points, c.radius);
                let tiles = mask.tiles(bounds);
                let rect = mask
                    .paint(|r| paint::stroke(r, &c.points, c.radius, c.hardness, c.flow, c.mode));
                debug_assert_eq!(rect.union(bounds), bounds);
                let m = Memento::Tiles {
                    layer: c.layer,
                    created,
                    tiles,
                };
                Ok((
                    m,
                    Dirty::LayerRect {
                        layer: c.layer,
                        rect,
                    },
                ))
            }
            Self::MaskFill(c) => {
                let layer = layer_mut(p, c.layer)?;
                let (created, mask) = mask_of(layer)?;
                let bounds = paint::fill_bounds(mask.raster(), &c.polygon);
                let tiles = mask.tiles(bounds);
                let rect = mask.paint(|r| paint::fill(r, &c.polygon, c.value, c.mode));
                let m = Memento::Tiles {
                    layer: c.layer,
                    created,
                    tiles,
                };
                Ok((
                    m,
                    Dirty::LayerRect {
                        layer: c.layer,
                        rect,
                    },
                ))
            }
            Self::MaskGradient(MaskGradient {
                layer,
                gradient,
                combine,
            }) => push_source(
                p,
                *layer,
                SourceEntry {
                    source: MaskSource::Gradient(*gradient),
                    combine: *combine,
                },
            ),
            Self::MaskKey(MaskKey {
                layer,
                key,
                combine,
            }) => push_source(
                p,
                *layer,
                SourceEntry {
                    source: MaskSource::Key(*key),
                    combine: *combine,
                },
            ),
            Self::MaskSetModifier(c) => {
                let layer = layer_mut(p, c.layer)?;
                let before = layer.mask().cloned();
                let (_, mask) = mask_of(layer)?;
                let m = mask.modifiers();
                mask.set_modifiers(match c.modifier {
                    ModifierChange::Invert(v) => m.with_invert(v),
                    ModifierChange::Levels(v) => m.with_levels(v),
                    ModifierChange::Snap(v) => m.with_snap(v),
                    ModifierChange::Feather(v) => m.with_feather(v)?,
                });
                Ok((
                    Memento::Mask {
                        layer: c.layer,
                        before,
                    },
                    Dirty::Layer(c.layer),
                ))
            }
            Self::MaskLoad(c) => replace_raster(p, c.layer, |_| Ok(c.raster.clone())),
            Self::MaskClear(c) => replace_raster(p, c.layer, |l| {
                let (w, h) = l.source().size();
                Ok(MaskRaster::filled(w, h, c.value)?)
            }),
            Self::MaskFromAlpha(c) => replace_raster(p, c.layer, |l| {
                let (w, h) = l.source().size();
                let alpha = l.source().pixels().iter().map(|px| px[3]).collect();
                Ok(MaskRaster::from_values(w, h, alpha)?)
            }),
            Self::LayerViaMask(c) => layer_via_mask(p, c),
            Self::CellPin(c) => steer(p, &c.cells, |s, cell| {
                s.set_pin(
                    cell,
                    Pin {
                        colours: colour_set(&c.colours),
                        full: c.full,
                    },
                );
            }),
            Self::CellLock(c) => steer(p, &c.cells, |s, cell| s.set_lock(cell, c.locked)),
            Self::CellClear(c) => steer(p, &c.cells, Steering::clear),
            Self::CellAllow(c) => steer(p, &c.cells, |s, cell| {
                s.set_allow(cell, colour_set(&c.colours))
            }),
            Self::CellDeny(c) => steer(p, &c.cells, |s, cell| {
                s.set_deny(cell, colour_set(&c.colours))
            }),
            Self::LayerAdd(c) => {
                if c.index > p.layers.len() {
                    return Err(param(
                        "index",
                        format!("{} is above the top ({})", c.index, p.layers.len()),
                    ));
                }
                p.layers.insert(c.index, c.layer.clone());
                Ok((inserted(c.index), Dirty::Stack))
            }
            Self::LayerRemove(c) => {
                layer_mut(p, c.layer)?;
                let layer = Box::new(p.layers.remove(c.layer));
                Ok((
                    Memento::Removed {
                        index: c.layer,
                        layer,
                    },
                    Dirty::Stack,
                ))
            }
            Self::LayerReorder(c) => {
                layer_mut(p, c.layer)?;
                if c.to >= p.layers.len() {
                    return Err(param("to", format!("there is no position {}", c.to)));
                }
                let l = p.layers.remove(c.layer);
                p.layers.insert(c.to, l);
                Ok((
                    Memento::Moved {
                        from: c.layer,
                        to: c.to,
                    },
                    Dirty::Stack,
                ))
            }
            Self::LayerSetVisible(c) => set_prop(p, c.layer, Prop::Visible(c.visible)),
            Self::LayerSetOpacity(c) => set_prop(p, c.layer, Prop::Opacity(c.opacity)),
            Self::LayerSetBlend(c) => set_prop(p, c.layer, Prop::Blend(c.blend)),
            Self::LayerSetTransform(c) => set_prop(p, c.layer, Prop::Transform(c.transform)),
            Self::LayerNudge(c) => {
                let t = layer_mut(p, c.layer)?
                    .transform()
                    .nudged(c.by)
                    .map_err(|e| param("by", e.to_string()))?;
                set_prop(p, c.layer, Prop::Transform(t))
            }
            Self::LayerRename(c) => set_prop(p, c.layer, Prop::Name(c.name.clone())),
            Self::LayerDuplicate(c) => {
                let original = layer_mut(p, c.layer)?;
                let mut copy = original.clone();
                copy.set_name(format!("{} copy", original.name()));
                p.layers.insert(c.layer + 1, copy);
                Ok((inserted(c.layer + 1), Dirty::Stack))
            }
            Self::AdjustmentsSet(c) => {
                let a = c.adjustments.apply(layer_mut(p, c.layer)?.adjustments());
                set_prop(p, c.layer, Prop::Adjustments(Box::new(a)))
            }
            Self::RecipeSet(c) => {
                let r = c.recipe.apply(layer_mut(p, c.layer)?.recipe())?;
                set_prop(p, c.layer, Prop::Recipe(Box::new(r)))
            }
            Self::ModeSet(c) => Ok(set_setting(p, Setting::Target(c.mode))),
            Self::PaletteSet(c) => Ok(set_setting(
                p,
                Setting::Palette(Box::new(c.palette.clone())),
            )),
            Self::BackgroundPin(c) => {
                let s = SolveSettings {
                    background: c.background,
                    ..p.solve.clone()
                };
                Ok(set_setting(p, Setting::Solve(Box::new(s))))
            }
            Self::SolveSet(c) => {
                let s = c.apply(&p.solve);
                s.check().map_err(|e| param("solve", e))?;
                Ok(set_setting(p, Setting::Solve(Box::new(s))))
            }
        }
    }
}

const fn inserted(index: usize) -> Memento {
    Memento::Inserted {
        index,
        original: index,
        original_mask: None,
    }
}

/// Sets a layer property, returning the memento of its old value.
fn set_prop(p: &mut Project, i: usize, value: Prop) -> Result<(Memento, Dirty), CommandError> {
    let before = swap_prop(layer_mut(p, i)?, value)?;
    Ok((Memento::Prop { layer: i, before }, Dirty::Layer(i)))
}

/// Puts `value` on the layer and returns the value it replaced.
fn swap_prop(l: &mut Layer, value: Prop) -> Result<Prop, CommandError> {
    Ok(match value {
        Prop::Visible(v) => {
            let old = l.visible();
            l.set_visible(v);
            Prop::Visible(old)
        }
        Prop::Opacity(v) => {
            let old = l.opacity();
            l.set_opacity(v)
                .map_err(|e| param("opacity", e.to_string()))?;
            Prop::Opacity(old)
        }
        Prop::Blend(v) => {
            let old = l.blend();
            l.set_blend(v);
            Prop::Blend(old)
        }
        Prop::Transform(v) => {
            let old = l.transform();
            l.set_transform(v);
            Prop::Transform(old)
        }
        Prop::Name(v) => {
            let old = l.name().to_owned();
            l.set_name(v);
            Prop::Name(old)
        }
        Prop::Adjustments(v) => {
            let old = Box::new(l.adjustments().clone());
            l.set_adjustments(*v);
            Prop::Adjustments(old)
        }
        Prop::Recipe(v) => {
            let old = Box::new(*l.recipe());
            l.set_recipe(*v);
            Prop::Recipe(old)
        }
    })
}

/// Puts a document setting in place and returns the memento of the one it replaced.
fn set_setting(p: &mut Project, value: Setting) -> (Memento, Dirty) {
    (Memento::Setting(swap_setting(p, value)), Dirty::Settings)
}

fn swap_setting(p: &mut Project, value: Setting) -> Setting {
    match value {
        Setting::Target(m) => Setting::Target(std::mem::replace(&mut p.target, m)),
        Setting::Palette(v) => Setting::Palette(Box::new(std::mem::replace(&mut p.palette, *v))),
        Setting::Solve(v) => Setting::Solve(Box::new(std::mem::replace(&mut p.solve, *v))),
    }
}

fn check_cells(cells: &[CellPos]) -> Result<(), CommandError> {
    if cells.is_empty() {
        return Err(param("cells", "name at least one cell"));
    }
    if let Some(c) = cells.iter().find(|c| c.x >= 40 || c.y >= 25) {
        return Err(param("cells", format!("{c} is off the 40 × 25 screen")));
    }
    Ok(())
}

fn steer(
    p: &mut Project,
    cells: &[CellPos],
    mut f: impl FnMut(&mut Steering, CellPos),
) -> Result<(Memento, Dirty), CommandError> {
    let before = p.steering.clone();
    let mut touched = crate::geom::CellSet::EMPTY;
    for &cell in cells {
        f(&mut p.steering, cell);
        touched.insert(cell);
    }
    Ok((
        Memento::Steering(Box::new(before), touched),
        Dirty::Cells(touched),
    ))
}

fn push_source(
    p: &mut Project,
    i: usize,
    s: SourceEntry,
) -> Result<(Memento, Dirty), CommandError> {
    let layer = layer_mut(p, i)?;
    let before = layer.mask().cloned();
    let (_, mask) = mask_of(layer)?;
    mask.push_source(s);
    Ok((Memento::Mask { layer: i, before }, Dirty::Layer(i)))
}

fn replace_raster(
    p: &mut Project,
    i: usize,
    raster: impl FnOnce(&Layer) -> Result<MaskRaster, CommandError>,
) -> Result<(Memento, Dirty), CommandError> {
    let layer = layer_mut(p, i)?;
    let new = raster(layer)?;
    if new.size() != layer.source().size() {
        return Err(MaskError::Size {
            expected: layer.source().size(),
            found: new.size(),
        }
        .into());
    }
    let before = layer.mask().cloned();
    let (_, mask) = mask_of(layer)?;
    mask.set_raster(new)?;
    Ok((Memento::Mask { layer: i, before }, Dirty::Layer(i)))
}

fn layer_via_mask(p: &mut Project, c: &LayerViaMask) -> Result<(Memento, Dirty), CommandError> {
    let original = p
        .layers
        .get(c.source_layer)
        .ok_or(CommandError::NoLayer(c.source_layer))?;
    let (w, h) = original.source().size();
    let mut raster = MaskRaster::filled(w, h, 0.0)?;
    for poly in &c.selection.polygons {
        paint::fill(&mut raster, poly, 1.0, PaintMode::Add);
    }
    let mut selection = Mask::new(raster);
    selection.set_sources(c.selection.sources.clone());
    selection.set_modifiers(c.selection.modifiers);

    let mut layer = original.clone();
    layer.set_name(format!("{} (mask)", original.name()));
    layer
        .set_mask(Some(selection.clone()))
        .expect("selection is the source's size");

    let original_mask = if c.cut {
        let before = original.mask().cloned();
        let cut = match &before {
            // No mask yet: the original gets the selection, inverted.
            None => {
                let mut m = selection.clone();
                m.set_modifiers(
                    c.selection
                        .modifiers
                        .with_invert(!c.selection.modifiers.invert()),
                );
                m
            }
            // A mask already: multiply its raster by the inverse of the selection.
            Some(m) => {
                let sel = selection.layer_values(original.source());
                let vals = m
                    .raster()
                    .values()
                    .iter()
                    .zip(sel.iter())
                    .map(|(v, s)| v * (1.0 - s))
                    .collect();
                let mut m = m.clone();
                m.set_raster(MaskRaster::from_values(w, h, vals)?)?;
                m
            }
        };
        p.layers[c.source_layer]
            .set_mask(Some(cut))
            .expect("same size");
        Some(before)
    } else {
        None
    };
    let index = c.source_layer + 1;
    p.layers.insert(index, layer);
    Ok((
        Memento::Inserted {
            index,
            original: c.source_layer,
            original_mask,
        },
        Dirty::Stack,
    ))
}

impl Memento {
    fn undo(self, p: &mut Project) -> Dirty {
        match self {
            Self::Tiles {
                layer,
                created,
                tiles,
            } => {
                let l = &mut p.layers[layer];
                if created {
                    l.set_mask(None).expect("removing always fits");
                } else {
                    l.mask_mut().expect("painted mask").restore(&tiles);
                }
                Dirty::Layer(layer)
            }
            Self::Mask { layer, before } => {
                p.layers[layer].set_mask(before).expect("was on this layer");
                Dirty::Layer(layer)
            }
            Self::Inserted {
                index,
                original,
                original_mask,
            } => {
                p.layers.remove(index);
                if let Some(m) = original_mask {
                    p.layers[original].set_mask(m).expect("was on this layer");
                }
                Dirty::Stack
            }
            Self::Steering(before, touched) => {
                p.steering = *before;
                Dirty::Cells(touched)
            }
            Self::Prop { layer, before } => {
                swap_prop(&mut p.layers[layer], before).expect("was on this layer");
                Dirty::Layer(layer)
            }
            Self::Removed { index, layer } => {
                p.layers.insert(index, *layer);
                Dirty::Stack
            }
            Self::Moved { from, to } => {
                let l = p.layers.remove(to);
                p.layers.insert(from, l);
                Dirty::Stack
            }
            Self::Setting(before) => {
                swap_setting(p, before);
                Dirty::Settings
            }
        }
    }
}

/// A project with its undo/redo history. Every mutation goes through [`Self::execute`].
#[derive(Clone, Debug, Default)]
pub struct Editor {
    project: Project,
    done: Vec<(Command, Memento)>,
    undone: Vec<Command>,
}

impl Editor {
    /// An editor with empty history.
    pub fn new(project: Project) -> Self {
        Self {
            project,
            done: Vec::new(),
            undone: Vec::new(),
        }
    }

    /// The project.
    pub const fn project(&self) -> &Project {
        &self.project
    }

    /// The project, ending the editing session.
    pub fn into_project(self) -> Project {
        self.project
    }

    /// Runs a command; on success it joins the undo history and clears redo. A failed
    /// command changes nothing.
    pub fn execute(&mut self, cmd: Command) -> Result<Dirty, CommandError> {
        let (memento, dirty) = cmd.apply(&mut self.project)?;
        self.done.push((cmd, memento));
        self.undone.clear();
        Ok(dirty)
    }

    /// Undoes the last command, if any.
    pub fn undo(&mut self) -> Option<Dirty> {
        let (cmd, memento) = self.done.pop()?;
        let dirty = memento.undo(&mut self.project);
        self.undone.push(cmd);
        Some(dirty)
    }

    /// Redoes the last undone command, if any.
    pub fn redo(&mut self) -> Option<Result<Dirty, CommandError>> {
        let cmd = self.undone.pop()?;
        let undone = std::mem::take(&mut self.undone);
        let result = self.execute(cmd);
        self.undone = undone;
        Some(result)
    }

    /// Commands that can be undone.
    pub fn undo_len(&self) -> usize {
        self.done.len()
    }

    /// Commands that can be redone.
    pub fn redo_len(&self) -> usize {
        self.undone.len()
    }

    /// Raster values saved by the history's tile mementos, for tests and budgeting.
    pub fn saved_tile_values(&self) -> usize {
        self.done
            .iter()
            .map(|(_, m)| match m {
                Memento::Tiles { tiles, .. } => tiles.iter().map(Tile::len).sum(),
                _ => 0,
            })
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::SourceImage;
    use crate::mask::{GradientKind, KeyChannel};
    use std::sync::Arc;

    fn project() -> Project {
        let px: Vec<u8> = (0..300 * 200)
            .flat_map(|i| {
                let x = (i % 300) as u8;
                [x, 255 - x, 40, if i % 300 < 150 { 255 } else { 128 }]
            })
            .collect();
        let src = Arc::new(SourceImage::from_srgba8(300, 200, &px).unwrap());
        Project {
            layers: vec![Layer::new("photo", src)],
            ..Project::default()
        }
    }

    fn stroke(layer: usize) -> Command {
        Command::MaskStroke(MaskStroke {
            layer,
            points: vec![
                Vec2::new(20.0, 20.0),
                Vec2::new(150.0, 90.0),
                Vec2::new(160.0, 160.0),
            ],
            radius: 12.0,
            hardness: 0.3,
            flow: 0.6,
            mode: PaintMode::Subtract,
        })
    }

    #[test]
    fn stroke_then_undo_is_byte_identical() {
        let mut e = Editor::new(project());
        e.execute(Command::MaskClear(MaskClear {
            layer: 0,
            value: 0.5,
        }))
        .unwrap();
        let before = e.project().layers[0].mask().unwrap().raster().clone();
        let dirty = e.execute(stroke(0)).unwrap();
        let Dirty::LayerRect { rect, .. } = dirty else {
            panic!("{dirty:?}")
        };
        assert_eq!(rect, PxRect::new(8, 8, 172, 172));
        assert_ne!(e.project().layers[0].mask().unwrap().raster(), &before);
        // The memento holds only the touched tiles, not the whole raster.
        assert!(e.saved_tile_values() < 300 * 200);
        e.undo().unwrap();
        assert_eq!(e.project().layers[0].mask().unwrap().raster(), &before);
        // Redo repeats it exactly.
        e.redo().unwrap().unwrap();
        let after_redo = e.project().layers[0].mask().unwrap().raster().clone();
        e.undo();
        e.execute(stroke(0)).unwrap();
        assert_eq!(e.project().layers[0].mask().unwrap().raster(), &after_redo);
    }

    #[test]
    fn undo_of_an_auto_created_mask_removes_it() {
        let mut e = Editor::new(project());
        e.execute(stroke(0)).unwrap();
        assert!(e.project().layers[0].mask().is_some());
        e.undo();
        assert!(e.project().layers[0].mask().is_none());
    }

    #[test]
    fn mask_from_alpha_on_an_opaque_png_gives_one() {
        let src = Arc::new(SourceImage::from_srgba8(4, 4, &[200; 64]).unwrap());
        let mut opaque = vec![255u8; 64];
        for i in 0..16 {
            opaque[i * 4] = 10;
        }
        let src2 = Arc::new(SourceImage::from_srgba8(4, 4, &opaque).unwrap());
        let mut e = Editor::new(Project {
            layers: vec![Layer::new("a", src), Layer::new("b", src2)],
            ..Project::default()
        });
        e.execute(Command::MaskFromAlpha(MaskFromAlpha { layer: 1 }))
            .unwrap();
        assert!(
            e.project().layers[1]
                .mask()
                .unwrap()
                .raster()
                .values()
                .iter()
                .all(|&v| v == 1.0)
        );
        // A translucent source gives its alpha.
        e.execute(Command::MaskFromAlpha(MaskFromAlpha { layer: 0 }))
            .unwrap();
        let v = e.project().layers[0].mask().unwrap().raster().get(0, 0);
        assert!((v - 200.0 / 255.0).abs() < 1e-6);
    }

    #[test]
    fn sources_and_modifiers_undo() {
        let mut e = Editor::new(project());
        let g =
            Gradient::new(GradientKind::Linear, Vec2::ZERO, Vec2::new(300.0, 0.0), 1.0).unwrap();
        e.execute(Command::MaskGradient(MaskGradient {
            layer: 0,
            gradient: g,
            combine: Combine::Min,
        }))
        .unwrap();
        let k = Key::new(KeyChannel::Luma, [0.5; 3], 0.1, 0.05).unwrap();
        e.execute(Command::MaskKey(MaskKey {
            layer: 0,
            key: k,
            combine: Combine::Max,
        }))
        .unwrap();
        e.execute(Command::MaskSetModifier(MaskSetModifier {
            layer: 0,
            modifier: ModifierChange::Feather(6.0),
        }))
        .unwrap();
        let m = e.project().layers[0].mask().unwrap();
        assert_eq!(m.sources().len(), 2);
        assert_eq!(m.modifiers().feather(), 6.0);
        e.undo();
        assert_eq!(
            e.project().layers[0].mask().unwrap().modifiers().feather(),
            0.0
        );
        e.undo();
        assert_eq!(e.project().layers[0].mask().unwrap().sources().len(), 1);
        e.undo();
        assert!(e.project().layers[0].mask().is_none());
        assert_eq!(e.undo(), None);
    }

    #[test]
    fn invalid_commands_change_nothing() {
        let mut e = Editor::new(project());
        let p0 = e.project().clone();
        for bad in [
            Command::MaskSetModifier(MaskSetModifier {
                layer: 0,
                modifier: ModifierChange::Feather(40.0),
            }),
            Command::MaskClear(MaskClear {
                layer: 3,
                value: 0.0,
            }),
            Command::MaskClear(MaskClear {
                layer: 0,
                value: 1.5,
            }),
            Command::MaskFill(MaskFill {
                layer: 0,
                polygon: vec![Vec2::ZERO; 2],
                value: 1.0,
                mode: PaintMode::Add,
            }),
            Command::MaskStroke(MaskStroke {
                layer: 0,
                points: vec![Vec2::new(f32::NAN, 0.0)],
                radius: 4.0,
                hardness: 0.5,
                flow: 1.0,
                mode: PaintMode::Add,
            }),
            Command::MaskLoad(MaskLoad {
                layer: 0,
                raster: MaskRaster::filled(3, 3, 1.0).unwrap(),
            }),
            Command::LayerViaMask(LayerViaMask {
                source_layer: 0,
                selection: Selection::default(),
                cut: false,
            }),
        ] {
            assert!(e.execute(bad.clone()).is_err(), "{bad:?}");
            assert_eq!(e.project(), &p0, "{}", bad.name());
        }
        assert_eq!(e.undo_len(), 0);
    }

    #[test]
    fn layer_via_mask_shares_pixels_and_cut_inverts_the_original() {
        let mut e = Editor::new(project());
        let sel = Selection {
            polygons: vec![vec![
                Vec2::new(10.0, 10.0),
                Vec2::new(100.0, 10.0),
                Vec2::new(100.0, 80.0),
                Vec2::new(10.0, 80.0),
            ]],
            sources: Vec::new(),
            modifiers: Modifiers::feathered(2.0).unwrap(),
        };
        let dirty = e
            .execute(Command::LayerViaMask(LayerViaMask {
                source_layer: 0,
                selection: sel,
                cut: true,
            }))
            .unwrap();
        assert_eq!(dirty, Dirty::Stack);
        let p = e.project();
        assert_eq!(p.layers.len(), 2);
        assert_eq!(p.layers[1].name(), "photo (mask)");
        assert!(Arc::ptr_eq(p.layers[0].source(), p.layers[1].source()));
        let new = p.layers[1].mask().unwrap();
        let old = p.layers[0].mask().unwrap();
        assert_eq!(new.raster().get(50, 50), 1.0);
        assert_eq!(new.raster().get(150, 50), 0.0);
        assert!(old.modifiers().invert() && !new.modifiers().invert());
        assert_eq!(new.modifiers().feather(), 2.0);
        e.undo();
        assert_eq!(e.project(), &project_with_names(&e));
        assert!(e.project().layers[0].mask().is_none());
    }

    fn project_with_names(e: &Editor) -> Project {
        let mut p = project();
        // Same source Arc as the editor's, so layers compare equal.
        let src = Arc::clone(e.project().layers[0].source());
        p.layers[0] = Layer::new("photo", src);
        p
    }

    #[test]
    fn steering_commands_apply_and_undo() {
        let mut e = Editor::new(project());
        let cells = vec![CellPos::new(1, 1), CellPos::new(2, 1)];
        e.execute(Command::CellPin(CellPin {
            cells: cells.clone(),
            colours: vec![ColourIndex::RED, ColourIndex::WHITE],
            full: false,
        }))
        .unwrap();
        e.execute(Command::CellLock(CellLock {
            cells: vec![CellPos::new(5, 5)],
            locked: true,
        }))
        .unwrap();
        e.execute(Command::CellDeny(CellDeny {
            cells: cells.clone(),
            colours: vec![ColourIndex::BLUE],
        }))
        .unwrap();
        let s = &e.project().steering;
        assert!(s.pin(CellPos::new(2, 1)).is_some());
        assert!(s.locks().contains(CellPos::new(5, 5)));
        assert!(s.deny(CellPos::new(1, 1)).contains(ColourIndex::BLUE));
        let dirty = e.execute(Command::CellClear(CellClear { cells })).unwrap();
        assert!(matches!(dirty, Dirty::Cells(c) if c.len() == 2));
        assert!(e.project().steering.pin(CellPos::new(1, 1)).is_none());
        for _ in 0..4 {
            e.undo();
        }
        assert!(e.project().steering.is_empty());
        assert!(
            e.execute(Command::CellPin(CellPin {
                cells: vec![CellPos::new(40, 0)],
                colours: vec![ColourIndex::RED],
                full: true
            }))
            .is_err()
        );
        assert!(
            e.execute(Command::CellPin(CellPin {
                cells: vec![CellPos::new(0, 0)],
                colours: vec![],
                full: true
            }))
            .is_err()
        );
    }

    #[test]
    fn registry_names_match_commands() {
        let names: Vec<&str> = REGISTRY.iter().map(|c| c.name).collect();
        for c in [
            stroke(0),
            Command::MaskClear(MaskClear {
                layer: 0,
                value: 0.0,
            }),
            Command::MaskFromAlpha(MaskFromAlpha { layer: 0 }),
        ] {
            assert!(names.contains(&c.name()));
        }
        assert_eq!(names.len(), 30);
    }

    /// Two layers sharing pixels, the second smaller and masked, so moves and copies show.
    fn stack() -> Project {
        let mut p = project();
        let mut top = Layer::new("top", Arc::clone(p.layers[0].source()));
        top.set_opacity(0.75).unwrap();
        p.layers.push(top);
        p
    }

    /// One of each command #193 added, each valid on [`stack`].
    fn edits() -> Vec<Command> {
        let small = Arc::new(SourceImage::from_srgba8(4, 4, &[90; 64]).unwrap());
        vec![
            Command::LayerAdd(Box::new(LayerAdd {
                index: 1,
                layer: Layer::new("new", small),
            })),
            Command::LayerRemove(LayerRemove { layer: 0 }),
            Command::LayerReorder(LayerReorder { layer: 0, to: 1 }),
            Command::LayerSetVisible(LayerSetVisible {
                layer: 1,
                visible: false,
            }),
            Command::LayerSetOpacity(LayerSetOpacity {
                layer: 0,
                opacity: 0.3,
            }),
            Command::LayerSetBlend(LayerSetBlend {
                layer: 1,
                blend: Blend::Screen,
            }),
            Command::LayerSetTransform(LayerSetTransform {
                layer: 0,
                transform: Transform::new(Vec2::new(3.0, -2.0), Vec2::new(1.5, 2.0), true, true)
                    .unwrap(),
            }),
            Command::LayerNudge(LayerNudge {
                layer: 1,
                by: Vec2::new(0.25, -0.5),
            }),
            Command::LayerRename(LayerRename {
                layer: 1,
                name: "renamed".into(),
            }),
            Command::LayerDuplicate(LayerDuplicate { layer: 0 }),
            Command::AdjustmentsSet(Box::new(AdjustmentsSet {
                layer: 0,
                adjustments: AdjustmentsChange {
                    gamma: Some(Gamma::new(1.4).unwrap()),
                    hsl: Some(Hsl::new(10.0, 0.2, 0.0).unwrap()),
                    ..AdjustmentsChange::default()
                },
            })),
            Command::RecipeSet(Box::new(RecipeSet {
                layer: 1,
                recipe: RecipeChange {
                    method: Some(Method::new(crate::layer::DitherKind::Bayer4)),
                    strength: Some(0.5),
                    deny: Some(vec![ColourIndex::YELLOW]),
                    mix_levels: Some(8),
                    ..RecipeChange::default()
                },
            })),
            Command::ModeSet(ModeSet {
                mode: Mode::HiresBitmap,
            }),
            Command::PaletteSet(Box::new(PaletteSet {
                palette: Palette::colodore(50.0, 100.0, 50.0),
            })),
            Command::BackgroundPin(BackgroundPin {
                background: Background::Fixed(ColourIndex::BLUE),
            }),
            Command::SolveSet(SolveSet {
                solver: Some(SolverKind::Native),
                divergences: Some(Divergences {
                    chroma_guard: true,
                    ..Divergences::default()
                }),
                hysteresis: Some(Some(0.97)),
                refinements: Some(1),
                ..SolveSet::default()
            }),
        ]
    }

    #[test]
    fn every_edit_undoes_and_redoes_exactly() {
        let base = stack();
        let all = edits();
        let names: Vec<&str> = all.iter().map(Command::name).collect();
        for c in all {
            let mut e = Editor::new(base.clone());
            let dirty = e.execute(c.clone()).unwrap();
            let after = e.project().clone();
            assert_ne!(after, base, "{} changed nothing", c.name());
            assert_eq!(
                e.undo(),
                Some(dirty),
                "{}: undo dirties what it did",
                c.name()
            );
            assert_eq!(e.project(), &base, "{}: undo", c.name());
            e.redo().unwrap().unwrap();
            assert_eq!(e.project(), &after, "{}: redo", c.name());
        }
        // Every registry command is covered here or by the mask and cell tests above.
        for info in REGISTRY {
            assert!(
                names.contains(&info.name)
                    || info.name.starts_with("mask_")
                    || info.name.starts_with("cell_")
                    || info.name == "layer_via_mask",
                "{} has no undo test",
                info.name
            );
        }
    }

    #[test]
    fn a_whole_session_of_edits_undoes_to_the_start() {
        let base = stack();
        let mut e = Editor::new(base.clone());
        let mut states = vec![base.clone()];
        for c in edits() {
            // Later edits name layers that earlier ones moved; skip what no longer fits.
            if e.execute(c).is_ok() {
                states.push(e.project().clone());
            }
        }
        assert!(states.len() > 10);
        while let Some(want) = states.pop() {
            assert_eq!(e.project(), &want);
            if states.is_empty() {
                break;
            }
            e.undo().unwrap();
        }
        assert_eq!(e.project(), &base);
        assert_eq!(e.undo(), None);
    }

    #[test]
    fn edits_change_what_they_name_and_keep_the_rest() {
        let mut e = Editor::new(stack());
        e.execute(Command::LayerDuplicate(LayerDuplicate { layer: 1 }))
            .unwrap();
        let p = e.project();
        assert_eq!(p.layers[2].name(), "top copy");
        assert!(Arc::ptr_eq(p.layers[1].source(), p.layers[2].source()));
        assert_eq!(p.layers[2].opacity(), 0.75);
        e.execute(Command::LayerReorder(LayerReorder { layer: 2, to: 0 }))
            .unwrap();
        assert_eq!(e.project().layers[0].name(), "top copy");
        e.execute(Command::LayerNudge(LayerNudge {
            layer: 0,
            by: Vec2::new(0.5, 0.25),
        }))
        .unwrap();
        e.execute(Command::LayerNudge(LayerNudge {
            layer: 0,
            by: Vec2::new(0.5, 0.25),
        }))
        .unwrap();
        assert_eq!(
            e.project().layers[0].transform().translate(),
            Vec2::new(1.0, 0.5)
        );
        let before = *e.project().layers[0].recipe();
        e.execute(Command::RecipeSet(Box::new(RecipeSet {
            layer: 0,
            recipe: RecipeChange {
                allowed: Some(vec![ColourIndex::BLACK, ColourIndex::WHITE]),
                allow: Some(vec![ColourIndex::RED]),
                deny: Some(vec![ColourIndex::WHITE]),
                ..RecipeChange::default()
            },
        })))
        .unwrap();
        let r = *e.project().layers[0].recipe();
        assert_eq!(
            r.allowed(),
            colour_set(&[ColourIndex::BLACK, ColourIndex::RED])
        );
        assert_eq!(r.method(), before.method());
        assert_eq!(r.strength(), before.strength());
        // An empty change is the identity.
        assert_eq!(RecipeChange::default().apply(&r).unwrap(), r);
        let a = e.project().layers[0].adjustments().clone();
        assert_eq!(AdjustmentsChange::default().apply(&a), a);
        e.execute(Command::BackgroundPin(BackgroundPin {
            background: Background::Fixed(ColourIndex::BLUE),
        }))
        .unwrap();
        e.execute(Command::SolveSet(SolveSet {
            finalists: Some(5),
            ..SolveSet::default()
        }))
        .unwrap();
        let s = &e.project().solve;
        assert_eq!(s.background, Background::Fixed(ColourIndex::BLUE));
        assert_eq!(s.finalists, 5);
        assert_eq!(s.refinements, SolveSettings::default().refinements);
    }

    #[test]
    fn invalid_edits_change_nothing() {
        let mut e = Editor::new(stack());
        let p0 = e.project().clone();
        let small = Arc::new(SourceImage::from_srgba8(4, 4, &[90; 64]).unwrap());
        for bad in [
            Command::LayerAdd(Box::new(LayerAdd {
                index: 3,
                layer: Layer::new("x", small),
            })),
            Command::LayerRemove(LayerRemove { layer: 2 }),
            Command::LayerReorder(LayerReorder { layer: 0, to: 2 }),
            Command::LayerSetOpacity(LayerSetOpacity {
                layer: 0,
                opacity: 1.5,
            }),
            Command::LayerNudge(LayerNudge {
                layer: 0,
                by: Vec2::new(f32::NAN, 0.0),
            }),
            Command::LayerRename(LayerRename {
                layer: 0,
                name: String::new(),
            }),
            Command::LayerDuplicate(LayerDuplicate { layer: 9 }),
            Command::RecipeSet(Box::new(RecipeSet {
                layer: 0,
                recipe: RecipeChange {
                    allowed: Some(vec![ColourIndex::RED]),
                    deny: Some(vec![ColourIndex::RED]),
                    ..RecipeChange::default()
                },
            })),
            Command::RecipeSet(Box::new(RecipeSet {
                layer: 0,
                recipe: RecipeChange {
                    strength: Some(2.0),
                    ..RecipeChange::default()
                },
            })),
            Command::ModeSet(ModeSet {
                mode: Mode::MulticolorCharset,
            }),
            Command::SolveSet(SolveSet {
                finalists: Some(0),
                ..SolveSet::default()
            }),
            Command::SolveSet(SolveSet {
                hysteresis: Some(Some(1.5)),
                ..SolveSet::default()
            }),
        ] {
            assert!(e.execute(bad.clone()).is_err(), "{bad:?}");
            assert_eq!(e.project(), &p0, "{}", bad.name());
        }
        assert_eq!(e.undo_len(), 0);
    }
}
