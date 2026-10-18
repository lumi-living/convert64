//! `convert64 convert`: a PNG or a project in, solved, every M1 output format out.

use crate::report::{Fail, NOT_LEGAL, Run};
use clap::ValueEnum;
use convert64_core::layer::{
    CellMetric, DitherKind, Downscale, Gamma, Metric, OrderedSemantics, Radius, RangeMatch,
    RangeMethod, Sharpen, suggest_range_match,
};
use convert64_core::mode::Mode;
use convert64_core::solve::{Background, BackgroundRule, Solution};
use convert64_formats::BitmapFormat;
use convert64_project::paths::{absolute, relative};
use convert64_project::{Embed, FsStore, JsonCommand, LayerRef, PaletteSpec, ProjectFile, Session};
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// Target mode on the command line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ModeArg {
    /// Multicolor bitmap, 160×200 (Koala); also `mc`, `multicolor`.
    #[value(alias = "mc", alias = "multicolor")]
    #[serde(alias = "mc", alias = "multicolor")]
    Koala,
    /// Hi-res bitmap, 320×200 (Art Studio); also `hi-res`, `art`.
    #[value(alias = "hi-res", alias = "art")]
    #[serde(alias = "hi-res", alias = "art")]
    Hires,
}

impl ModeArg {
    pub(crate) const fn mode(self) -> Mode {
        match self {
            Self::Koala => Mode::MulticolorBitmap,
            Self::Hires => Mode::HiresBitmap,
        }
    }
}

/// An output format.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum FormatArg {
    /// Koala Painter (multicolor).
    Kla,
    /// Art Studio (hi-res).
    Art,
    /// A self-displaying PRG: `LOAD"…",8` then `RUN` shows the picture.
    Prg,
    /// A raw set: `<name>.bitmap.bin`, `.screen.bin`, (multicolor) `.colour.bin` and
    /// `.bg.bin`, and the `<name>.json` manifest, next to each other.
    Raw,
    /// A PNG preview: every pixel a palette colour, multicolor pixels twice as wide,
    /// scaled by --zoom.
    Png,
}

impl FormatArg {
    /// The format an output path's extension names; `.json` is a raw set's manifest.
    pub(crate) fn from_path(path: &Path) -> Option<Self> {
        let ext = path.extension()?.to_str()?;
        if ext.eq_ignore_ascii_case("prg") {
            return Some(Self::Prg);
        }
        if ext.eq_ignore_ascii_case("json") {
            return Some(Self::Raw);
        }
        if ext.eq_ignore_ascii_case("png") {
            return Some(Self::Png);
        }
        BitmapFormat::from_extension(ext).map(|f| match f {
            BitmapFormat::Koala => Self::Kla,
            BitmapFormat::ArtStudio => Self::Art,
        })
    }

    /// The mode this format forces, if any.
    const fn mode(self) -> Option<Mode> {
        match self {
            Self::Kla => Some(Mode::MulticolorBitmap),
            Self::Art => Some(Mode::HiresBitmap),
            Self::Prg | Self::Raw | Self::Png => None,
        }
    }

    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Kla => "kla",
            Self::Art => "art",
            Self::Prg => "prg",
            Self::Raw => "raw",
            Self::Png => "png",
        }
    }

    /// The extension of the path an output of this format is named by (the manifest,
    /// for a raw set).
    pub(crate) const fn extension(self) -> &'static str {
        match self {
            Self::Raw => "json",
            other => other.name(),
        }
    }
}

/// The directory and set name a raw output path names: `build/title.json` and
/// `build/title` are both the set `title` in `build/`.
fn raw_name(path: &Path) -> Result<(PathBuf, String), Fail> {
    let dir = path.parent().map(Path::to_owned).unwrap_or_default();
    let name = if FormatArg::from_path(path) == Some(FormatArg::Raw) {
        path.file_stem()
    } else {
        path.file_name()
    }
    .and_then(|n| n.to_str())
    .filter(|n| !n.is_empty())
    .ok_or_else(|| Fail::input(format!("{}: not a raw set name", path.display())))?;
    Ok((dir, name.to_owned()))
}

