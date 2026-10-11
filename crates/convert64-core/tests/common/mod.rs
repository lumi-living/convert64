//! Builders for hand-made images, shared by the integration tests.
//!
//! Every builder produces a **legal** image by default. Tests then break exactly one
//! thing, so each test reads as "a legal image, except …".

#![allow(
    dead_code,
    reason = "each integration test binary uses a different subset of these helpers"
)]

use convert64_core::mode::{BlockKind, Mode};
use convert64_core::model::{
    BitmapImage, CharColour, CharsetGlobals, CharsetImage, IndexedBitmap, OverlayPair, Sprite,
    SpritePos, SpriteSet, TileSize,
};
use convert64_core::{ColourIndex, Placement};

/// Shorthand for a colour by index; panics on values above 15 (test input only).
pub fn c(index: u8) -> ColourIndex {
    ColourIndex::new(index).expect("test colours are 0-15")
}

// ---------------------------------------------------------------------------------------
// Bitmaps.

/// A legal multicolor picture: background black, and every cell striped with three
/// colours that change from cell to cell, so all 1000 cells use the full 4.
pub fn mc_bitmap() -> IndexedBitmap {
    let mut img = IndexedBitmap::filled(Mode::MulticolorBitmap, ColourIndex::BLACK).expect("legal");
    for y in 0..200_u16 {
        for x in 0..160_u16 {
            let cell = (x / 4) + (y / 8) * 40;
            let stripe = x % 4;
            if stripe > 0 {
                // Three non-black colours per cell, chosen from 1..=15.
                let colour = 1 + ((cell + stripe * 5) % 15);
                img.set_pixel(x, y, c(u8::try_from(colour).expect("1..=15")));
            }
        }
    }
    img
}

/// A legal hi-res picture: each 8×8 cell uses exactly two colours.
pub fn hires_bitmap() -> IndexedBitmap {
    let mut img = IndexedBitmap::filled(Mode::HiresBitmap, ColourIndex::BLACK).expect("legal");
    for y in 0..200_u16 {
        for x in 0..320_u16 {
            let cell = (x / 8) + (y / 8) * 40;
            if (x + y) % 2 == 0 {
                img.set_pixel(x, y, c(u8::try_from(1 + cell % 15).expect("1..=15")));
            }
        }
    }
    img
}

/// A legal encoded multicolor bitmap from arbitrary (but fixed) bytes.
pub fn encoded_mc_bitmap() -> BitmapImage {
    let bitmap: Vec<u8> = (0..8000_u32).map(|i| (i * 37 % 256) as u8).collect();
    let screen: Vec<u8> = (0..1000_u32).map(|i| (i * 13 % 256) as u8).collect();
    let colour: Vec<u8> = (0..1000_u32).map(|i| (i % 16) as u8).collect();
    BitmapImage::multicolor_from_bytes(&bitmap, &screen, &colour, 6).expect("legal")
}

/// A legal encoded hi-res bitmap from arbitrary (but fixed) bytes.
pub fn encoded_hires_bitmap() -> BitmapImage {
    let bitmap: Vec<u8> = (0..8000_u32).map(|i| (i * 91 % 256) as u8).collect();
    let screen: Vec<u8> = (0..1000_u32).map(|i| (i * 7 % 256) as u8).collect();
    BitmapImage::hires_from_bytes(&bitmap, &screen).expect("legal")
}

// ---------------------------------------------------------------------------------------
// Charsets.

/// Builds a charset picture; defaults to a legal 40×25 screen.
#[derive(Clone, Debug)]
pub struct CharsetBuilder {
    globals: CharsetGlobals,
    chars: usize,
    width: u16,
    height: u16,
    codes: Option<Vec<u8>>,
    colours: Option<Vec<CharColour>>,
}

impl CharsetBuilder {
    fn new(globals: CharsetGlobals, chars: usize) -> Self {
        Self {
            globals,
            chars,
            width: 40,
            height: 25,
            codes: None,
            colours: None,
        }
    }

    /// Hi-res charset, 200 chars, background blue.
    pub fn hires() -> Self {
        Self::new(
            CharsetGlobals::Hires {
                background: ColourIndex::BLUE,
            },
            200,
        )
    }

    /// Multicolor charset, 200 chars, globals black/red/green.
    pub fn multicolor() -> Self {
        Self::new(
            CharsetGlobals::Multicolor {
                background: ColourIndex::BLACK,
                multicolor1: ColourIndex::RED,
                multicolor2: ColourIndex::GREEN,
            },
            200,
        )
    }

    /// ECM charset, 64 chars, four backgrounds.
    pub fn ecm() -> Self {
        Self::new(
            CharsetGlobals::Ecm {
                backgrounds: [
                    ColourIndex::BLACK,
                    ColourIndex::BLUE,
                    ColourIndex::BROWN,
                    ColourIndex::DARK_GREY,
                ],
            },
            64,
        )
    }

    /// Number of chars in the charset.
    pub fn chars(mut self, n: usize) -> Self {
        self.chars = n;
        self
    }

    /// Map size in cells.
    pub fn size(mut self, width: u16, height: u16) -> Self {
        self.width = width;
        self.height = height;
        self
    }

