//! Parser for VICE `.vpl` palette files (syntax in `docs/reference/file-formats.md`).
//!
//! Takes bytes, never a path: the core does no I/O. Behaviour follows VICE's own loader
//! (`palette_load_core` in `src/palette.c`), except that text after the colour values
//! other than an optional dither nybble is an error here rather than a warning.

use super::{Palette, Rgb8};
use std::fmt;

/// Why a `.vpl` file could not be read. Line numbers are 1-based.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VplError {
    /// An entry line ended before its red, green or blue value.
    MissingField {
        /// Line number.
        line: usize,
        /// Which value was missing: `"red"`, `"green"` or `"blue"`.
        field: &'static str,
    },
    /// A value is not a hexadecimal number.
    BadHex {
        /// Line number.
        line: usize,
        /// The offending text.
        text: String,
    },
    /// A colour value is above `FF`, or the dither value above `F`.
    OutOfRange {
        /// Line number.
        line: usize,
        /// The parsed value.
        value: u32,
    },
    /// Unexpected text after the colour values and optional dither value.
    TrailingText {
        /// Line number.
        line: usize,
        /// The extra text.
        text: String,
    },
    /// A 17th entry was found.
    TooManyEntries {
        /// Line of the 17th entry.
        line: usize,
    },
    /// The file ended with fewer than 16 entries.
    TooFewEntries {
        /// Entries found.
        found: usize,
    },
}

impl fmt::Display for VplError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingField { line, field } => {
                write!(f, "line {line}: {field} value expected")
            }
            Self::BadHex { line, text } => {
                write!(f, "line {line}: {text:?} is not a hexadecimal number")
            }
            Self::OutOfRange { line, value } => {
                write!(f, "line {line}: value {value:X} is out of range")
            }
            Self::TrailingText { line, text } => {
                write!(f, "line {line}: unexpected text {text:?} after the colour")
            }
            Self::TooManyEntries { line } => {
                write!(f, "line {line}: more than 16 colour entries")
            }
            Self::TooFewEntries { found } => {
                write!(f, "{found} colour entries found, 16 expected")
            }
        }
    }
}

impl std::error::Error for VplError {}

/// Parses one hexadecimal field like C `strtol(s, _, 16)` on a whole token:
/// optional `0x`/`0X` prefix, then one or more hex digits.
fn parse_hex(token: &str, line: usize) -> Result<u32, VplError> {
    let digits = token
        .strip_prefix("0x")
        .or_else(|| token.strip_prefix("0X"))
        .unwrap_or(token);
    let bad = || VplError::BadHex {
        line,
        text: token.to_owned(),
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(bad());
    }
    // Leading zeros are allowed; anything that overflows is certainly out of range.
    u32::from_str_radix(digits, 16).map_err(|_| VplError::OutOfRange {
        line,
        value: u32::MAX,
    })
}

