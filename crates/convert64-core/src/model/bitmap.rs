//! [`BitmapImage`]: an encoded multicolor or hi-res bitmap, as the VIC-II reads it.

use super::{CellPalette, CellPos, IndexedBitmap, ModelError};
use crate::colour::ColourIndex;
use crate::mode::Mode;
use serde::{Deserialize, Serialize};
use std::fmt;

const BITMAP_BYTES: usize = 8000;
const CELLS: usize = 1000;
const CELLS_X: u16 = 40;
const CELLS_Y: u16 = 25;

/// One Screen RAM byte in a bitmap mode: two colours, one per nybble.
///
/// Multicolor: `high` is bit pair `01`, `low` is bit pair `10`.
/// Hi-res: `high` is shown for set bits, `low` for clear bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ScreenCell {
    /// High nybble.
    pub high: ColourIndex,
    /// Low nybble.
    pub low: ColourIndex,
}

impl ScreenCell {
    /// Both colours from one Screen RAM byte. Every byte is valid.
    pub const fn from_byte(byte: u8) -> Self {
        Self {
            high: ColourIndex::from_high_nybble(byte),
            low: ColourIndex::from_low_nybble(byte),
        }
    }

    /// The Screen RAM byte.
    pub const fn to_byte(self) -> u8 {
        (self.high.get() << 4) | self.low.get()
    }
}

/// An encoded bitmap picture: 8000 bytes of bitmap, 1000 Screen RAM cells and, in
/// multicolor mode, 1000 Colour RAM nybbles and the background colour.
///
/// Byte layouts follow `docs/reference/vic-ii-constraints.md` §2: bitmap bytes are
/// ordered by cell (`cell_row * 320 + cell_col * 8 + pixel_row`), bit 7 leftmost.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "BitmapImageData")]
pub struct BitmapImage {
    mode: Mode,
    bitmap: Vec<u8>,
    screen: Vec<ScreenCell>,
    colour: Vec<ColourIndex>,
    background: Option<ColourIndex>,
}

#[derive(Deserialize)]
struct BitmapImageData {
    mode: Mode,
    bitmap: Vec<u8>,
    screen: Vec<ScreenCell>,
    colour: Vec<ColourIndex>,
    background: Option<ColourIndex>,
}

impl TryFrom<BitmapImageData> for BitmapImage {
    type Error = ModelError;

    fn try_from(d: BitmapImageData) -> Result<Self, Self::Error> {
        match (d.mode, d.background) {
            (Mode::MulticolorBitmap, Some(bg)) => {
                Self::multicolor(d.bitmap, d.screen, d.colour, bg)
            }
            (Mode::HiresBitmap, None) => {
                ModelError::check_len("colour", 0, d.colour.len())?;
                Self::hires(d.bitmap, d.screen)
            }
            (Mode::MulticolorBitmap | Mode::HiresBitmap, _) => {
                Err(ModelError::Background { mode: d.mode })
            }
            (mode, _) => Err(ModelError::WrongMode {
                what: "bitmap image",
                mode,
            }),
        }
    }
}

impl BitmapImage {
    /// A multicolor bitmap from its parts.
    ///
    /// # Errors
    /// [`ModelError::WrongLength`] unless `bitmap` is 8000 bytes and `screen` and
    /// `colour` are 1000 cells each.
    pub fn multicolor(
        bitmap: Vec<u8>,
        screen: Vec<ScreenCell>,
        colour: Vec<ColourIndex>,
        background: ColourIndex,
    ) -> Result<Self, ModelError> {
        ModelError::check_len("bitmap", BITMAP_BYTES, bitmap.len())?;
        ModelError::check_len("screen", CELLS, screen.len())?;
        ModelError::check_len("colour", CELLS, colour.len())?;
        Ok(Self {
            mode: Mode::MulticolorBitmap,
            bitmap,
            screen,
            colour,
            background: Some(background),
        })
    }

    /// A hi-res bitmap from its parts.
    ///
    /// # Errors
    /// [`ModelError::WrongLength`] unless `bitmap` is 8000 bytes and `screen` 1000 cells.
    pub fn hires(bitmap: Vec<u8>, screen: Vec<ScreenCell>) -> Result<Self, ModelError> {
        ModelError::check_len("bitmap", BITMAP_BYTES, bitmap.len())?;
        ModelError::check_len("screen", CELLS, screen.len())?;
        Ok(Self {
            mode: Mode::HiresBitmap,
            bitmap,
            screen,
            colour: Vec::new(),
            background: None,
        })
    }

