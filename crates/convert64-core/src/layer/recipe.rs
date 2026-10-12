//! The dither recipe a layer (or region) contributes to the attribute map
//! (`docs/TAD.md` §3.5, ADR-002).

use crate::colour::ColourSet;
use serde::{Deserialize, Serialize};
use std::fmt;

/// How a pixel is dithered (`docs/reference/dither-methods.md`). Grouped into
/// [`DitherFamily`]s in the UI (UX §6). The serialised id is the reference's; png2c64's
/// own names are accepted too.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DitherKind {
    /// Nearest colour, no dither.
    None,
    /// Ordered, 2 × 2 Bayer matrix.
    #[serde(alias = "bayer2x2")]
    Bayer2,
    /// Ordered, 4 × 4 Bayer matrix.
    #[serde(alias = "bayer4x4")]
    Bayer4,
    /// Ordered, 8 × 8 Bayer matrix.
    #[serde(alias = "bayer8x8")]
    Bayer8,
    /// Error diffusion, Floyd–Steinberg.
    #[serde(alias = "fs")]
    FloydSteinberg,
    /// Error diffusion, Atkinson (diffuses 6/8 of the error).
    Atkinson,
    /// Error diffusion, Jarvis–Judice–Ninke.
    #[serde(alias = "jarvis")]
    JarvisJudiceNinke,
    /// Error diffusion, Sierra (three rows).
    #[serde(alias = "sierra3")]
    Sierra,
    /// Threshold against the committed 64 × 64 blue-noise tile.
    BlueNoise,
    /// Ordered, 16 × 16 Bayer matrix: smooth gradients over large areas.
    Bayer16,
    /// Ordered, png2c64's 2 × 2 Bayer for 2:1 pixels (the same matrix as `bayer2`).
    Bayer2x2Mc,
    /// Ordered, 50 % checkerboard: two levels, png2c64's default.
    Checker,
    /// Ordered, png2c64's 2 × 4 Bayer: eight levels, square on 2:1 pixels.
    H2x4,
    /// Ordered, png2c64's 4 × 4 clustered dot, shaped for 2:1 pixels.
    #[serde(alias = "clustered-dot")]
    Clustered4,
    /// Ordered, 8 × 8 clustered dot: one round dot per tile.
    Clustered8,
    /// Ordered, png2c64's 8 × 8 45° round-dot halftone.
    #[serde(alias = "halftone8x8")]
    Halftone8,
    /// Ordered, ArcPaint64 "Dot": a 6 × 6 round dot.
    Dot,
    /// Ordered, ArcPaint64 "Diamond": an 8 × 8 dot growing as a diamond.
    Diamond,
    /// Ordered, ArcPaint64 "Star": an 8 × 8 dot growing along its axes first.
    Star,
    /// Ordered, horizontal lines with period 2 (png2c64 `line2`).
    #[serde(alias = "line2")]
    LineH2,
    /// Ordered, horizontal lines with period 4 (png2c64 `line4`).
    #[serde(alias = "line4")]
    LineH4,
    /// Ordered, horizontal lines with period 8 (png2c64 `line8`).
    #[serde(alias = "line8")]
    LineH8,
    /// Ordered, vertical lines with period 2 (ArcPaint64 "Vertical").
    LineV2,
    /// Ordered, vertical lines with period 4 (ArcPaint64 "Vertical 4").
    LineV4,
    /// Ordered, vertical lines with period 8 (ArcPaint64 "Vertical 8").
    LineV8,
    /// Ordered, png2c64's lines with a checker offset (ArcPaint64 "Horizontal +
    /// Checker").
    LineChecker,
    /// Ordered, 45° lines falling to the right (ArcPaint64 "Diagonal").
    DiagDown,
    /// Ordered, 45° lines rising to the right.
    DiagUp,
    /// Ordered, png2c64's 8 × 8 diagonal clustered dot.
    #[serde(alias = "diagonal8x8")]
    Diagonal8,
    /// Ordered, png2c64's 5 × 5 spiral.
    #[serde(alias = "spiral5x5")]
    Spiral5,
    /// Ordered, png2c64's 5 × 5 slanted tiling.
    #[serde(alias = "hex5x5")]
    Hex5,
    /// Ordered, png2c64's 8 × 8 hexagonal tiling.
    #[serde(alias = "hex8x8")]
    Hex8,
    /// Ordered, a user 8 × 8 or 16 × 16 matrix ([`Method::custom`]); Bayer 8 without
    /// one.
    Custom,
    /// Noise, interleaved gradient noise (Jimenez).
    Ign,
    /// Noise, the R2 low-discrepancy sequence (Roberts).
    #[serde(alias = "r2-sequence")]
    R2,
    /// Noise, a per-pixel hash (png2c64's; the same everywhere, so goldens hold).
    WhiteNoise,
    /// Effect, png2c64's overlaid angled lines (pen and ink).
    Crosshatch,
    /// Effect, png2c64's concentric circles around the screen centre (engraving).
    Radial,
    /// Effect, png2c64's smooth hash noise (organic clumps).
    ValueNoise,
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
    /// Thresholds that are effects rather than quantisers (`radial`, `crosshatch`,
    /// `value-noise`): kept out of the quality-bar comparisons.
    Effects,
    /// Methods that search colour mixes for the small fixed palette (#115).
    PaletteAware,
}

