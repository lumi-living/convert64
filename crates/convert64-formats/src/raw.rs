//! Raw bitmap set: headerless `.bin` files plus a JSON manifest
//! (`docs/reference/file-formats.md`, "Raw bitmap set").
//!
//! [`write`] turns a [`BitmapImage`] into
//!
//! ```text
//! <name>.bitmap.bin   8000 bytes
//! <name>.screen.bin   1000 bytes
//! <name>.colour.bin   1000 bytes   (multicolor only)
//! <name>.bg.bin       1 byte       (multicolor only)
//! <name>.json         the manifest
//! ```
//!
//! and [`read`] rebuilds the picture from the manifest and the files it names. Like the
//! rest of the crate this module does no file I/O: [`write`] returns file names and bytes,
//! [`read`] asks the caller for each file's bytes by name.

use convert64_core::Palette;
use convert64_core::mode::Mode;
use convert64_core::model::{BitmapImage, ModelError};
use convert64_core::palette::Rgb8;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;

/// The manifest's `format` value.
pub const FORMAT_ID: &str = "convert64-raw-bitmap";

/// The manifest version this crate writes and the newest it reads.
pub const VERSION: u32 = 1;

/// The suggested load addresses, the VIC bank 1 layout the display PRG also uses
/// ([`crate::prg`]): bitmap `$6000`, Screen RAM `$5C00`, Colour RAM `$D800`.
pub const BITMAP_ADDRESS: u16 = crate::prg::BITMAP_ADDRESS;
/// See [`BITMAP_ADDRESS`].
pub const SCREEN_ADDRESS: u16 = crate::prg::SCREEN_ADDRESS;
/// See [`BITMAP_ADDRESS`].
pub const COLOUR_ADDRESS: u16 = crate::prg::COLOUR_RAM_ADDRESS;

/// One block of the picture and the file that holds it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Block {
    /// The 8000 bitmap bytes.
    Bitmap,
    /// The 1000 Screen RAM bytes.
    Screen,
    /// The 1000 Colour RAM bytes, low nybbles (multicolor).
    Colour,
    /// The background colour, one byte (multicolor).
    Background,
}

impl Block {
    /// The blocks a mode's set holds, in file order.
    pub const fn for_mode(mode: Mode) -> &'static [Self] {
        match mode {
            Mode::MulticolorBitmap => &[Self::Bitmap, Self::Screen, Self::Colour, Self::Background],
            _ => &[Self::Bitmap, Self::Screen],
        }
    }

    /// The block's size in bytes.
    pub const fn size(self) -> usize {
        match self {
            Self::Bitmap => 8000,
            Self::Screen | Self::Colour => 1000,
            Self::Background => 1,
        }
    }

    /// The file name suffix after `<name>.`: `bitmap.bin`, `screen.bin`, `colour.bin`,
    /// `bg.bin`.
    pub const fn suffix(self) -> &'static str {
        match self {
            Self::Bitmap => "bitmap.bin",
            Self::Screen => "screen.bin",
            Self::Colour => "colour.bin",
            Self::Background => "bg.bin",
        }
    }

    /// Where the block is meant to go in C64 memory; the background is a register
    /// value (`$D021`), not loaded anywhere.
    pub const fn load_address(self) -> Option<u16> {
        match self {
            Self::Bitmap => Some(BITMAP_ADDRESS),
            Self::Screen => Some(SCREEN_ADDRESS),
            Self::Colour => Some(COLOUR_ADDRESS),
            Self::Background => None,
        }
    }

    /// The block's bytes in `image`: what its file holds (empty for a block the
    /// image's mode does not have).
    pub fn bytes(self, image: &BitmapImage) -> Vec<u8> {
        match self {
            Self::Bitmap => image.bitmap().to_vec(),
            Self::Screen => image.screen_bytes(),
            Self::Colour => image.colour_bytes(),
            Self::Background => image
                .background()
                .map(|c| vec![c.get()])
                .unwrap_or_default(),
        }
    }
}

/// The JSON manifest (`<name>.json`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// Always [`FORMAT_ID`].
    pub format: String,
    /// The manifest version, [`VERSION`].
    pub version: u32,
    /// `multicolor-bitmap` or `hires-bitmap`.
    pub mode: Mode,
    /// The palette the picture was solved and previewed with.
    pub palette: PaletteEntry,
    /// One entry per block, in [`Block::for_mode`] order.
    pub files: Vec<FileEntry>,
}