/// One `--diverge` entry, `name` or `name=value`, applied to `div`.
fn apply_divergence(div: &mut convert64_core::solve::Divergences, entry: &str) -> Result<(), Fail> {
    let (name, value) = match entry.split_once('=') {
        Some((n, v)) => (n.trim(), Some(v.trim())),
        None => (entry.trim(), None),
    };
    let bad = || {
        Fail::input(format!(
            "--diverge {entry:?}: see `convert64 convert --help`"
        ))
    };
    let number = |lo: f64, hi: f64| -> Result<f64, Fail> {
        let v: f64 = value.ok_or_else(bad)?.parse().map_err(|_| bad())?;
        if (lo..=hi).contains(&v) {
            Ok(v)
        } else {
            Err(bad())
        }
    };
    match name {
        "chroma-guard" if value.is_none() => div.chroma_guard = true,
        "cell-clamp" if value.is_none() => div.cell_clamp = true,
        "bridging" if value.is_none() => div.bridging = true,
        "max-luma-diff" => div.max_luma_diff = Some(number(0.0, 8.0)? as u8),
        "mix-levels" => div.mix_levels = Some(number(1.0, 16.0)? as u8),
        "pair-bias" => div.pair_bias = number(-1.0, 1.0)? as f32,
        _ => return Err(bad()),
    }
    Ok(())
}

/// `--solver`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum SolverArg {
    /// png2amiga's C64 path, ported (ADR-013, #261), the default.
    Reference,
    /// png2c64's pipeline, ported (#181): the old reference, kept for comparison.
    #[value(name = "png2c64")]
    #[serde(rename = "png2c64")]
    Png2c64,
    /// Convert64's own solver.
    Native,
}

impl From<SolverArg> for convert64_core::solve::SolverKind {
    fn from(s: SolverArg) -> Self {
        match s {
            SolverArg::Reference => Self::Reference,
            SolverArg::Png2c64 => Self::Png2c64,
            SolverArg::Native => Self::Native,
        }
    }
}

/// `--bg-rule`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum BgRuleArg {
    /// png2amiga's: the background is always colour 0 (the default).
    Black,
    /// png2c64's: all 16 backgrounds tried, the lowest total cell error kept.
    Search,
}

impl From<BgRuleArg> for BackgroundRule {
    fn from(r: BgRuleArg) -> Self {
        match r {
            BgRuleArg::Black => Self::Black,
            BgRuleArg::Search => Self::Search,
        }
    }
}

/// `--match-range-method`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum RangeMethodArg {
    /// png2amiga's: each pixel's chroma scaled toward the palette's reachable hull,
    /// lightness and hue kept (the default).
    Hull,
    /// png2c64's: each OKLab axis stretched onto the palette's bounding box.
    Box,
}

impl From<RangeMethodArg> for RangeMethod {
    fn from(m: RangeMethodArg) -> Self {
        match m {
            RangeMethodArg::Hull => Self::Hull,
            RangeMethodArg::Box => Self::Box,
        }
    }
}