    /// A multicolor bitmap from raw memory: 8000 bitmap bytes, 1000 Screen RAM bytes,
    /// 1000 Colour RAM bytes and the background byte.
    ///
    /// Colour RAM is four bits wide on the hardware, so only the low nybble of each
    /// byte is used (memory dumps of `$D800` carry random high bits). The background
    /// byte is a register value and must be 0–15.
    ///
    /// # Errors
    /// [`ModelError::WrongLength`] for wrong sizes, [`ModelError::Colour`] (rule 2) for a
    /// background above 15.
    pub fn multicolor_from_bytes(
        bitmap: &[u8],
        screen: &[u8],
        colour_ram: &[u8],
        background: u8,
    ) -> Result<Self, ModelError> {
        let background = ModelError::colour("background", background)?;
        Self::multicolor(
            bitmap.to_vec(),
            screen.iter().map(|&b| ScreenCell::from_byte(b)).collect(),
            colour_ram
                .iter()
                .map(|&b| ColourIndex::from_low_nybble(b))
                .collect(),
            background,
        )
    }

    /// A hi-res bitmap from raw memory: 8000 bitmap bytes and 1000 Screen RAM bytes.
    ///
    /// # Errors
    /// [`ModelError::WrongLength`] for wrong sizes.
    pub fn hires_from_bytes(bitmap: &[u8], screen: &[u8]) -> Result<Self, ModelError> {
        Self::hires(
            bitmap.to_vec(),
            screen.iter().map(|&b| ScreenCell::from_byte(b)).collect(),
        )
    }

    /// The mode: [`Mode::MulticolorBitmap`] or [`Mode::HiresBitmap`].
    pub const fn mode(&self) -> Mode {
        self.mode
    }

    /// The 8000 bitmap bytes.
    pub fn bitmap(&self) -> &[u8] {
        &self.bitmap
    }

    /// The 1000 Screen RAM cells.
    pub fn screen(&self) -> &[ScreenCell] {
        &self.screen
    }

    /// Screen RAM as 1000 bytes.
    pub fn screen_bytes(&self) -> Vec<u8> {
        self.screen.iter().map(|c| c.to_byte()).collect()
    }

    /// The 1000 Colour RAM colours (multicolor), or an empty slice (hi-res).
    pub fn colour(&self) -> &[ColourIndex] {
        &self.colour
    }

    /// Colour RAM as 1000 bytes with clear high nybbles (multicolor), or empty (hi-res).
    pub fn colour_bytes(&self) -> Vec<u8> {
        self.colour.iter().map(|c| c.get()).collect()
    }

    /// The background colour (multicolor only).
    pub const fn background(&self) -> Option<ColourIndex> {
        self.background
    }

    fn cell_index(cell: CellPos) -> usize {
        assert!(cell.x < CELLS_X && cell.y < CELLS_Y, "cell out of range");
        usize::from(cell.y) * usize::from(CELLS_X) + usize::from(cell.x)
    }

    /// The colours `cell` can show, in bit-pattern order (see [`CellPalette`]).
    ///
    /// # Panics
    /// If `cell` is outside the 40×25 grid.
    pub fn cell_palette(&self, cell: CellPos) -> CellPalette {
        let i = Self::cell_index(cell);
        let s = self.screen[i];
        match self.background {
            Some(bg) => CellPalette::four([bg, s.high, s.low, self.colour[i]]),
            None => CellPalette::two(s.low, s.high),
        }
    }

    /// The colour of logical pixel `x`, `y` (160×200 multicolor, 320×200 hi-res).
    ///
    /// # Panics
    /// If `x` or `y` is outside the picture.
    pub fn pixel(&self, x: u16, y: u16) -> ColourIndex {
        let multicolor = self.background.is_some();
        let (width, per_cell) = if multicolor { (160, 4) } else { (320, 8) };
        assert!(x < width && y < 200, "pixel out of range");
        let cell = CellPos::new(x / per_cell, y / 8);
        let byte =
            self.bitmap[usize::from(cell.y) * 320 + usize::from(cell.x) * 8 + usize::from(y % 8)];
        let value = if multicolor {
            (byte >> (6 - 2 * (x % 4))) & 0b11
        } else {
            (byte >> (7 - x % 8)) & 1
        };
        self.cell_palette(cell)
            .get(value)
            .unwrap_or_else(|| unreachable!("bit pattern is within the palette"))
    }

