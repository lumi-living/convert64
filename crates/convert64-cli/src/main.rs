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
    },
    /// List the commands a recipe can contain.
    Commands,
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
        Command::Recipe { action } => match action {
            RecipeAction::Apply {
                file,
                masks,
                composite,
                attributes,
            } => match run_recipe(&file, &masks, composite.as_deref(), attributes.as_deref()) {
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
    Ok(())
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
