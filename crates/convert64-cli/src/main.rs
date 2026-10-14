//! The `convert64` binary: convert, validate, export and batch from the command line (`docs/TAD.md §4`).
//!
//! `convert64 convert` solves a PNG or a `.convert64` project into every M1 output
//! format, `convert64 validate` checks a picture against the rules in
//! `docs/reference/vic-ii-constraints.md` §6, `convert64 apply` runs any registry command
//! on a project (ADR-005) and `convert64 commands` lists them. Every command takes `--json` and exits 0 (ok),
//! 1 (not legal, or too many clashes), 2 (input or option error) or 3 (write error);
//! `docs/reference/cli.md` documents them.

mod apply;
mod batch;
mod convert;
mod export;
mod json;
mod report;
mod validate;

use clap::{Parser, Subcommand};
use convert::{ConvertJob, FormatArg, Overrides};
use convert64_project::{Embed, LayerRef, ProjectError, Session};
use report::{Fail, Run};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "convert64",
    version,
    about = "Legal Commodore 64 graphics from layered images",
    after_help = "Exit codes: 0 ok, 1 not legal or more clashes than --max-clashes, \
                  2 input or option error, 3 write error."
)]
struct Cli {
    /// Print one JSON report on stdout: command, ok, exit_code, error, timing_ms,
    /// files_written and the command's own fields.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

/// How a saved project stores its sources.
#[derive(clap::Args, Clone, Copy, Debug, Default)]
struct EmbedArgs {
    /// Embed every layer's source PNG in the saved project.
    #[arg(long, conflicts_with = "link")]
    embed: bool,
    /// Link every layer's source by path (fails for embedded sources).
    #[arg(long)]
    link: bool,
}

impl EmbedArgs {
    const fn embed(self) -> Embed {
        match (self.embed, self.link) {
            (true, _) => Embed::All,
            (_, true) => Embed::None,
            _ => Embed::Keep,
        }
    }
}

#[derive(Subcommand)]
enum Command {
    /// Convert a PNG or a .convert64 project to legal C64 files.
    ///
    /// A PNG is scaled to fill the 320×200 screen with the default recipe; a project
    /// (or a version-1 recipe) uses its own settings. Options override either. The
    /// picture is composited at the supersample factor, solved (background search and
    /// refinement) and written in every -o's format: .kla (multicolor), .art (hi-res)
    /// or .prg (a self-displaying program). Exit codes: 0 ok; 1 not legal or more
    /// clashes than --max-clashes (the files are still written); 2 input or option
    /// error; 3 write error.
    Convert {
        /// The source: a PNG, a .convert64 project or a version-1 recipe (JSON).
        input: PathBuf,
        /// Output file; repeatable. The extension names the format (.kla/.koa, .art/.aas,
        /// .prg, .png preview, .json raw set) unless --format is given.
        #[arg(short, long = "output", value_name = "PATH")]
        output: Vec<PathBuf>,
        /// Output format, for an -o whose extension says nothing.
        #[arg(long, value_enum)]
        format: Option<FormatArg>,
        /// Exit 1 when the picture has more clashing cells than this.
        #[arg(long, value_name = "N")]
        max_clashes: Option<usize>,
        /// Also save the project (with the overrides applied) as a .convert64 file.
        #[arg(long, value_name = "PATH")]
        save_project: Option<PathBuf>,
        #[command(flatten)]
        embed: EmbedArgs,
        #[command(flatten)]
        overrides: Overrides,
        /// Print the clash summary (and, with --json, the ten worst cells).
        #[arg(long)]
        report: bool,
        /// Zoom of .png previews, 1–8 (multicolor pixels are twice as wide).
        #[arg(long, default_value_t = 1, value_name = "N")]
        zoom: u8,
    },
    /// Check a picture file against the VIC-II rules.
    ///
    /// Prints `legal` and exits 0, or lists every violation and exits 1. Exits 2 when
    /// the file cannot be read or its format cannot be determined.
    Validate {
        /// A Koala (.kla, .koa) or Art Studio (.art, .aas) picture.
        file: PathBuf,
        /// Read the file as this format instead of guessing from extension and size.
        #[arg(long, value_enum)]
        format: Option<validate::ReadFormat>,
    },
    /// Convert every PNG and .convert64 project in a directory, in parallel.
    ///
    /// Per-file settings come from a TOML manifest (--manifest, or DIR/batch.toml when
    /// it exists): `out`, `[defaults]` and `[files."name"]` tables holding convert's
    /// options in kebab-case, plus `outputs` (formats, e.g. ["kla", "prg"]),
    /// `max-clashes` and `skip`. Options given here override the manifest. A PNG with a
    /// project of the same name next to it is skipped: the project is converted. One
    /// report covers every file; the exit code is the worst file's.
    Batch {
        /// The directory.
        dir: PathBuf,
        /// The TOML manifest (default: DIR/batch.toml, if present).
        #[arg(long, value_name = "FILE")]
        manifest: Option<PathBuf>,
        /// Output directory (default: the manifest's `out`, else DIR).
        #[arg(long, value_name = "DIR")]
        out: Option<PathBuf>,
        /// Files converted at once (default: one per CPU).
        #[arg(short, long, value_name = "N")]
        jobs: Option<usize>,
        #[command(flatten)]
        overrides: Overrides,
    },
    /// Solve a project, or every project in a directory, and write assembler source.
    ///
    /// For each project and each --asm target: `<name>.asm` (KickAssembler), `.a` (ACME),
    /// `.tas` (64tass), `.s` (ca65) or `.h` (C), plus the binaries it includes, in
    /// --out. Options override the projects' settings. Exit codes as for convert; the
    /// worst project's wins.
    Export {
        /// A .convert64 project, or a directory of them.
        input: PathBuf,
        /// Targets: kickass, acme, 64tass, ca65, c, or all. Repeatable or comma-separated.
        #[arg(long, value_delimiter = ',', required = true, value_name = "TARGET")]
        asm: Vec<String>,
        /// How the data is written: include-raw (the raw set's .bin files, the
        /// default), include-picture (the .kla/.art file) or bytes (in the source).
        #[arg(long, value_enum, default_value = "include-raw")]
        data: export::DataArg,
        /// Output directory (created if missing).
        #[arg(long, value_name = "DIR")]
        out: PathBuf,
        /// Base name of the files and labels (default: the project's file name; one
        /// project only).
        #[arg(long)]
        name: Option<String>,
        /// Exit 1 when a picture has more clashing cells than this.
        #[arg(long, value_name = "N")]
        max_clashes: Option<usize>,
        #[command(flatten)]
        overrides: Overrides,
    },
    /// Run registry commands on a project and save it.
    ///
    /// Every command in the core's registry (`convert64 commands` lists them) runs here,
    /// exactly as the app runs it. Commands come from --commands FILE (a JSON array,
    /// paths in it relative to the file) and then from each --command (paths relative
    /// to the working directory), in order. Without --save or --in-place nothing is
    /// written: the commands are only checked. Exits 0 on success, 2 when the project
    /// cannot be read or a command fails (nothing is written then), 3 when the project
    /// cannot be written.
    Apply {
        /// The project (.convert64) or version-1 recipe.
        file: PathBuf,
        /// A command as JSON: {"command": "<name>", ...arguments}. Repeatable.
        #[arg(long = "command", value_name = "JSON")]
        command: Vec<String>,
        /// A JSON file holding an array of commands, run before any --command.
        #[arg(long = "commands", value_name = "FILE")]
        commands_file: Option<PathBuf>,
        /// Save the result as a .convert64 project.
        #[arg(short = 'o', long, value_name = "PATH")]
        save: Option<PathBuf>,
        /// Save the result over the input file.
        #[arg(long, conflicts_with = "save")]
        in_place: bool,
        #[command(flatten)]
        embed: EmbedArgs,
    },
    /// List every registry command with its argument schema.
    ///
    /// With --json, each command's JSON Schema and a working example.
    Commands,
    /// Work with recipe and project files: layers plus commands, in JSON.
    Recipe {
        #[command(subcommand)]
        action: RecipeAction,
    },
}

#[derive(Subcommand)]
enum RecipeAction {
    /// Load a recipe's (or a .convert64 project's) layers, run its commands and write
    /// the results.
    ///
    /// Exits 0 on success, 1 when --report finds the result illegal, 2 when the file
    /// cannot be read or a command fails, 3 when an output cannot be written.
    Apply {
        /// The recipe file (JSON, version 1) or project file (.convert64).
        file: PathBuf,
        /// Save the result as a .convert64 project (the commands' results, no commands).
        #[arg(long, value_name = "PATH")]
        save: Option<PathBuf>,
        #[command(flatten)]
        embed: EmbedArgs,
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
        /// Solve the picture and print the clash summary (with --json, the ten worst
        /// cells too).
        #[arg(long)]
        report: bool,
    },
    /// List the commands a recipe can contain.
    Commands,
}

fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => return usage_error(&e),
    };
    let json = cli.json;
    match cli.command {
        Command::Convert {
            input,
            output,
            format,
            max_clashes,
            save_project,
            embed,
            overrides,
            report,
            zoom,
        } => {
            let mut run = Run::new("convert", json);
            let job = ConvertJob {
                input: &input,
                outputs: &output,
                format,
                overrides: &overrides,
                max_clashes,
                save_project: save_project.as_deref().map(|p| (p, embed.embed())),
                report,
                zoom,
            };
            let r = convert::run(&mut run, &job);
            run.finish(r)
        }
        Command::Validate { file, format } => {
            let mut run = Run::new("validate", json);
            let r = validate::run(&mut run, &file, format);
            run.finish(r)
        }
        Command::Recipe { action } => match action {
            RecipeAction::Apply {
                file,
                save,
                embed,
                masks,
                composite,
                attributes,
                report,
            } => {
                let mut run = Run::new("recipe apply", json);
                let outputs = RecipeOutputs {
                    save: save.as_deref().map(|p| (p, embed.embed())),
                    masks: &masks,
                    composite: composite.as_deref(),
                    attributes: attributes.as_deref(),
                    report,
                };
                let r = run_recipe(&mut run, &file, &outputs);
                run.finish(r)
            }
            RecipeAction::Commands => {
                let mut run = Run::new("recipe commands", json);
                apply::list(&mut run);
                run.finish(Ok(0))
            }
        },
        Command::Apply {
            file,
            command,
            commands_file,
            save,
            in_place,
            embed,
        } => {
            let mut run = Run::new("apply", json);
            let save = if in_place { Some(file.clone()) } else { save };
            let job = apply::ApplyJob {
                file: &file,
                commands: &command,
                commands_file: commands_file.as_deref(),
                save: save.as_deref().map(|p| (p, embed.embed())),
            };
            let r = apply::run(&mut run, &job);
            run.finish(r)
        }
        Command::Export {
            input,
            asm,
            data,
            out,
            name,
            max_clashes,
            overrides,
        } => {
            let mut run = Run::new("export", json);
            let job = export::ExportJob {
                input: &input,
                asm: &asm,
                data,
                out: &out,
                name: name.as_deref(),
                max_clashes,
                overrides: &overrides,
            };
            let r = export::run(&mut run, &job);
            run.finish(r)
        }
        Command::Batch {
            dir,
            manifest,
            out,
            jobs,
            overrides,
        } => {
            let mut run = Run::new("batch", json);
            let job = batch::BatchJob {
                dir: &dir,
                manifest: manifest.as_deref(),
                out: out.as_deref(),
                jobs,
                overrides: &overrides,
            };
            let r = batch::run(&mut run, &job);
            run.finish(r)
        }
        Command::Commands => {
            let mut run = Run::new("commands", json);
            apply::list(&mut run);
            run.finish(Ok(0))
        }
    }
}

