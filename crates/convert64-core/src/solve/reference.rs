//! The reference path of [`super::Solver`] since ADR-013 (#261): png2amiga's C64 path
//! ([`super::png2amiga_ref`]) on the compositor's frame, the default for the bitmap
//! modes.
//!
//! The frame's colours go through png2amiga's preprocessing at its defaults (gamma 1:
//! the layers' adjustments already applied any gamma) unless the compositor's front end
//! prepared them already: every pixel is converted to OKLab and back and clamped, as
//! png2amiga does. Each pixel's recipe (attribute map) gives its png2amiga method,
//! strength, error clamp and pattern anchor; with one recipe for the whole frame the
//! result is png2amiga's. A frame the reference cannot solve ([`unsupported`] says why)
//! goes to the png2c64 port if that can solve it, else to the native solver.
//!
//! **Hooks**, each inactive by default so the output stays png2amiga's: pins,
//! allow/deny and the recipes' allowed colours constrain a cell's colours
//! ([`CellRule`]); a fixed background (png2amiga always uses colour 0); locked cells,
//! and on the interactive fast path every clean cell, are copied through with the
//! previous background; per-pixel recipes from the attribute map.
//!
//! **Refine** (`SolveOptions::refine`, off by default): png2amiga's `--cell-refine`
//! after the solve, kept only if SSIMULACRA2 improves.

use super::png2amiga_ref::c64::{self, Hooks, Metric as RefMetric};
use super::png2amiga_ref::dither::{Method as RefMethod, PxSettings};
use super::png2amiga_ref::{Cell, Image, Mode as RefMode, colour, preprocess, refine};
use super::reference_png2c64::{assemble, previous_cell, rules};
use super::solver::{Background, Solution, SolveMode, SolveOptions, SolverKind};
use super::steering::Steering;
use crate::composite::{AttrSource, TargetFrame};
use crate::model::CellPos;
use crate::layer::{CellMetric, DitherKind, FitMode, Metric, Orientation, PatternPhase, Recipe};
use crate::mode::Mode;
use crate::palette::Palette;

/// png2amiga's method for one of our dither kinds: every method of png2amiga's C64 path
/// has a kind (`docs/reference/dither-methods.md`); `None` for our kinds png2amiga does
/// not have. Ours that share an id with png2amiga's (`riemersma`, `zhou-fang`,
/// `yliluoma1/2`, `knoll`) take png2amiga's algorithm on this path.
pub const fn ref_method(kind: DitherKind) -> Option<RefMethod> {
    use DitherKind as K;
    use RefMethod as M;
    Some(match kind {
        K::None => M::None,
        K::Bayer2 | K::Bayer2x2Mc => M::Bayer2x2,
        K::Bayer4 => M::Bayer4x4,
        K::Bayer8 => M::Bayer8x8,
        K::Bayer3 => M::Bayer3x3,
        K::Bayer5 => M::Bayer5x5,
        K::Bayer6 => M::Bayer6x6,
        K::Bayer7 => M::Bayer7x7,
        K::Checker => M::Checker,
        K::H2x4 => M::H2x4,
        K::Clustered4 => M::ClusteredDot,
        K::LineH2 => M::Line2,
        K::LineChecker => M::LineChecker,
        K::LineH4 => M::Line4,
        K::LineH8 => M::Line8,
        K::V4x2 => M::V4x2,
        K::Bayer4x2 => M::Bayer4x2,
        K::Bayer2x4 => M::Bayer2x4,
        K::LineV2 => M::Vline2,
        K::LineVChecker => M::VlineChecker,
        K::LineV4 => M::Vline4,
        K::LineV8 => M::Vline8,
        K::Halftone8 => M::Halftone8x8,
        K::Diagonal8 => M::Diagonal8x8,
        K::Spiral5 => M::Spiral5x5,
        K::Hex8 => M::Hex8x8,
        K::Hex5 => M::Hex5x5,
        K::BlueNoise => M::BlueNoise,
        K::VoidCluster => M::VoidCluster,
        K::ClusterNoise => M::ClusterNoise,
        K::Fractal16 => M::Fractal16,
        K::FloydSteinberg => M::FloydSteinberg,
        K::FloydSteinbergOklab => M::FloydSteinbergOklab,
        K::Atkinson => M::Atkinson,
        K::SierraLite => M::SierraLite,
        K::Stucki => M::Stucki,
        K::JarvisJudiceNinke => M::Jarvis,
        K::Dbs => M::Dbs,
        K::Gilbert => M::Gilbert,
        K::Riemersma => M::Riemersma,
        K::StructureFs => M::StructureFs,
        K::ContrastFs => M::ContrastFs,
        K::ZhouFang => M::Zhoufang,
        K::Yliluoma => M::Yliluoma,
        K::Yliluoma2 => M::Yliluoma2,
        K::OptChecker => M::OptChecker,
        K::Knoll => M::Knoll,
        K::TriTone => M::TriTone,
        K::Yliluoma1 => M::Yliluoma1,
        K::OptLine => M::OptLine,
        K::OptLineChecker => M::OptLineChecker,
        K::OptVline => M::OptVline,
        K::OptVlineChecker => M::OptVlineChecker,
        K::AsepriteOld => M::AsepriteOld,
        K::Libcaca3 => M::Libcaca3,
        K::Libcaca6 => M::Libcaca6,
        K::Pegasus => M::Pegasus,
        K::CranleyBayer => M::CranleyBayer,
        K::Quasicrystal => M::Quasicrystal,
        K::Truchet => M::Truchet,
        K::Ign => M::Ign,
        K::IgnTri => M::IgnTri,
        K::WhiteNoise => M::WhiteNoise,
        K::R2 => M::R2,
        K::R2Tri => M::R2Tri,
        K::Crosshatch => M::Crosshatch,
        K::Radial => M::Radial,
        K::ValueNoise => M::ValueNoise,
        _ => return None,
    })
}

