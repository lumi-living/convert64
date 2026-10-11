//! [`CharsetImage`]: a character set plus a map of char codes and per-cell colours.

use super::{CellPalette, CellPos, ModelError};
use crate::colour::ColourIndex;
use crate::mode::Mode;
use serde::{Deserialize, Serialize};
use std::fmt;

/// The global colour registers of a charset picture; the variant decides the mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "mode")]
pub enum CharsetGlobals {
    /// Hi-res (standard) charset: background `$D021`.
    Hires {
        /// `$D021`.
        background: ColourIndex,
    },
    /// Multicolor charset: background `$D021`, MC1 `$D022`, MC2 `$D023`.
    Multicolor {
        /// `$D021`, bit pair `00`.
        background: ColourIndex,
        /// `$D022`, bit pair `01`.
        multicolor1: ColourIndex,
        /// `$D023`, bit pair `10`.
        multicolor2: ColourIndex,
    },
    /// ECM charset: four backgrounds `$D021`–`$D024`, chosen by char code bits 6–7.
    Ecm {
        /// `$D021`–`$D024`.
        backgrounds: [ColourIndex; 4],
    },
}

impl CharsetGlobals {
    /// The mode these globals belong to.
    pub const fn mode(self) -> Mode {
        match self {
            Self::Hires { .. } => Mode::HiresCharset,
            Self::Multicolor { .. } => Mode::MulticolorCharset,
            Self::Ecm { .. } => Mode::EcmCharset,
        }
    }
}

/// The per-cell (Colour RAM) colour of a char.
///
/// In multicolor charset mode Colour RAM bit 3 marks the char as multicolor and bits 0–2
/// hold its colour, so `colour` must be 0–7 there (validator rule 2). In the other modes
/// `multicolor` is always `false` and `colour` may be 0–15.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CharColour {
    /// The char's own colour.
    pub colour: ColourIndex,
    /// Whether the char is drawn in multicolor (multicolor charset mode only).
    pub multicolor: bool,
}

impl CharColour {
    /// A hi-res char colour.
    pub const fn hires(colour: ColourIndex) -> Self {
        Self {
            colour,
            multicolor: false,
        }
    }

    /// A multicolor char colour (multicolor charset mode).
    pub const fn multicolor(colour: ColourIndex) -> Self {
        Self {
            colour,
            multicolor: true,
        }
    }

    /// Decodes a Colour RAM nybble for `mode`: in multicolor charset mode bit 3 is the
    /// multicolor flag and bits 0–2 the colour; otherwise all four bits are the colour.
    pub const fn from_colour_ram(nybble: u8, mode: Mode) -> Self {
        let n = nybble & 0x0F;
        if matches!(mode, Mode::MulticolorCharset) {
            Self {
                colour: ColourIndex::from_low_nybble(n & 0x07),
                multicolor: n & 0x08 != 0,
            }
        } else {
            Self::hires(ColourIndex::from_low_nybble(n))
        }
    }
}

/// Tile size in chars, 1–4 in each direction (CharPad's tile sizes).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TileSize {
    /// Width in chars.
    pub width: u8,
    /// Height in chars.
    pub height: u8,
}

impl TileSize {
    /// One char per tile.
    pub const SINGLE: Self = Self {
        width: 1,
        height: 1,
    };
}

/// An encoded charset picture: the character set (8 bytes per char), a map of char codes
/// `width × height` cells, one [`CharColour`] per map cell, the global colours and the
/// tile size the map was built for.
///
/// The map is always in chars; `tile_size` records the tile grid so exporters can group
/// it (`docs/reference/file-formats.md`, CharPad layout). The map may be any size, so a
/// scrolling level is as valid as one 40×25 screen.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "CharsetImageData")]
pub struct CharsetImage {
    charset: Vec<[u8; 8]>,
    map: Vec<u8>,
    width: u16,
    height: u16,
    colour: Vec<CharColour>,
    globals: CharsetGlobals,
    tile_size: TileSize,
}

#[derive(Deserialize)]
struct CharsetImageData {
    charset: Vec<[u8; 8]>,
    map: Vec<u8>,
    width: u16,
    height: u16,
    colour: Vec<CharColour>,
    globals: CharsetGlobals,
    tile_size: TileSize,
}

impl TryFrom<CharsetImageData> for CharsetImage {
    type Error = ModelError;

    fn try_from(d: CharsetImageData) -> Result<Self, Self::Error> {
        Self::new(
            d.charset,
            d.map,
            d.width,
            d.height,
            d.colour,
            d.globals,
            d.tile_size,
        )
    }
}