impl DitherFamily {
    /// Whether the family thresholds each pixel against a pattern (ordered, noise and
    /// effects): positional, parallel, no error carried between pixels.
    pub const fn is_threshold(self) -> bool {
        matches!(self, Self::Ordered | Self::Noise | Self::Effects)
    }
}

impl DitherKind {
    /// Every kind: E5's nine first (their order is part of the attribute map's debug
    /// colours), then the rest by family.
    pub const ALL: [Self; 39] = [
        Self::None,
        Self::Bayer2,
        Self::Bayer4,
        Self::Bayer8,
        Self::FloydSteinberg,
        Self::Atkinson,
        Self::JarvisJudiceNinke,
        Self::Sierra,
        Self::BlueNoise,
        Self::Bayer16,
        Self::Bayer2x2Mc,
        Self::Checker,
        Self::H2x4,
        Self::Clustered4,
        Self::Clustered8,
        Self::Halftone8,
        Self::Dot,
        Self::Diamond,
        Self::Star,
        Self::LineH2,
        Self::LineH4,
        Self::LineH8,
        Self::LineV2,
        Self::LineV4,
        Self::LineV8,
        Self::LineChecker,
        Self::DiagDown,
        Self::DiagUp,
        Self::Diagonal8,
        Self::Spiral5,
        Self::Hex5,
        Self::Hex8,
        Self::Custom,
        Self::Ign,
        Self::R2,
        Self::WhiteNoise,
        Self::Crosshatch,
        Self::Radial,
        Self::ValueNoise,
    ];

    /// The family this kind belongs to.
    pub const fn family(self) -> DitherFamily {
        match self {
            Self::None => DitherFamily::None,
            Self::FloydSteinberg | Self::Atkinson | Self::JarvisJudiceNinke | Self::Sierra => {
                DitherFamily::Diffusion
            }
            Self::BlueNoise | Self::Ign | Self::R2 | Self::WhiteNoise => DitherFamily::Noise,
            Self::Crosshatch | Self::Radial | Self::ValueNoise => DitherFamily::Effects,
            Self::Bayer2
            | Self::Bayer4
            | Self::Bayer8
            | Self::Bayer16
            | Self::Bayer2x2Mc
            | Self::Checker
            | Self::H2x4
            | Self::Clustered4
            | Self::Clustered8
            | Self::Halftone8
            | Self::Dot
            | Self::Diamond
            | Self::Star
            | Self::LineH2
            | Self::LineH4
            | Self::LineH8
            | Self::LineV2
            | Self::LineV4
            | Self::LineV8
            | Self::LineChecker
            | Self::DiagDown
            | Self::DiagUp
            | Self::Diagonal8
            | Self::Spiral5
            | Self::Hex5
            | Self::Hex8
            | Self::Custom => DitherFamily::Ordered,
        }
    }
}

/// Orientation preference for ordered and noise patterns (the PDD's "line- and
/// 2:1-biased variants").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Orientation {
    /// The pattern as is, sampled in target pixels.
    #[default]
    Auto,
    /// Line variant: the threshold changes along x and holds for `n` rows, so the
    /// pattern forms short vertical lines, which survive PAL colour bleed.
    VerticalBiased,
    /// 2:1 variant: each matrix row covers two target rows, so on a multicolor screen
    /// (2:1 pixels) the pattern's dots are square.
    TwoToOne,
}

/// A dither method: the kind, its orientation preference and, for
/// [`DitherKind::Custom`], the user's matrix.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Method {
    /// Which algorithm.
    pub kind: DitherKind,
    /// Orientation preference.
    #[serde(default)]
    pub orientation: Orientation,
    /// The threshold matrix of [`DitherKind::Custom`] (ignored by other kinds).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom: Option<CustomMatrix>,
}