    /// Decodes every pixel into an [`IndexedBitmap`] (the preview's view of the picture).
    pub fn to_indexed(&self) -> IndexedBitmap {
        let (width, height) = if self.background.is_some() {
            (160, 200)
        } else {
            (320, 200)
        };
        let pixels = (0..height)
            .flat_map(|y| (0..width).map(move |x| (x, y)))
            .map(|(x, y)| self.pixel(x, y))
            .collect();
        IndexedBitmap::new(self.mode, pixels, self.background)
            .unwrap_or_else(|_| unreachable!("sizes match the mode"))
    }
}

impl fmt::Debug for BitmapImage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BitmapImage")
            .field("mode", &self.mode)
            .field("background", &self.background.map(ColourIndex::get))
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::{BitmapImage, ScreenCell};
    use crate::colour::ColourIndex;
    use crate::mode::Mode;
    use crate::model::{CellPos, ModelError};

    #[test]
    fn rejects_wrong_lengths_and_bad_background() {
        assert_eq!(
            BitmapImage::multicolor_from_bytes(&[0; 7999], &[0; 1000], &[0; 1000], 0),
            Err(ModelError::WrongLength {
                what: "bitmap",
                expected: 8000,
                found: 7999
            })
        );
        assert_eq!(
            BitmapImage::hires_from_bytes(&[0; 8000], &[0; 1001]),
            Err(ModelError::WrongLength {
                what: "screen",
                expected: 1000,
                found: 1001
            })
        );
        let err = BitmapImage::multicolor_from_bytes(&[0; 8000], &[0; 1000], &[0; 1000], 0x1F)
            .expect_err("background 0x1F");
        assert_eq!(err.rule().map(|r| r.number()), Some(2));
    }

    #[test]
    fn colour_ram_high_nybble_is_ignored() {
        let img = BitmapImage::multicolor_from_bytes(&[0; 8000], &[0; 1000], &[0xA7; 1000], 0)
            .expect("legal");
        assert_eq!(img.colour()[0], ColourIndex::YELLOW);
        assert_eq!(img.colour_bytes()[0], 0x07);
    }

    #[test]
    fn decodes_multicolor_bit_pairs_and_cell_layout() {
        let mut bitmap = vec![0_u8; 8000];
        // Cell (1, 1) row 2 lives at 1*320 + 1*8 + 2.
        bitmap[320 + 8 + 2] = 0b00_01_10_11;
        let mut screen = vec![0_u8; 1000];
        screen[41] = 0x25; // high = Red (01), low = Green (10)
        let mut colour = vec![0_u8; 1000];
        colour[41] = 0x07; // Yellow (11)
        let img = BitmapImage::multicolor_from_bytes(&bitmap, &screen, &colour, 6).expect("ok");
        assert_eq!(img.mode(), Mode::MulticolorBitmap);
        let y = 8 + 2;
        let got: Vec<ColourIndex> = (4..8).map(|x| img.pixel(x, y)).collect();
        assert_eq!(
            got,
            [
                ColourIndex::BLUE,
                ColourIndex::RED,
                ColourIndex::GREEN,
                ColourIndex::YELLOW
            ]
        );
        assert_eq!(
            img.cell_palette(CellPos::new(1, 1)).as_slice(),
            [
                ColourIndex::BLUE,
                ColourIndex::RED,
                ColourIndex::GREEN,
                ColourIndex::YELLOW
            ]
        );
        let indexed = img.to_indexed();
        assert_eq!(indexed.pixel(5, y), ColourIndex::RED);
        assert_eq!(indexed.background(), Some(ColourIndex::BLUE));
    }

    #[test]
    fn decodes_hires_bits() {
        let mut bitmap = vec![0_u8; 8000];
        bitmap[0] = 0b1000_0001;
        let mut screen = vec![0_u8; 1000];
        screen[0] = ScreenCell {
            high: ColourIndex::WHITE,
            low: ColourIndex::BLACK,
        }
        .to_byte();
        let img = BitmapImage::hires_from_bytes(&bitmap, &screen).expect("ok");
        assert_eq!(img.pixel(0, 0), ColourIndex::WHITE);
        assert_eq!(img.pixel(1, 0), ColourIndex::BLACK);
        assert_eq!(img.pixel(7, 0), ColourIndex::WHITE);
        assert_eq!(img.screen_bytes()[0], 0x10);
        assert!(img.colour().is_empty() && img.background().is_none());
        assert_eq!(
            img.cell_palette(CellPos::new(0, 0)).as_slice(),
            [ColourIndex::BLACK, ColourIndex::WHITE]
        );
    }
}
