//! The reference path of [`super::Solver`] (#181): png2c64's pipeline
//! ([`super::png2c64_ref`]) on the compositor's frame, the default for the bitmap modes.
//!
//! The frame's colours go through png2c64's preprocessing at its defaults with gamma 1
//! (the layers' adjustments already applied any gamma): every pixel is converted to
//! OKLab and back and clamped, as png2c64 does. Each pixel's recipe (attribute map) gives
//! its png2c64 method, strength, error clamp, `adaptive` and pattern anchor; with one
//! recipe for the whole frame the result is png2c64's. Frames the reference cannot
//! solve yet fall back to the native solver ([`unsupported`] says why).
//!
//! **Hooks** (#181 step 6), each inactive by default so the output stays png2c64's:
//! - pins, allow/deny and the recipes' allowed colours constrain a cell's search
//!   ([`CellRule`]); a free cell is searched exactly as png2c64 searches it;
//! - locked cells, given a previous solution, are copied through untouched, the
//!   background kept;
//! - the interactive fast path locks every clean cell and re-runs the ported search on
//!   the dirty ones, with the previous background;
//! - the attribute map gives each pixel its own method, strength (alpha and feather
//!   included) and pattern anchor, and regions override it, upstream in the compositor.

use super::png2c64_ref::dither::{Method as RefMethod, PxSettings};
use super::png2c64_ref::quantize::{Cell, CellRule, Hooks, Mode as RefMode, Screen};
use super::png2c64_ref::{Image, colour, convert_px, preprocess};
use super::report::{CellReport, ClashReport, Dropped};
use super::solver::{Background, Solution, SolveMode, SolveOptions, SolverKind};
use super::steering::Steering;
use crate::colour::{ColourIndex, ColourSet};
use crate::composite::{AttrSource, TargetFrame};
use crate::layer::{CellMetric, DitherKind, FitMode, Metric, Orientation, PatternPhase, Recipe};
use crate::mode::Mode;
use crate::model::{BitmapImage, CellPos, IndexedBitmap};
use crate::palette::Palette;

/// png2c64's method for one of our dither kinds: the 30 ids both have (png2c64's
/// `sierra` is Sierra Lite; its `blue-noise` is interleaved gradient noise over a 64 × 64
/// tile, which the reference uses for our `blue-noise`). `None` for the kinds png2c64
/// does not have.
pub const fn ref_method(kind: DitherKind) -> Option<RefMethod> {
    Some(match kind {
        DitherKind::None => RefMethod::None,
        DitherKind::Bayer4 => RefMethod::Bayer4,
        DitherKind::Bayer8 => RefMethod::Bayer8,
        DitherKind::FloydSteinberg => RefMethod::FloydSteinberg,
        DitherKind::Atkinson => RefMethod::Atkinson,
        DitherKind::SierraLite => RefMethod::SierraLite,
        DitherKind::Checker => RefMethod::Checker,
        DitherKind::Bayer2x2Mc => RefMethod::Bayer2x2,
        DitherKind::H2x4 => RefMethod::H2x4,
        DitherKind::Clustered4 => RefMethod::Clustered,
        DitherKind::FsWide => RefMethod::FsWide,
        DitherKind::JarvisJudiceNinke => RefMethod::Jarvis,
        DitherKind::LineH2 => RefMethod::Line2,
        DitherKind::LineChecker => RefMethod::LineChecker,
        DitherKind::LineH4 => RefMethod::Line4,
        DitherKind::LineH8 => RefMethod::Line8,
        DitherKind::LineFs => RefMethod::LineFs,
        DitherKind::Halftone8 => RefMethod::Halftone8,
        DitherKind::Diagonal8 => RefMethod::Diagonal8,
        DitherKind::Spiral5 => RefMethod::Spiral5,
        DitherKind::Hex8 => RefMethod::Hex8,
        DitherKind::Hex5 => RefMethod::Hex5,
        DitherKind::BlueNoise => RefMethod::BlueNoise,
        DitherKind::Ign => RefMethod::Ign,
        DitherKind::R2 => RefMethod::R2,
        DitherKind::WhiteNoise => RefMethod::WhiteNoise,
        DitherKind::Crosshatch => RefMethod::Crosshatch,
        DitherKind::Radial => RefMethod::Radial,
        DitherKind::ValueNoise => RefMethod::ValueNoise,
        DitherKind::Ostromoukhov => RefMethod::Ostromoukhov,
        _ => return None,
    })
}

/// png2c64's mode for a target mode.
const fn ref_mode(mode: Mode) -> Option<RefMode> {
    match mode {
        Mode::MulticolorBitmap => Some(RefMode::Multicolor),
        Mode::HiresBitmap => Some(RefMode::Hires),
        _ => None,
    }
}