/// Settings that override a project's own (or the defaults, for a PNG). Also the shape of
/// a batch manifest's per-file settings.
#[derive(clap::Args, Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) struct Overrides {
    /// Solver: reference (png2amiga's C64 path, the default since ADR-013), png2c64
    /// (png2c64's pipeline, the old reference) or native (Convert64's own). The native
    /// solver is what the recipe knobs `--max-luma-diff`, `--no-cell-clamp`,
    /// `--mix-levels`, `--smooth`, `--ordered-semantics` and `--no-chroma-guard` act on.
    #[arg(long, value_enum)]
    pub(crate) solver: Option<SolverArg>,
    /// After the reference solve, run png2amiga's cell refine (its --cell-refine) and
    /// keep the result only if its SSIMULACRA2 improves. Off by default.
    #[arg(long)]
    #[serde(default)]
    pub(crate) cell_refine: bool,
    /// A named divergence of the png2c64 solver, all off by default (#181):
    /// chroma-guard, max-luma-diff=N (0–8), pair-bias=X, cell-clamp, mix-levels=N
    /// (1–16), bridging. Repeatable or comma-separated; asking for one solves with
    /// png2c64's port.
    #[arg(long, value_delimiter = ',', value_name = "NAME[=VALUE]")]
    #[serde(default)]
    pub(crate) diverge: Vec<String>,
    /// Target mode: koala (multicolor; also mc) or hires. Default: the project's, or the
    /// output format's, or koala.
    #[arg(long, value_enum)]
    pub(crate) mode: Option<ModeArg>,
    /// Palette: pepto (pepto-pal), pepto-ntsc, colodore, or a VICE .vpl file. Default:
    /// the project's, or pepto.
    #[arg(long)]
    pub(crate) palette: Option<String>,
    /// Colour metric: oklab, cielab, pepto-yuv or rgb. As in png2c64, mse, blur or ssim
    /// set the cell metric instead (see --cell-metric). Default oklab.
    #[arg(long)]
    pub(crate) metric: Option<String>,
    /// How the cell-set search scores sets: mse (default), blur or ssim.
    #[arg(long)]
    pub(crate) cell_metric: Option<String>,
    /// Dither method: any id in docs/reference/dither-methods.md, e.g. none, bayer4,
    /// checker, halftone8, line-h4, blue-noise, ign, fs, atkinson (png2c64's names work
    /// too). Default floyd-steinberg.
    #[arg(long)]
    pub(crate) dither: Option<String>,
    /// Dither strength, 0–1. Default 1.
    #[arg(long)]
    pub(crate) strength: Option<f32>,
    /// Background: auto (the solver chooses; see --bg-rule) or a colour index 0–15
    /// (multicolor only). Default auto.
    #[arg(long)]
    pub(crate) bg: Option<String>,
    /// How the reference solver chooses an unpinned multicolor background: black
    /// (png2amiga's colour 0, the default) or search (png2c64's search of all 16). The
    /// png2c64 and native solvers always search; hi-res has no background.
    #[arg(long, value_enum)]
    pub(crate) bg_rule: Option<BgRuleArg>,
    /// Supersample factor, 1–8. Default 4.
    #[arg(long)]
    pub(crate) supersample: Option<u8>,
    /// Let diffused error cross cell borders (png2c64 behaviour) instead of dropping it.
    #[arg(long)]
    #[serde(default)]
    pub(crate) no_cell_clamp: bool,
    /// Restrict dither mixes to colours within N luma levels, 0–8 (0 = same-luma only,
    /// PAL-blend safe; 8 = off, the default).
    #[arg(long)]
    pub(crate) max_luma_diff: Option<u8>,
    /// How ordered dithers pick colours: png2c64 (png2c64's formula, the default) or
    /// legacy (E5's, with mix levels).
    #[arg(long)]
    pub(crate) ordered_semantics: Option<String>,
    /// Let ordered mixes add chroma the source lacks (the chroma guard is on by
    /// default).
    #[arg(long)]
    #[serde(default)]
    pub(crate) no_chroma_guard: bool,
    /// Clamp diffused error to ± this per metric channel, 0–1 (png2c64 --error-clamp;
    /// 0 = off). Default 0.1.
    #[arg(long)]
    pub(crate) error_clamp: Option<f32>,
    /// Dither less where the picture has detail, 0–1 (png2c64 --adaptive). Default 0.
    #[arg(long)]
    pub(crate) adaptive: Option<f32>,
    /// Refinement hysteresis: keep a cell's colour set unless a re-pick beats it by this
    /// ratio (e.g. 0.97). Default off.
    #[arg(long)]
    pub(crate) hysteresis: Option<f32>,
    /// Ordered and noise dithers: mix each cell's colour pairs at one of N + 1 levels,
    /// 0–16 (4 = 0/25/50/75/100 %, the default; 0 = continuous per-pixel ratio).
    #[arg(long)]
    pub(crate) mix_levels: Option<u8>,
    /// Ordered and noise dithers: take the mix ratio from an edge-aware blur of the
    /// picture (σ ≈ 1 target pixel).
    #[arg(long)]
    #[serde(default)]
    pub(crate) smooth: bool,
    /// png2c64-style gamma, 0.1–8: every channel raised to this power in linear light
    /// before anything else (> 1 darkens; png2c64's default is 1.5). Default 1.
    #[arg(long)]
    pub(crate) gamma: Option<f32>,
    /// Map the picture's colour range onto the palette's (png2amiga --match-range):
    /// each pixel's chroma scaled toward what the palette can reach, lightness and hue
    /// kept. See --match-range-amount and --match-range-method.
    #[arg(long)]
    #[serde(default)]
    pub(crate) match_range: bool,
    /// With --match-range: how much of the matching to apply, 0–100 % (100, the
    /// default, is png2amiga's; 0 keeps the source's chroma). Hull method only.
    #[arg(long, value_name = "PERCENT")]
    pub(crate) match_range_amount: Option<f32>,
    /// With --match-range: hull (png2amiga's, the default) or box (png2c64's: each
    /// OKLab axis stretched onto the palette's extent, kept for comparison).
    #[arg(long, value_enum, value_name = "hull|box")]
    pub(crate) match_range_method: Option<RangeMethodArg>,
    /// Sharpen every layer's lightness at the C64 grid, −1–3 (png2c64 --sharpen):
    /// positive adds that much of the detail an unsharp mask finds, negative blends that
    /// far (at most 1) toward a blur. Default 0, off.
    #[arg(long, value_name = "AMOUNT", allow_hyphen_values = true)]
    pub(crate) sharpen: Option<f32>,
    /// The sharpen's blur: png2c64 (its 3×3 box, the default) or a Gaussian σ in C64
    /// pixels, 0.3–3.
    #[arg(long, value_name = "PX|png2c64")]
    pub(crate) sharpen_radius: Option<Radius>,
    /// Leave pixels whose lightness differs from the blur by less than this unsharpened,
    /// 0–0.1 in OKLab L (png2c64 has none: 0, the default).
    #[arg(long, value_name = "L")]
    pub(crate) sharpen_threshold: Option<f32>,
    /// How every layer shrunk a long way is sampled (#235): auto (the default) takes a
    /// cached, box-reduced mip level of the source past a 4× shrink, so large sources do
    /// not alias; exact always samples the original with png2c64's four Mitchell taps,
    /// as png2c64 does. Up to 4× (sources up to 1280 px fitted) the two are identical.
    #[arg(long, value_name = "auto|exact")]
    pub(crate) downscale: Option<Downscale>,
    /// Turn every layer to this many degrees, clockwise on screen, about its anchor
    /// (`layer_rotate`, ADR-011). The project's snap applies.
    #[arg(long, value_name = "DEG", allow_hyphen_values = true)]
    pub(crate) rotate: Option<f32>,
    /// The pivot to turn about, as X,Y: 0–1 across each layer's box (0,0 its top-left;
    /// a layer's default is 0.5,0.5, the centre). The picture stays in place while the
    /// pivot moves.
    #[arg(long, value_name = "X,Y")]
    pub(crate) anchor: Option<String>,
}

