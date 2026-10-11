//! The `convert64` binary: convert, validate, export and batch from the command line (`docs/TAD.md §4`).
//!
//! So far `convert64 validate`, which checks a Koala or Art Studio picture against the
//! rules in `docs/reference/vic-ii-constraints.md` §6, and `convert64 recipe apply`,
//! which runs a recipe file's commands headless (ADR-005). The full CLI design is E7.

mod json;

use clap::{Parser, Subcommand, ValueEnum};
use convert64_core::mode::BlockKind;
use convert64_core::{Location, Placement, validate, validate_placement};
use convert64_formats::{BitmapFormat, ReadError};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "convert64",
    version,
    about = "Legal Commodore 64 graphics from layered images"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Check a picture file against the VIC-II rules.
    ///
    /// Prints `legal` and exits 0, or lists every violation and exits 1. Exits 2 when
    /// the file cannot be read or its format cannot be determined.
    Validate {
        /// A Koala (.kla, .koa) or Art Studio (.art, .aas) picture.
        file: PathBuf,
        /// Print the result as JSON.
        #[arg(long)]
        json: bool,
        /// Read the file as this format instead of guessing from extension and size.
        #[arg(long, value_enum)]
        format: Option<FormatArg>,
    },
    /// Convert a PNG to a legal C64 picture (Koala for multicolor, Art Studio for hi-res).
    ///
    /// The image is scaled to fill the 320×200 screen, composited at the supersample
    /// factor, solved (background search and refinement) and written. Exits 0 on
    /// success, 2 when the input cannot be read or an option is invalid.
    Convert {
        /// The source image (PNG).
        input: PathBuf,
        /// Target mode.
        #[arg(long, value_enum, default_value = "mc")]
        mode: ModeArg,
        /// Colour metric: oklab, cielab, pepto-yuv or rgb.
        #[arg(long, default_value = "oklab")]
        metric: String,
        /// Dither method, e.g. none, bayer2, bayer4, bayer8, blue-noise,
        /// floyd-steinberg, atkinson, jarvis-judice-ninke, sierra.
        #[arg(long, default_value = "floyd-steinberg")]
        dither: String,
        /// Dither strength, 0–1.
        #[arg(long, default_value_t = 1.0)]
        strength: f32,
        /// Background: auto (search all 16) or a colour index 0–15 (multicolor only).
        #[arg(long, default_value = "auto")]
        bg: String,
        /// Supersample factor, 1–8.
        #[arg(long, default_value_t = 4)]
        supersample: u8,
        /// Output file (.kla for multicolor, .art for hi-res).
        #[arg(short, long)]
        output: PathBuf,
        /// Print the clash summary.
        #[arg(long)]
        report: bool,
        /// Print the result (and clash report) as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Work with recipe files: layers plus commands, in JSON.
    Recipe {
        #[command(subcommand)]
        action: RecipeAction,
    },
}

#[derive(Subcommand)]
enum RecipeAction {
    /// Load a recipe's layers, run its commands and write the results.
    ///
    /// Exits 0 on success, 2 when the recipe cannot be read or a command fails.
    Apply {
        /// The recipe file (JSON).
        file: PathBuf,
        /// Write a layer's evaluated mask as a 16-bit greyscale PNG on the super grid:
        /// `LAYER=PATH`, where LAYER is an index or a name. Repeatable.
        #[arg(long = "mask", value_name = "LAYER=PATH")]
        masks: Vec<String>,
        /// Write the composite on the target grid (sRGB, alpha = coverage) as a PNG.
        #[arg(long, value_name = "PATH")]
        composite: Option<PathBuf>,
        /// Write the attribute map's debug render as a PNG.
        #[arg(long, value_name = "PATH")]
        attributes: Option<PathBuf>,
        /// Solve the picture and print the clash summary.
        #[arg(long)]
        report: bool,
        /// With --report, print the clash report (summary and the worst cells) as JSON.
        #[arg(long, requires = "report")]
        json: bool,
    },
    /// List the commands a recipe can contain.
    Commands,
}

#[derive(Clone, Copy, ValueEnum)]
enum ModeArg {
    /// Multicolor bitmap (160×200, Koala).
    Mc,
    /// Hi-res bitmap (320×200, Art Studio).
    Hires,
}

#[derive(Clone, Copy, ValueEnum)]
enum FormatArg {
    /// Koala Painter.
    Kla,
    /// Art Studio.
    Art,
}

/// One broken rule as reported by the CLI: rules 1–5 from the validator, rule 6 (file
/// size) and rule 2 (colour bytes that cannot even be read) from the reader.
pub(crate) struct Finding {
    pub(crate) rule: u8,
    /// `None` means the file as a whole.
    pub(crate) location: Option<Location>,
    pub(crate) message: String,
}

impl Finding {
    fn line(&self) -> String {
        let at = self
            .location
            .map_or_else(|| "file".to_owned(), |l| l.to_string());
        format!("rule {}: {at}: {}", self.rule, self.message)
    }
}