/// The palette in the manifest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaletteEntry {
    /// The preset's name, e.g. `Pepto (PAL)`.
    pub name: String,
    /// The 16 colours in index order as `#RRGGBB`.
    pub rgb: Vec<String>,
}

impl PaletteEntry {
    /// The entry for `palette`.
    pub fn new(palette: &Palette) -> Self {
        Self {
            name: palette.name().to_owned(),
            rgb: palette
                .rgb_table()
                .iter()
                .map(ToString::to_string)
                .collect(),
        }
    }

    /// The palette the entry describes.
    ///
    /// # Errors
    /// [`RawError::Manifest`] unless there are 16 colours, each `#RRGGBB`.
    pub fn palette(&self) -> Result<Palette, RawError> {
        let bad = || {
            RawError::Manifest(format!(
                "palette.rgb must be 16 \"#RRGGBB\" colours, got {:?}",
                self.rgb
            ))
        };
        let colours: Vec<Rgb8> = self
            .rgb
            .iter()
            .map(|s| {
                let hex = s
                    .strip_prefix('#')
                    .filter(|h| h.len() == 6 && h.bytes().all(|b| b.is_ascii_hexdigit()))
                    .ok_or_else(bad)?;
                u32::from_str_radix(hex, 16)
                    .map(Rgb8::from_hex)
                    .map_err(|_| bad())
            })
            .collect::<Result<_, _>>()?;
        let table: [Rgb8; 16] = colours.try_into().map_err(|_| bad())?;
        Ok(Palette::from_rgb(self.name.clone(), table))
    }
}

/// One file in the manifest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileEntry {
    /// Which block the file holds.
    pub block: Block,
    /// The file name, relative to the manifest, without directories.
    pub file: String,
    /// The file's size in bytes.
    pub size: usize,
    /// The suggested load address as `$XXXX`; absent for the background.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub load_address: Option<String>,
}

/// A raw set read back.
#[derive(Clone, Debug, PartialEq)]
pub struct RawSet {
    /// The manifest.
    pub manifest: Manifest,
    /// The picture.
    pub image: BitmapImage,
    /// The palette from the manifest.
    pub palette: Palette,
}

/// Why a raw set could not be written or read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RawError {
    /// The name is empty or is not a plain file stem (it has a path separator or is
    /// `.`/`..`).
    Name(String),
    /// The picture is not a bitmap mode.
    Mode(Mode),
    /// The manifest is not valid JSON, has the wrong format or version, or does not
    /// describe the mode's blocks.
    Manifest(String),
    /// The caller has no file of this name.
    Missing(String),
    /// A file's size is not its block's (validator rule 6).
    Size {
        /// The file.
        file: String,
        /// The block's size.
        expected: usize,
        /// The file's size.
        found: usize,
    },
    /// The bytes do not form a legal picture (a background above 15, rule 2).
    Model(ModelError),
}

impl RawError {
    /// The rule of `docs/reference/vic-ii-constraints.md` §6 the error breaks, if any.
    pub fn rule_number(&self) -> Option<u8> {
        match self {
            Self::Size { .. } => Some(6),
            Self::Model(e) => e.rule().map(convert64_core::Rule::number),
            _ => None,
        }
    }
}

impl fmt::Display for RawError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Name(n) => write!(f, "{n:?} is not a plain file name for a raw set"),
            Self::Mode(m) => write!(f, "a raw bitmap set cannot store a {m} picture"),
            Self::Manifest(m) => write!(f, "raw set manifest: {m}"),
            Self::Missing(file) => write!(f, "raw set file {file} is missing"),
            Self::Size {
                file,
                expected,
                found,
            } => write!(f, "{file} must be {expected} bytes; it is {found}"),
            Self::Model(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for RawError {}

impl From<ModelError> for RawError {
    fn from(e: ModelError) -> Self {
        Self::Model(e)
    }
}

/// A plain file name: not empty, no path separators, not `.` or `..`.
fn check_name(name: &str) -> Result<(), RawError> {
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\', '\0']) {
        Err(RawError::Name(name.to_owned()))
    } else {
        Ok(())
    }
}

/// The manifest's file name for a set called `name`: `<name>.json`.
pub fn manifest_name(name: &str) -> String {
    format!("{name}.json")
}