/// The kind png2amiga's method is, as a recipe spells it.
pub fn kind_of(method: RefMethod) -> DitherKind {
    DitherKind::ALL
        .into_iter()
        .find(|&k| ref_method(k) == Some(method) && k != DitherKind::Bayer2x2Mc)
        .expect("every png2amiga method has a kind")
}

/// png2amiga's mode for a target mode.
const fn ref_mode(mode: Mode) -> Option<RefMode> {
    match mode {
        Mode::MulticolorBitmap => Some(RefMode::Multicolor),
        Mode::HiresBitmap => Some(RefMode::Hires),
        _ => None,
    }
}

/// png2amiga's cell metric for ours.
const fn ref_metric(m: CellMetric) -> Option<RefMetric> {
    match m {
        CellMetric::Mse => Some(RefMetric::Mse),
        CellMetric::Blur => Some(RefMetric::Blur),
        CellMetric::Ssim => None,
    }
}

/// Why a recipe's pixels need another solver, if they do.
fn recipe_unsupported(r: &Recipe) -> Option<String> {
    let m = r.method();
    if ref_method(m.kind).is_none() {
        return Some(format!("method {:?} is not one of png2amiga's", m.kind));
    }
    if m.orientation != Orientation::Auto {
        return Some(format!("orientation {:?}", m.orientation));
    }
    if r.metric() != Metric::Oklab {
        return Some(format!("metric {:?}", r.metric()));
    }
    if ref_metric(r.cell_metric()).is_none() {
        return Some(format!("cell metric {:?}", r.cell_metric()));
    }
    if r.fit_mode() != FitMode::Adaptive {
        return Some("fit mode Force".to_owned());
    }
    if r.adaptive() != 0.0 {
        return Some("`adaptive` (png2c64's; png2amiga has none)".to_owned());
    }
    let d = Recipe::default();
    let knobs = [
        ("cell_clamp", r.cell_clamp() != d.cell_clamp()),
        ("max_luma_diff", r.max_luma_diff() != d.max_luma_diff()),
        ("mix_levels", r.mix_levels() != d.mix_levels()),
        ("smooth", r.smooth() != d.smooth()),
        (
            "ordered_semantics",
            r.ordered_semantics() != d.ordered_semantics(),
        ),
        ("chroma_guard", r.chroma_guard() != d.chroma_guard()),
    ];
    knobs
        .iter()
        .find(|(_, changed)| *changed)
        .map(|(knob, _)| format!("recipe knob `{knob}` (native solver only)"))
}

