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
    #[serde(alias = "vline2")]
    LineV2,
    /// Ordered, vertical lines with period 4 (ArcPaint64 "Vertical 4").
    #[serde(alias = "vline4")]
    LineV4,
    /// Ordered, vertical lines with period 8 (ArcPaint64 "Vertical 8").
    #[serde(alias = "vline8")]
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
    /// Error diffusion, "false" Floyd–Steinberg 3/3/2: lighter and faster.
    FalseFs,
    /// Error diffusion, png2c64's Floyd–Steinberg for 2:1 pixels (5/3/6/2).
    FsWide,
    /// Error diffusion, png2c64's line-biased Floyd–Steinberg: all error goes down.
    LineFs,
    /// Error diffusion, Stucki.
    Stucki,
    /// Error diffusion, Burkes.
    Burkes,
    /// Error diffusion, two-row Sierra.
    Sierra2,
    /// Error diffusion, Sierra Lite.
    SierraLite,
    /// Error diffusion, Stevenson–Arce (hexagonal grid weights).
    StevensonArce,
    /// Error diffusion, Shiau–Fan (four taps): fewer worms than Floyd–Steinberg.
    ShiauFan1,
    /// Error diffusion, Shiau–Fan (five taps).
    ShiauFan2,
    /// Error diffusion, png2c64's Ostromoukhov: Floyd–Steinberg weights scaled by how
    /// uncertain the pixel's choice was.
    Ostromoukhov,
    /// Error diffusion, Zhou–Fang: Ostromoukhov plus threshold modulation by the
    /// blue-noise tile, which removes the remaining worms.
    #[serde(alias = "zhoufang")]
    ZhouFang,
    /// Error diffusion along a Hilbert curve with a 16-sample history (Riemersma): no
    /// directional artefacts.
    Riemersma,
    /// Palette-aware, Yliluoma's algorithm 1: per pixel the best mix of two set
    /// colours and its ratio, placed by Bayer 8.
    Yliluoma1,
    /// Palette-aware, Yliluoma's algorithm 2: a 16-entry candidate list whose running
    /// average approaches the pixel, indexed by Bayer 8.
    Yliluoma2,
    /// Palette-aware, Yliluoma's algorithm 3: Knoll's error-accumulating candidate list
    /// in linear light.
    Yliluoma3,
    /// Palette-aware, Thomas Knoll's pattern dithering (Photoshop "Pattern"): the
    /// candidate list built by accumulating error in the metric's space.
    Knoll,
    /// Palette-aware, dithering on lightness only: the nearest colour and the set colour
    /// on the other side in lightness closest to its hue, so no colour speckle.
    Luma,
    /// Palette-aware, same-luma pair weaving: the two set colours whose even mix is
    /// nearest, same-luma pairs preferred, alternated by scanline (multicolor, so a
    /// PAL display blends them) or as a checker (hi-res).
    Pair,
    /// Ordered, png2amiga's 3 × 3 dispersed dot (a magic-square permutation).
    #[serde(alias = "bayer3x3")]
    Bayer3,
    /// Ordered, png2amiga's 5 × 5 dispersed dot.
    #[serde(alias = "bayer5x5")]
    Bayer5,
    /// Ordered, png2amiga's 6 × 6 dispersed dot.
    #[serde(alias = "bayer6x6")]
    Bayer6,
    /// Ordered, png2amiga's 7 × 7 dispersed dot.
    #[serde(alias = "bayer7x7")]
    Bayer7,
    /// Ordered, png2amiga's 4 × 2 vertical-bias matrix (for tall pixels).
    V4x2,
    /// Ordered, Bayer ordering 4 wide × 2 tall.
    Bayer4x2,
    /// Ordered, Bayer ordering 2 wide × 4 tall.
    Bayer2x4,
    /// Ordered, png2amiga's column-biased 2 × 2 (vertical lines with a row shift).
    #[serde(alias = "vline-checker")]
    LineVChecker,
    /// Ordered, a 16 × 16 self-nested Bayer recursion (Obra Dinn's).
    Fractal16,
    /// Ordered, Aseprite's hand-tuned "old" 4 × 4, checker-biased.
    AsepriteOld,
    /// Ordered, libcaca's hand-tuned 3 × 3 dispersed dot.
    Libcaca3,
    /// Ordered, libcaca's hand-tuned 6 × 6 dispersed dot.
    Libcaca6,
    /// Ordered, the Pegasus/REXPaint 8 × 8 mosaic (centre-biased).
    Pegasus,
    /// Noise, png2amiga's 64 × 64 void-and-cluster blue noise (Ulichney).
    VoidCluster,
    /// Noise, png2amiga's 64 × 64 cluster blue noise (wider clusters).
    ClusterNoise,
    /// Noise, Bayer 8 with a hashed per-tile offset (Cranley–Patterson).
    CranleyBayer,
    /// Noise, interleaved gradient noise remapped to a triangular distribution.
    IgnTri,
    /// Noise, R2 remapped to a triangular distribution (Wronski).
    R2Tri,
    /// Effect, a sum of five cosine waves (Sloan's quasicrystal).
    Quasicrystal,
    /// Effect, Truchet tiles of quarter-arc gradients, one rotation per 8 × 8 tile.
    Truchet,
    /// Error diffusion, png2amiga's Floyd–Steinberg with the residual carried in OKLab.
    FloydSteinbergOklab,
    /// png2amiga's Gilbert-curve diffusion; on its C64 path a plain nearest pick (it has no kernel there).
    Gilbert,
    /// png2amiga's direct binary search; on its C64 path a plain nearest pick (it has no kernel there).
    Dbs,
    /// Error diffusion, Floyd–Steinberg with a Laplacian-modulated threshold (keeps edges).
    StructureFs,
    /// Error diffusion, Floyd–Steinberg with a local-contrast-modulated threshold.
    ContrastFs,
    /// Palette-aware, png2amiga's Yliluoma: a 64-entry greedy mixing plan on Bayer 8.
    Yliluoma,
    /// Palette-aware, png2amiga's optimal checker: the nearest colour and its best partner on a 2 × 2 checker.
    OptChecker,
    /// Palette-aware, the optimal pair alternated by row.
    OptLine,
    /// Palette-aware, a 4-colour plan on the line-checker 2 × 2.
    OptLineChecker,
    /// Palette-aware, the optimal pair alternated by column.
    OptVline,
    /// Palette-aware, a 4-colour plan on the column-checker 2 × 2.
    OptVlineChecker,
    /// Palette-aware, Yliluoma's tri-tone: a 4-entry plan on Bayer 2 (one colour 50 %, two 25 %).
    TriTone,
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
    /// Methods that search colour mixes for the small fixed palette, placed by Bayer 8
    /// (`docs/reference/dither-methods.md` §4).
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
    pub const ALL: [Self; 90] = [
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
        Self::FalseFs,
        Self::FsWide,
        Self::LineFs,
        Self::Stucki,
        Self::Burkes,
        Self::Sierra2,
        Self::SierraLite,
        Self::StevensonArce,
        Self::ShiauFan1,
        Self::ShiauFan2,
        Self::Ostromoukhov,
        Self::ZhouFang,
        Self::Riemersma,
        Self::Yliluoma1,
        Self::Yliluoma2,
        Self::Yliluoma3,
        Self::Knoll,
        Self::Luma,
        Self::Pair,
        Self::Bayer3,
        Self::Bayer5,
        Self::Bayer6,
        Self::Bayer7,
        Self::V4x2,
        Self::Bayer4x2,
        Self::Bayer2x4,
        Self::LineVChecker,
        Self::Fractal16,
        Self::AsepriteOld,
        Self::Libcaca3,
        Self::Libcaca6,
        Self::Pegasus,
        Self::VoidCluster,
        Self::ClusterNoise,
        Self::CranleyBayer,
        Self::IgnTri,
        Self::R2Tri,
        Self::Quasicrystal,
        Self::Truchet,
        Self::FloydSteinbergOklab,
        Self::Gilbert,
        Self::Dbs,
        Self::StructureFs,
        Self::ContrastFs,
        Self::Yliluoma,
        Self::OptChecker,
        Self::OptLine,
        Self::OptLineChecker,
        Self::OptVline,
        Self::OptVlineChecker,
        Self::TriTone,
    ];

    /// The id recipes and the command line spell this kind with (its serialised name).
    pub const fn id(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Bayer2 => "bayer2",
            Self::Bayer4 => "bayer4",
            Self::Bayer8 => "bayer8",
            Self::FloydSteinberg => "floyd-steinberg",
            Self::Atkinson => "atkinson",
            Self::JarvisJudiceNinke => "jarvis-judice-ninke",
            Self::Sierra => "sierra",
            Self::BlueNoise => "blue-noise",
            Self::Bayer16 => "bayer16",
            Self::Bayer2x2Mc => "bayer2x2-mc",
            Self::Checker => "checker",
            Self::H2x4 => "h2x4",
            Self::Clustered4 => "clustered4",
            Self::Clustered8 => "clustered8",
            Self::Halftone8 => "halftone8",
            Self::Dot => "dot",
            Self::Diamond => "diamond",
            Self::Star => "star",
            Self::LineH2 => "line-h2",
            Self::LineH4 => "line-h4",
            Self::LineH8 => "line-h8",
            Self::LineV2 => "line-v2",
            Self::LineV4 => "line-v4",
            Self::LineV8 => "line-v8",
            Self::LineChecker => "line-checker",
            Self::DiagDown => "diag-down",
            Self::DiagUp => "diag-up",
            Self::Diagonal8 => "diagonal8",
            Self::Spiral5 => "spiral5",
            Self::Hex5 => "hex5",
            Self::Hex8 => "hex8",
            Self::Custom => "custom",
            Self::Ign => "ign",
            Self::R2 => "r2",
            Self::WhiteNoise => "white-noise",
            Self::Crosshatch => "crosshatch",
            Self::Radial => "radial",
            Self::ValueNoise => "value-noise",
            Self::FalseFs => "false-fs",
            Self::FsWide => "fs-wide",
            Self::LineFs => "line-fs",
            Self::Stucki => "stucki",
            Self::Burkes => "burkes",
            Self::Sierra2 => "sierra2",
            Self::SierraLite => "sierra-lite",
            Self::StevensonArce => "stevenson-arce",
            Self::ShiauFan1 => "shiau-fan1",
            Self::ShiauFan2 => "shiau-fan2",
            Self::Ostromoukhov => "ostromoukhov",
            Self::ZhouFang => "zhou-fang",
            Self::Riemersma => "riemersma",
            Self::Yliluoma1 => "yliluoma1",
            Self::Yliluoma2 => "yliluoma2",
            Self::Yliluoma3 => "yliluoma3",
            Self::Knoll => "knoll",
            Self::Luma => "luma",
            Self::Pair => "pair",
            Self::Bayer3 => "bayer3",
            Self::Bayer5 => "bayer5",
            Self::Bayer6 => "bayer6",
            Self::Bayer7 => "bayer7",
            Self::V4x2 => "v4x2",
            Self::Bayer4x2 => "bayer4x2",
            Self::Bayer2x4 => "bayer2x4",
            Self::LineVChecker => "line-v-checker",
            Self::Fractal16 => "fractal16",
            Self::AsepriteOld => "aseprite-old",
            Self::Libcaca3 => "libcaca3",
            Self::Libcaca6 => "libcaca6",
            Self::Pegasus => "pegasus",
            Self::VoidCluster => "void-cluster",
            Self::ClusterNoise => "cluster-noise",
            Self::CranleyBayer => "cranley-bayer",
            Self::IgnTri => "ign-tri",
            Self::R2Tri => "r2-tri",
            Self::Quasicrystal => "quasicrystal",
            Self::Truchet => "truchet",
            Self::FloydSteinbergOklab => "floyd-steinberg-oklab",
            Self::Gilbert => "gilbert",
            Self::Dbs => "dbs",
            Self::StructureFs => "structure-fs",
            Self::ContrastFs => "contrast-fs",
            Self::Yliluoma => "yliluoma",
            Self::OptChecker => "opt-checker",
            Self::OptLine => "opt-line",
            Self::OptLineChecker => "opt-line-checker",
            Self::OptVline => "opt-vline",
            Self::OptVlineChecker => "opt-vline-checker",
            Self::TriTone => "tri-tone",
        }
    }

    /// Whether this is one of the 32 methods only png2amiga has (#261): the png2amiga
    /// solver's. On the reference (png2c64's port) they go to the native solver, which runs
    /// [`Self::native`]'s stand-in, until each is judged on our pipeline (ADR-014 §6).
    pub const fn is_png2amiga_only(self) -> bool {
        matches!(
            self,
            Self::Bayer3
                | Self::Bayer5
                | Self::Bayer6
                | Self::Bayer7
                | Self::V4x2
                | Self::Bayer4x2
                | Self::Bayer2x4
                | Self::LineVChecker
                | Self::Fractal16
                | Self::AsepriteOld
                | Self::Libcaca3
                | Self::Libcaca6
                | Self::Pegasus
                | Self::VoidCluster
                | Self::ClusterNoise
                | Self::CranleyBayer
                | Self::IgnTri
                | Self::R2Tri
                | Self::Quasicrystal
                | Self::Truchet
                | Self::FloydSteinbergOklab
                | Self::Gilbert
                | Self::Dbs
                | Self::StructureFs
                | Self::ContrastFs
                | Self::Yliluoma
                | Self::OptChecker
                | Self::OptLine
                | Self::OptLineChecker
                | Self::OptVline
                | Self::OptVlineChecker
                | Self::TriTone
        )
    }

    /// The kind the native solver runs for this one (#261). The kinds only png2amiga has
    /// are the reference's; on the native solver its threshold patterns are used as they
    /// are (`solve::ordered::threshold`), and the others stand in for the nearest native
    /// method: the Floyd–Steinberg variants for Floyd–Steinberg, `gilbert` and `dbs` for
    /// no dither (as png2amiga's C64 path runs them), `yliluoma` for `yliluoma2`, the
    /// optimal pairs and plans for `pair`, `tri-tone` for `knoll`.
    pub const fn native(self) -> Self {
        match self {
            Self::FloydSteinbergOklab | Self::StructureFs | Self::ContrastFs => {
                Self::FloydSteinberg
            }
            Self::Gilbert | Self::Dbs => Self::None,
            Self::Yliluoma => Self::Yliluoma2,
            Self::OptChecker
            | Self::OptLine
            | Self::OptLineChecker
            | Self::OptVline
            | Self::OptVlineChecker => Self::Pair,
            Self::TriTone => Self::Knoll,
            k => k,
        }
    }

    /// The family this kind belongs to.
    pub const fn family(self) -> DitherFamily {
        match self {
            Self::None => DitherFamily::None,
            Self::FloydSteinberg
            | Self::Atkinson
            | Self::JarvisJudiceNinke
            | Self::Sierra
            | Self::FalseFs
            | Self::FsWide
            | Self::LineFs
            | Self::Stucki
            | Self::Burkes
            | Self::Sierra2
            | Self::SierraLite
            | Self::StevensonArce
            | Self::ShiauFan1
            | Self::ShiauFan2
            | Self::Ostromoukhov
            | Self::ZhouFang
            | Self::Riemersma
            | Self::FloydSteinbergOklab
            | Self::Gilbert
            | Self::Dbs
            | Self::StructureFs
            | Self::ContrastFs => DitherFamily::Diffusion,
            Self::BlueNoise
            | Self::Ign
            | Self::R2
            | Self::WhiteNoise
            | Self::VoidCluster
            | Self::ClusterNoise
            | Self::CranleyBayer
            | Self::IgnTri
            | Self::R2Tri => DitherFamily::Noise,
            Self::Crosshatch
            | Self::Radial
            | Self::ValueNoise
            | Self::Quasicrystal
            | Self::Truchet => DitherFamily::Effects,
            Self::Yliluoma1
            | Self::Yliluoma2
            | Self::Yliluoma3
            | Self::Knoll
            | Self::Luma
            | Self::Pair
            | Self::Yliluoma
            | Self::OptChecker
            | Self::OptLine
            | Self::OptLineChecker
            | Self::OptVline
            | Self::OptVlineChecker
            | Self::TriTone => DitherFamily::PaletteAware,
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
            | Self::Custom
            | Self::Bayer3
            | Self::Bayer5
            | Self::Bayer6
            | Self::Bayer7
            | Self::V4x2
            | Self::Bayer4x2
            | Self::Bayer2x4
            | Self::LineVChecker
            | Self::Fractal16
            | Self::AsepriteOld
            | Self::Libcaca3
            | Self::Libcaca6
            | Self::Pegasus => DitherFamily::Ordered,
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

/// How an ordered, noise or effect pixel turns its threshold into a colour (#156).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OrderedSemantics {
    /// png2c64's `apply_ordered_matrix`: the nearest and second-nearest colours of the
    /// cell's set (metric distances `dA ≤ dB`), `t = √dA / (√dA + √dB)`, and the second
    /// colour where `threshold < t × strength`. The pattern spans the whole interval
    /// between the two colours.
    #[default]
    Png2c64,
    /// What E5 shipped: the positional pick with thresholds pulled towards ½ by
    /// strength, and #142's mix levels (`mix_levels`, linear-light ratio, best
    /// partner). Kept so earlier goldens can be reproduced.
    Legacy,
}

/// How the cell-set search scores a candidate set (`docs/reference/dither-methods.md`
/// §6, #120). The dither method then places the pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CellMetric {
    /// Sum of squared metric distances to each pixel's nearest set colour.
    #[default]
    Mse,
    /// The same after a small blur of both the target and the cell rendered with the
    /// set (Pappas–Neuhoff): the eye at viewing distance.
    Blur,
    /// Squared distances plus structural dissimilarity (Wang et al.'s SSIM on
    /// lightness) over the cell, png2c64's hybrid.
    Ssim,
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
    /// `error_clamp` is not a number in `0..=1`.
    ErrorClamp(f32),
}

