//! `convert64 validate`: checks a picture file against the VIC-II rules of
//! `docs/reference/vic-ii-constraints.md` §6.

use crate::json;
use crate::report::{Fail, NOT_LEGAL, Run};
use clap::ValueEnum;
use convert64_core::mode::BlockKind;
use convert64_core::{Location, Placement, validate, validate_placement};
use convert64_formats::{BitmapFormat, ReadError};
use std::path::Path;

/// A format `validate` reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum ReadFormat {
    /// Koala Painter.
    Kla,
    /// Art Studio.
    Art,
    /// A self-displaying PRG written by convert64 (Koala or Art Studio inside).
    Prg,
    /// A .convert64 project (or version-1 recipe): loaded, solved and checked.
    Project,
    /// A raw set's JSON manifest, with its block files beside it.
    Raw,
}

impl ReadFormat {
    /// The format a path's extension, or failing that a picture file's size, names.
    fn guess(path: &Path, bytes: &[u8]) -> Option<Self> {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if ext.eq_ignore_ascii_case("prg") {
            return Some(Self::Prg);
        }
        if ext.eq_ignore_ascii_case("json") {
            // A raw set's manifest names its format; anything else is a recipe.
            let raw = serde_json::from_slice::<serde_json::Value>(bytes)
                .is_ok_and(|v| v["format"] == convert64_formats::raw::FORMAT_ID);
            return Some(if raw { Self::Raw } else { Self::Project });
        }
        if ext.eq_ignore_ascii_case(convert64_project::EXTENSION) {
            return Some(Self::Project);
        }
        BitmapFormat::from_extension(ext)
            .or_else(|| BitmapFormat::from_size(bytes.len()))
            .map(|f| match f {
                BitmapFormat::Koala => Self::Kla,
                BitmapFormat::ArtStudio => Self::Art,
            })
    }
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
    pub(crate) format: String,
    pub(crate) mode: Option<&'static str>,
    pub(crate) findings: Vec<Finding>,
}

/// Runs `validate`: 0 when legal, [`NOT_LEGAL`] when any rule is broken.
pub(crate) fn run(run: &mut Run, path: &Path, format: Option<ReadFormat>) -> Result<u8, Fail> {
    run.set("file", path.display().to_string());
    let bytes = std::fs::read(path)
        .map_err(|e| Fail::input(format!("cannot read {}: {e}", path.display())))?;
    let Some(format) = format.or_else(|| ReadFormat::guess(path, &bytes)) else {
        return Err(Fail::input(format!(
            "cannot tell the format of {}: use a .kla/.koa, .art/.aas, .prg or .convert64 \
             file, or --format",
            path.display()
        )));
    };
    let file = path.display().to_string();
    let report = match format {
        ReadFormat::Kla => run.time("validate", || check(file, BitmapFormat::Koala, &bytes)),
        ReadFormat::Art => run.time("validate", || check(file, BitmapFormat::ArtStudio, &bytes)),
        ReadFormat::Prg => {
            let (inner, payload) = display_prg_payload(&bytes)
                .ok_or_else(|| Fail::input(format!("{file}: not a convert64 display PRG")))?;
            let mut r = run.time("validate", || check(file, inner, payload));
            r.format = format!("display PRG ({})", r.format);
            r
        }
        ReadFormat::Raw => {
            let dir = path.parent().map(Path::to_owned).unwrap_or_default();
            run.time("validate", || raw_report(file, &dir, &bytes))
        }
        ReadFormat::Project => {
            let mut session = run.time("load", || {
                convert64_project::Session::open(path).map_err(Fail::input)
            })?;
            run.notes_of(&session);
            let sol = run.time("solve", || session.solve(true).clone());
            let s2 = run.time("ssimulacra2", || session.ssimulacra2());
            run.set("ssimulacra2", s2);
            run.set("clashes", sol.report.summary().clashes);
            run.time("validate", || Report {
                file,
                format: "convert64 project".to_owned(),
                mode: Some(sol.image.mode().name()),
                findings: violations(validate(&sol.image)),
            })
        }
    };
    finish(run, &report)
}

/// The format and embedded picture file of a display PRG written by
/// `convert64_formats::prg`: everything before the picture must be that writer's
/// header and loader for the picture's format, byte for byte.
fn display_prg_payload(bytes: &[u8]) -> Option<(BitmapFormat, &[u8])> {
    use convert64_formats::prg::{PAYLOAD_OFFSET, display_prg};
    let payload = bytes.get(PAYLOAD_OFFSET..)?;
    let format = BitmapFormat::from_size(payload.len())?;
    // The loader depends on the format only, so any legal picture shows what it is.
    let mut blank = format.default_load_address().to_le_bytes().to_vec();
    blank.resize(format.sizes()[0], 0);
    let header = display_prg(format, &blank).ok()?;
    (bytes[..PAYLOAD_OFFSET] == header[..PAYLOAD_OFFSET]).then_some((format, payload))
}

/// Reads a raw set (its block files from `dir`) and validates the picture; a block of
/// the wrong size is rule 6, a background above 15 rule 2.
fn raw_report(file: String, dir: &Path, manifest: &[u8]) -> Report {
    use convert64_formats::raw;
    match raw::read(manifest, |f| std::fs::read(dir.join(f)).ok()) {
        Ok(set) => Report {
            file,
            format: "raw set".to_owned(),
            mode: Some(set.image.mode().name()),
            findings: violations(validate(&set.image)),
        },
        Err(e) => Report {
            file,
            format: "raw set".to_owned(),
            mode: None,
            findings: vec![Finding {
                rule: e.rule_number().unwrap_or(6),
                location: None,
                message: e.to_string(),
            }],
        },
    }
}

fn violations(r: Result<(), Vec<convert64_core::Violation>>) -> Vec<Finding> {
    r.err()
        .into_iter()
        .flatten()
        .map(|v| Finding {
            rule: v.rule.number(),
            location: Some(v.location),
            message: v.message,
        })
        .collect()
}

/// Reports `report`'s findings: JSON fields, or the human-readable lines.
pub(crate) fn finish(run: &mut Run, report: &Report) -> Result<u8, Fail> {
    json::set_report(run, report);
    if report.findings.is_empty() {
        run.say("legal");
        return Ok(0);
    }
    let n = report.findings.len();
    run.say(format!(
        "illegal: {n} violation{} in {} ({}{})",
        if n == 1 { "" } else { "s" },
        report.file,
        report.format,
        report.mode.map(|m| format!(", {m}")).unwrap_or_default()
    ));
    for f in &report.findings {
        run.say(f.line());
    }
    Ok(NOT_LEGAL)
}

/// Reads `bytes` as `format` and validates the picture and its load address.
pub(crate) fn check(file: String, format: BitmapFormat, bytes: &[u8]) -> Report {
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
        format: format.name().to_owned(),
        mode,
        findings,
    }
}
