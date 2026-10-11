//! `--json` output, serialised with `serde_json`.

use crate::{Finding, Report};
use convert64_core::Location;
use serde::Serialize;

#[derive(Serialize)]
struct JsonReport<'a> {
    file: &'a str,
    format: &'static str,
    mode: Option<&'static str>,
    legal: bool,
    violations: Vec<JsonFinding<'a>>,
}

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

/// The whole report as pretty-printed JSON.
pub(crate) fn report(r: &Report) -> String {
    let json = JsonReport {
        file: &r.file,
        format: r.format.name(),
        mode: r.mode,
        legal: r.findings.is_empty(),
        violations: r
            .findings
            .iter()
            .map(|f: &Finding| JsonFinding {
                rule: f.rule,
                location: f
                    .location
                    .map_or(JsonLocation::File { kind: "file" }, JsonLocation::At),
                message: &f.message,
            })
            .collect(),
    };
    serde_json::to_string_pretty(&json).unwrap_or_else(|e| unreachable!("plain data: {e}"))
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