/// Why `frame` cannot be solved on the reference path, or `None` if it can.
pub(crate) fn unsupported(frame: &TargetFrame, opts: &SolveOptions) -> Option<String> {
    if opts.luma_pair_bias != 0.0 {
        return Some("solve option `luma_pair_bias` (native solver only)".to_owned());
    }
    if opts.hysteresis.is_some() {
        return Some("solve option `hysteresis` (native solver only)".to_owned());
    }
    if !opts.divergences.none() {
        return Some("a png2c64 divergence (`solve.divergences`)".to_owned());
    }
    if ref_mode(frame.grid().target()).is_none() {
        return Some(format!("mode {:?}", frame.grid().target()));
    }
    let attrs = frame.attributes();
    let default_recipe = Recipe::default();
    let mut seen: Vec<AttrSource> = Vec::new();
    for a in attrs.pixels() {
        if seen.contains(&a.source) {
            continue;
        }
        seen.push(a.source);
        let r = attrs.recipe(a.source).unwrap_or(&default_recipe);
        if let Some(why) = recipe_unsupported(r) {
            return Some(why);
        }
    }
    None
}

/// Solves `frame` on the reference path, steered by `steering`; locked cells (and, on the
/// interactive fast path, every clean cell) are copied from `previous`. Call only when
/// [`unsupported`] is `None`.
pub(crate) fn solve(
    frame: &TargetFrame,
    palette: &Palette,
    steering: &Steering,
    previous: Option<&Solution>,
    opts: &SolveOptions,
) -> Solution {
    let grid = frame.grid();
    let target = grid.target();
    let mode = ref_mode(target).expect("a bitmap mode");
    let (w, h) = mode.screen();
    let mut image = Image::new(w, h, frame.colour().to_vec());
    if !frame.png2c64_prepared() {
        preprocess::apply(&mut image, &preprocess::Settings::default());
    }
    let attrs = frame.attributes();
    let default_recipe = Recipe::default();
    let (cols, rows) = mode.cells();

    let frozen: Vec<bool> = match previous {
        None => vec![false; cols * rows],
        Some(_) => (0..cols * rows)
            .map(|i| {
                let cell = CellPos::new((i % cols) as u16, (i / cols) as u16);
                steering.locks().contains(cell)
                    || matches!(opts.mode, SolveMode::Interactive { dirty } if !dirty.contains(cell))
            })
            .collect(),
    };
    // png2amiga's metric is one per picture: the first recipe's (all of them, with one
    // recipe for the frame).
    let mut metric = None;
    let px: Vec<PxSettings> = attrs
        .pixels()
        .iter()
        .map(|a| {
            let r = attrs.recipe(a.source).unwrap_or(&default_recipe);
            metric.get_or_insert(ref_metric(r.cell_metric()).expect("checked by `unsupported`"));
            let origin = match (r.phase(), a.source) {
                (PatternPhase::Layer, AttrSource::Layer(l)) => opts
                    .layer_origins
                    .get(usize::from(l))
                    .copied()
                    .unwrap_or((0, 0)),
                _ => (0, 0),
            };
            PxSettings {
                method: ref_method(r.method().kind).expect("checked by `unsupported`"),
                strength: a.strength,
                // Our recipes' 0 means "no clamp"; png2amiga would clamp everything to 0.
                error_clamp: if r.error_clamp() > 0.0 {
                    r.error_clamp()
                } else {
                    f32::INFINITY
                },
                origin,
            }
        })
        .collect();
    let fixed: Vec<Option<Cell>> = match previous {
        Some(prev) if frozen.contains(&true) => {
            let out = prev.image.to_indexed();
            (0..cols * rows)
                .map(|i| frozen[i].then(|| previous_cell(prev, out.pixels(), mode, i)))
                .collect()
        }
        _ => Vec::new(),
    };
    let keep_bg = previous
        .and_then(|p| p.background)
        .filter(|_| !fixed.is_empty() || matches!(opts.mode, SolveMode::Interactive { .. }));
    let hooks = Hooks {
        background: match (keep_bg, opts.background) {
            _ if mode != RefMode::Multicolor => None,
            (Some(b), _) | (None, Background::Fixed(b)) => Some(b.get()),
            (None, Background::Auto) => None,
        },
        rules: rules(frame, steering, mode),
        fixed,
    };
    let pal = colour::palette_of(palette);
    let metric = metric.unwrap_or_default();
    let mut screen = c64::encode(&image, &pal, mode, &px, metric, true, &hooks);
    // Refine changes every register, so it runs on a full solve with no fixed cells.
    if opts.refine && hooks.fixed.is_empty() && matches!(opts.mode, SolveMode::Full) {
        refine::refine(&image, &mut screen, &pal, &px[0], true);
    }
    assemble(frame, &image, &pal, &screen, SolverKind::Reference)
}
