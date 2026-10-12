//! `convert64 export`: solve a project, or every project in a directory, and write
//! assembler source (KickAssembler, ACME, 64tass, ca65) or a C header with its data.

use crate::convert::{self, Overrides};
use crate::report::{Fail, Run};
use clap::ValueEnum;
use convert64_formats::asm::{self, Assembler, DataStyle, EmitOptions};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// How the data is written, on the command line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub(crate) enum DataArg {
    /// Byte lines in the source.
    Bytes,
    /// Include directives for the raw set's block files, written beside the source.
    #[default]
    IncludeRaw,
    /// One include of the Koala or Art Studio file, written beside the source.
    IncludePicture,
}

impl DataArg {
    const fn style(self) -> DataStyle {
        match self {
            Self::Bytes => DataStyle::Bytes,
            Self::IncludeRaw => DataStyle::IncludeRaw,
            Self::IncludePicture => DataStyle::IncludePicture,
        }
    }
}

/// What `export` was asked to do.
pub(crate) struct ExportJob<'a> {
    pub(crate) input: &'a Path,
    pub(crate) asm: &'a [String],
    pub(crate) data: DataArg,
    pub(crate) out: &'a Path,
    pub(crate) name: Option<&'a str>,
    pub(crate) max_clashes: Option<usize>,
    pub(crate) overrides: &'a Overrides,
}

/// The assemblers named: ids (`kickass`, `acme`, `64tass`, `ca65`, `c`) or `all`.
fn assemblers(names: &[String]) -> Result<Vec<Assembler>, Fail> {
    let mut out = Vec::new();
    for n in names {
        let found: Vec<Assembler> = if n.eq_ignore_ascii_case("all") {
            Assembler::ALL.to_vec()
        } else {
            vec![Assembler::from_id(n).ok_or_else(|| {
                Fail::input(format!(
                    "unknown assembler {n:?}: use kickass, acme, 64tass, ca65, c or all"
                ))
            })?]
        };
        for a in found {
            if !out.contains(&a) {
                out.push(a);
            }
        }
    }
    if out.is_empty() {
        return Err(Fail::input("name an assembler with --asm"));
    }
    Ok(out)
}

/// The projects to export: `input` itself, or every `.convert64` in it.
fn projects(input: &Path) -> Result<Vec<PathBuf>, Fail> {
    if !input.is_dir() {
        return Ok(vec![input.to_owned()]);
    }
    let mut found: Vec<PathBuf> = std::fs::read_dir(input)
        .map_err(|e| Fail::input(format!("{}: {e}", input.display())))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case(convert64_cli::project::EXTENSION))
        })
        .collect();
    found.sort();
    if found.is_empty() {
        return Err(Fail::input(format!(
            "{}: no .convert64 projects in it",
            input.display()
        )));
    }
    Ok(found)
}

/// Runs `export`: the worst project's exit code.
pub(crate) fn run(run: &mut Run, job: &ExportJob<'_>) -> Result<u8, Fail> {
    run.set("input", job.input.display().to_string());
    let targets = assemblers(job.asm)?;
    run.set(
        "assemblers",
        targets.iter().map(|a| a.id()).collect::<Vec<_>>(),
    );
    let list = projects(job.input)?;
    if job.name.is_some() && list.len() > 1 {
        return Err(Fail::input("--name applies to a single project"));
    }
    std::fs::create_dir_all(job.out).map_err(|e| Fail::write(job.out, e))?;
    let mut worst = 0;
    let mut reports = Vec::with_capacity(list.len());
    for project in &list {
        let name = job.name.map_or_else(
            || {
                project.file_stem().map_or_else(
                    || "picture".to_owned(),
                    |s| s.to_string_lossy().into_owned(),
                )
            },
            str::to_owned,
        );
        let mut r = Run::new("export", true);
        let code = match export_one(&mut r, job, project, &name, &targets) {
            Ok(c) => c,
            Err(f) => {
                eprintln!("error: {}: {}", project.display(), f.message);
                let v = r.to_value(f.code, Some(&f.message));
                worst = worst.max(f.code);
                reports.push(v);
                continue;
            }
        };
        worst = worst.max(code);
        let v = r.to_value(code, None);
        for f in v["files_written"].as_array().into_iter().flatten() {
            if let Some(f) = f.as_str() {
                run.wrote(Path::new(f));
            }
        }
        run.say(format!(
            "{} -> {} ({})",
            project.display(),
            job.out.display(),
            targets
                .iter()
                .map(|a| a.id())
                .collect::<Vec<_>>()
                .join(", ")
        ));
        reports.push(v);
    }
    run.set("projects", reports);
    Ok(worst)
}

fn export_one(
    r: &mut Run,
    job: &ExportJob<'_>,
    project: &Path,
    name: &str,
    targets: &[Assembler],
) -> Result<u8, Fail> {
    r.set("input", project.display().to_string());
    let (mut session, _) = convert::load(r, project, job.overrides, None)?;
    r.set("mode", session.target());
    let sol = r.time("solve", || session.solve(true).clone());
    let opts = EmitOptions::new(name).with_data(job.data.style());
    let mut sources = Vec::new();
    let mut written: Vec<PathBuf> = Vec::new();
    for &a in targets {
        let files = asm::emit(a, &sol.image, &opts).map_err(Fail::input)?;
        for (i, (file, bytes)) in files.iter().enumerate() {
            let path = job.out.join(file);
            if i == 0 {
                sources.push(json!({ "assembler": a.id(), "path": path.display().to_string() }));
            }
            // Includes are the same bytes for every assembler: write them once.
            if !written.contains(&path) {
                r.write(&path, bytes)?;
                written.push(path);
            }
        }
    }
    r.set("sources", Value::Array(sources));
    convert::finish_solve(r, &sol, false, job.max_clashes)
}