/// A serde enum value from its kebab-case name.
fn named<T: serde::de::DeserializeOwned>(what: &str, name: &str) -> Result<T, Fail> {
    serde_json::from_value(Value::String(name.to_owned()))
        .map_err(|_| Fail::input(format!("unknown {what} {name:?}")))
}

impl Overrides {
    /// `self` on top of `base`: every value `self` sets wins (a batch file's settings
    /// over the manifest's defaults over the command line's).
    pub(crate) fn or(self, base: &Self) -> Self {
        Self {
            solver: self.solver.or(base.solver),
            cell_refine: self.cell_refine || base.cell_refine,
            diverge: if self.diverge.is_empty() {
                base.diverge.clone()
            } else {
                self.diverge.clone()
            },
            mode: self.mode.or(base.mode),
            palette: self.palette.or_else(|| base.palette.clone()),
            metric: self.metric.or_else(|| base.metric.clone()),
            cell_metric: self.cell_metric.or_else(|| base.cell_metric.clone()),
            dither: self.dither.or_else(|| base.dither.clone()),
            strength: self.strength.or(base.strength),
            bg: self.bg.or_else(|| base.bg.clone()),
            bg_rule: self.bg_rule.or(base.bg_rule),
            supersample: self.supersample.or(base.supersample),
            no_cell_clamp: self.no_cell_clamp || base.no_cell_clamp,
            max_luma_diff: self.max_luma_diff.or(base.max_luma_diff),
            ordered_semantics: self
                .ordered_semantics
                .or_else(|| base.ordered_semantics.clone()),
            no_chroma_guard: self.no_chroma_guard || base.no_chroma_guard,
            error_clamp: self.error_clamp.or(base.error_clamp),
            adaptive: self.adaptive.or(base.adaptive),
            hysteresis: self.hysteresis.or(base.hysteresis),
            mix_levels: self.mix_levels.or(base.mix_levels),
            smooth: self.smooth || base.smooth,
            gamma: self.gamma.or(base.gamma),
            match_range: self.match_range || base.match_range,
            match_range_amount: self.match_range_amount.or(base.match_range_amount),
            match_range_method: self.match_range_method.or(base.match_range_method),
            sharpen: self.sharpen.or(base.sharpen),
            sharpen_radius: self.sharpen_radius.or(base.sharpen_radius),
            sharpen_threshold: self.sharpen_threshold.or(base.sharpen_threshold),
            downscale: self.downscale.or(base.downscale),
            rotate: self.rotate.or(base.rotate),
            anchor: self.anchor.or_else(|| base.anchor.clone()),
        }
    }