/// What `validate` learned about a file.
pub(crate) struct Report {
    pub(crate) file: String,
    pub(crate) format: BitmapFormat,
    pub(crate) mode: Option<&'static str>,
    pub(crate) findings: Vec<Finding>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Validate { file, json, format } => run_validate(&file, json, format),
        Command::Convert {
            input,
            mode,
            metric,
            dither,
            strength,
            bg,
            supersample,
            output,
            report,
            json,
        } => {
            let opts = ConvertOptions {
                mode,
                metric,
                dither,
                strength,
                bg,
                supersample,
            };
            match run_convert(&input, &opts, &output, report, json) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("error: {e}");
                    ExitCode::from(2)
                }
            }
        }
        Command::Recipe { action } => match action {
            RecipeAction::Apply {
                file,
                masks,
                composite,
                attributes,
                report,
                json,
            } => match run_recipe(
                &file,
                &masks,
                composite.as_deref(),
                attributes.as_deref(),
                report.then_some(json),
            ) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("error: {e}");
                    ExitCode::from(2)
                }
            },
            RecipeAction::Commands => {
                for c in convert64_core::command::REGISTRY {
                    println!("{:<18} {}", c.name, c.summary);
                }
                ExitCode::SUCCESS
            }
        },
    }
}

fn run_recipe(
    file: &Path,
    masks: &[String],
    composite: Option<&Path>,
    attributes: Option<&Path>,
    report: Option<bool>,
) -> Result<(), String> {
    use convert64_cli::recipe::{LayerRef, Session};
    let mut session = Session::open(file).map_err(|e| e.to_string())?;
    println!(
        "{}: {} layer(s), {} command(s) applied",
        file.display(),
        session.project().layers.len(),
        session.editor_mut().undo_len()
    );
    for m in masks {
        let (layer, path) = m
            .split_once('=')
            .ok_or_else(|| format!("--mask {m}: expected LAYER=PATH"))?;
        let r = layer
            .parse()
            .map_or_else(|_| LayerRef::Name(layer.to_owned()), LayerRef::Index);
        let i = session.resolve(&r).map_err(|e| e.to_string())?;
        session
            .write_mask(i, Path::new(path))
            .map_err(|e| e.to_string())?;
        println!("mask of layer {i} -> {path}");
    }
    if let Some(p) = composite {
        session.write_composite(p).map_err(|e| e.to_string())?;
        println!("composite -> {}", p.display());
    }
    if let Some(p) = attributes {
        session.write_attributes(p).map_err(|e| e.to_string())?;
        println!("attribute map -> {}", p.display());
    }
    if let Some(as_json) = report {
        session.solve(true);
        if as_json {
            let v = session.clash_report_json(10).expect("solved above");
            println!(
                "{}",
                serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?
            );
        } else {
            print_summary(session.solution().expect("solved above"));
        }
    }
    Ok(())
}

struct ConvertOptions {
    mode: ModeArg,
    metric: String,
    dither: String,
    strength: f32,
    bg: String,
    supersample: u8,
}

/// A serde enum value from its kebab-case name.
fn named<T: serde::de::DeserializeOwned>(what: &str, name: &str) -> Result<T, String> {
    serde_json::from_value(serde_json::Value::String(name.to_owned()))
        .map_err(|_| format!("unknown {what} {name:?}"))
}

