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
    pub(crate) format: &'static str,
    pub(crate) mode: Option<&'static str>,
    pub(crate) findings: Vec<Finding>,
}

/// Runs `validate`: 0 when legal, [`NOT_LEGAL`] when any rule is broken.
pub(crate) fn run(run: &mut Run, path: &Path, format: Option<ReadFormat>) -> Result<u8, Fail> {
    run.set("file", path.display().to_string());
    let bytes = std::fs::read(path)
        .map_err(|e| Fail::input(format!("cannot read {}: {e}", path.display())))?;
    let format = match format {
        Some(ReadFormat::Kla) => Some(BitmapFormat::Koala),
        Some(ReadFormat::Art) => Some(BitmapFormat::ArtStudio),
        None => path
            .extension()
            .and_then(|e| e.to_str())
            .and_then(BitmapFormat::from_extension)
            .or_else(|| BitmapFormat::from_size(bytes.len())),
    };
    let Some(format) = format else {
        return Err(Fail::input(format!(
            "cannot tell the format of {}: use a .kla/.koa or .art/.aas file, or --format",
            path.display()
        )));
    };
    let report = run.time("validate", || {
        check(path.display().to_string(), format, &bytes)
    });
    finish(run, &report)
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
        format: format.name(),
        mode,
        findings,
    }
}
