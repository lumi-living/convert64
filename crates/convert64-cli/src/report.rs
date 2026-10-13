//! What every command reports: an exit code, and with `--json` one JSON object on stdout.
//!
//! Exit codes (documented in `docs/reference/cli.md`):
//!
//! | Code | Meaning |
//! |---|---|
//! | 0 | Success |
//! | 1 | The result is not legal VIC-II data, or it has more clashes than `--max-clashes` |
//! | 2 | An input cannot be read or an option is invalid |
//! | 3 | An output cannot be written |
//!
//! Every JSON report has `command`, `ok`, `exit_code`, `error` (a message or `null`),
//! `timing_ms` (`total` plus the phases the command ran), `files_written` and `notes`
//! (what loading changed, such as an older project's migration, #236), then the
//! command's own fields.

use serde::Serialize;
use serde_json::{Map, Value, json};
use std::path::Path;
use std::process::ExitCode;
use std::time::Instant;

/// Exit code 1: not legal, or too many clashes.
pub(crate) const NOT_LEGAL: u8 = 1;
/// Exit code 2: an input or option error.
pub(crate) const INPUT_ERROR: u8 = 2;
/// Exit code 3: a write error.
pub(crate) const WRITE_ERROR: u8 = 3;

/// Why a command stopped.
#[derive(Debug)]
pub(crate) struct Fail {
    pub(crate) code: u8,
    pub(crate) message: String,
}

impl Fail {
    /// An input or option error (exit 2).
    pub(crate) fn input(e: impl std::fmt::Display) -> Self {
        Self {
            code: INPUT_ERROR,
            message: e.to_string(),
        }
    }

    /// A write error (exit 3) on `path`.
    pub(crate) fn write(path: &Path, e: impl std::fmt::Display) -> Self {
        Self {
            code: WRITE_ERROR,
            message: format!("{}: {e}", path.display()),
        }
    }
}

/// One command's run: its report fields, the files it wrote and its timings.
pub(crate) struct Run {
    command: &'static str,
    json: bool,
    started: Instant,
    fields: Map<String, Value>,
    files: Vec<String>,
    timing: Map<String, Value>,
    notes: Vec<String>,
}

impl Run {
    /// Starts the clock for `command`.
    pub(crate) fn new(command: &'static str, json: bool) -> Self {
        Self {
            command,
            json,
            started: Instant::now(),
            fields: Map::new(),
            files: Vec::new(),
            timing: Map::new(),
            notes: Vec::new(),
        }
    }

    /// Records what loading `session` changed (an older project's migration, #236) and
    /// prints it once on stderr, unless the report is JSON, where it is in `notes`.
    pub(crate) fn notes_of(&mut self, session: &convert64_project::Session) {
        for n in session.notes() {
            if !self.json {
                eprintln!("note: {n}");
            }
            self.notes.push(n.clone());
        }
    }

    /// Prints the `notes` of a per-file `report` (batch, export) on stderr, prefixed by
    /// `name`, unless the report is JSON, where they are in that file's report.
    pub(crate) fn echo_notes(&self, name: &str, report: &Value) {
        if self.json {
            return;
        }
        for n in report["notes"].as_array().into_iter().flatten() {
            if let Some(n) = n.as_str() {
                eprintln!("{name}: note: {n}");
            }
        }
    }

    /// Sets a report field.
    pub(crate) fn set(&mut self, key: &str, value: impl Serialize) {
        let v = serde_json::to_value(value).unwrap_or_else(|e| unreachable!("plain data: {e}"));
        self.fields.insert(key.to_owned(), v);
    }

    /// Runs `f` and records how long it took under `phase` (added up if repeated).
    pub(crate) fn time<T>(&mut self, phase: &str, f: impl FnOnce() -> T) -> T {
        let t = Instant::now();
        let out = f();
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        let before = self
            .timing
            .get(phase)
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        self.timing.insert(phase.to_owned(), json!(before + ms));
        out
    }

    /// Writes `bytes` to `path` and records it.
    pub(crate) fn write(&mut self, path: &Path, bytes: &[u8]) -> Result<(), Fail> {
        self.time("write", || std::fs::write(path, bytes))
            .map_err(|e| Fail::write(path, e))?;
        self.files.push(path.display().to_string());
        Ok(())
    }

    /// Records a file written by someone else.
    pub(crate) fn wrote(&mut self, path: &Path) {
        self.files.push(path.display().to_string());
    }

    /// Prints a human-readable line, unless the report is JSON.
    pub(crate) fn say(&self, line: impl std::fmt::Display) {
        if !self.json {
            println!("{line}");
        }
    }

    /// The report as JSON.
    pub(crate) fn to_value(&self, code: u8, error: Option<&str>) -> Value {
        let mut timing = self.timing.clone();
        timing.insert(
            "total".to_owned(),
            json!(self.started.elapsed().as_secs_f64() * 1000.0),
        );
        let mut v = Map::new();
        v.insert("command".to_owned(), json!(self.command));
        v.insert("ok".to_owned(), json!(code == 0));
        v.insert("exit_code".to_owned(), json!(code));
        v.insert("error".to_owned(), json!(error));
        v.insert("timing_ms".to_owned(), Value::Object(timing));
        v.insert("files_written".to_owned(), json!(self.files));
        v.insert("notes".to_owned(), json!(self.notes));
        for (k, f) in &self.fields {
            v.insert(k.clone(), f.clone());
        }
        Value::Object(v)
    }

    /// Ends the run: prints the error (stderr) and the JSON report (stdout, with
    /// `--json`), and returns the exit code. `Ok(code)` is 0 or [`NOT_LEGAL`].
    pub(crate) fn finish(self, result: Result<u8, Fail>) -> ExitCode {
        let (code, error) = match result {
            Ok(code) => (code, None),
            Err(f) => {
                eprintln!("error: {}", f.message);
                (f.code, Some(f.message))
            }
        };
        if self.json {
            println!(
                "{}",
                serde_json::to_string_pretty(&self.to_value(code, error.as_deref()))
                    .unwrap_or_else(|e| unreachable!("plain data: {e}"))
            );
        }
        ExitCode::from(code)
    }
}