impl CharsetImage {
    /// A charset picture from its parts.
    ///
    /// The charset may hold any number of chars; whether that fits the mode's budget and
    /// whether every map code refers to an existing char is checked by the validator
    /// (rule 3), as is the 0–7 colour limit of multicolor charset mode (rule 2).
    ///
    /// # Errors
    /// [`ModelError`] if the map is empty, `map` or `colour` is not `width × height`
    /// long, a cell is flagged multicolor outside multicolor charset mode, or the tile
    /// size is outside 1–4 or does not divide the map.
    pub fn new(
        charset: Vec<[u8; 8]>,
        map: Vec<u8>,
        width: u16,
        height: u16,
        colour: Vec<CharColour>,
        globals: CharsetGlobals,
        tile_size: TileSize,
    ) -> Result<Self, ModelError> {
        if width == 0 || height == 0 {
            return Err(ModelError::EmptyMap);
        }
        let cells = usize::from(width) * usize::from(height);
        ModelError::check_len("map", cells, map.len())?;
        ModelError::check_len("colour", cells, colour.len())?;
        let mode = globals.mode();
        if mode != Mode::MulticolorCharset && colour.iter().any(|c| c.multicolor) {
            return Err(ModelError::WrongMode {
                what: "multicolor char",
                mode,
            });
        }
        let ok = |t: u8, dim: u16| (1..=4).contains(&t) && dim.is_multiple_of(u16::from(t));
        if !ok(tile_size.width, width) || !ok(tile_size.height, height) {
            return Err(ModelError::TileSize {
                reason: format!(
                    "tile size {}x{} must be 1-4 chars each way and divide the {width}x{height} map",
                    tile_size.width, tile_size.height
                ),
            });
        }
        Ok(Self {
            charset,
            map,
            width,
            height,
            colour,
            globals,
            tile_size,
        })
    }

    /// The mode, decided by the globals.
    pub const fn mode(&self) -> Mode {
        self.globals.mode()
    }

    /// The character set, 8 bytes per char.
    pub fn charset(&self) -> &[[u8; 8]] {
        &self.charset
    }

    /// Map width in cells.
    pub const fn width(&self) -> u16 {
        self.width
    }

    /// Map height in cells.
    pub const fn height(&self) -> u16 {
        self.height
    }

    /// The char codes, row-major.
    pub fn map(&self) -> &[u8] {
        &self.map
    }

    /// The per-cell colours, row-major.
    pub fn colours(&self) -> &[CharColour] {
        &self.colour
    }

    /// The global colours.
    pub const fn globals(&self) -> CharsetGlobals {
        self.globals
    }

    /// The tile size.
    pub const fn tile_size(&self) -> TileSize {
        self.tile_size
    }

    /// Every cell position, row by row.
    pub fn cells(&self) -> impl Iterator<Item = CellPos> + use<> {
        let (w, h) = (self.width, self.height);
        (0..h).flat_map(move |y| (0..w).map(move |x| CellPos::new(x, y)))
    }

    fn index(&self, cell: CellPos) -> usize {
        assert!(
            cell.x < self.width && cell.y < self.height,
            "cell out of range"
        );
        usize::from(cell.y) * usize::from(self.width) + usize::from(cell.x)
    }

    /// The char code stored at `cell` (Screen RAM value).
    ///
    /// # Panics
    /// If `cell` is outside the map.
    pub fn code(&self, cell: CellPos) -> u8 {
        self.map[self.index(cell)]
    }

    /// The charset index `cell` shows: the code itself, or its low six bits in ECM.
    ///
    /// # Panics
    /// If `cell` is outside the map.
    pub fn char_index(&self, cell: CellPos) -> usize {
        let code = self.code(cell);
        match self.globals {
            CharsetGlobals::Ecm { .. } => usize::from(code & 0x3F),
            _ => usize::from(code),
        }
    }

    /// The colour of `cell`.
    ///
    /// # Panics
    /// If `cell` is outside the map.
    pub fn colour(&self, cell: CellPos) -> CharColour {
        self.colour[self.index(cell)]
    }

    /// The colours `cell` can show, in bit-pattern order (see [`CellPalette`]).
    ///
    /// # Panics
    /// If `cell` is outside the map.
    pub fn cell_palette(&self, cell: CellPos) -> CellPalette {
        let c = self.colour(cell);
        match self.globals {
            CharsetGlobals::Hires { background } => CellPalette::two(background, c.colour),
            CharsetGlobals::Multicolor {
                background,
                multicolor1,
                multicolor2,
            } => {
                if c.multicolor {
                    CellPalette::four([background, multicolor1, multicolor2, c.colour])
                } else {
                    CellPalette::two(background, c.colour)
                }
            }
            CharsetGlobals::Ecm { backgrounds } => {
                CellPalette::two(backgrounds[usize::from(self.code(cell) >> 6)], c.colour)
            }
        }
    }
}

