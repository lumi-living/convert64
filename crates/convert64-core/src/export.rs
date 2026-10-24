//! The project's export set (UX §3, Export; usability review M4, #287): rows of a format
//! and its options, saved in the project as its `exports`. The app's Export panel shows
//! and edits them, `export_set` replaces them, and `convert64 export` and the MCP server's
//! `export_run` write them. This is data only; the writers are
//! `convert64_formats::export`'s, as this crate does no I/O.

use serde::{Deserialize, Serialize};
use std::fmt;

/// What a row writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ExportFormat {
    /// The mode's picture file: Koala (`.kla`, multicolor) or Art Studio (`.art`, hi-res).
    #[serde(rename = "picture")]
    Picture,
    /// A self-displaying PRG of the picture file.
    #[serde(rename = "prg")]
    Prg,
    /// The raw bitmap set: headerless `.bin` blocks and a JSON manifest.
    #[serde(rename = "raw")]
    Raw,
    /// KickAssembler source (`.asm`).
    #[serde(rename = "kickass")]
    KickAssembler,
    /// ACME source (`.a`).
    #[serde(rename = "acme")]
    Acme,
    /// 64tass source (`.tas`).
    #[serde(rename = "64tass")]
    Tass64,
    /// ca65 source (`.s`).
    #[serde(rename = "ca65")]
    Ca65,
    /// A C header (`.h`).
    #[serde(rename = "c")]
    CHeader,
    /// A palette-exact PNG preview at an integer zoom.
    #[serde(rename = "png")]
    Png,
}

/// The groups the formats are offered in (M4: C64 files, Source code, Preview).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ExportGroup {
    /// Files a C64 loads: the picture file, the PRG, the raw set.
    C64Files,
    /// Assembler source and C headers.
    SourceCode,
    /// A PNG to look at.
    Preview,
}

impl ExportGroup {
    /// Every group, in menu order.
    pub const ALL: [Self; 3] = [Self::C64Files, Self::SourceCode, Self::Preview];

    /// The group's heading.
    pub const fn label(self) -> &'static str {
        match self {
            Self::C64Files => "C64 files",
            Self::SourceCode => "Source code",
            Self::Preview => "Preview",
        }
    }
}

impl ExportFormat {
    /// Every format, grouped, in menu order.
    pub const ALL: [Self; 9] = [
        Self::Picture,
        Self::Prg,
        Self::Raw,
        Self::KickAssembler,
        Self::Acme,
        Self::Tass64,
        Self::Ca65,
        Self::CHeader,
        Self::Png,
    ];

    /// The id a project file and the command line use: `picture`, `prg`, `raw`,
    /// `kickass`, `acme`, `64tass`, `ca65`, `c`, `png`.
    pub const fn id(self) -> &'static str {
        match self {
            Self::Picture => "picture",
            Self::Prg => "prg",
            Self::Raw => "raw",
            Self::KickAssembler => "kickass",
            Self::Acme => "acme",
            Self::Tass64 => "64tass",
            Self::Ca65 => "ca65",
            Self::CHeader => "c",
            Self::Png => "png",
        }
    }

    /// The format an id names, case-insensitively.
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|f| f.id().eq_ignore_ascii_case(id))
    }

    /// Its group.
    pub const fn group(self) -> ExportGroup {
        match self {
            Self::Picture | Self::Prg | Self::Raw => ExportGroup::C64Files,
            Self::KickAssembler | Self::Acme | Self::Tass64 | Self::Ca65 | Self::CHeader => {
                ExportGroup::SourceCode
            }
            Self::Png => ExportGroup::Preview,
        }
    }

    /// Whether it is assembler source or a C header (and so takes [`ExportData`]).
    pub const fn is_source(self) -> bool {
        matches!(self.group(), ExportGroup::SourceCode)
    }
}

impl fmt::Display for ExportFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id())
    }
}

/// How source code carries the picture data.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExportData {
    /// Byte lines in the source (one file; the default).
    #[default]
    Bytes,
    /// Include directives for the raw set's `.bin` blocks, written beside the source.
    IncludeRaw,
    /// One include of the Koala / Art Studio file, written beside the source.
    IncludePicture,
}