impl fmt::Display for RecipeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Strength(s) => write!(f, "dither strength must be in 0–1, got {s}"),
            Self::EmptyAllowedSet => f.write_str("a recipe must allow at least one colour"),
            Self::MaxLumaDiff(n) => write!(f, "max_luma_diff must be 0–8 luma levels, got {n}"),
            Self::Adaptive(a) => write!(f, "adaptive must be in 0–1, got {a}"),
            Self::ErrorClamp(c) => write!(f, "error_clamp must be in 0–1, got {c}"),
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
    #[serde(default)]
    cell_metric: CellMetric,
    #[serde(default = "default_error_clamp")]
    error_clamp: f32,
    #[serde(default)]
    ordered_semantics: OrderedSemantics,
    #[serde(default = "yes")]
    chroma_guard: bool,
}

const fn default_error_clamp() -> f32 {
    DEFAULT_ERROR_CLAMP
}

/// The default `error_clamp`: png2c64's, 0.1 per metric channel.
pub const DEFAULT_ERROR_CLAMP: f32 = 0.1;

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
    cell_metric: CellMetric,
    error_clamp: f32,
    ordered_semantics: OrderedSemantics,
    chroma_guard: bool,
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
        .map(|r| r.with_smooth(d.smooth).with_cell_metric(d.cell_metric))?
        .with_error_clamp(d.error_clamp)
        .map(|r| {
            r.with_ordered_semantics(d.ordered_semantics)
                .with_chroma_guard(d.chroma_guard)
        })
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
            cell_metric: CellMetric::Mse,
            error_clamp: DEFAULT_ERROR_CLAMP,
            ordered_semantics: OrderedSemantics::Png2c64,
            chroma_guard: true,
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
            cell_metric: CellMetric::Mse,
            error_clamp: DEFAULT_ERROR_CLAMP,
            ordered_semantics: OrderedSemantics::Png2c64,
            chroma_guard: true,
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

    /// This recipe with the cell-set search scored by `metric` (see [`CellMetric`]).
    pub const fn with_cell_metric(mut self, metric: CellMetric) -> Self {
        self.cell_metric = metric;
        self
    }

    /// This recipe with `error_clamp` set (diffusion; #150): the error a pixel has
    /// received is clamped to `±error_clamp` in each channel of its metric, as png2c64
    /// does (its default, 0.1, is ours). Without it an out-of-gamut colour, which no
    /// palette colour reaches, accumulates error row after row and smears. 0 is off.
    pub fn with_error_clamp(mut self, clamp: f32) -> Result<Self, RecipeError> {
        if !(0.0..=1.0).contains(&clamp) {
            return Err(RecipeError::ErrorClamp(clamp));
        }
        self.error_clamp = clamp;
        Ok(self)
    }

    /// This recipe with ordered pixels turned into colours by `semantics` (#156).
    pub const fn with_ordered_semantics(mut self, semantics: OrderedSemantics) -> Self {
        self.ordered_semantics = semantics;
        self
    }

    /// How ordered pixels pick colours (see [`OrderedSemantics`]).
    pub const fn ordered_semantics(&self) -> OrderedSemantics {
        self.ordered_semantics
    }

    /// This recipe with the chroma guard on or off (#156): with it, an ordered pixel
    /// never mixes in a second colour whose mean OKLab chroma with its first exceeds
    /// the pixel's own chroma by more than `CHROMA_MARGIN`, so neutral areas dither
    /// with greys only. On by default.
    pub const fn with_chroma_guard(mut self, guard: bool) -> Self {
        self.chroma_guard = guard;
        self
    }

    /// Whether the chroma guard is on (see [`Self::with_chroma_guard`]).
    pub const fn chroma_guard(&self) -> bool {
        self.chroma_guard
    }

    /// The diffused-error clamp (see [`Self::with_error_clamp`]).
    pub const fn error_clamp(&self) -> f32 {
        self.error_clamp
    }

    /// The cell-set search's score (see [`CellMetric`]).
    pub const fn cell_metric(&self) -> CellMetric {
        self.cell_metric
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

    /// This recipe with `method` in place of its own, every other setting kept (the
    /// method picker's thumbnails and live preview, UX §6a).
    pub const fn with_dither_method(mut self, method: Method) -> Self {
        self.method = method;
        self
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
            assert!(
                text.starts_with(&format!("kind = \"{}\"\n", k.id())),
                "{k:?}: {text}"
            );
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
            ("false-fs", DitherKind::FalseFs),
            ("fs-wide", DitherKind::FsWide),
            ("sierra-lite", DitherKind::SierraLite),
            ("stevenson-arce", DitherKind::StevensonArce),
            ("shiau-fan1", DitherKind::ShiauFan1),
            ("zhou-fang", DitherKind::ZhouFang),
            ("riemersma", DitherKind::Riemersma),
            ("yliluoma2", DitherKind::Yliluoma2),
            ("knoll", DitherKind::Knoll),
            ("luma", DitherKind::Luma),
            ("pair", DitherKind::Pair),
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
        assert_eq!(d.error_clamp(), DEFAULT_ERROR_CLAMP);
        assert_eq!(d.with_error_clamp(1.5), Err(RecipeError::ErrorClamp(1.5)));
    }

    #[test]
    fn mix_levels_and_smooth_parse_and_validate() {
        let r: Recipe = toml::from_str(
            "strength = 0.7\nmix_levels = 8\nsmooth = true\n[method]\nkind = \"bayer2\"\n",
        )
        .unwrap();
        assert_eq!((r.mix_levels(), r.smooth()), (8, true));
        assert_eq!(r.cell_metric(), CellMetric::Mse);
        let b: Recipe =
            toml::from_str("strength = 0.7\ncell_metric = \"ssim\"\n[method]\nkind = \"bayer2\"\n")
                .unwrap();
        assert_eq!(b.cell_metric(), CellMetric::Ssim);
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