impl fmt::Debug for CharsetImage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CharsetImage")
            .field("globals", &self.globals)
            .field("chars", &self.charset.len())
            .field("map", &format_args!("{}x{}", self.width, self.height))
            .field("tile_size", &self.tile_size)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::{CharColour, CharsetGlobals, CharsetImage, TileSize};
    use crate::colour::ColourIndex;
    use crate::mode::Mode;
    use crate::model::{CellPos, ModelError};

    fn hires(background: ColourIndex) -> CharsetGlobals {
        CharsetGlobals::Hires { background }
    }

    #[test]
    fn enforces_map_sizes_and_tiles() {
        let col = |n| vec![CharColour::hires(ColourIndex::WHITE); n];
        let g = hires(ColourIndex::BLACK);
        assert!(
            CharsetImage::new(vec![[0; 8]], vec![0; 6], 3, 2, col(6), g, TileSize::SINGLE).is_ok()
        );
        assert_eq!(
            CharsetImage::new(vec![[0; 8]], vec![0; 5], 3, 2, col(6), g, TileSize::SINGLE),
            Err(ModelError::WrongLength {
                what: "map",
                expected: 6,
                found: 5
            })
        );
        assert_eq!(
            CharsetImage::new(vec![], vec![], 0, 2, vec![], g, TileSize::SINGLE),
            Err(ModelError::EmptyMap)
        );
        let tile = TileSize {
            width: 2,
            height: 2,
        };
        assert!(matches!(
            CharsetImage::new(vec![[0; 8]], vec![0; 6], 3, 2, col(6), g, tile),
            Err(ModelError::TileSize { .. })
        ));
        let multicolor_in_hires = vec![CharColour::multicolor(ColourIndex::RED); 6];
        assert!(matches!(
            CharsetImage::new(
                vec![[0; 8]],
                vec![0; 6],
                3,
                2,
                multicolor_in_hires,
                g,
                TileSize::SINGLE
            ),
            Err(ModelError::WrongMode { .. })
        ));
    }

    #[test]
    fn colour_ram_decoding() {
        assert_eq!(
            CharColour::from_colour_ram(0x0B, Mode::MulticolorCharset),
            CharColour::multicolor(ColourIndex::CYAN)
        );
        assert_eq!(
            CharColour::from_colour_ram(0x03, Mode::MulticolorCharset),
            CharColour::hires(ColourIndex::CYAN)
        );
        assert_eq!(
            CharColour::from_colour_ram(0x0B, Mode::HiresCharset),
            CharColour::hires(ColourIndex::DARK_GREY)
        );
    }

    #[test]
    fn cell_palettes_per_mode() {
        let mc = CharsetGlobals::Multicolor {
            background: ColourIndex::BLACK,
            multicolor1: ColourIndex::RED,
            multicolor2: ColourIndex::GREEN,
        };
        let colours = vec![
            CharColour::multicolor(ColourIndex::YELLOW),
            CharColour::hires(ColourIndex::WHITE),
        ];
        let img = CharsetImage::new(
            vec![[0; 8]],
            vec![0, 0],
            2,
            1,
            colours,
            mc,
            TileSize::SINGLE,
        )
        .expect("ok");
        assert_eq!(img.mode(), Mode::MulticolorCharset);
        assert_eq!(
            img.cell_palette(CellPos::new(0, 0)).as_slice(),
            [
                ColourIndex::BLACK,
                ColourIndex::RED,
                ColourIndex::GREEN,
                ColourIndex::YELLOW
            ]
        );
        assert_eq!(
            img.cell_palette(CellPos::new(1, 0)).as_slice(),
            [ColourIndex::BLACK, ColourIndex::WHITE]
        );

        let ecm = CharsetGlobals::Ecm {
            backgrounds: [
                ColourIndex::BLACK,
                ColourIndex::BLUE,
                ColourIndex::RED,
                ColourIndex::GREEN,
            ],
        };
        let img = CharsetImage::new(
            vec![[0; 8]; 3],
            vec![0x82],
            1,
            1,
            vec![CharColour::hires(ColourIndex::WHITE)],
            ecm,
            TileSize::SINGLE,
        )
        .expect("ok");
        assert_eq!(img.char_index(CellPos::new(0, 0)), 2);
        assert_eq!(
            img.cell_palette(CellPos::new(0, 0)).as_slice(),
            [ColourIndex::RED, ColourIndex::WHITE]
        );
    }
}
