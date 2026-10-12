//! `validate`'s fields in the `--json` report, serialised with `serde_json`.

use crate::report::Run;
use crate::validate::{Finding, Report};
use convert64_core::Location;
use serde::Serialize;

#[derive(Serialize)]
struct JsonFinding<'a> {
    rule: u8,
    location: JsonLocation,
    message: &'a str,
}

/// Where a finding is: the file as a whole (`{"kind":"file"}`), or a validator
/// [`Location`] (`{"kind":"cell","x":…,"y":…}`, `{"kind":"address",…}`, …).
#[derive(Serialize)]
#[serde(untagged)]
enum JsonLocation {
    File { kind: &'static str },
    At(Location),
}

/// Sets `file`, `format`, `mode`, `legal` and `violations`.
pub(crate) fn set_report(run: &mut Run, r: &Report) {
    run.set("file", &r.file);
    run.set("format", r.format);
    run.set("mode", r.mode);
    run.set("legal", r.findings.is_empty());
    let violations: Vec<_> = r
        .findings
        .iter()
        .map(|f: &Finding| JsonFinding {
            rule: f.rule,
            location: f
                .location
                .map_or(JsonLocation::File { kind: "file" }, JsonLocation::At),
            message: &f.message,
        })
        .collect();
    run.set("violations", violations);
}

#[cfg(test)]
mod tests {
    use super::JsonLocation;
    use convert64_core::Location;
    use convert64_core::model::CellPos;

    #[test]
    fn locations_serialise_flat() {
        let file = serde_json::to_value(JsonLocation::File { kind: "file" }).expect("json");
        assert_eq!(file, serde_json::json!({"kind": "file"}));
        let cell = serde_json::to_value(JsonLocation::At(Location::Cell {
            cell: CellPos::new(3, 24),
        }))
        .expect("json");
        assert_eq!(cell, serde_json::json!({"kind": "cell", "x": 3, "y": 24}));
    }
}
