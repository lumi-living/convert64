//! The dither method catalogue (UX §6a): every [`DitherKind`] with its display name,
//! search aliases, one-line description and whether its full solve is slow, in the
//! picker's order (families in UX order, then the order of
//! `docs/reference/dither-methods.md`).
//!
//! A new method is one more [`MethodInfo`] in [`CATALOGUE`], in its family's run; the
//! tests check that every kind appears exactly once and that families stay together in
//! [`FAMILIES`] order.

use crate::layer::{DitherFamily, DitherKind};

/// One method as the picker shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MethodInfo {
    /// The method.
    pub kind: DitherKind,
    /// Its display name, e.g. `Floyd–Steinberg`.
    pub name: &'static str,
    /// Other names the search finds it by (png2c64's ids, common abbreviations).
    pub aliases: &'static [&'static str],
    /// One line on what it looks like and when to use it (the card's tooltip).
    pub description: &'static str,
    /// Its full solve of a whole picture exceeds 200 ms on one thread (the browser
    /// build; the card's *slow* badge). Measured by the `full_solve_timing` test: only
    /// Yliluoma 2 in multicolor (about 250 ms; everything else under 90 ms).
    pub slow: bool,
}

impl MethodInfo {
    /// The CLI and recipe id (`--dither <id>`).
    pub const fn id(&self) -> &'static str {
        self.kind.id()
    }

    /// The family.
    pub const fn family(&self) -> DitherFamily {
        self.kind.family()
    }

    /// Whether png2c64 has this method (the reference solver solves it; the search
    /// finds it by `png2c64`).
    pub const fn in_png2c64(&self) -> bool {
        crate::solve::reference::ref_method(self.kind).is_some()
    }

    /// Whether every word of `query` (case-insensitive) is part of the name, the id, an
    /// alias or the family name, or is `png2c64` for one of png2c64's methods. An empty
    /// query matches everything.
    pub fn matches(&self, query: &str) -> bool {
        let mut hay = format!(
            "{} {} {}",
            self.name.to_lowercase(),
            self.id(),
            family_name(self.family()).to_lowercase()
        );
        for a in self.aliases {
            hay.push(' ');
            hay.push_str(&a.to_lowercase());
        }
        if self.in_png2c64() {
            hay.push_str(" png2c64");
        }
        // Names spell `×` and `–`; let the search type `x` and `-`.
        let hay = hay.replace('×', "x").replace('–', "-");
        query
            .to_lowercase()
            .replace('×', "x")
            .split_whitespace()
            .all(|w| hay.contains(w))
    }

    /// The accessibility label: "Atkinson, error diffusion, selected".
    pub fn label(&self, selected: bool) -> String {
        let family = family_name(self.family()).to_lowercase();
        if selected {
            format!("{}, {family}, selected", self.name)
        } else {
            format!("{}, {family}", self.name)
        }
    }
}

/// The families in the picker's order (UX §6a), with their headings.
pub const FAMILIES: [(DitherFamily, &str); 6] = [
    (DitherFamily::None, "None"),
    (DitherFamily::Ordered, "Ordered"),
    (DitherFamily::Noise, "Ordered noise"),
    (DitherFamily::Diffusion, "Error diffusion"),
    (DitherFamily::PaletteAware, "Palette-aware"),
    (DitherFamily::Effects, "Effects"),
];

/// A family's heading.
pub fn family_name(family: DitherFamily) -> &'static str {
    FAMILIES
        .iter()
        .find(|(f, _)| *f == family)
        .map_or("", |(_, n)| n)
}

const fn m(
    kind: DitherKind,
    name: &'static str,
    aliases: &'static [&'static str],
    description: &'static str,
) -> MethodInfo {
    MethodInfo {
        kind,
        name,
        aliases,
        description,
        slow: false,
    }
}

const fn slow(info: MethodInfo) -> MethodInfo {
    MethodInfo { slow: true, ..info }
}

use DitherKind as K;

