//! Koala Painter and Art Studio readers (`docs/reference/file-formats.md`, "Bitmap pictures").

use convert64_core::mode::BlockKind;
use convert64_core::model::{BitmapImage, ModelError};
use convert64_core::{Placement, Rule};
use std::fmt;

/// A bitmap picture file format.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BitmapFormat {
    /// Koala Painter (`.kla`, `.koa`), multicolor, 10 003 bytes.
    Koala,
    /// Art Studio (`.art`, `.aas`), hi-res, 9002 bytes (9009 with padding).
    ArtStudio,
}

impl BitmapFormat {
    /// Format name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Koala => "Koala",
            Self::ArtStudio => "Art Studio",
        }
    }

    /// File sizes the format allows.
    pub const fn sizes(self) -> &'static [usize] {
        match self {
            Self::Koala => &[KOALA_SIZE],
            Self::ArtStudio => &[ART_SIZE, ART_SIZE_PADDED],
        }
    }

    /// The format a file extension names, case-insensitively (`kla`, `koa`, `art`,
    /// `aas`).
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_ascii_lowercase().as_str() {
            "kla" | "koa" => Some(Self::Koala),
            "art" | "aas" => Some(Self::ArtStudio),
            _ => None,
        }
    }

    /// The format a file of `len` bytes must be, if its size is unique to one format.
    pub fn from_size(len: usize) -> Option<Self> {
        [Self::Koala, Self::ArtStudio]
            .into_iter()
            .find(|f| f.sizes().contains(&len))
    }

    /// Reads `bytes` as this format.
    ///
    /// # Errors
    /// See [`read_koala`] and [`read_art_studio`].
    pub fn read(self, bytes: &[u8]) -> Result<BitmapFile, ReadError> {
        match self {
            Self::Koala => read_koala(bytes),
            Self::ArtStudio => read_art_studio(bytes),
        }
    }
}

impl fmt::Display for BitmapFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A decoded picture file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BitmapFile {
    /// The format it was read as.
    pub format: BitmapFormat,
    /// The two-byte load address from the file header.
    pub load_address: u16,
    /// The picture.
    pub image: BitmapImage,
}

impl BitmapFile {
    /// Where the file puts its bitmap in C64 memory: the bitmap is the first block
    /// after the load address in both formats. Screen and Colour RAM follow it
    /// unaligned in the file and are copied to their display addresses by a viewer, so
    /// only the bitmap's placement is a property of the file.
    pub fn placement(&self) -> Placement {
        Placement::new().with(BlockKind::Bitmap, self.load_address)
    }
}

/// Why a picture file could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReadError {
    /// The file size does not match the format's layout (validator rule 6).
    Size {
        /// The format.
        format: BitmapFormat,
        /// Allowed sizes.
        expected: &'static [usize],
        /// Actual size.
        found: usize,
    },
    /// The bytes do not form a legal image (for example a background byte above 15).
    Model(ModelError),
}