    /// The range matching `--match-range` asks for onto the project's palette:
    /// png2amiga's hull at `--match-range-amount` (100 % by default) or, with
    /// `--match-range-method box`, png2c64's box. `None` without `--match-range`; the
    /// amount and method alone are errors, as is an amount with the box.
    fn range_match(&self, file: &ProjectFile, base: &Path) -> Result<Option<RangeMatch>, Fail> {
        if !self.match_range {
            if self.match_range_amount.is_some() || self.match_range_method.is_some() {
                return Err(Fail::input(
                    "--match-range-amount and --match-range-method need --match-range",
                ));
            }
            return Ok(None);
        }
        let method = self
            .match_range_method
            .map_or(RangeMethod::Hull, Into::into);
        let palette = file.palette.resolve(base, &FsStore).map_err(Fail::input)?;
        let m = match method {
            RangeMethod::Hull => {
                let pct = self.match_range_amount.unwrap_or(100.0);
                if !(0.0..=100.0).contains(&pct) {
                    return Err(Fail::input(format!(
                        "--match-range-amount must be 0–100, got {pct}"
                    )));
                }
                RangeMatch::hull(&palette, pct / 100.0)
            }
            RangeMethod::Box => {
                if self.match_range_amount.is_some() {
                    return Err(Fail::input(
                        "--match-range-amount applies to the hull method, not box",
                    ));
                }
                RangeMatch::for_palette(
                    &palette,
                    RangeMatch::DEFAULT_PERCENTILE,
                    RangeMatch::DEFAULT_MARGIN,
                )
            }
        };
        m.map(Some).map_err(Fail::input)
    }

    /// `--rotate` and `--anchor` as a `layer_rotate` per layer, run after the project's
    /// own commands; `--anchor` alone moves the pivot and keeps the angle.
    fn rotations(&self, layers: usize) -> Result<Vec<JsonCommand>, Fail> {
        let anchor = match &self.anchor {
            None => None,
            Some(text) => {
                let v: Vec<f32> = text
                    .split(',')
                    .map(|s| s.trim().parse::<f32>())
                    .collect::<Result<_, _>>()
                    .map_err(|_| Fail::input(format!("--anchor must be X,Y, got {text:?}")))?;
                let [x, y] = v[..] else {
                    return Err(Fail::input(format!("--anchor must be X,Y, got {text:?}")));
                };
                if !(0.0..=1.0).contains(&x) || !(0.0..=1.0).contains(&y) {
                    return Err(Fail::input(format!(
                        "--anchor must be 0–1 on both axes, got {text:?}"
                    )));
                }
                Some(convert64_core::Vec2::new(x, y))
            }
        };
        if let Some(r) = self.rotate
            && !r.is_finite()
        {
            return Err(Fail::input(format!("--rotate must be finite, got {r}")));
        }
        if self.rotate.is_none() && anchor.is_none() {
            return Ok(Vec::new());
        }
        Ok((0..layers)
            .map(|i| JsonCommand::LayerRotate {
                layer: LayerRef::Index(i),
                degrees: self.rotate.unwrap_or(0.0),
                relative: self.rotate.is_none(),
                anchor,
                snap: None,
            })
            .collect())
    }

    /// The recipe fields these overrides set, as recipe JSON (`method.kind` as `kind`).
    fn recipe_patch(&self) -> Result<Vec<(&'static str, Value)>, Fail> {
        let mut patch = Vec::new();
        if let Some(d) = &self.dither {
            named::<DitherKind>("dither method", d)?;
            patch.push(("kind", json!(d)));
        }
        if let Some(s) = self.strength {
            patch.push(("strength", json!(s)));
        }
        // png2c64's --metric names the cell metric; ours names the colour metric, and
        // accepts png2c64's names too.
        if let Some(m) = &self.metric {
            if named::<CellMetric>("cell metric", m).is_ok() {
                patch.push(("cell_metric", json!(m)));
            } else {
                named::<Metric>("metric", m)?;
                patch.push(("metric", json!(m)));
            }
        }
        if let Some(m) = &self.cell_metric {
            named::<CellMetric>("cell metric", m)?;
            patch.push(("cell_metric", json!(m)));
        }
        if self.no_cell_clamp {
            patch.push(("cell_clamp", json!(false)));
        }
        if let Some(v) = self.max_luma_diff {
            patch.push(("max_luma_diff", json!(v)));
        }
        if let Some(s) = &self.ordered_semantics {
            named::<OrderedSemantics>("ordered semantics", s)?;
            patch.push(("ordered_semantics", json!(s)));
        }
        if self.no_chroma_guard {
            patch.push(("chroma_guard", json!(false)));
        }
        if let Some(v) = self.error_clamp {
            patch.push(("error_clamp", json!(v)));
        }
        if let Some(v) = self.adaptive {
            patch.push(("adaptive", json!(v)));
        }
        if let Some(v) = self.mix_levels {
            patch.push(("mix_levels", json!(v)));
        }
        if self.smooth {
            patch.push(("smooth", json!(true)));
        }
        Ok(patch)
    }

