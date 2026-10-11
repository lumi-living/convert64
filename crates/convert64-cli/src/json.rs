//! A minimal JSON writer for `--json` output.
//!
//! Hand-written because `serde_json` is not in the dependency list of `docs/TAD.md` §4;
//! the output is small and flat. Replace with `serde_json` when E7 needs project files.

use crate::{Finding, Report};
use convert64_core::Location;
use convert64_core::mode::BlockKind;
use std::fmt::Write as _;

/// `s` as a JSON string literal.
fn string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if u32::from(c) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn block(kind: BlockKind) -> &'static str {
    match kind {
        BlockKind::Bitmap => "bitmap",
        BlockKind::ScreenRam => "screen-ram",
        BlockKind::ColourRam => "colour-ram",
        BlockKind::Background => "background",
        BlockKind::Charset => "charset",
        BlockKind::Sprite => "sprite",
    }
}

fn location(l: Option<Location>) -> String {
    match l {
        None => r#"{"kind":"file"}"#.to_owned(),
        Some(Location::Image) => r#"{"kind":"image"}"#.to_owned(),
        Some(Location::Cell { cell }) => {
            format!(r#"{{"kind":"cell","x":{},"y":{}}}"#, cell.x, cell.y)
        }
        Some(Location::Char { index }) => format!(r#"{{"kind":"char","index":{index}}}"#),
        Some(Location::Sprite { index }) => format!(r#"{{"kind":"sprite","index":{index}}}"#),
        Some(Location::Address { block: b, address }) => format!(
            r#"{{"kind":"address","block":{},"address":{address}}}"#,
            string(block(b))
        ),
    }
}

fn finding(f: &Finding) -> String {
    format!(
        r#"{{"rule":{},"location":{},"message":{}}}"#,
        f.rule,
        location(f.location),
        string(&f.message)
    )
}

/// The whole report as one JSON object.
pub(crate) fn report(r: &Report) -> String {
    let violations: Vec<String> = r.findings.iter().map(finding).collect();
    format!(
        r#"{{"file":{},"format":{},"mode":{},"legal":{},"violations":[{}]}}"#,
        string(&r.file),
        string(r.format.name()),
        r.mode.map_or_else(|| "null".to_owned(), string),
        r.findings.is_empty(),
        violations.join(",")
    )
}

#[cfg(test)]
mod tests {
    use super::{location, string};
    use convert64_core::Location;
    use convert64_core::model::CellPos;

    #[test]
    fn escapes_strings() {
        assert_eq!(string("a\"b\\c\nd\u{1}"), r#""a\"b\\c\nd\u0001""#);
        assert_eq!(string("$1000-$1FFF"), r#""$1000-$1FFF""#);
    }

    #[test]
    fn locations() {
        assert_eq!(location(None), r#"{"kind":"file"}"#);
        assert_eq!(
            location(Some(Location::Cell {
                cell: CellPos::new(3, 24)
            })),
            r#"{"kind":"cell","x":3,"y":24}"#
        );
    }
}