    /// Sets the code at one cell (after the default codes are laid out).
    pub fn code_at(mut self, x: u16, y: u16, code: u8) -> Self {
        let mut codes = self.codes.take().unwrap_or_else(|| self.default_codes());
        codes[usize::from(y) * usize::from(self.width) + usize::from(x)] = code;
        self.codes = Some(codes);
        self
    }

    /// Sets the colour at one cell.
    pub fn colour_at(mut self, x: u16, y: u16, colour: CharColour) -> Self {
        let mut colours = self
            .colours
            .take()
            .unwrap_or_else(|| self.default_colours());
        colours[usize::from(y) * usize::from(self.width) + usize::from(x)] = colour;
        self.colours = Some(colours);
        self
    }

    fn cells(&self) -> usize {
        usize::from(self.width) * usize::from(self.height)
    }

    /// Codes cycle through every char; in ECM the background bits cycle too.
    fn default_codes(&self) -> Vec<u8> {
        let chars = self.chars.max(1);
        (0..self.cells())
            .map(|i| {
                let index = (i % chars.min(256)) as u8;
                match self.globals {
                    CharsetGlobals::Ecm { .. } => (index & 0x3F) | (((i % 4) as u8) << 6),
                    _ => index,
                }
            })
            .collect()
    }

    /// Colours cycle through the legal range for the mode.
    fn default_colours(&self) -> Vec<CharColour> {
        (0..self.cells())
            .map(|i| match self.globals {
                CharsetGlobals::Multicolor { .. } => {
                    let colour = c((i % 8) as u8);
                    if i % 3 == 0 {
                        CharColour::hires(colour)
                    } else {
                        CharColour::multicolor(colour)
                    }
                }
                _ => CharColour::hires(c((i % 16) as u8)),
            })
            .collect()
    }

    /// The picture.
    pub fn build(self) -> CharsetImage {
        let charset: Vec<[u8; 8]> = (0..self.chars)
            .map(|i| [(i % 256) as u8, 0x81, 0x42, 0x24, 0x18, 0x24, 0x42, 0x81])
            .collect();
        let codes = self.codes.clone().unwrap_or_else(|| self.default_codes());
        let colours = self
            .colours
            .clone()
            .unwrap_or_else(|| self.default_colours());
        CharsetImage::new(
            charset,
            codes,
            self.width,
            self.height,
            colours,
            self.globals,
            TileSize::SINGLE,
        )
        .expect("builder sizes are consistent")
    }
}

// ---------------------------------------------------------------------------------------
// Sprites.

/// A hi-res sprite with a diagonal line.
pub fn hires_sprite(colour: ColourIndex) -> Sprite {
    let mut px = [0_u8; 63];
    for y in 0..21 {
        px[y * 3 + y / 8] |= 0x80 >> (y % 8);
    }
    Sprite::new(px, colour, false)
}

/// A multicolor sprite using all three colours.
pub fn multicolor_sprite(colour: ColourIndex) -> Sprite {
    let mut px = [0_u8; 63];
    for (i, b) in px.iter_mut().enumerate() {
        *b = [0b01_10_11_00, 0b11_10_01_00, 0b10_10_10_10][i % 3];
    }
    Sprite::new(px, colour, true)
}

/// Builds a sprite set; defaults to two hi-res and two multicolor sprites, the second
/// hi-res sprite overlaid on the second multicolor one at the same position.
#[derive(Clone, Debug)]
pub struct SpriteSetBuilder {
    set: SpriteSet,
}

impl SpriteSetBuilder {
    /// The default legal set.
    pub fn new() -> Self {
        let sprites = vec![
            hires_sprite(ColourIndex::WHITE),
            hires_sprite(ColourIndex::YELLOW),
            multicolor_sprite(ColourIndex::RED),
            multicolor_sprite(ColourIndex::PURPLE),
        ];
        let positions = vec![
            SpritePos { x: 24, y: 50 },
            SpritePos { x: 100, y: 120 },
            SpritePos { x: 60, y: 50 },
            SpritePos { x: 100, y: 120 },
        ];
        let set = SpriteSet::new(sprites, ColourIndex::BLACK, ColourIndex::LIGHT_BLUE)
            .with_positions(positions)
            .expect("one position per sprite")
            .with_overlays(vec![OverlayPair {
                hires: 1,
                multicolor: 3,
            }]);
        Self { set }
    }

    /// Moves sprite `index`.
    pub fn position(mut self, index: usize, x: u16, y: u16) -> Self {
        self.set.positions_mut()[index] = SpritePos { x, y };
        self
    }

    /// Replaces the overlay pairs.
    pub fn overlays(mut self, overlays: Vec<OverlayPair>) -> Self {
        self.set = self.set.with_overlays(overlays);
        self
    }

    /// The set.
    pub fn build(self) -> SpriteSet {
        self.set
    }
}

impl Default for SpriteSetBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------------------
// Placement.

/// The recommended bank-1 layout (reference §5): bitmap $6000, screen $5C00,
/// charset $4800, sprites from $4000, colour RAM $D800.
pub fn bank1_placement() -> Placement {
    Placement::new()
        .with(BlockKind::Bitmap, 0x6000)
        .with(BlockKind::ScreenRam, 0x5C00)
        .with(BlockKind::Charset, 0x4800)
        .with(BlockKind::Sprite, 0x4000)
        .with(BlockKind::Sprite, 0x4040)
        .with(BlockKind::ColourRam, 0xD800)
}