impl Method {
    /// `kind` with the default orientation.
    pub const fn new(kind: DitherKind) -> Self {
        Self {
            kind,
            orientation: Orientation::Auto,
            custom: None,
        }
    }

    /// [`DitherKind::Custom`] with `matrix`.
    pub const fn custom(matrix: CustomMatrix) -> Self {
        Self {
            kind: DitherKind::Custom,
            orientation: Orientation::Auto,
            custom: Some(matrix),
        }
    }
}

/// A user threshold matrix for [`DitherKind::Custom`] (`docs/reference/dither-methods.md`
/// §1): 8 × 8 or 16 × 16 values, row-major. Only their order matters: they are ranked
/// (ties by position) and the pixel at rank `r` of `n²` gets threshold `(r + ½) / n²`.
/// Serialised as the ranks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "Vec<u16>", into = "Vec<u16>")]
pub struct CustomMatrix {
    size: u8,
    ranks: [u8; 256],
}

impl CustomMatrix {
    /// A matrix from 64 (8 × 8) or 256 (16 × 16) values, row-major.
    pub fn new(values: &[u16]) -> Result<Self, String> {
        let size: u8 = match values.len() {
            64 => 8,
            256 => 16,
            n => return Err(format!("a custom matrix has 64 or 256 values, got {n}")),
        };
        let mut order: Vec<usize> = (0..values.len()).collect();
        order.sort_by_key(|&i| (values[i], i));
        let mut ranks = [0u8; 256];
        for (rank, &i) in order.iter().enumerate() {
            ranks[i] = u8::try_from(rank).expect("at most 256 values");
        }
        Ok(Self { size, ranks })
    }

    /// Side length, 8 or 16.
    pub const fn size(&self) -> usize {
        self.size as usize
    }

    /// The rank of cell `(x, y)`, `0..size²`.
    pub const fn rank(&self, x: usize, y: usize) -> u8 {
        self.ranks[y * self.size as usize + x]
    }
}

impl TryFrom<Vec<u16>> for CustomMatrix {
    type Error = String;
    fn try_from(v: Vec<u16>) -> Result<Self, String> {
        Self::new(&v)
    }
}

impl From<CustomMatrix> for Vec<u16> {
    fn from(m: CustomMatrix) -> Self {
        let n = m.size() * m.size();
        m.ranks[..n].iter().map(|&r| u16::from(r)).collect()
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
    /// `max_luma_diff` is above 8.
    MaxLumaDiff(u8),
    /// `adaptive` is not a number in `0..=1`.
    Adaptive(f32),
    /// `mix_levels` is above [`MAX_MIX_LEVELS`].
    MixLevels(u8),
}

impl fmt::Display for RecipeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Strength(s) => write!(f, "dither strength must be in 0–1, got {s}"),
            Self::EmptyAllowedSet => f.write_str("a recipe must allow at least one colour"),
            Self::MaxLumaDiff(n) => write!(f, "max_luma_diff must be 0–8 luma levels, got {n}"),
            Self::Adaptive(a) => write!(f, "adaptive must be in 0–1, got {a}"),
            Self::MixLevels(n) => {
                write!(f, "mix_levels must be 0–{MAX_MIX_LEVELS}, got {n}")
            }
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
    #[serde(default = "yes")]
    cell_clamp: bool,
    #[serde(default = "luma_off")]
    max_luma_diff: u8,
    #[serde(default)]
    adaptive: f32,
    #[serde(default = "default_mix_levels")]
    mix_levels: u8,
    #[serde(default)]
    smooth: bool,
}

const fn all_colours() -> ColourSet {
    ColourSet::ALL
}

const fn yes() -> bool {
    true
}

const fn luma_off() -> u8 {
    MAX_LUMA_DIFF_OFF
}

const fn default_mix_levels() -> u8 {
    DEFAULT_MIX_LEVELS
}

/// The default `mix_levels`: a cell mixes its two colours at 0, 25, 50, 75 or 100 %.
pub const DEFAULT_MIX_LEVELS: u8 = 4;

/// The largest `mix_levels`.
pub const MAX_MIX_LEVELS: u8 = 16;

/// `max_luma_diff` at which it restricts nothing: the nine luma levels are at most 8
/// apart.
pub const MAX_LUMA_DIFF_OFF: u8 = 8;

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
    cell_clamp: bool,
    max_luma_diff: u8,
    adaptive: f32,
    mix_levels: u8,
    smooth: bool,
}