/// `$XXXX`.
fn hex_address(addr: u16) -> String {
    format!("${addr:04X}")
}

/// The manifest for `image` written as a set called `name`.
///
/// # Errors
/// [`RawError::Name`] for a name that is not a plain file stem, [`RawError::Mode`]
/// for a picture that is not a bitmap.
pub fn manifest(image: &BitmapImage, name: &str, palette: &Palette) -> Result<Manifest, RawError> {
    check_name(name)?;
    let mode = image.mode();
    if !matches!(mode, Mode::MulticolorBitmap | Mode::HiresBitmap) {
        return Err(RawError::Mode(mode));
    }
    Ok(Manifest {
        format: FORMAT_ID.to_owned(),
        version: VERSION,
        mode,
        palette: PaletteEntry::new(palette),
        files: Block::for_mode(mode)
            .iter()
            .map(|&block| FileEntry {
                block,
                file: format!("{name}.{}", block.suffix()),
                size: block.size(),
                load_address: block.load_address().map(hex_address),
            })
            .collect(),
    })
}

/// Writes `image` as a raw set called `name`: the block files in
/// [`Block::for_mode`] order, then the manifest `<name>.json` (pretty-printed JSON, LF
/// line endings, trailing newline). Paths are bare file names, to be written next to
/// each other.
///
/// # Errors
/// See [`manifest`].
pub fn write(
    image: &BitmapImage,
    name: &str,
    palette: &Palette,
) -> Result<Vec<(PathBuf, Vec<u8>)>, RawError> {
    let manifest = manifest(image, name, palette)?;
    let mut out: Vec<(PathBuf, Vec<u8>)> = manifest
        .files
        .iter()
        .map(|f| (PathBuf::from(&f.file), f.block.bytes(image)))
        .collect();
    let mut json =
        serde_json::to_vec_pretty(&manifest).map_err(|e| RawError::Manifest(e.to_string()))?;
    json.push(b'\n');
    out.push((PathBuf::from(manifest_name(name)), json));
    Ok(out)
}

/// Reads a raw set: parses `manifest` (the `<name>.json` bytes), asks `file` for the
/// bytes of each file it names, checks every size and rebuilds the picture.
///
/// File names in the manifest must be plain names (no directories), so a manifest
/// cannot point outside its own directory. Colour RAM keeps only its low nybbles, as
/// the hardware does.
///
/// # Errors
/// [`RawError::Manifest`] for a manifest that is not valid JSON, is another format,
/// is newer than [`VERSION`], or does not list exactly the mode's blocks;
/// [`RawError::Name`] for a file name with a directory; [`RawError::Missing`] when
/// `file` returns `None`; [`RawError::Size`] (rule 6) for a file of the wrong size
/// (in the manifest or on disk); [`RawError::Model`] for a background above 15.
pub fn read(
    manifest: &[u8],
    mut file: impl FnMut(&str) -> Option<Vec<u8>>,
) -> Result<RawSet, RawError> {
    let manifest: Manifest =
        serde_json::from_slice(manifest).map_err(|e| RawError::Manifest(e.to_string()))?;
    if manifest.format != FORMAT_ID {
        return Err(RawError::Manifest(format!(
            "format is {:?}, expected {FORMAT_ID:?}",
            manifest.format
        )));
    }
    if manifest.version == 0 || manifest.version > VERSION {
        return Err(RawError::Manifest(format!(
            "version {} is not supported (this reader reads 1 to {VERSION})",
            manifest.version
        )));
    }
    let blocks = Block::for_mode(manifest.mode);
    if !matches!(manifest.mode, Mode::MulticolorBitmap | Mode::HiresBitmap) {
        return Err(RawError::Mode(manifest.mode));
    }
    let listed: Vec<Block> = manifest.files.iter().map(|f| f.block).collect();
    if listed != blocks {
        return Err(RawError::Manifest(format!(
            "a {} set lists {blocks:?}, this one {listed:?}",
            manifest.mode
        )));
    }
    let mut data = Vec::with_capacity(blocks.len());
    for entry in &manifest.files {
        check_name(&entry.file)?;
        let expected = entry.block.size();
        if entry.size != expected {
            return Err(RawError::Size {
                file: entry.file.clone(),
                expected,
                found: entry.size,
            });
        }
        let bytes = file(&entry.file).ok_or_else(|| RawError::Missing(entry.file.clone()))?;
        if bytes.len() != expected {
            return Err(RawError::Size {
                file: entry.file.clone(),
                expected,
                found: bytes.len(),
            });
        }
        data.push(bytes);
    }
    let image = match manifest.mode {
        Mode::MulticolorBitmap => {
            BitmapImage::multicolor_from_bytes(&data[0], &data[1], &data[2], data[3][0])?
        }
        _ => BitmapImage::hires_from_bytes(&data[0], &data[1])?,
    };
    let palette = manifest.palette.palette()?;
    Ok(RawSet {
        manifest,
        image,
        palette,
    })
}