    /// Applies the overrides to a project file. `base` resolves the palette's `.vpl`.
    pub(crate) fn apply(&self, file: &mut ProjectFile, base: &Path) -> Result<(), Fail> {
        if let Some(m) = self.mode {
            file.target = m.mode();
        }
        if let Some(n) = self.supersample {
            file.supersample = n;
        }
        if let Some(p) = &self.palette {
            file.palette = PaletteSpec::from_arg(p).map_err(Fail::input)?;
            if let PaletteSpec::Vpl(path) = &file.palette {
                // A .vpl named on the command line is relative to the working directory.
                file.palette = PaletteSpec::Vpl(relative(&absolute(base), &absolute(path)));
            }
        }
        if let Some(bg) = &self.bg {
            file.solve.background = match bg.as_str() {
                "auto" => Background::Auto,
                n => Background::Fixed(
                    n.parse::<u8>()
                        .ok()
                        .and_then(|v| convert64_core::ColourIndex::new(v).ok())
                        .ok_or_else(|| {
                            Fail::input(format!("--bg must be auto or 0–15, got {n:?}"))
                        })?,
                ),
            };
        }
        if let Some(r) = self.bg_rule {
            file.solve.background_rule = r.into();
        }
        if let Some(s) = self.solver {
            file.solve.solver = s.into();
        }
        if self.cell_refine {
            file.solve.refine = true;
        }
        for d in &self.diverge {
            apply_divergence(&mut file.solve.divergences, d)?;
        }
        if let Some(h) = self.hysteresis {
            if !(h > 0.0 && h <= 1.0) {
                return Err(Fail::input(format!(
                    "--hysteresis must be in (0, 1], got {h}"
                )));
            }
            file.solve.hysteresis = Some(h);
        }
        let gamma = self
            .gamma
            .map(Gamma::new)
            .transpose()
            .map_err(Fail::input)?;
        let match_range = self.range_match(file, base)?;
        let sharpen = self.sharpen.is_some()
            || self.sharpen_radius.is_some()
            || self.sharpen_threshold.is_some();
        let patch = self.recipe_patch()?;
        for layer in &mut file.layers {
            if !patch.is_empty() {
                let mut v = serde_json::to_value(layer.recipe).map_err(Fail::input)?;
                for (k, x) in &patch {
                    if *k == "kind" {
                        v["method"]["kind"] = x.clone();
                    } else {
                        v[*k] = x.clone();
                    }
                }
                layer.recipe = serde_json::from_value(v)
                    .map_err(|e| Fail::input(format!("layer {:?}: {e}", layer.name)))?;
            }
            if let Some(g) = gamma {
                layer.adjustments.gamma = g;
            }
            if match_range.is_some() {
                layer.adjustments.match_range = match_range;
            }
            if sharpen {
                let s = layer.adjustments.sharpen;
                layer.adjustments.sharpen = Sharpen::new(
                    self.sharpen.unwrap_or(s.amount()),
                    self.sharpen_radius.unwrap_or(s.radius()),
                    self.sharpen_threshold.unwrap_or(s.threshold()),
                )
                .map_err(Fail::input)?;
            }
            if let Some(d) = self.downscale {
                layer.downscale = d;
            }
        }
        let rotations = self.rotations(file.layers.len())?;
        file.commands.extend(rotations);
        Ok(())
    }
}

/// What `convert` was asked to do.
pub(crate) struct ConvertJob<'a> {
    pub(crate) input: &'a Path,
    pub(crate) outputs: &'a [PathBuf],
    pub(crate) format: Option<FormatArg>,
    pub(crate) overrides: &'a Overrides,
    pub(crate) max_clashes: Option<usize>,
    pub(crate) save_project: Option<(&'a Path, Embed)>,
    pub(crate) report: bool,
    /// PNG preview zoom, 1–8.
    pub(crate) zoom: u8,
}

