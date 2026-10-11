//! The dither recipe a layer (or region) contributes to the attribute map
//! (`docs/TAD.md` §3.5, ADR-002).

use crate::colour::ColourSet;
use serde::{Deserialize, Serialize};
use std::fmt;

/// How a pixel is dithered. Grouped into families in the UI (UX §6).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DitherKind {
    /// Nearest colour, no dither.
    None,
    /// Ordered, 2 × 2 Bayer matrix.
    Bayer2,
    /// Ordered, 4 × 4 Bayer matrix.
    Bayer4,
    /// Ordered, 8 × 8 Bayer matrix.
    Bayer8,
    /// Error diffusion, Floyd–Steinberg.
    FloydSteinberg,
    /// Error diffusion, Atkinson (diffuses 6/8 of the error).
    Atkinson,
    /// Error diffusion, Jarvis–Judice–Ninke.
    JarvisJudiceNinke,
    /// Error diffusion, Sierra.
    Sierra,
    /// Threshold against a blue-noise texture.
    BlueNoise,
}

/// The UI's grouping of [`DitherKind`]s (UX §6, group 4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DitherFamily {
    /// No dither.
    None,
    /// Threshold matrices.
    Ordered,
    /// Error diffusion.
    Diffusion,
    /// Noise thresholds.
    Noise,
}

impl DitherKind {
    /// Every kind, in UI order.
    pub const ALL: [Self; 9] = [
        Self::None,
        Self::Bayer2,
        Self::Bayer4,
        Self::Bayer8,
        Self::FloydSteinberg,
        Self::Atkinson,
        Self::JarvisJudiceNinke,
        Self::Sierra,
        Self::BlueNoise,
    ];

    /// The family this kind belongs to.
    pub const fn family(self) -> DitherFamily {
        match self {
            Self::None => DitherFamily::None,
            Self::Bayer2 | Self::Bayer4 | Self::Bayer8 => DitherFamily::Ordered,
            Self::FloydSteinberg | Self::Atkinson | Self::JarvisJudiceNinke | Self::Sierra => {
                DitherFamily::Diffusion
            }
            Self::BlueNoise => DitherFamily::Noise,
        }
    }
}

/// Orientation preference: `VerticalBiased` favours patterns that survive the 2:1
/// multicolor stretch and PAL colour bleed (the "line- and 2:1-biased variants" in PDD).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Orientation {
    /// Let the method choose.
    #[default]
    Auto,
    /// Prefer vertical structures.
    VerticalBiased,
}

/// A dither method: the kind and its orientation preference.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Method {
    /// Which algorithm.
    pub kind: DitherKind,
    /// Orientation preference.
    #[serde(default)]
    pub orientation: Orientation,
}

impl Method {
    /// `kind` with the default orientation.
    pub const fn new(kind: DitherKind) -> Self {
        Self {
            kind,
            orientation: Orientation::Auto,
        }
    }
}

/// Whether an ordered or noise pattern is anchored to the screen or moves with the layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PatternPhase {
    /// Anchored to the screen: neighbouring layers' patterns line up.
    #[default]
    Screen,
    /// Anchored to the layer: the pattern moves when the layer is nudged.
    Layer,
}

/// The colour-difference metric the solver minimises for these pixels (TAD §3.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Metric {
    /// OKLab Euclidean distance (default).
    #[default]
    Oklab,
    /// CIELAB (D50) Euclidean distance.
    Cielab,
    /// Pepto Y'UV with luma weighted.
    PeptoYuv,
    /// Raw sRGB distance.
    Rgb,
}

/// Force vs Adaptive (TAD §3.1): whether this region's colours win cell-set choices.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FitMode {
    /// The solver minimises total error.
    #[default]
    Adaptive,
    /// This region's intended colours are weighted heavily; neighbours pay.
    Force,
}

/// Why a [`Recipe`] could not be built.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RecipeError {
    /// Strength is not a number in `0..=1`.
    Strength(f32),
    /// No colour is allowed.
    EmptyAllowedSet,
}

impl fmt::Display for RecipeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Strength(s) => write!(f, "dither strength must be in 0–1, got {s}"),
            Self::EmptyAllowedSet => f.write_str("a recipe must allow at least one colour"),
        }
    }
}

impl std::error::Error for RecipeError {}

#[derive(Deserialize)]
struct RecipeData {
    method: Method,
    strength: f32,
    #[serde(default)]
    phase: PatternPhase,
    #[serde(default)]
    metric: Metric,
    #[serde(default = "all_colours")]
    allowed: ColourSet,
    #[serde(default)]
    fit_mode: FitMode,
}