/// Why a recipe's pixels need the native solver, if they do.
fn recipe_unsupported(r: &Recipe) -> Option<String> {
    let m = r.method();
    if ref_method(m.kind).is_none() {
        return Some(format!("method {:?} is not one of png2c64's", m.kind));
    }
    if m.orientation != Orientation::Auto {
        return Some(format!("orientation {:?}", m.orientation));
    }
    if r.metric() != Metric::Oklab {
        return Some(format!("metric {:?}", r.metric()));
    }
    if r.cell_metric() != CellMetric::Mse {
        return Some(format!("cell metric {:?}", r.cell_metric()));
    }
    if r.fit_mode() != FitMode::Adaptive {
        return Some("fit mode Force".to_owned());
    }
    // The native solver's own knobs: a recipe that changes one asks for native
    // behaviour (the reference has opt-in versions of its own).
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

/// Why `frame` must be solved by the native solver although the reference was asked
/// for, or `None` if the reference can solve it.
pub(crate) fn unsupported(frame: &TargetFrame, opts: &SolveOptions) -> Option<String> {
    if opts.luma_pair_bias != 0.0 {
        return Some("solve option `luma_pair_bias` (native solver only)".to_owned());
    }
    if opts.hysteresis.is_some() {
        return Some("solve option `hysteresis` (native solver only)".to_owned());
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
    preprocess::apply(
        &mut image,
        &preprocess::Settings {
            gamma: 1.0,
            ..preprocess::Settings::default()
        },
    );
    let attrs = frame.attributes();
    let default_recipe = Recipe::default();
    let (cols, rows) = mode.cells();
    let (cw, ch) = mode.cell();
    let cell_of = |i: usize| ((i % w) / cw) + ((i / w) / ch) * cols;
    // Cells copied from the previous solution: locked ones, and on the interactive fast
    // path every clean one.
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
    let mut px: Vec<PxSettings> = attrs
        .pixels()
        .iter()
        .map(|a| {
            let r = attrs.recipe(a.source).unwrap_or(&default_recipe);
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
                // Our recipes' 0 means "no clamp"; png2c64 would clamp everything to 0.
                error_clamp: if r.error_clamp() > 0.0 {
                    r.error_clamp()
                } else {
                    f32::INFINITY
                },
                adaptive: r.adaptive(),
                origin,
            }
        })
        .collect();
    // Frozen pixels neither take nor pass on diffused error.
    for (i, s) in px.iter_mut().enumerate() {
        if frozen[cell_of(i)] {
            s.method = RefMethod::None;
        }
    }
    let fixed: Vec<Option<Cell>> = match previous {
        Some(prev) if frozen.contains(&true) => {
            let out = prev.image.to_indexed();
            (0..cols * rows)
                .map(|i| frozen[i].then(|| previous_cell(prev, out.pixels(), mode, i)))
                .collect()
        }
        _ => Vec::new(),
    };
    // Locked cells may show the previous background, and a drag keeps it (as native).
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
    let screen = convert_px(&image, &pal, mode, &px, true, &hooks);
    assemble(frame, &image, &pal, &screen)
}

/// The solution of a reference screen: the encoded picture, each cell's own colours, the
/// clash report and the error, measured as png2c64 measures (squared OKLab distance,
/// weighted by coverage).
fn assemble(frame: &TargetFrame, image: &Image, pal: &[[f32; 3]; 16], screen: &Screen) -> Solution {
    let target = frame.grid().target();
    let background = (screen.mode == RefMode::Multicolor).then(|| colour_index(screen.bg));
    let sets: Vec<ColourSet> = screen
        .cells
        .iter()
        .map(|c| {
            let own = if screen.mode == RefMode::Multicolor {
                &c.colors[1..]
            } else {
                &c.colors[..]
            };
            own.iter()
                .map(|&v| colour_index(v))
                .filter(|&v| Some(v) != background)
                .collect()
        })
        .collect();
    let out: Vec<ColourIndex> = screen.indices().into_iter().map(colour_index).collect();
    let report = report(frame, image, pal, screen, &out, background);
    let error = report.cells.iter().map(|c| c.error).sum();
    let indexed = IndexedBitmap::new(target, out, background).expect("the target's size");
    let image = BitmapImage::encode(&indexed).expect("every cell keeps to its colours");
    Solution {
        image,
        background,
        rounds: vec![sets.clone()],
        sets,
        error,
        refined: 0,
        report,
        solver: SolverKind::Reference,
        fallback: None,
    }
}

fn colour_index(v: u8) -> ColourIndex {
    ColourIndex::new(v).expect("palette index below 16")
}

/// The clash report as [`ClashReport::build`] makes it, in png2c64's OKLab distances:
/// per cell the colours its pixels are nearest to (by coverage), what it shows, what the
/// missing ones cost and its error.
fn report(
    frame: &TargetFrame,
    image: &Image,
    pal: &[[f32; 3]; 16],
    screen: &Screen,
    out: &[ColourIndex],
    background: Option<ColourIndex>,
) -> ClashReport {
    let pal_lab = pal.map(colour::linear_to_oklab);
    let (w, _) = screen.mode.screen();
    let (cw, ch) = screen.mode.cell();
    let (cols, rows) = screen.mode.cells();
    let coverage = frame.coverage();
    let cells = (0..cols * rows)
        .map(|i| {
            let (gx, gy) = (i % cols, i / cols);
            let mut wanted = [0.0f32; 16];
            let mut cost = [0.0f32; 16];
            let mut chosen = background.map_or(ColourSet::EMPTY, |b| ColourSet::EMPTY.with(b));
            let mut error = 0.0f32;
            for y in gy * ch..(gy + 1) * ch {
                for x in gx * cw..(gx + 1) * cw {
                    let k = y * w + x;
                    let got = out[k];
                    chosen.insert(got);
                    let lab = colour::linear_to_oklab(image.px[k]);
                    let d = pal_lab.map(|c| colour::dist2(lab, c));
                    let d_got = d[got.as_usize()];
                    let weight = coverage[k];
                    error += weight * d_got;
                    if weight <= 0.0 {
                        continue;
                    }
                    let mut near = 0;
                    for c in 1..16 {
                        if d[c] < d[near] {
                            near = c;
                        }
                    }
                    wanted[near] += weight;
                    cost[near] += weight * (d_got - d[near]);
                }
            }
            let mut want: Vec<(ColourIndex, f32)> = ColourIndex::ALL
                .into_iter()
                .zip(wanted)
                .filter(|(_, w)| *w > 0.0)
                .collect();
            want.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
            let mut dropped: Vec<Dropped> = want
                .iter()
                .filter(|(c, _)| !chosen.contains(*c))
                .map(|&(c, weight)| Dropped {
                    colour: c,
                    weight,
                    cost: cost[c.as_usize()],
                })
                .collect();
            dropped.sort_by(|a, b| b.cost.total_cmp(&a.cost).then(a.colour.cmp(&b.colour)));
            CellReport {
                cell: CellPos::new(gx as u16, gy as u16),
                wanted: want,
                chosen,
                dropped,
                error,
                luma_cross: false,
            }
        })
        .collect();
    ClashReport { cells }
}

/// Each cell's search constraints: steering's (pins, allow, deny) with the allowed
/// colours of every covered pixel's recipe; empty when nothing constrains any cell.
fn rules(frame: &TargetFrame, steering: &Steering, mode: RefMode) -> Vec<CellRule> {
    let attrs = frame.attributes();
    let (w, _) = mode.screen();
    let (cw, ch) = mode.cell();
    let (cols, rows) = mode.cells();
    let out: Vec<CellRule> = (0..cols * rows)
        .map(|i| {
            let (gx, gy) = (i % cols, i / cols);
            let c = steering.constraints(CellPos::new(gx as u16, gy as u16));
            let mut candidates = c.candidates;
            for y in gy * ch..(gy + 1) * ch {
                for x in gx * cw..(gx + 1) * cw {
                    let k = y * w + x;
                    if frame.coverage()[k] > 0.0
                        && let Some(r) = attrs.recipe(attrs.pixels()[k].source)
                    {
                        candidates = candidates.intersection(r.allowed());
                    }
                }
            }
            CellRule {
                candidates: candidates.bits(),
                required: c.required.bits(),
            }
        })
        .collect();
    if out.iter().all(|r| *r == CellRule::FREE) {
        Vec::new()
    } else {
        out
    }
}

/// Cell `i` of a previous solution as the port's [`Cell`]: its colours by slot
/// (multicolor: the background, then its own colours ascending, padded with the
/// background) and each pixel's slot.
fn previous_cell(prev: &Solution, out: &[ColourIndex], mode: RefMode, i: usize) -> Cell {
    let (w, _) = mode.screen();
    let (cw, ch) = mode.cell();
    let cols = mode.cells().0;
    let own: Vec<u8> = prev.sets[i].iter().map(ColourIndex::get).collect();
    let colors: Vec<u8> = match (mode, prev.background) {
        (RefMode::Multicolor, Some(bg)) => {
            let mut c = vec![bg.get()];
            c.extend(own.iter().copied().filter(|&v| v != bg.get()));
            while c.len() < 4 {
                c.push(bg.get());
            }
            c
        }
        _ => {
            let mut c = own;
            let first = c.first().copied().unwrap_or(0);
            while c.len() < 2 {
                c.push(first);
            }
            c
        }
    };
    let (gx, gy) = (i % cols, i / cols);
    let px = (0..ch)
        .flat_map(|dy| (0..cw).map(move |dx| (gx * cw + dx, gy * ch + dy)))
        .map(|(x, y)| {
            let v = out[y * w + x].get();
            colors.iter().position(|&c| c == v).unwrap_or(0) as u8
        })
        .collect();
    Cell {
        colors,
        px,
        error: 0.0,
    }
}