fn run_convert(
    input: &Path,
    o: &ConvertOptions,
    output: &Path,
    report: bool,
    json: bool,
) -> Result<(), String> {
    use convert64_core::geom::Vec2;
    use convert64_core::layer::{DitherKind, Layer, Method, Metric, Recipe, Transform};
    use convert64_core::mode::Mode;
    use convert64_core::solve::{Background, SolveOptions, Solver, Steering};
    use convert64_core::{Compositor, Palette, SuperGrid};
    use convert64_formats::BitmapFormat;
    let mode = match o.mode {
        ModeArg::Mc => Mode::MulticolorBitmap,
        ModeArg::Hires => Mode::HiresBitmap,
    };
    let kind: DitherKind = named("dither method", &o.dither)?;
    let metric: Metric = named("metric", &o.metric)?;
    let background = match o.bg.as_str() {
        "auto" => Background::Auto,
        n => Background::Fixed(
            n.parse::<u8>()
                .ok()
                .and_then(|v| convert64_core::ColourIndex::new(v).ok())
                .ok_or_else(|| format!("--bg must be auto or 0–15, got {n:?}"))?,
        ),
    };
    let d = Recipe::default();
    let recipe = Recipe::new(
        Method::new(kind),
        o.strength,
        d.phase(),
        metric,
        d.allowed(),
        d.fit_mode(),
    )
    .map_err(|e| e.to_string())?;
    let src = convert64_cli::recipe::load_source(input).map_err(|e| e.to_string())?;
    let (w, h) = src.size();
    let mut layer = Layer::new("source", std::sync::Arc::new(src));
    layer.set_transform(
        Transform::new(
            Vec2::ZERO,
            Vec2::new(320.0 / w as f32, 200.0 / h as f32),
            false,
            false,
        )
        .map_err(|e| e.to_string())?,
    );
    layer.set_recipe(recipe);
    let grid = SuperGrid::new(mode, o.supersample).map_err(|e| e.to_string())?;
    let mut c = Compositor::new(grid);
    c.update(&[layer], &[]);
    let started = std::time::Instant::now();
    let sol = Solver::new(&Palette::default()).solve(
        c.frame(),
        &Steering::new(),
        None,
        &SolveOptions {
            background,
            ..SolveOptions::default()
        },
    );
    let ms = started.elapsed().as_secs_f64() * 1000.0;
    let format = BitmapFormat::for_mode(mode).expect("bitmap mode");
    let bytes = format.write(&sol.image).map_err(|e| e.to_string())?;
    std::fs::write(output, &bytes).map_err(|e| format!("{}: {e}", output.display()))?;
    if json {
        let order = sol.report.worst_first();
        let worst: Vec<_> = order
            .iter()
            .take(10)
            .filter_map(|c| sol.report.cell(*c))
            .collect();
        let v = serde_json::json!({
            "input": input.display().to_string(),
            "output": output.display().to_string(),
            "mode": mode,
            "format": format.name(),
            "background": sol.background,
            "error": sol.error,
            "solve_ms": ms,
            "summary": sol.report.summary(),
            "worst": if report { serde_json::json!(worst) } else { serde_json::Value::Null },
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?
        );
    } else {
        println!(
            "{} -> {} ({}, {ms:.0} ms)",
            input.display(),
            output.display(),
            format.name()
        );
        if report {
            print_summary(&sol);
        }
    }
    Ok(())
}

/// The clash summary as the status bar would show it.
fn print_summary(sol: &convert64_core::solve::Solution) {
    let s = sol.report.summary();
    println!(
        "solved: background {}, error {:.4}, {} clash{}{}",
        sol.background
            .map_or_else(|| "none".to_owned(), |b| b.to_string()),
        s.total_error,
        s.clashes,
        if s.clashes == 1 { "" } else { "es" },
        s.worst
            .map(|c| format!(", worst cell {c}"))
            .unwrap_or_default()
    );
}

fn run_validate(path: &Path, as_json: bool, format: Option<FormatArg>) -> ExitCode {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error: cannot read {}: {e}", path.display());
            return ExitCode::from(2);
        }
    };
    let format = match format {
        Some(FormatArg::Kla) => Some(BitmapFormat::Koala),
        Some(FormatArg::Art) => Some(BitmapFormat::ArtStudio),
        None => path
            .extension()
            .and_then(|e| e.to_str())
            .and_then(BitmapFormat::from_extension)
            .or_else(|| BitmapFormat::from_size(bytes.len())),
    };
    let Some(format) = format else {
        eprintln!(
            "error: cannot tell the format of {}: use a .kla/.koa or .art/.aas file, or --format",
            path.display()
        );
        return ExitCode::from(2);
    };

    let report = check(path.display().to_string(), format, &bytes);
    if as_json {
        println!("{}", json::report(&report));
    } else if report.findings.is_empty() {
        println!("legal");
    } else {
        let n = report.findings.len();
        println!(
            "illegal: {n} violation{} in {} ({}{})",
            if n == 1 { "" } else { "s" },
            report.file,
            report.format,
            report.mode.map(|m| format!(", {m}")).unwrap_or_default()
        );
        for f in &report.findings {
            println!("{}", f.line());
        }
    }
    if report.findings.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

fn check(file: String, format: BitmapFormat, bytes: &[u8]) -> Report {
    let mut findings = Vec::new();
    let mut mode = None;
    let mut push = |v: convert64_core::Violation| {
        findings.push(Finding {
            rule: v.rule.number(),
            location: Some(v.location),
            message: v.message,
        });
    };
    match format.read(bytes) {
        Ok(read) => {
            mode = Some(read.image.mode().name());
            validate(&read.image)
                .err()
                .into_iter()
                .chain(validate_placement(&read.placement()).err())
                .flatten()
                .for_each(&mut push);
        }
        Err(e) => {
            // The header is still meaningful when only the contents are illegal.
            if let ReadError::Model(_) = e {
                let load_address = u16::from_le_bytes([bytes[0], bytes[1]]);
                let placement = Placement::new().with(BlockKind::Bitmap, load_address);
                validate_placement(&placement)
                    .err()
                    .into_iter()
                    .flatten()
                    .for_each(&mut push);
            }
            findings.push(Finding {
                rule: e.rule_number().unwrap_or(6),
                location: None,
                message: e.to_string(),
            });
        }
    }
    findings.sort_by_key(|f| f.rule);
    Report {
        file,
        format,
        mode,
        findings,
    }
}
