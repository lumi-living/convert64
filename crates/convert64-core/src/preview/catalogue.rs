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

    /// Whether png2c64 has this method (the search finds it by `png2c64`).
    pub const fn in_png2c64(&self) -> bool {
        crate::solve::reference::ref_method(self.kind).is_some()
    }

    /// Whether png2amiga's C64 path has this method: the png2amiga solver solves it
    /// (#261; the search finds it by `png2amiga`).
    pub const fn in_png2amiga(&self) -> bool {
        crate::solve::reference_png2amiga::ref_method(self.kind).is_some()
    }

    /// The tooltip's CLI line: `--dither <id>`, and for one of png2amiga's own methods
    /// whose png2amiga name differs, that name (an accepted alias): `--dither bayer3
    /// (png2amiga: bayer3x3)`.
    pub fn cli(&self) -> String {
        match crate::solve::reference_png2amiga::ref_method(self.kind)
            .map(|m| m.name())
            .filter(|n| self.kind.is_png2amiga_only() && *n != self.id())
        {
            Some(theirs) => format!("--dither {} (png2amiga: {theirs})", self.id()),
            None => format!("--dither {}", self.id()),
        }
    }

    /// The card's tooltip: the description, and for one of png2amiga's own methods
    /// (#275) where it comes from and how the solvers run it.
    pub fn tooltip(&self) -> String {
        if self.kind.is_png2amiga_only() {
            format!(
                "{}\nFrom png2amiga, on png2c64's pipeline; the png2amiga solver runs \
                 png2amiga's own.",
                self.description
            )
        } else {
            self.description.to_owned()
        }
    }

    /// Whether every word of `query` (case-insensitive) is part of the name, the id, an
    /// alias or the family name, or is `png2c64` for one of png2c64's methods. An empty
    /// query matches everything; `png2amiga` likewise for png2amiga's methods.
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
        if self.in_png2amiga() {
            hay.push_str(" png2amiga");
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
pub static CATALOGUE: [MethodInfo; 90] = [
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
        K::Bayer3,
        "Bayer 3×3",
        &["bayer3x3", "png2amiga"],
        "png2amiga's dispersed-dot 3×3: ten levels on a non-power-of-two grid.",
    ),
    m(
        K::Bayer5,
        "Bayer 5×5",
        &["bayer5x5", "png2amiga"],
        "png2amiga's dispersed-dot 5×5.",
    ),
    m(
        K::Bayer6,
        "Bayer 6×6",
        &["bayer6x6", "png2amiga"],
        "png2amiga's dispersed-dot 6×6.",
    ),
    m(
        K::Bayer7,
        "Bayer 7×7",
        &["bayer7x7", "png2amiga"],
        "png2amiga's dispersed-dot 7×7.",
    ),
    m(
        K::V4x2,
        "V 4×2",
        &["v4x2", "tall pixels"],
        "A 4×2 vertical-bias matrix, for tall pixels.",
    ),
    m(
        K::Bayer4x2,
        "Bayer 4×2",
        &["bayer4x2"],
        "Bayer ordering 4 wide, 2 tall.",
    ),
    m(
        K::Bayer2x4,
        "Bayer 2×4",
        &["bayer2x4"],
        "Bayer ordering 2 wide, 4 tall.",
    ),
    m(
        K::LineVChecker,
        "Vertical line checker",
        &["vline-checker", "columns"],
        "Column-biased 2×2: vertical lines with a row shift.",
    ),
    m(
        K::Fractal16,
        "Fractal 16×16",
        &["obra dinn", "self-nested"],
        "Self-nested Bayer recursion: reads at several zoom levels (Obra Dinn).",
    ),
    m(
        K::AsepriteOld,
        "Aseprite old 4×4",
        &["aseprite"],
        "Aseprite's hand-tuned 4×4, checker-biased.",
    ),
    m(
        K::Libcaca3,
        "libcaca 3×3",
        &["libcaca"],
        "libcaca's hand-tuned 3×3 dispersed dot.",
    ),
    m(
        K::Libcaca6,
        "libcaca 6×6",
        &["libcaca"],
        "libcaca's hand-tuned 6×6 dispersed dot.",
    ),
    m(
        K::Pegasus,
        "Pegasus 8×8",
        &["rexpaint", "mosaic"],
        "The Pegasus/REXPaint 8×8 mosaic, centre-biased.",
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
    m(
        K::VoidCluster,
        "Void and cluster",
        &["void-cluster", "blue noise", "ulichney"],
        "png2amiga's 64×64 void-and-cluster blue noise: per-rank optimal.",
    ),
    m(
        K::ClusterNoise,
        "Cluster noise",
        &["cluster-noise", "film grain"],
        "Blue noise with wider clusters: a coarser, film-grain texture.",
    ),
    m(
        K::CranleyBayer,
        "Cranley–Patterson Bayer",
        &["cranley-bayer", "random offset"],
        "Bayer 8×8 with a hashed offset per tile: a patchwork of phases.",
    ),
    m(
        K::IgnTri,
        "IGN triangular",
        &["ign-tri", "triangle"],
        "Interleaved gradient noise remapped to a triangle: less banding at the ends.",
    ),
    m(
        K::R2Tri,
        "R2 triangular",
        &["r2-tri", "triangle"],
        "R2 remapped to a triangle (Wronski): even darks and lights.",
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
    m(
        K::FloydSteinbergOklab,
        "Floyd–Steinberg (OKLab)",
        &["floyd-steinberg-oklab", "fs oklab"],
        "Floyd–Steinberg with the residual carried in OKLab: often better on photos, worse on logos.",
    ),
    m(
        K::StructureFs,
        "Structure-aware FS",
        &["structure-fs", "edges"],
        "Floyd–Steinberg with a Laplacian bias on lightness: keeps edges.",
    ),
    m(
        K::ContrastFs,
        "Contrast-aware FS",
        &["contrast-fs", "mould"],
        "Floyd–Steinberg with a local-contrast bias on lightness.",
    ),
    m(
        K::Gilbert,
        "Gilbert curve",
        &["gilbert", "png2amiga"],
        "png2amiga's Gilbert-curve diffusion. On C64, png2amiga runs this as a nearest pick, so it looks like None.",
    ),
    m(
        K::Dbs,
        "Direct binary search",
        &["dbs", "png2amiga"],
        "png2amiga's direct binary search. On C64, png2amiga runs this as a nearest pick, so it looks like None.",
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
    m(
        K::Yliluoma,
        "Yliluoma (png2amiga)",
        &["yliluoma", "mixing plan"],
        "png2amiga's Yliluoma: a 64-entry mixing plan on Bayer 8.",
    ),
    m(
        K::OptChecker,
        "Optimal checker",
        &["opt-checker", "pair"],
        "The nearest colour and its best partner, as a 2×2 checker.",
    ),
    m(
        K::OptLine,
        "Optimal line",
        &["opt-line", "scanline"],
        "The optimal pair alternated by row: CRT-scanline friendly.",
    ),
    m(
        K::OptLineChecker,
        "Optimal line checker",
        &["opt-line-checker"],
        "A 4-colour plan on the line-checker 2×2.",
    ),
    m(
        K::OptVline,
        "Optimal vertical line",
        &["opt-vline", "columns"],
        "The optimal pair alternated by column.",
    ),
    m(
        K::OptVlineChecker,
        "Optimal vertical line checker",
        &["opt-vline-checker"],
        "A 4-colour plan on the column-checker 2×2.",
    ),
    m(
        K::TriTone,
        "Tri-tone",
        &["tri-tone", "yliluoma"],
        "Yliluoma's tri-tone: one colour at 50 %, two at 25 %, on Bayer 2×2.",
    ),
    // Effects.
    m(
        K::Quasicrystal,
        "Quasicrystal",
        &["cosines", "shimmer"],
        "A sum of five cosine waves: an aperiodic shimmer.",
    ),
    m(
        K::Truchet,
        "Truchet",
        &["tiles", "woven"],
        "Quarter-arc Truchet tiles: a woven texture.",
    ),
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
        assert_eq!(palette_aware.len(), 13);
        assert!(search("png2amiga").contains(&DitherKind::OptChecker));
        assert!(!search("png2amiga").contains(&DitherKind::Ostromoukhov));
    }

    /// #275: png2amiga's 32 own methods are catalogue methods on png2c64's pipeline and
    /// say so on their card; every other card is its description. `gilbert` and `dbs`
    /// say that png2amiga runs them as a nearest pick on C64.
    #[test]
    fn png2amiga_s_own_methods_say_so() {
        let own: Vec<_> = CATALOGUE
            .iter()
            .filter(|m| m.kind.is_png2amiga_only())
            .collect();
        assert_eq!(own.len(), 32);
        for m in &CATALOGUE {
            assert_eq!(
                m.tooltip().contains("on png2c64's pipeline"),
                m.kind.is_png2amiga_only(),
                "{:?}",
                m.kind
            );
            assert!(!m.tooltip().contains("experimental"), "{:?}", m.kind);
            if m.kind.is_png2amiga_only() {
                assert!(m.in_png2amiga() && !m.in_png2c64(), "{:?}", m.kind);
            }
        }
        for k in [DitherKind::Gilbert, DitherKind::Dbs] {
            assert!(
                info(k)
                    .description
                    .contains("On C64, png2amiga runs this as a nearest pick"),
                "{k:?}"
            );
        }
    }

    /// #275: each of png2amiga's 32 methods takes its CLI id and png2amiga's own name
    /// (an alias where they differ), and the picker's search finds it by both.
    #[test]
    fn png2amiga_s_names_reach_its_methods() {
        use serde::Deserialize;
        use serde::de::{IntoDeserializer, value::Error};
        for m in CATALOGUE.iter().filter(|m| m.kind.is_png2amiga_only()) {
            let theirs = crate::solve::reference_png2amiga::ref_method(m.kind)
                .expect("png2amiga's")
                .name();
            for name in [m.id(), theirs] {
                let parsed = DitherKind::deserialize(name.into_deserializer())
                    .map_err(|e: Error| e.to_string());
                assert_eq!(parsed, Ok(m.kind), "{name}");
                assert!(search(name).contains(&m.kind), "{name}");
            }
            assert_eq!(
                m.cli().contains("png2amiga:"),
                theirs != m.id(),
                "{}",
                m.cli()
            );
        }
    }

    /// `docs/reference/dither-methods.md`'s catalogue lists every method once, under its
    /// family's heading, in picker order, with the card's name and description.
    #[test]
    fn the_reference_doc_lists_all_90_in_picker_order() {
        let doc = include_str!("../../../../docs/reference/dither-methods.md");
        let section = &doc[doc.find("## The catalogue").expect("the catalogue section")..];
        let section = &section[..section.find("\n## 1.").expect("§1 follows")];
        let mut family = "";
        let mut rows = Vec::new();
        for line in section.lines() {
            if let Some(h) = line.strip_prefix("### ") {
                family = h.split(" (").next().unwrap_or(h);
            } else if let Some(row) = line.strip_prefix("| `") {
                let cols: Vec<&str> = row.split(" | ").collect();
                rows.push((family, cols[0].trim_end_matches('`'), cols[2], cols[3]));
            }
        }
        assert_eq!(rows.len(), CATALOGUE.len());
        for (m, (family, id, name, description)) in CATALOGUE.iter().zip(rows) {
            assert_eq!(id, m.id());
            assert_eq!(family, family_name(m.family()), "{id}");
            assert_eq!(name, m.name, "{id}");
            assert!(description.starts_with(m.description), "{id}");
        }
    }

    #[test]
    fn step_walks_picker_order_and_clamps() {
        assert_eq!(step(DitherKind::None, -1), DitherKind::None);
        assert_eq!(step(DitherKind::None, 1), DitherKind::Bayer2);
        assert_eq!(step(DitherKind::ValueNoise, 1), DitherKind::ValueNoise);
        assert_eq!(step(DitherKind::Custom, 1), DitherKind::BlueNoise);
    }

    #[test]
    fn the_cli_line_names_png2amiga_s_spelling_where_it_differs() {
        assert_eq!(
            info(DitherKind::Bayer3).cli(),
            "--dither bayer3 (png2amiga: bayer3x3)"
        );
        assert_eq!(
            info(DitherKind::LineVChecker).cli(),
            "--dither line-v-checker (png2amiga: vline-checker)"
        );
        assert_eq!(info(DitherKind::TriTone).cli(), "--dither tri-tone");
        assert_eq!(info(DitherKind::Bayer4).cli(), "--dither bayer4");
    }

    #[test]
    fn labels_read_name_family_and_state() {
        let a = info(DitherKind::Atkinson);
        assert_eq!(a.label(true), "Atkinson, error diffusion, selected");
        assert_eq!(a.label(false), "Atkinson, error diffusion");
    }
}