impl ReadError {
    /// The rule of `docs/reference/vic-ii-constraints.md` §6 this error breaks: rule 6
    /// for sizes, or the model's rule (2 for colour values).
    pub fn rule_number(&self) -> Option<u8> {
        match self {
            Self::Size { .. } => Some(6),
            Self::Model(e) => e.rule().map(Rule::number),
        }
    }
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Size {
                format,
                expected,
                found,
            } => {
                let sizes: Vec<String> = expected.iter().map(ToString::to_string).collect();
                write!(
                    f,
                    "{format} files are {} bytes; this one is {found}",
                    sizes.join(" or ")
                )
            }
            Self::Model(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for ReadError {}

impl From<ModelError> for ReadError {
    fn from(e: ModelError) -> Self {
        Self::Model(e)
    }
}

const KOALA_SIZE: usize = 10_003;
const ART_SIZE: usize = 9002;
const ART_SIZE_PADDED: usize = 9009;

fn check_size(format: BitmapFormat, bytes: &[u8]) -> Result<u16, ReadError> {
    if format.sizes().contains(&bytes.len()) {
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    } else {
        Err(ReadError::Size {
            format,
            expected: format.sizes(),
            found: bytes.len(),
        })
    }
}

/// Reads a Koala Painter picture:
///
/// ```text
/// 0      2     load address (normally $6000)
/// 2      8000  bitmap
/// 8002   1000  Screen RAM (high nybble = bit pair 01, low nybble = 10)
/// 9002   1000  Colour RAM (low nybble = bit pair 11)
/// 10002  1     background colour (bit pair 00)
/// ```
///
/// Colour RAM keeps only its low nybble, as the hardware does.
///
/// # Errors
/// [`ReadError::Size`] unless the file is 10 003 bytes; [`ReadError::Model`] for a
/// background byte above 15 (rule 2).
pub fn read_koala(bytes: &[u8]) -> Result<BitmapFile, ReadError> {
    let load_address = check_size(BitmapFormat::Koala, bytes)?;
    let image = BitmapImage::multicolor_from_bytes(
        &bytes[2..8002],
        &bytes[8002..9002],
        &bytes[9002..10_002],
        bytes[10_002],
    )?;
    Ok(BitmapFile {
        format: BitmapFormat::Koala,
        load_address,
        image,
    })
}

/// Reads an Art Studio picture:
///
/// ```text
/// 0      2     load address (normally $2000)
/// 2      8000  bitmap
/// 8002   1000  Screen RAM (high nybble = bit 1 colour, low nybble = bit 0 colour)
/// [9002  7     padding, in some files]
/// ```
///
/// # Errors
/// [`ReadError::Size`] unless the file is 9002 or 9009 bytes.
pub fn read_art_studio(bytes: &[u8]) -> Result<BitmapFile, ReadError> {
    let load_address = check_size(BitmapFormat::ArtStudio, bytes)?;
    let image = BitmapImage::hires_from_bytes(&bytes[2..8002], &bytes[8002..9002])?;
    Ok(BitmapFile {
        format: BitmapFormat::ArtStudio,
        load_address,
        image,
    })
}

#[cfg(test)]
mod tests {
    use super::{BitmapFormat, ReadError, read_art_studio, read_koala};
    use convert64_core::mode::Mode;
    use convert64_core::{ColourIndex, validate, validate_placement};

    /// A Koala file assembled from the documented layout, from fixed arbitrary data.
    fn koala_bytes(load: u16, background: u8) -> Vec<u8> {
        let mut f = load.to_le_bytes().to_vec();
        f.extend((0..8000_u32).map(|i| (i * 31 % 251) as u8));
        f.extend((0..1000_u32).map(|i| (i * 17 % 256) as u8));
        f.extend((0..1000_u32).map(|i| (i % 16) as u8));
        f.push(background);
        f
    }

    fn art_bytes(load: u16, padded: bool) -> Vec<u8> {
        let mut f = load.to_le_bytes().to_vec();
        f.extend((0..8000_u32).map(|i| (i * 7 % 256) as u8));
        f.extend((0..1000_u32).map(|i| (i * 29 % 256) as u8));
        if padded {
            f.extend([0; 7]);
        }
        f
    }

    #[test]
    fn koala_round_trips_against_the_layout() {
        let bytes = koala_bytes(0x6000, 0x0B);
        let file = read_koala(&bytes).expect("legal Koala");
        assert_eq!(file.format, BitmapFormat::Koala);
        assert_eq!(file.load_address, 0x6000);
        assert_eq!(file.image.mode(), Mode::MulticolorBitmap);
        assert_eq!(file.image.background(), Some(ColourIndex::DARK_GREY));
        // Re-assemble the file from the model and compare byte for byte.
        let mut again = file.load_address.to_le_bytes().to_vec();
        again.extend(file.image.bitmap());
        again.extend(file.image.screen_bytes());
        again.extend(file.image.colour_bytes());
        again.push(file.image.background().map_or(0, ColourIndex::get));
        assert_eq!(again, bytes);
        assert_eq!(validate(&file.image), Ok(()));
        assert_eq!(validate_placement(&file.placement()), Ok(()));
    }

    #[test]
    fn koala_colour_ram_high_nybbles_are_dropped() {
        let mut bytes = koala_bytes(0x6000, 0);
        bytes[9002] = 0xF3;
        let file = read_koala(&bytes).expect("legal");
        assert_eq!(file.image.colour()[0], ColourIndex::CYAN);
    }

    #[test]
    fn art_studio_round_trips_with_and_without_padding() {
        for padded in [false, true] {
            let bytes = art_bytes(0x2000, padded);
            let file = read_art_studio(&bytes).expect("legal Art Studio");
            assert_eq!(file.image.mode(), Mode::HiresBitmap);
            let mut again = file.load_address.to_le_bytes().to_vec();
            again.extend(file.image.bitmap());
            again.extend(file.image.screen_bytes());
            assert_eq!(again, bytes[..9002]);
            assert_eq!(validate_placement(&file.placement()), Ok(()));
        }
    }

    #[test]
    fn wrong_sizes_are_rule_6() {
        let err = read_koala(&[0; 10_002]).expect_err("short");
        assert_eq!(err.rule_number(), Some(6));
        assert_eq!(
            err.to_string(),
            "Koala files are 10003 bytes; this one is 10002"
        );
        let err = read_art_studio(&[0; 9003]).expect_err("odd size");
        assert_eq!(
            err.to_string(),
            "Art Studio files are 9002 or 9009 bytes; this one is 9003"
        );
    }

    #[test]
    fn koala_background_above_15_is_rule_2() {
        let err = read_koala(&koala_bytes(0x6000, 0x1F)).expect_err("bg $1F");
        assert!(matches!(err, ReadError::Model(_)));
        assert_eq!(err.rule_number(), Some(2));
    }

    #[test]
    fn format_detection() {
        assert_eq!(
            BitmapFormat::from_extension("KLA"),
            Some(BitmapFormat::Koala)
        );
        assert_eq!(
            BitmapFormat::from_extension("aas"),
            Some(BitmapFormat::ArtStudio)
        );
        assert_eq!(BitmapFormat::from_extension("png"), None);
        assert_eq!(BitmapFormat::from_size(10_003), Some(BitmapFormat::Koala));
        assert_eq!(BitmapFormat::from_size(9009), Some(BitmapFormat::ArtStudio));
        assert_eq!(BitmapFormat::from_size(64), None);
    }
}