/// Every method in picker order: families in [`FAMILIES`] order, each family's methods
/// together.
pub static CATALOGUE: [MethodInfo; 58] = [
    m(
        K::None,
        "None",
        &["nearest", "off"],
        "Nearest colour in the cell, no dither: flat areas, hard steps.",
    ),
    // Ordered.
    m(
        K::Bayer2,
        "Bayer 2×2",
        &["bayer2x2", "ordered"],
        "Ordered 2×2 matrix: five levels, a coarse regular grid.",
    ),
    m(
        K::Bayer4,
        "Bayer 4×4",
        &["bayer4x4", "ordered"],
        "Ordered 4×4 matrix: the classic crosshatch look.",
    ),
    m(
        K::Bayer8,
        "Bayer 8×8",
        &["bayer8x8", "ordered"],
        "Ordered 8×8 matrix: finer steps, the pattern still visible.",
    ),
    m(
        K::Bayer16,
        "Bayer 16×16",
        &["bayer16x16", "ordered"],
        "Ordered 16×16 matrix: smooth gradients over large areas.",
    ),
    m(
        K::Bayer2x2Mc,
        "Bayer 2×2 (2:1)",
        &["bayer2x2", "mc", "multicolor"],
        "png2c64's 2×2 Bayer for 2:1 pixels.",
    ),
    m(
        K::Checker,
        "Checker",
        &["checkerboard", "50%"],
        "50 % checkerboard, two levels: png2c64's default and the scene's workhorse.",
    ),
    m(
        K::H2x4,
        "H 2×4",
        &["h2x4", "bayer2x4"],
        "png2c64's 2×4 Bayer: eight levels, square on 2:1 pixels.",
    ),
    m(
        K::Clustered4,
        "Clustered 4×4",
        &["clustered", "clustered-dot", "halftone"],
        "Clustered-dot halftone shaped for 2:1 pixels: stable on a CRT.",
    ),
    m(
        K::Clustered8,
        "Clustered 8×8",
        &["clustered", "halftone"],
        "One round dot per 8×8 tile: newspaper halftone.",
    ),
    m(
        K::Halftone8,
        "Halftone 8×8",
        &["halftone8x8", "dot"],
        "png2c64's 45° round-dot halftone.",
    ),
    m(
        K::Dot,
        "Dot",
        &["arcpaint64", "round"],
        "ArcPaint64's 6×6 round dot.",
    ),
    m(
        K::Diamond,
        "Diamond",
        &["arcpaint64"],
        "ArcPaint64's 8×8 dot that grows as a diamond.",
    ),
    m(
        K::Star,
        "Star",
        &["arcpaint64"],
        "ArcPaint64's 8×8 dot that grows along its axes first.",
    ),
    m(
        K::LineH2,
        "Lines 2",
        &["line2", "horizontal", "line-h2"],
        "Horizontal lines, period 2: cheap on 2:1 pixels.",
    ),
    m(
        K::LineH4,
        "Lines 4",
        &["line4", "horizontal", "line-h4"],
        "Horizontal lines, period 4.",
    ),
    m(
        K::LineH8,
        "Lines 8",
        &["line8", "horizontal", "line-h8"],
        "Horizontal lines, period 8.",
    ),
    m(
        K::LineV2,
        "Vertical lines 2",
        &["vertical", "line-v2"],
        "Vertical lines, period 2: costly in multicolor, cheap in hi-res.",
    ),
    m(
        K::LineV4,
        "Vertical lines 4",
        &["vertical", "line-v4", "arcpaint64"],
        "Vertical lines, period 4 (ArcPaint64 Vertical 4).",
    ),
    m(
        K::LineV8,
        "Vertical lines 8",
        &["vertical", "line-v8", "arcpaint64"],
        "Vertical lines, period 8 (ArcPaint64 Vertical 8).",
    ),
    m(
        K::LineChecker,
        "Line checker",
        &["horizontal", "checker"],
        "Lines with a checker offset (ArcPaint64 Horizontal + Checker).",
    ),
    m(
        K::DiagDown,
        "Diagonal ↘",
        &["diagonal", "45"],
        "45° lines falling to the right.",
    ),
    m(
        K::DiagUp,
        "Diagonal ↗",
        &["diagonal", "45"],
        "45° lines rising to the right.",
    ),
    m(
        K::Diagonal8,
        "Diagonal 8×8",
        &["diagonal8x8", "halftone"],
        "png2c64's 8×8 diagonal clustered dot.",
    ),
    m(
        K::Spiral5,
        "Spiral 5×5",
        &["spiral5x5"],
        "png2c64's 5×5 spiral.",
    ),
    m(
        K::Hex5,
        "Hex 5×5",
        &["hex5x5", "hexagonal"],
        "png2c64's 5×5 slanted tiling.",
    ),
    m(
        K::Hex8,
        "Hex 8×8",
        &["hex8x8", "hexagonal"],
        "png2c64's 8×8 hexagonal tiling.",
    ),
    m(
        K::Custom,
        "Custom matrix",
        &["user", "matrix"],
        "Your own 8×8 or 16×16 matrix from a recipe file; Bayer 8×8 without one.",
    ),
    // Ordered noise.
    m(
        K::BlueNoise,
        "Blue noise",
        &["void-and-cluster", "noise"],
        "64×64 blue-noise tile: the best general-purpose ordered dither for photos.",
    ),
    m(
        K::Ign,
        "Interleaved gradient",
        &["ign", "noise", "jimenez"],
        "Interleaved gradient noise: cheap, no tile.",
    ),
    m(
        K::R2,
        "R2 sequence",
        &["r2-sequence", "noise", "low discrepancy"],
        "The R2 low-discrepancy sequence: even, no visible tile.",
    ),
    m(
        K::WhiteNoise,
        "White noise",
        &["random", "noise"],
        "Seeded random thresholds: grainy, for when it must look random.",
    ),
    // Error diffusion.
    m(
        K::FloydSteinberg,
        "Floyd–Steinberg",
        &["fs", "floyd", "diffusion"],
        "The classic 7/3/5/1 error diffusion.",
    ),
    m(
        K::FalseFs,
        "False Floyd–Steinberg",
        &["false-fs", "fs"],
        "3/3/2 diffusion: lighter and faster than Floyd–Steinberg.",
    ),
    m(
        K::FsWide,
        "Floyd–Steinberg wide",
        &["fs-wide", "fs", "mc"],
        "png2c64's Floyd–Steinberg for 2:1 pixels.",
    ),
    m(
        K::LineFs,
        "Line Floyd–Steinberg",
        &["line-fs", "fs"],
        "png2c64's line-biased diffusion: all error goes down.",
    ),
    m(
        K::JarvisJudiceNinke,
        "Jarvis–Judice–Ninke",
        &["jarvis", "jjn"],
        "Wide 12-tap diffusion: smooth, slightly soft.",
    ),
    m(
        K::Stucki,
        "Stucki",
        &[],
        "Jarvis-like 12-tap diffusion, a little sharper.",
    ),
    m(K::Burkes, "Burkes", &[], "Two-row Stucki: fast and clean."),
    m(
        K::Sierra,
        "Sierra",
        &["sierra3"],
        "Three-row Sierra diffusion.",
    ),
    m(
        K::Sierra2,
        "Sierra two-row",
        &["sierra2"],
        "Two-row Sierra diffusion.",
    ),
    m(
        K::SierraLite,
        "Sierra Lite",
        &["sierra-lite", "sierra"],
        "Three-tap Sierra: png2c64's `sierra`.",
    ),
    m(
        K::Atkinson,
        "Atkinson",
        &["mac", "apple"],
        "Diffuses 3/4 of the error: the Mac look, high contrast, good on the C64.",
    ),
    m(
        K::StevensonArce,
        "Stevenson–Arce",
        &["stevenson-arce", "hex"],
        "Hexagonal-grid weights: fewer directional artefacts.",
    ),
    m(
        K::ShiauFan1,
        "Shiau–Fan",
        &["shiau-fan1"],
        "Four taps: fewer worms than Floyd–Steinberg.",
    ),
    m(
        K::ShiauFan2,
        "Shiau–Fan 2",
        &["shiau-fan2"],
        "Five taps: fewer worms still.",
    ),
    m(
        K::Ostromoukhov,
        "Ostromoukhov",
        &["variable"],
        "Coefficients by input level: the best neutral diffusion.",
    ),
    m(
        K::ZhouFang,
        "Zhou–Fang",
        &["zhou-fang"],
        "Ostromoukhov plus threshold modulation: removes the remaining worms.",
    ),
    m(
        K::Riemersma,
        "Riemersma",
        &["hilbert"],
        "Error carried along a Hilbert curve: no directional artefacts.",
    ),
    // Palette-aware.
    m(
        K::Yliluoma1,
        "Yliluoma 1",
        &["yliluoma", "mix"],
        "The best two-colour mix per pixel and its ratio, placed by Bayer 8×8.",
    ),
    slow(m(
        K::Yliluoma2,
        "Yliluoma 2",
        &["yliluoma", "candidates"],
        "A 16-entry candidate list per pixel whose running average approaches it, indexed by Bayer 8×8.",
    )),
    m(
        K::Yliluoma3,
        "Yliluoma 3",
        &["yliluoma", "knoll", "gamma"],
        "Knoll-style candidate list in linear light.",
    ),
    m(
        K::Knoll,
        "Knoll pattern",
        &["photoshop", "pattern"],
        "Thomas Knoll's pattern dither (Photoshop's Pattern).",
    ),
    m(
        K::Luma,
        "Luma",
        &["lightness"],
        "Dithers lightness only, hue from the nearest colour: no colour speckle.",
    ),
    m(
        K::Pair,
        "Luma pair",
        &["pair", "pal blend", "weave"],
        "Weaves two near-luma colours into a third, as a PAL screen blends them.",
    ),
    // Effects.
    m(
        K::Crosshatch,
        "Crosshatch",
        &["pen", "ink"],
        "png2c64's overlaid angled lines: pen and ink.",
    ),
    m(
        K::Radial,
        "Radial",
        &["circles", "engraving"],
        "Concentric circles around the screen centre: engraving.",
    ),
    m(
        K::ValueNoise,
        "Value noise",
        &["organic", "noise"],
        "Smooth hash noise: organic clumps.",
    ),
];

