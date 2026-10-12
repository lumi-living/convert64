//! `convert64 convert`: a PNG or a project in, solved, every M1 output format out.

use crate::report::{Fail, NOT_LEGAL, Run};
use clap::ValueEnum;
use convert64_cli::project::{Embed, PaletteSpec, ProjectFile, absolute};
use convert64_cli::recipe::Session;
use convert64_core::layer::{
    CellMetric, DitherKind, Gamma, Metric, OrderedSemantics, RangeMatch, suggest_range_match,
};
use convert64_core::mode::Mode;
use convert64_core::solve::{Background, Solution};
use convert64_formats::BitmapFormat;
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

/// `--solver`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum SolverArg {
    /// png2c64's pipeline, ported (#181).
    Reference,
    /// Convert64's own solver.
    Native,
}

impl From<SolverArg> for convert64_core::solve::SolverKind {
    fn from(s: SolverArg) -> Self {
        match s {
            SolverArg::Reference => Self::Reference,
            SolverArg::Native => Self::Native,
        }
    }
}

/// Settings that override a project's own (or the defaults, for a PNG). Also the shape of
/// a batch manifest's per-file settings.
#[derive(clap::Args, Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) struct Overrides {
    /// Solver: reference (png2c64's pipeline, the default) or native (Convert64's own).
    /// The native solver is what the recipe knobs `--max-luma-diff`, `--no-cell-clamp`,
    /// `--mix-levels`, `--smooth`, `--ordered-semantics` and `--no-chroma-guard` act on.
    #[arg(long, value_enum)]
    pub(crate) solver: Option<SolverArg>,
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
    /// Background: auto (search all 16) or a colour index 0–15 (multicolor only).
    /// Default auto.
    #[arg(long)]
    pub(crate) bg: Option<String>,
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
    /// Map the picture's OKLab range onto the palette's (png2c64 --match-range).
    #[arg(long)]
    #[serde(default)]
    pub(crate) match_range: bool,
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
            mode: self.mode.or(base.mode),
            palette: self.palette.or_else(|| base.palette.clone()),
            metric: self.metric.or_else(|| base.metric.clone()),
            cell_metric: self.cell_metric.or_else(|| base.cell_metric.clone()),
            dither: self.dither.or_else(|| base.dither.clone()),
            strength: self.strength.or(base.strength),
            bg: self.bg.or_else(|| base.bg.clone()),
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
        }
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
                file.palette = PaletteSpec::Vpl(convert64_cli::project::relative(
                    &absolute(base),
                    &absolute(path),
                ));
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
        if let Some(s) = self.solver {
            file.solve.solver = s.into();
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
        let match_range = if self.match_range {
            let palette = file.palette.resolve(base).map_err(Fail::input)?;
            Some(
                RangeMatch::for_palette(
                    &palette,
                    RangeMatch::DEFAULT_PERCENTILE,
                    RangeMatch::DEFAULT_MARGIN,
                )
                .map_err(Fail::input)?,
            )
        } else {
            None
        };
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
        }
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
                "hint: the picture's lightness range is narrow; --match-range stretches it to \
                 the palette's (png2c64 --match-range)"
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
    finish_solve(run, &sol, job.report, job.max_clashes)
}

/// Reports a solve: clashes, legality and the clash report; returns the exit code.
pub(crate) fn finish_solve(
    run: &mut Run,
    sol: &Solution,
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
        run.say(summary_line(sol));
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

/// The clash summary as the status bar would show it.
pub(crate) fn summary_line(sol: &Solution) -> String {
    let s = sol.report.summary();
    format!(
        "solved: background {}, error {:.4}, {} clash{}{}",
        sol.background
            .map_or_else(|| "none".to_owned(), |b| b.to_string()),
        s.total_error,
        s.clashes,
        if s.clashes == 1 { "" } else { "es" },
        s.worst
            .map(|c| format!(", worst cell {c}"))
            .unwrap_or_default()
    )
}