impl ExportData {
    /// Every style, in menu order.
    pub const ALL: [Self; 3] = [Self::Bytes, Self::IncludeRaw, Self::IncludePicture];

    /// Its name in the Export panel.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Bytes => "Inline bytes",
            Self::IncludeRaw => "Include raw set",
            Self::IncludePicture => "Include picture file",
        }
    }
}

/// The largest PNG preview zoom (`convert64_formats::png::MAX_ZOOM`).
pub const MAX_ZOOM: u8 = 8;

/// The PNG preview zoom a row has unless it says otherwise.
pub const DEFAULT_ZOOM: u8 = 2;

const fn default_zoom() -> u8 {
    DEFAULT_ZOOM
}

const fn is_default_zoom(z: &u8) -> bool {
    *z == DEFAULT_ZOOM
}

fn is_bytes(d: &ExportData) -> bool {
    *d == ExportData::Bytes
}

/// One row of the export set: a format, its options and, optionally, its own file name.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportEntry {
    /// What it writes.
    pub format: ExportFormat,
    /// The files' base name (no extension); absent, the project's name. Written only
    /// when set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// PNG preview zoom, 1–8 (`png` only). Written only when not 2.
    #[serde(default = "default_zoom", skip_serializing_if = "is_default_zoom")]
    pub zoom: u8,
    /// How source code carries the data (source formats only). Written only when not
    /// `bytes`.
    #[serde(default, skip_serializing_if = "is_bytes")]
    pub data: ExportData,
}

impl ExportEntry {
    /// A row writing `format` with the default options, named after the project.
    pub const fn new(format: ExportFormat) -> Self {
        Self {
            format,
            name: None,
            zoom: DEFAULT_ZOOM,
            data: ExportData::Bytes,
        }
    }

    /// The base name its files get: its own, else `project`.
    pub fn stem<'a>(&'a self, project: &'a str) -> &'a str {
        self.name.as_deref().unwrap_or(project)
    }

    /// Why the row can't be written, if it can't: a zoom outside 1–8 or a name that is
    /// empty, a path or hidden.
    pub fn check(&self) -> Result<(), String> {
        if !(1..=MAX_ZOOM).contains(&self.zoom) {
            return Err(format!("zoom {} is outside 1–{MAX_ZOOM}", self.zoom));
        }
        if let Some(name) = &self.name {
            check_stem(name)?;
        }
        Ok(())
    }
}

/// Why `name` can't be the base name of exported files: empty, a path (`/`, `\`), hidden
/// (a leading `.`) or with control characters.
pub fn check_stem(name: &str) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("the file name is empty".to_owned());
    }
    if name.contains(['/', '\\']) || name.starts_with('.') || name.chars().any(char::is_control) {
        return Err(format!(
            "{name:?} is not a file name: no folders, no leading dot"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip_and_groups_hold_what_m4_lists() {
        for f in ExportFormat::ALL {
            assert_eq!(ExportFormat::from_id(f.id()), Some(f));
        }
        let in_group = |g| {
            ExportFormat::ALL
                .into_iter()
                .filter(|f| f.group() == g)
                .map(ExportFormat::id)
                .collect::<Vec<_>>()
        };
        assert_eq!(in_group(ExportGroup::C64Files), ["picture", "prg", "raw"]);
        assert_eq!(
            in_group(ExportGroup::SourceCode),
            ["kickass", "acme", "64tass", "ca65", "c"]
        );
        assert_eq!(in_group(ExportGroup::Preview), ["png"]);
    }

    #[test]
    fn bad_rows_are_refused() {
        let row = |zoom, name: Option<&str>| ExportEntry {
            zoom,
            name: name.map(str::to_owned),
            ..ExportEntry::new(ExportFormat::Png)
        };
        assert!(row(2, None).check().is_ok());
        assert!(row(8, Some("title@8x")).check().is_ok());
        assert_eq!(row(4, Some("title@4x")).stem("title"), "title@4x");
        assert_eq!(row(4, None).stem("title"), "title");
        for bad in [
            row(0, None),
            row(9, None),
            row(2, Some("")),
            row(2, Some("../x")),
            row(2, Some("a\\b")),
            row(2, Some(".hidden")),
        ] {
            assert!(bad.check().is_err(), "{bad:?}");
        }
    }
}
