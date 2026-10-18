//! The document commands act on (ADR-005): the layer stack, regions and steering, and
//! since #193 the target mode, palette and solve settings, which the registry's
//! `mode_set`, `palette_set`, `background_pin` and `solve_set` commands change.

use crate::layer::{Layer, Snap};
use crate::mode::Mode;
use crate::palette::Palette;
use crate::region::Region;
use crate::solve::png2c64_ref::Divergences;
use crate::solve::steering::Steering;
use crate::solve::{Background, BackgroundRule, SolveMode, SolveOptions, SolverKind};
use serde::{Deserialize, Serialize};

/// The document: everything a `.convert64` project file holds except where its sources
/// came from and the supersample factor (the front-end's concern).
#[derive(Clone, Debug, PartialEq)]
pub struct Project {
    /// Layers, bottom first.
    pub layers: Vec<Layer>,
    /// Region overrides, applied in order.
    pub regions: Vec<Region>,
    /// Cell pins, locks and allow/deny lists (UX §7).
    pub steering: Steering,
    /// The VIC-II mode the picture is solved for.
    pub target: Mode,
    /// The palette the picture is solved and previewed with (CLAUDE.md non-negotiable 5).
    pub palette: Palette,
    /// How the picture is solved.
    pub solve: SolveSettings,
    /// What transform commands snap to (ADR-011); never stored in a transform.
    pub snap: Snap,
}

impl Default for Project {
    /// No layers, multicolor bitmap, Pepto PAL, the default solve settings, no snap.
    fn default() -> Self {
        Self {
            layers: Vec::new(),
            regions: Vec::new(),
            steering: Steering::default(),
            target: Mode::MulticolorBitmap,
            palette: Palette::pepto_pal(),
            solve: SolveSettings::default(),
            snap: Snap::Off,
        }
    }
}

/// The project-wide solve settings (a project file's `solve`; the per-pixel knobs live in
/// each layer's recipe).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SolveSettings {
    /// The solver: `reference` (png2amiga's C64 path, the default since ADR-013),
    /// `png2c64` (png2c64's port) or `native`.
    #[serde(default)]
    pub solver: SolverKind,
    /// png2amiga's `--cell-refine` after the reference solve (#261), kept only if
    /// SSIMULACRA2 improves; off by default, written only when on.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub refine: bool,
    /// The png2c64 port's named divergences, all off by default.
    #[serde(default, skip_serializing_if = "Divergences::none")]
    pub divergences: Divergences,
    /// Background: `auto` (default: not pinned) or `{"fixed": n}`.
    #[serde(default)]
    pub background: Background,
    /// How the reference chooses an unpinned multicolor background: `black` (default,
    /// png2amiga's colour 0) or `search` (png2c64's search of all 16); written only when
    /// not the default.
    #[serde(default, skip_serializing_if = "BackgroundRule::is_default")]
    pub background_rule: BackgroundRule,
    /// Refinement hysteresis ratio, e.g. 0.97; absent = off.
    #[serde(default)]
    pub hysteresis: Option<f32>,
    /// Same-luma pair bias per pair (positive penalises, negative rewards).
    #[serde(default)]
    pub luma_pair_bias: f32,
    /// Refinement rounds, 0–2.
    #[serde(default = "default_refinements")]
    pub refinements: u8,
    /// Backgrounds carried from the search into dithering, at least 1.
    #[serde(default = "default_finalists")]
    pub finalists: u8,
}

fn default_refinements() -> u8 {
    SolveOptions::default().refinements
}

fn default_finalists() -> u8 {
    SolveOptions::default().finalists
}

impl Default for SolveSettings {
    fn default() -> Self {
        Self {
            solver: SolverKind::default(),
            refine: false,
            divergences: Divergences::default(),
            background: Background::default(),
            background_rule: BackgroundRule::default(),
            hysteresis: None,
            luma_pair_bias: 0.0,
            refinements: default_refinements(),
            finalists: default_finalists(),
        }
    }
}

impl SolveSettings {
    /// Checks the values a solve cannot take; the error names the field.
    pub fn check(&self) -> Result<(), String> {
        if let Some(h) = self.hysteresis
            && !(h > 0.0 && h <= 1.0)
        {
            return Err(format!("hysteresis must be in (0, 1], got {h}"));
        }
        if !self.luma_pair_bias.is_finite() {
            return Err("luma_pair_bias must be finite".to_owned());
        }
        if self.refinements > 2 {
            return Err(format!("refinements must be 0–2, got {}", self.refinements));
        }
        if self.finalists == 0 || self.finalists > 16 {
            return Err(format!("finalists must be 1–16, got {}", self.finalists));
        }
        let d = &self.divergences;
        if !d.pair_bias.is_finite() {
            return Err("divergences: pair_bias must be finite".to_owned());
        }
        Ok(())
    }

    /// The solver's options for these settings.
    pub fn options(&self, mode: SolveMode, layer_origins: Vec<(i64, i64)>) -> SolveOptions {
        SolveOptions {
            solver: self.solver,
            refine: self.refine,
            divergences: self.divergences,
            mode,
            background: self.background,
            background_rule: self.background_rule,
            hysteresis: self.hysteresis,
            luma_pair_bias: self.luma_pair_bias,
            refinements: self.refinements,
            finalists: self.finalists,
            layer_origins,
            ..SolveOptions::default()
        }
    }
}