const fn all_colours() -> ColourSet {
    ColourSet::ALL
}

/// The attribute tuple `(method, strength, phase, metric, allowed_set, fit_mode)` a layer
/// carries into the attribute map (ADR-002, TAD §3.5).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "RecipeData")]
pub struct Recipe {
    method: Method,
    strength: f32,
    phase: PatternPhase,
    metric: Metric,
    allowed: ColourSet,
    fit_mode: FitMode,
}

impl TryFrom<RecipeData> for Recipe {
    type Error = RecipeError;
    fn try_from(d: RecipeData) -> Result<Self, RecipeError> {
        Self::new(
            d.method, d.strength, d.phase, d.metric, d.allowed, d.fit_mode,
        )
    }
}

impl Default for Recipe {
    /// Floyd–Steinberg at 100 %, screen-locked, OKLab, all colours, Adaptive.
    fn default() -> Self {
        Self {
            method: Method::new(DitherKind::FloydSteinberg),
            strength: 1.0,
            phase: PatternPhase::Screen,
            metric: Metric::Oklab,
            allowed: ColourSet::ALL,
            fit_mode: FitMode::Adaptive,
        }
    }
}

impl Recipe {
    /// A recipe; `strength` must be in `0..=1` and `allowed` non-empty.
    pub fn new(
        method: Method,
        strength: f32,
        phase: PatternPhase,
        metric: Metric,
        allowed: ColourSet,
        fit_mode: FitMode,
    ) -> Result<Self, RecipeError> {
        if !(0.0..=1.0).contains(&strength) {
            return Err(RecipeError::Strength(strength));
        }
        if allowed.is_empty() {
            return Err(RecipeError::EmptyAllowedSet);
        }
        Ok(Self {
            method,
            strength,
            phase,
            metric,
            allowed,
            fit_mode,
        })
    }

    /// The default recipe with another method and strength.
    pub fn with_method(kind: DitherKind, strength: f32) -> Result<Self, RecipeError> {
        let d = Self::default();
        Self::new(
            Method::new(kind),
            strength,
            d.phase,
            d.metric,
            d.allowed,
            d.fit_mode,
        )
    }

    /// The dither method.
    pub const fn method(&self) -> Method {
        self.method
    }

    /// Dither strength, `0..=1`, before the attribute map multiplies it by alpha.
    pub const fn strength(&self) -> f32 {
        self.strength
    }

    /// Pattern phase.
    pub const fn phase(&self) -> PatternPhase {
        self.phase
    }

    /// Colour metric.
    pub const fn metric(&self) -> Metric {
        self.metric
    }

    /// Colours this recipe allows into a cell.
    pub const fn allowed(&self) -> ColourSet {
        self.allowed
    }

    /// Force or Adaptive.
    pub const fn fit_mode(&self) -> FitMode {
        self.fit_mode
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_strength_and_empty_set() {
        let d = Recipe::default();
        for s in [-0.01, 1.01, f32::NAN, f32::INFINITY] {
            assert!(matches!(
                Recipe::new(
                    d.method(),
                    s,
                    d.phase(),
                    d.metric(),
                    d.allowed(),
                    d.fit_mode()
                ),
                Err(RecipeError::Strength(_))
            ));
        }
        assert_eq!(
            Recipe::new(
                d.method(),
                0.5,
                d.phase(),
                d.metric(),
                ColourSet::EMPTY,
                d.fit_mode()
            ),
            Err(RecipeError::EmptyAllowedSet)
        );
        assert!(Recipe::with_method(DitherKind::Bayer4, 0.0).is_ok());
        assert!(Recipe::with_method(DitherKind::Bayer4, 1.0).is_ok());
    }

    #[test]
    fn families() {
        assert_eq!(DitherKind::None.family(), DitherFamily::None);
        assert_eq!(DitherKind::Bayer8.family(), DitherFamily::Ordered);
        assert_eq!(DitherKind::Sierra.family(), DitherFamily::Diffusion);
        assert_eq!(DitherKind::BlueNoise.family(), DitherFamily::Noise);
    }

    #[test]
    fn deserialises_through_the_constructor_with_defaults() {
        let r: Recipe = toml::from_str("strength = 0.6\n[method]\nkind = \"bayer4\"\n").unwrap();
        assert_eq!(r, Recipe::with_method(DitherKind::Bayer4, 0.6).unwrap());
        assert!(toml::from_str::<Recipe>("strength = 1.5\n[method]\nkind = \"bayer4\"\n").is_err());
        assert!(
            toml::from_str::<Recipe>("strength = 0.5\nallowed = 0\n[method]\nkind = \"none\"\n")
                .is_err()
        );
    }
}