/// Reads a raw set from files already in memory, as [`write`] returns them: the
/// manifest is the file whose name ends in `.json`.
///
/// # Errors
/// [`RawError::Missing`] without a manifest, otherwise see [`read`].
pub fn read_files(files: &[(PathBuf, Vec<u8>)]) -> Result<RawSet, RawError> {
    let manifest = files
        .iter()
        .find(|(p, _)| p.extension().is_some_and(|e| e == "json"))
        .ok_or_else(|| RawError::Missing("<name>.json".to_owned()))?;
    read(&manifest.1, |name| {
        files
            .iter()
            .find(|(p, _)| p.as_os_str() == name)
            .map(|(_, b)| b.clone())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use convert64_core::ColourIndex;

    fn koala_image() -> BitmapImage {
        let bitmap: Vec<u8> = (0..8000_u32).map(|i| (i * 31 % 251) as u8).collect();
        let screen: Vec<u8> = (0..1000_u32).map(|i| (i * 17 % 256) as u8).collect();
        let colour: Vec<u8> = (0..1000_u32).map(|i| (i % 16) as u8).collect();
        BitmapImage::multicolor_from_bytes(&bitmap, &screen, &colour, 0x0B).expect("legal")
    }

    fn hires_image() -> BitmapImage {
        let bitmap: Vec<u8> = (0..8000_u32).map(|i| (i * 7 % 256) as u8).collect();
        let screen: Vec<u8> = (0..1000_u32).map(|i| (i * 29 % 256) as u8).collect();
        BitmapImage::hires_from_bytes(&bitmap, &screen).expect("legal")
    }

    fn find<'a>(files: &'a [(PathBuf, Vec<u8>)], name: &str) -> &'a [u8] {
        &files
            .iter()
            .find(|(p, _)| p.as_os_str() == name)
            .unwrap_or_else(|| panic!("{name}"))
            .1
    }

    #[test]
    fn multicolor_set_files_and_bytes() {
        let img = koala_image();
        let files = write(&img, "pic", &Palette::pepto_pal()).expect("writes");
        let names: Vec<_> = files.iter().map(|(p, _)| p.display().to_string()).collect();
        assert_eq!(
            names,
            [
                "pic.bitmap.bin",
                "pic.screen.bin",
                "pic.colour.bin",
                "pic.bg.bin",
                "pic.json"
            ]
        );
        assert_eq!(find(&files, "pic.bitmap.bin"), img.bitmap());
        assert_eq!(find(&files, "pic.screen.bin"), img.screen_bytes());
        assert_eq!(find(&files, "pic.colour.bin"), img.colour_bytes());
        assert_eq!(find(&files, "pic.bg.bin"), [0x0B]);
        // The block files concatenated in order are a Koala file without its header.
        let body: Vec<u8> = files[..4].iter().flat_map(|(_, b)| b.clone()).collect();
        let kla = crate::BitmapFormat::Koala.write(&img).expect("koala");
        assert_eq!(body, kla[2..]);
        let back = read_files(&files).expect("reads");
        assert_eq!(back.image, img);
        assert_eq!(back.palette, Palette::pepto_pal());
    }

    #[test]
    fn hires_set_has_two_blocks() {
        let img = hires_image();
        let files = write(&img, "line", &Palette::pepto_pal()).expect("writes");
        let names: Vec<_> = files.iter().map(|(p, _)| p.display().to_string()).collect();
        assert_eq!(names, ["line.bitmap.bin", "line.screen.bin", "line.json"]);
        let body: Vec<u8> = files[..2].iter().flat_map(|(_, b)| b.clone()).collect();
        let art = crate::BitmapFormat::ArtStudio.write(&img).expect("art");
        assert_eq!(body, art[2..]);
        assert_eq!(read_files(&files).expect("reads").image, img);
    }

    /// The manifest is byte-stable and says what `file-formats.md` documents.
    #[test]
    fn manifest_text() {
        let files = write(&hires_image(), "line", &Palette::pepto_pal()).expect("writes");
        let json = String::from_utf8(find(&files, "line.json").to_vec()).expect("utf-8");
        let expected = r##"{
  "format": "convert64-raw-bitmap",
  "version": 1,
  "mode": "hires-bitmap",
  "palette": {
    "name": "Pepto (PAL)",
    "rgb": [
      "#000000",
      "#FFFFFF",
      "#68372B",
      "#70A4B2",
      "#6F3D86",
      "#588D43",
      "#352879",
      "#B8C76F",
      "#6F4F25",
      "#433900",
      "#9A6759",
      "#444444",
      "#6C6C6C",
      "#9AD284",
      "#6C5EB5",
      "#959595"
    ]
  },
  "files": [
    {
      "block": "bitmap",
      "file": "line.bitmap.bin",
      "size": 8000,
      "load_address": "$6000"
    },
    {
      "block": "screen",
      "file": "line.screen.bin",
      "size": 1000,
      "load_address": "$5C00"
    }
  ]
}
"##;
        assert_eq!(json, expected);
    }

    #[test]
    fn names_with_directories_are_refused() {
        for bad in ["", ".", "..", "a/b", "a\\b"] {
            assert_eq!(
                write(&koala_image(), bad, &Palette::pepto_pal()),
                Err(RawError::Name(bad.to_owned()))
            );
        }
        let mut files = write(&koala_image(), "pic", &Palette::pepto_pal()).expect("writes");
        let json = String::from_utf8(files[4].1.clone())
            .expect("utf-8")
            .replace("\"pic.screen.bin\"", "\"../pic.screen.bin\"");
        files[4].1 = json.into_bytes();
        assert_eq!(
            read_files(&files),
            Err(RawError::Name("../pic.screen.bin".to_owned()))
        );
    }

    #[test]
    fn bad_sets_are_refused() {
        let pepto = Palette::pepto_pal();
        let good = write(&koala_image(), "pic", &pepto).expect("writes");
        let edit = |from: &str, to: &str| {
            let mut files = good.clone();
            let json = String::from_utf8(files[4].1.clone()).expect("utf-8");
            assert!(json.contains(from), "{from}");
            files[4].1 = json.replacen(from, to, 1).into_bytes();
            read_files(&files)
        };
        assert!(matches!(
            edit("\"version\": 1", "\"version\": 2"),
            Err(RawError::Manifest(_))
        ));
        assert!(matches!(
            edit("convert64-raw-bitmap", "something-else"),
            Err(RawError::Manifest(_))
        ));
        assert!(matches!(
            edit("multicolor-bitmap", "hires-bitmap"),
            Err(RawError::Manifest(_))
        ));
        assert!(matches!(
            edit("\"#000000\"", "\"black\""),
            Err(RawError::Manifest(_))
        ));
        let err = edit("\"size\": 8000", "\"size\": 8001").expect_err("size");
        assert_eq!(err.rule_number(), Some(6));

        // A file of the wrong length, a missing file, a background above 15.
        let mut short = good.clone();
        short[1].1.pop();
        let err = read_files(&short).expect_err("short screen");
        assert_eq!(
            err.to_string(),
            "pic.screen.bin must be 1000 bytes; it is 999"
        );
        assert_eq!(err.rule_number(), Some(6));
        let missing: Vec<_> = good
            .iter()
            .filter(|(p, _)| !p.ends_with("pic.bg.bin"))
            .cloned()
            .collect();
        assert_eq!(
            read_files(&missing),
            Err(RawError::Missing("pic.bg.bin".to_owned()))
        );
        let mut bg = good.clone();
        bg[3].1 = vec![0x10];
        assert_eq!(read_files(&bg).expect_err("bg").rule_number(), Some(2));
        // Colour RAM high nybbles are dropped, as on the hardware.
        let mut high = good;
        high[2].1[0] |= 0xF0;
        assert_eq!(
            read_files(&high).expect("reads").image.colour()[0],
            ColourIndex::BLACK
        );
    }
}