/// Whether `path` holds a PNG (by its signature), else a project or recipe (JSON).
fn is_png(path: &Path) -> Result<bool, Fail> {
    use std::io::Read;
    let mut head = [0u8; 8];
    let mut f =
        std::fs::File::open(path).map_err(|e| Fail::input(format!("{}: {e}", path.display())))?;
    let n = f
        .read(&mut head)
        .map_err(|e| Fail::input(format!("{}: {e}", path.display())))?;
    Ok(head[..n] == *b"\x89PNG\r\n\x1a\n")
}

/// Loads the input as a project with the overrides applied.
pub(crate) fn load(
    run: &mut Run,
    input: &Path,
    overrides: &Overrides,
    implied_mode: Option<Mode>,
) -> Result<(Session, bool), Fail> {
    let png = is_png(input)?;
    let (mut file, base) = if png {
        let size = image::image_dimensions(input)
            .map_err(|e| Fail::input(format!("{}: {e}", input.display())))?;
        let name = input
            .file_stem()
            .map_or_else(|| "source".to_owned(), |s| s.to_string_lossy().into_owned());
        let target = implied_mode.unwrap_or(Mode::MulticolorBitmap);
        (
            ProjectFile::from_png(target, &name, input, size),
            PathBuf::new(),
        )
    } else {
        let file = ProjectFile::read(input).map_err(Fail::input)?;
        (file, input.parent().map(Path::to_owned).unwrap_or_default())
    };
    overrides.apply(&mut file, &base)?;
    let session = run.time("load", || {
        Session::from_project(file, &base).map_err(Fail::input)
    })?;
    run.notes_of(&session);
    Ok((session, png))
}

/// The mode the outputs force, if any; an error if they disagree.
fn mode_of_outputs(outputs: &[(PathBuf, FormatArg)]) -> Result<Option<Mode>, Fail> {
    let mut mode = None;
    for (path, f) in outputs {
        if let Some(m) = f.mode() {
            if mode.is_some_and(|x| x != m) {
                return Err(Fail::input(format!(
                    "{} needs {}, another output needs {}",
                    path.display(),
                    m.name(),
                    mode.map_or("", Mode::name)
                )));
            }
            mode = Some(m);
        }
    }
    Ok(mode)
}

/// Runs `convert`. Returns 0, or [`NOT_LEGAL`] for an illegal result or too many
/// clashes (the files are still written).
pub(crate) fn run(run: &mut Run, job: &ConvertJob<'_>) -> Result<u8, Fail> {
    run.set("input", job.input.display().to_string());
    if job.outputs.is_empty() && job.save_project.is_none() {
        return Err(Fail::input("nothing to write: give -o (or --save-project)"));
    }
    if job.format.is_some() && job.outputs.len() > 1 {
        return Err(Fail::input(
            "--format applies to a single -o; use extensions instead",
        ));
    }
    let mut outputs = Vec::with_capacity(job.outputs.len());
    for o in job.outputs {
        let f = job
            .format
            .or_else(|| FormatArg::from_path(o))
            .ok_or_else(|| {
                Fail::input(format!(
                    "cannot tell the format of {}: use .kla, .art, .prg, .png or .json (raw set), or \
                     --format",
                    o.display()
                ))
            })?;
        outputs.push((o.clone(), f));
    }
    let forced = mode_of_outputs(&outputs)?;
    let (mut session, png) = load(run, job.input, job.overrides, forced)?;
    let mode = session.target();
    run.set("mode", mode);
    run.set("palette", session.palette().name());
    run.set("refine", session.project().solve.refine);
    run.set("background_rule", session.project().solve.background_rule);
    if let Some(m) = forced
        && m != mode
    {
        let (path, _) = outputs
            .iter()
            .find(|(_, f)| f.mode() == Some(m))
            .expect("forced by one");
        return Err(Fail::input(format!(
            "{} is {} but the picture is {}: use --mode or another extension",
            path.display(),
            m.name(),
            mode.name()
        )));
    }
    let bitmap = BitmapFormat::for_mode(mode)
        .ok_or_else(|| Fail::input(format!("{} cannot be converted yet", mode.name())))?;
    if png && !job.overrides.match_range {
        let src = session.project().layers[0].source();
        if suggest_range_match(src, session.palette()).is_some() {
            eprintln!(
                "hint: the picture's lightness range is narrow; --match-range maps its colours \
                 onto the palette's (png2amiga --match-range)"
            );
        }
    }
    if let Some((path, embed)) = job.save_project {
        let dir = path.parent().unwrap_or(Path::new(""));
        let text = session
            .to_project_file(dir, embed)
            .map_err(Fail::input)?
            .to_json();
        run.write(path, text.as_bytes())?;
        run.say(format!("project -> {}", path.display()));
    }
    if outputs.is_empty() {
        return Ok(0);
    }
    let started = std::time::Instant::now();
    let sol = run.time("solve", || session.solve(true).clone());
    let ms = started.elapsed().as_secs_f64() * 1000.0;
    let s2 = run.time("ssimulacra2", || session.ssimulacra2());
    let file = bitmap.write(&sol.image).map_err(Fail::input)?;
    let mut written = Vec::new();
    for (path, f) in &outputs {
        let bytes = match f {
            FormatArg::Kla | FormatArg::Art => file.clone(),
            FormatArg::Prg => {
                convert64_formats::prg::display_prg(bitmap, &file).map_err(Fail::input)?
            }
            FormatArg::Png => {
                convert64_formats::png::preview(&sol.image, session.palette(), job.zoom)
                    .map_err(Fail::input)?
            }
            FormatArg::Raw => {
                let (dir, name) = raw_name(path)?;
                let set = convert64_formats::raw::write(&sol.image, &name, session.palette())
                    .map_err(Fail::input)?;
                for (file, bytes) in &set {
                    run.write(&dir.join(file), bytes)?;
                }
                written.push(json!({ "path": path.display().to_string(), "format": f.name() }));
                run.say(format!(
                    "{} -> {} ({}, {ms:.0} ms)",
                    job.input.display(),
                    dir.join(convert64_formats::raw::manifest_name(&name))
                        .display(),
                    f.name(),
                ));
                continue;
            }
        };
        run.write(path, &bytes)?;
        written.push(json!({ "path": path.display().to_string(), "format": f.name() }));
        run.say(format!(
            "{} -> {} ({}, {ms:.0} ms)",
            job.input.display(),
            path.display(),
            f.name(),
        ));
    }
    run.set("outputs", written);
    finish_solve(run, &sol, s2, job.report, job.max_clashes)
}