/// A command line clap rejects: its message and exit 2, or with `--json` anywhere on the
/// line, a JSON report too. `--help` and `--version` print and exit 0.
fn usage_error(e: &clap::Error) -> ExitCode {
    use clap::error::ErrorKind;
    if matches!(e.kind(), ErrorKind::DisplayHelp | ErrorKind::DisplayVersion) {
        let _ = e.print();
        return ExitCode::SUCCESS;
    }
    let json = std::env::args_os().any(|a| a == "--json");
    let _ = e.print();
    if json {
        let run = Run::new("usage", true);
        let message = e
            .kind()
            .as_str()
            .unwrap_or("invalid command line")
            .to_owned();
        println!(
            "{}",
            serde_json::to_string_pretty(&run.to_value(report::INPUT_ERROR, Some(&message)))
                .unwrap_or_else(|e| unreachable!("plain data: {e}"))
        );
    }
    ExitCode::from(report::INPUT_ERROR)
}

/// A library error while writing outputs: file errors are write errors (3), the rest
/// input errors (2).
fn output_fail(e: ProjectError) -> Fail {
    match e {
        ProjectError::Io(path, msg) => Fail::write(&path, msg),
        other => Fail::input(other),
    }
}

/// What `recipe apply` writes.
struct RecipeOutputs<'a> {
    save: Option<(&'a Path, Embed)>,
    masks: &'a [String],
    composite: Option<&'a Path>,
    attributes: Option<&'a Path>,
    report: bool,
}