/// The catalogue entry of `kind`.
///
/// # Panics
/// If `kind` has no entry (a test keeps the catalogue complete).
pub fn info(kind: DitherKind) -> &'static MethodInfo {
    CATALOGUE
        .iter()
        .find(|i| i.kind == kind)
        .expect("every dither kind is in the catalogue")
}

/// The methods matching `query`, in picker order.
pub fn search(query: &str) -> Vec<DitherKind> {
    CATALOGUE
        .iter()
        .filter(|i| i.matches(query))
        .map(|i| i.kind)
        .collect()
}

/// The method `steps` places after (negative: before) `kind` in picker order, clamped
/// to the first and last (the closed Method field's ▲▼ and `↑`/`↓`).
pub fn step(kind: DitherKind, steps: isize) -> DitherKind {
    let i = CATALOGUE.iter().position(|c| c.kind == kind).unwrap_or(0) as isize;
    let j = (i + steps).clamp(0, CATALOGUE.len() as isize - 1) as usize;
    CATALOGUE[j].kind
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_is_in_the_catalogue_once() {
        for k in DitherKind::ALL {
            assert_eq!(CATALOGUE.iter().filter(|i| i.kind == k).count(), 1, "{k:?}");
        }
        assert_eq!(CATALOGUE.len(), DitherKind::ALL.len());
    }

    #[test]
    fn families_come_in_ux_order_each_together() {
        let order: Vec<usize> = CATALOGUE
            .iter()
            .map(|i| FAMILIES.iter().position(|(f, _)| *f == i.family()).unwrap())
            .collect();
        assert!(order.windows(2).all(|w| w[0] <= w[1]), "{order:?}");
    }

    #[test]
    fn search_finds_names_ids_and_aliases() {
        assert_eq!(search("atkin"), vec![DitherKind::Atkinson]);
        assert!(search("fs").contains(&DitherKind::FloydSteinberg));
        assert!(search("floyd").contains(&DitherKind::FloydSteinberg));
        assert!(search("CHECKER").contains(&DitherKind::Checker));
        assert_eq!(search("png2c64 line2"), vec![DitherKind::LineH2]);
        assert_eq!(search("bayer 4x4"), vec![DitherKind::Bayer4]);
        assert_eq!(search("zhou-fang"), vec![DitherKind::ZhouFang]);
        assert_eq!(search("").len(), CATALOGUE.len());
        assert!(search("no such method").is_empty());
        // The family name finds the family.
        let palette_aware = search("palette-aware");
        assert_eq!(palette_aware.len(), 6);
    }

    #[test]
    fn step_walks_picker_order_and_clamps() {
        assert_eq!(step(DitherKind::None, -1), DitherKind::None);
        assert_eq!(step(DitherKind::None, 1), DitherKind::Bayer2);
        assert_eq!(step(DitherKind::ValueNoise, 1), DitherKind::ValueNoise);
        assert_eq!(step(DitherKind::Custom, 1), DitherKind::BlueNoise);
    }

    #[test]
    fn labels_read_name_family_and_state() {
        let a = info(DitherKind::Atkinson);
        assert_eq!(a.label(true), "Atkinson, error diffusion, selected");
        assert_eq!(a.label(false), "Atkinson, error diffusion");
    }
}