/// Reports a solve: its SSIMULACRA2 score `s2` (`Session::ssimulacra2`), clashes,
/// legality and the clash report; returns the exit code.
pub(crate) fn finish_solve(
    run: &mut Run,
    sol: &Solution,
    s2: Option<f64>,
    worst: bool,
    max_clashes: Option<usize>,
) -> Result<u8, Fail> {
    let summary = sol.report.summary();
    let violations: Vec<String> = convert64_core::validate(&sol.image)
        .err()
        .into_iter()
        .flatten()
        .map(|v| v.to_string())
        .collect();
    run.set("ssimulacra2", s2);
    run.set("background", sol.background);
    run.set("clashes", summary.clashes);
    run.set("max_clashes", max_clashes);
    run.set("legal", violations.is_empty());
    run.set("violations", &violations);
    let cells: Vec<_> = if worst {
        sol.report
            .worst_first()
            .iter()
            .take(10)
            .filter_map(|c| sol.report.cell(*c))
            .collect()
    } else {
        Vec::new()
    };
    run.set("solve_error", sol.error);
    run.set("solver", sol.solver);
    if let Some(why) = &sol.fallback {
        run.set("solver_fallback", why);
    }
    run.set("summary", summary);
    run.set("worst", cells);
    if worst {
        run.say(summary_line(sol, s2));
    }
    if !violations.is_empty() {
        for v in &violations {
            eprintln!("illegal: {v}");
        }
        return Ok(NOT_LEGAL);
    }
    if let Some(max) = max_clashes
        && summary.clashes > max
    {
        eprintln!("{} clashes, more than --max-clashes {max}", summary.clashes);
        return Ok(NOT_LEGAL);
    }
    Ok(0)
}

/// The clash summary as the status bar would show it, with the SSIMULACRA2 score.
pub(crate) fn summary_line(sol: &Solution, s2: Option<f64>) -> String {
    let s = sol.report.summary();
    format!(
        "solved: background {}, error {:.4}, {} clash{}{}{}",
        sol.background
            .map_or_else(|| "none".to_owned(), |b| b.to_string()),
        s.total_error,
        s.clashes,
        if s.clashes == 1 { "" } else { "es" },
        s.worst
            .map(|c| format!(", worst cell {c}"))
            .unwrap_or_default(),
        s2.map(|v| format!(", SSIMULACRA2 {v:.2}"))
            .unwrap_or_default()
    )
}