fn run_recipe(run: &mut Run, file: &Path, o: &RecipeOutputs<'_>) -> Result<u8, Fail> {
    run.set("file", file.display().to_string());
    let mut session = run.time("load", || Session::open(file).map_err(Fail::input))?;
    run.notes_of(&session);
    let layers = session.project().layers.len();
    let applied = session.editor_mut().undo_len();
    run.set("layers", layers);
    run.set("commands_applied", applied);
    run.say(format!(
        "{}: {layers} layer(s), {applied} command(s) applied",
        file.display(),
    ));
    if let Some((path, embed)) = o.save {
        let dir = path.parent().unwrap_or(Path::new(""));
        let text = session
            .to_project_file(dir, embed)
            .map_err(Fail::input)?
            .to_json();
        run.write(path, text.as_bytes())?;
        run.say(format!("project -> {}", path.display()));
    }
    for m in o.masks {
        let (layer, path) = m
            .split_once('=')
            .ok_or_else(|| Fail::input(format!("--mask {m}: expected LAYER=PATH")))?;
        let r = layer
            .parse()
            .map_or_else(|_| LayerRef::Name(layer.to_owned()), LayerRef::Index);
        let i = session.resolve(&r).map_err(Fail::input)?;
        session
            .write_mask(i, Path::new(path))
            .map_err(output_fail)?;
        run.wrote(Path::new(path));
        run.say(format!("mask of layer {i} -> {path}"));
    }
    if let Some(p) = o.composite {
        session.write_composite(p).map_err(output_fail)?;
        run.wrote(p);
        run.say(format!("composite -> {}", p.display()));
    }
    if let Some(p) = o.attributes {
        session.write_attributes(p).map_err(output_fail)?;
        run.wrote(p);
        run.say(format!("attribute map -> {}", p.display()));
    }
    if o.report {
        let sol = run.time("solve", || session.solve(true).clone());
        let s2 = run.time("ssimulacra2", || session.ssimulacra2());
        return convert::finish_solve(run, &sol, s2, true, None);
    }
    Ok(0)
}