impl Palette {
    /// Reads a VICE `.vpl` palette with exactly 16 entries.
    ///
    /// The palette is named after a `# NAME:` comment when the file has one, otherwise
    /// `"Custom (.vpl)"`. Non-UTF-8 bytes are only allowed inside comments.
    ///
    /// # Errors
    /// A [`VplError`] naming the line for malformed entries, and for fewer or more
    /// than 16 entries. Never panics on any input.
    pub fn from_vpl(bytes: &[u8]) -> Result<Self, VplError> {
        const FIELDS: [&str; 3] = ["red", "green", "blue"];
        let mut entries: Vec<Rgb8> = Vec::with_capacity(16);
        let mut name: Option<String> = None;

        for (i, raw) in bytes.split(|&b| b == b'\n').enumerate() {
            let line = i + 1;
            let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
            let text = String::from_utf8_lossy(raw);
            let trimmed = text.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Some(comment) = trimmed.strip_prefix('#') {
                if let Some(n) = comment.trim().strip_prefix("NAME:") {
                    name = Some(n.trim().to_owned());
                }
                continue;
            }

            let mut tokens = trimmed.split_ascii_whitespace();
            let mut rgb = [0_u8; 3];
            for (slot, field) in rgb.iter_mut().zip(FIELDS) {
                let token = tokens
                    .next()
                    .ok_or(VplError::MissingField { line, field })?;
                let value = parse_hex(token, line)?;
                *slot = u8::try_from(value).map_err(|_| VplError::OutOfRange { line, value })?;
            }
            // Optional dither nybble (older VICE files); VICE itself ignores it.
            if let Some(token) = tokens.next() {
                let value = parse_hex(token, line).map_err(|_| VplError::TrailingText {
                    line,
                    text: token.to_owned(),
                })?;
                if value > 0xF {
                    return Err(VplError::OutOfRange { line, value });
                }
            }
            if let Some(token) = tokens.next() {
                return Err(VplError::TrailingText {
                    line,
                    text: token.to_owned(),
                });
            }
            if entries.len() == 16 {
                return Err(VplError::TooManyEntries { line });
            }
            entries.push(Rgb8::new(rgb[0], rgb[1], rgb[2]));
        }

        let rgb: [Rgb8; 16] = entries
            .try_into()
            .map_err(|e: Vec<Rgb8>| VplError::TooFewEntries { found: e.len() })?;
        Ok(Self::from_rgb(
            name.unwrap_or_else(|| "Custom (.vpl)".to_owned()),
            rgb,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::VplError;
    use crate::palette::Palette;

    // MIT-licensed fixtures generated from docs/reference/vic-ii-constraints.md §1 (and,
    // for b47-s70, from this crate's Colodore port); see fixtures/palettes/README.md.
    const PEPTO_PAL: &[u8] = include_bytes!("../../../../fixtures/palettes/pepto-pal.vpl");
    const PEPTO_NTSC: &[u8] = include_bytes!("../../../../fixtures/palettes/pepto-ntsc.vpl");
    const COLODORE_DEFAULT: &[u8] =
        include_bytes!("../../../../fixtures/palettes/colodore-default.vpl");
    const COLODORE_B47_S70: &[u8] =
        include_bytes!("../../../../fixtures/palettes/colodore-b47-s70.vpl");

    fn lines(entries: usize) -> String {
        (0..entries).map(|i| format!("{i:02x} 00 ff\n")).collect()
    }

    #[test]
    fn parses_pepto_pal_fixture() {
        // Lowercase two-digit values, a comment and a blank line per entry.
        let p = Palette::from_vpl(PEPTO_PAL).expect("pepto-pal.vpl parses");
        assert_eq!(p.name(), "Pepto (PAL)");
        assert_eq!(p, Palette::pepto_pal());
    }

    #[test]
    fn parses_pepto_ntsc_fixture() {
        // Uppercase values without leading zeros ("0  0  0", "42  38  0"), double
        // spaces, and a dither nybble on some lines.
        let p = Palette::from_vpl(PEPTO_NTSC).expect("pepto-ntsc.vpl parses");
        assert_eq!(p.name(), "Pepto (NTSC)");
        assert_eq!(p, Palette::pepto_ntsc());
    }

    #[test]
    fn parses_colodore_default_fixture() {
        // Compact: comment then entry, no blank lines.
        let p = Palette::from_vpl(COLODORE_DEFAULT).expect("colodore-default.vpl parses");
        assert_eq!(p, Palette::colodore(50.0, 100.0, 50.0));
    }

    #[test]
    fn generator_reproduces_the_colodore_b47_s70_fixture() {
        let p = Palette::from_vpl(COLODORE_B47_S70).expect("colodore-b47-s70.vpl parses");
        assert_eq!(p, Palette::colodore(47.0, 100.0, 70.0));
    }

    #[test]
    fn accepts_crlf_dither_column_prefixes_and_no_name() {
        let mut text = String::from("  # indented comment\r\n\r\n");
        text.push_str("0x0A 0XFF 0 f\r\n");
        for _ in 1..16 {
            text.push_str("00 00 00\r\n");
        }
        let p = Palette::from_vpl(text.as_bytes()).expect("parses");
        assert_eq!(p.name(), "Custom (.vpl)");
        assert_eq!(p.rgb_table()[0].to_hex(), 0x0AFF00);
    }

    #[test]
    fn rejects_wrong_entry_counts() {
        assert_eq!(
            Palette::from_vpl(lines(15).as_bytes()),
            Err(VplError::TooFewEntries { found: 15 })
        );
        assert_eq!(
            Palette::from_vpl(lines(17).as_bytes()),
            Err(VplError::TooManyEntries { line: 17 })
        );
        assert_eq!(
            Palette::from_vpl(b""),
            Err(VplError::TooFewEntries { found: 0 })
        );
    }

    #[test]
    fn rejects_malformed_fields_with_line_numbers() {
        let with = |bad: &str| format!("# header\n{bad}\n{}", lines(15));
        assert_eq!(
            Palette::from_vpl(with("00 ff").as_bytes()),
            Err(VplError::MissingField {
                line: 2,
                field: "blue"
            })
        );
        assert_eq!(
            Palette::from_vpl(with("00 zz 00").as_bytes()),
            Err(VplError::BadHex {
                line: 2,
                text: "zz".to_owned()
            })
        );
        assert_eq!(
            Palette::from_vpl(with("100 00 00").as_bytes()),
            Err(VplError::OutOfRange {
                line: 2,
                value: 0x100
            })
        );
        assert_eq!(
            Palette::from_vpl(with("00 00 00 10").as_bytes()),
            Err(VplError::OutOfRange {
                line: 2,
                value: 0x10
            })
        );
        assert_eq!(
            Palette::from_vpl(with("00 00 00 0 junk").as_bytes()),
            Err(VplError::TrailingText {
                line: 2,
                text: "junk".to_owned()
            })
        );
        assert_eq!(
            Palette::from_vpl(with("-1 00 00").as_bytes()),
            Err(VplError::BadHex {
                line: 2,
                text: "-1".to_owned()
            })
        );
    }

    #[test]
    fn never_panics_on_arbitrary_bytes() {
        let mut seed: u32 = 0x1234_5678;
        for len in 0..400 {
            let bytes: Vec<u8> = (0..len)
                .map(|_| {
                    seed ^= seed << 13;
                    seed ^= seed >> 17;
                    seed ^= seed << 5;
                    seed.to_le_bytes()[0]
                })
                .collect();
            let _ = Palette::from_vpl(&bytes);
        }
        let _ = Palette::from_vpl(&[0xFF, 0xFE, b'\n', b'#', 0x80]);
        let _ = Palette::from_vpl("ff ff ffffffffffffffffffff".as_bytes());
    }
}