impl TryFrom<RecipeData> for Recipe {
    type Error = RecipeError;
    fn try_from(d: RecipeData) -> Result<Self, RecipeError> {
        Self::new(
            d.method, d.strength, d.phase, d.metric, d.allowed, d.fit_mode,
        )?
        .with_cell_clamp(d.cell_clamp)
        .with_max_luma_diff(d.max_luma_diff)?
        .with_adaptive(d.adaptive)?
        .with_mix_levels(d.mix_levels)
        .map(|r| r.with_smooth(d.smooth))
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
            cell_clamp: true,
            max_luma_diff: MAX_LUMA_DIFF_OFF,
            adaptive: 0.0,
            mix_levels: DEFAULT_MIX_LEVELS,
            smooth: false,
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
            cell_clamp: true,
            max_luma_diff: MAX_LUMA_DIFF_OFF,
            adaptive: 0.0,
            mix_levels: DEFAULT_MIX_LEVELS,
            smooth: false,
        })
    }

    /// This recipe with `cell_clamp` set: `true` (the default) drops diffused error at
    /// cell borders where the receiving cell lacks the colour it pulls towards (TAD
    /// §3.1); `false` lets error flow into neighbouring cells as png2c64 does.
    pub const fn with_cell_clamp(mut self, cell_clamp: bool) -> Self {
        self.cell_clamp = cell_clamp;
        self
    }

    /// This recipe with `max_luma_diff` set: every dither mix (ordered, diffusion, pair,
    /// Yliluoma) is restricted to colours within this many of the nine luma levels of
    /// `docs/reference/colour-pairs.md`. 0 allows same-luma pairs only (PAL-blend safe);
    /// 8, the default, restricts nothing.
    pub fn with_max_luma_diff(mut self, levels: u8) -> Result<Self, RecipeError> {
        if levels > MAX_LUMA_DIFF_OFF {
            return Err(RecipeError::MaxLumaDiff(levels));
        }
        self.max_luma_diff = levels;
        Ok(self)
    }

    /// This recipe with `adaptive` set (png2c64 `--adaptive`): dithering is scaled by
    /// `1 − adaptive × detail`, where detail is the mean OKLab |ΔL| to the 4 neighbours
    /// / 0.15, capped at 1. 0 (the default) is off.
    pub fn with_adaptive(mut self, adaptive: f32) -> Result<Self, RecipeError> {
        if !(0.0..=1.0).contains(&adaptive) {
            return Err(RecipeError::Adaptive(adaptive));
        }
        self.adaptive = adaptive;
        Ok(self)
    }

    /// This recipe with `mix_levels` set (ordered and noise methods; #142): each cell
    /// mixes its two colours at one of `mix_levels + 1` evenly spaced ratios (4, the
    /// default: 0/25/50/75/100 %), taken from the mean over its pixels and snapped to
    /// what its neighbours show when within one step, so a flat area gets one calm
    /// pattern. Pixels far from their cell's mean (edges, detail) keep their own ratio,
    /// quantised the same way. 0 keeps the continuous per-pixel ratio of E5.
    pub fn with_mix_levels(mut self, levels: u8) -> Result<Self, RecipeError> {
        if levels > MAX_MIX_LEVELS {
            return Err(RecipeError::MixLevels(levels));
        }
        self.mix_levels = levels;
        Ok(self)
    }

    /// This recipe with `smooth` set (ordered and noise methods; #142): `true` takes the
    /// mix ratio from an edge-aware blur of the composite (σ ≈ 1 target pixel;
    /// neighbours more than 0.08 OKLab ΔE away are left out) while colours are still
    /// chosen from the unblurred one, so micro-texture cannot move a pixel's ratio.
    /// `false` (the default) uses the composite as is: with mix levels a flat area
    /// already gets one pattern, and on the golden fixtures smoothing raised the blurred
    /// error slightly on every one (title past png2c64's).
    pub const fn with_smooth(mut self, smooth: bool) -> Self {
        self.smooth = smooth;
        self
    }

    /// Mix-ratio levels (see [`Self::with_mix_levels`]).
    pub const fn mix_levels(&self) -> u8 {
        self.mix_levels
    }

    /// Edge-aware pre-smoothing (see [`Self::with_smooth`]).
    pub const fn smooth(&self) -> bool {
        self.smooth
    }

    /// Whether diffused error is dropped at cell borders (see [`Self::with_cell_clamp`]).
    pub const fn cell_clamp(&self) -> bool {
        self.cell_clamp
    }

    /// Luma-level limit of dither mixes (see [`Self::with_max_luma_diff`]).
    pub const fn max_luma_diff(&self) -> u8 {
        self.max_luma_diff
    }

    /// Detail-adaptive dither reduction (see [`Self::with_adaptive`]).
    pub const fn adaptive(&self) -> f32 {
        self.adaptive
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
        for k in [
            DitherKind::Radial,
            DitherKind::Crosshatch,
            DitherKind::ValueNoise,
        ] {
            assert_eq!(k.family(), DitherFamily::Effects);
        }
        assert!(DitherFamily::Effects.is_threshold() && !DitherFamily::Diffusion.is_threshold());
    }

    #[test]
    fn every_kind_is_listed_once_and_round_trips_by_its_id() {
        let mut seen = std::collections::HashSet::new();
        for k in DitherKind::ALL {
            assert!(seen.insert(k), "{k:?} twice");
            assert_eq!(
                DitherKind::ALL[k as usize],
                k,
                "ALL is in declaration order"
            );
            let text = toml::to_string(&Method::new(k)).unwrap();
            let back: Method = toml::from_str(&text).unwrap();
            assert_eq!(back.kind, k, "{text}");
        }
        // The reference's ids and png2c64's names are accepted.
        for (id, want) in [
            ("fs", DitherKind::FloydSteinberg),
            ("jarvis", DitherKind::JarvisJudiceNinke),
            ("sierra3", DitherKind::Sierra),
            ("bayer16", DitherKind::Bayer16),
            ("bayer2x2-mc", DitherKind::Bayer2x2Mc),
            ("line-h2", DitherKind::LineH2),
            ("line2", DitherKind::LineH2),
            ("line-v8", DitherKind::LineV8),
            ("diag-down", DitherKind::DiagDown),
            ("clustered-dot", DitherKind::Clustered4),
            ("halftone8x8", DitherKind::Halftone8),
            ("r2-sequence", DitherKind::R2),
            ("white-noise", DitherKind::WhiteNoise),
            ("value-noise", DitherKind::ValueNoise),
        ] {
            let m: Method = toml::from_str(&format!("kind = \"{id}\"")).unwrap();
            assert_eq!(m.kind, want, "{id}");
        }
    }

    #[test]
    fn custom_matrices_serialise_as_ranks() {
        let values: Vec<u16> = (0..64).rev().collect();
        let m = Method::custom(CustomMatrix::new(&values).unwrap());
        let text = toml::to_string(&m).unwrap();
        let back: Method = toml::from_str(&text).unwrap();
        assert_eq!(back, m);
        assert!(toml::from_str::<Method>("kind = \"custom\"\ncustom = [1, 2]").is_err());
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
        let knobs: Recipe = toml::from_str(
            "strength = 0.5\ncell_clamp = false\nmax_luma_diff = 0\nadaptive = 0.4\n[method]\nkind = \"bayer4\"\n",
        )
        .unwrap();
        assert!(!knobs.cell_clamp() && knobs.max_luma_diff() == 0 && knobs.adaptive() == 0.4);
        assert!(
            toml::from_str::<Recipe>(
                "strength = 0.5\nmax_luma_diff = 9\n[method]\nkind = \"none\"\n"
            )
            .is_err()
        );
        assert!(
            toml::from_str::<Recipe>("strength = 0.5\nadaptive = 1.5\n[method]\nkind = \"none\"\n")
                .is_err()
        );
        let d = Recipe::default();
        assert!(d.cell_clamp() && d.max_luma_diff() == MAX_LUMA_DIFF_OFF && d.adaptive() == 0.0);
        assert!(d.mix_levels() == DEFAULT_MIX_LEVELS && !d.smooth());
    }

    #[test]
    fn mix_levels_and_smooth_parse_and_validate() {
        let r: Recipe = toml::from_str(
            "strength = 0.7\nmix_levels = 8\nsmooth = true\n[method]\nkind = \"bayer2\"\n",
        )
        .unwrap();
        assert_eq!((r.mix_levels(), r.smooth()), (8, true));
        assert!(
            toml::from_str::<Recipe>(
                "strength = 0.7\nmix_levels = 17\n[method]\nkind = \"bayer2\"\n"
            )
            .is_err()
        );
        assert_eq!(
            Recipe::default().with_mix_levels(MAX_MIX_LEVELS + 1),
            Err(RecipeError::MixLevels(MAX_MIX_LEVELS + 1))
        );
        let back: Recipe = toml::from_str(&toml::to_string(&r).unwrap()).unwrap();
        assert_eq!(back, r);
    }
}
