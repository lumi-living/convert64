//! [`SpriteSet`]: sprites sharing one pair of multicolor registers.

use super::ModelError;
use crate::colour::ColourIndex;
use serde::{Deserialize, Serialize};
use std::fmt;

/// One 64-byte sprite block: 21 rows × 3 bytes of pixels (bit 7 leftmost) and the
/// attribute byte at offset 63 (SpritePad convention, ignored by the hardware): bit 7
/// multicolor, bit 6 overlay, bits 0–3 the sprite's own colour (`$D027+n`).
///
/// The colour and multicolor flag live only in the attribute byte, so they cannot
/// disagree with the data.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Sprite {
    bytes: [u8; 64],
}

impl Sprite {
    /// Bytes per sprite block.
    pub const BYTES: usize = 64;

    /// A sprite from its 64-byte block (pixels and attribute byte). Any 64 bytes are a
    /// valid block.
    pub const fn from_block(bytes: [u8; 64]) -> Self {
        Self { bytes }
    }

    /// A sprite from 63 pixel bytes, its colour and whether it is multicolor.
    pub fn new(pixels: [u8; 63], colour: ColourIndex, multicolor: bool) -> Self {
        let mut bytes = [0_u8; 64];
        bytes[..63].copy_from_slice(&pixels);
        bytes[63] = colour.get() | if multicolor { 0x80 } else { 0 };
        Self { bytes }
    }

    /// The 64-byte block.
    pub const fn bytes(&self) -> &[u8; 64] {
        &self.bytes
    }

    /// The sprite's own colour (`$D027+n`), from the attribute byte.
    pub const fn colour(&self) -> ColourIndex {
        ColourIndex::from_low_nybble(self.bytes[63])
    }

    /// Whether the sprite is multicolor, from the attribute byte.
    pub const fn is_multicolor(&self) -> bool {
        self.bytes[63] & 0x80 != 0
    }

    /// The raw pixel value at `x`, `y`: 0–1 for hi-res (`x` 0–23), 0–3 for multicolor
    /// (`x` 0–11, double-wide pixels).
    ///
    /// # Panics
    /// If `x` or `y` is outside the sprite.
    pub fn raw_pixel(&self, x: u8, y: u8) -> u8 {
        let width = if self.is_multicolor() { 12 } else { 24 };
        assert!(x < width && y < 21, "sprite pixel out of range");
        let row = usize::from(y) * 3;
        if self.is_multicolor() {
            let byte = self.bytes[row + usize::from(x / 4)];
            (byte >> (6 - 2 * (x % 4))) & 0b11
        } else {
            let byte = self.bytes[row + usize::from(x / 8)];
            (byte >> (7 - x % 8)) & 1
        }
    }
}

impl fmt::Debug for Sprite {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Sprite")
            .field("colour", &self.colour().get())
            .field("multicolor", &self.is_multicolor())
            .finish_non_exhaustive()
    }
}

impl Serialize for Sprite {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.bytes.as_slice().serialize(s)
    }
}

impl<'de> Deserialize<'de> for Sprite {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = Vec::<u8>::deserialize(d)?;
        let bytes: [u8; 64] = v.try_into().map_err(|v: Vec<u8>| {
            serde::de::Error::custom(ModelError::SpriteSize { found: v.len() })
        })?;
        Ok(Self::from_block(bytes))
    }
}

/// Where a sprite is shown, in VIC-II sprite coordinates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SpritePos {
    /// X (`$D000+2n` with the high bit in `$D010`).
    pub x: u16,
    /// Y (`$D001+2n`).
    pub y: u16,
}

/// A hi-res sprite drawn over a multicolor sprite. Rule 4: both must share coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct OverlayPair {
    /// Index of the hi-res sprite in the set.
    pub hires: usize,
    /// Index of the multicolor sprite in the set.
    pub multicolor: usize,
}

/// A set of sprites shown together, with the two multicolor registers every
/// multicolor sprite shares (`$D025`, `$D026`), each sprite's position, and overlay
/// pairs.
///
/// One MC1/MC2 pair per set is enforced by the type (rule 4); positions default to
/// `(0, 0)`, which suits a sprite sheet whose overlay frames are designed in place.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "SpriteSetData")]
pub struct SpriteSet {
    sprites: Vec<Sprite>,
    positions: Vec<SpritePos>,
    multicolor1: ColourIndex,
    multicolor2: ColourIndex,
    overlays: Vec<OverlayPair>,
}

#[derive(Deserialize)]
struct SpriteSetData {
    sprites: Vec<Sprite>,
    positions: Vec<SpritePos>,
    multicolor1: ColourIndex,
    multicolor2: ColourIndex,
    overlays: Vec<OverlayPair>,
}

impl TryFrom<SpriteSetData> for SpriteSet {
    type Error = ModelError;

    fn try_from(d: SpriteSetData) -> Result<Self, Self::Error> {
        let set = Self::new(d.sprites, d.multicolor1, d.multicolor2);
        set.with_positions(d.positions)
            .map(|s| s.with_overlays(d.overlays))
    }
}

impl SpriteSet {
    /// A set of sprites at `(0, 0)` with no overlays.
    pub fn new(sprites: Vec<Sprite>, multicolor1: ColourIndex, multicolor2: ColourIndex) -> Self {
        let positions = vec![SpritePos::default(); sprites.len()];
        Self {
            sprites,
            positions,
            multicolor1,
            multicolor2,
            overlays: Vec::new(),
        }
    }

    /// A set from raw sprite memory (64 bytes per sprite) and the two register values.
    ///
    /// # Errors
    /// [`ModelError::SpriteSize`] (rule 4) unless `data` is a whole number of 64-byte
    /// blocks; [`ModelError::Colour`] (rule 2) for a register value above 15.
    pub fn from_bytes(data: &[u8], multicolor1: u8, multicolor2: u8) -> Result<Self, ModelError> {
        if !data.len().is_multiple_of(Sprite::BYTES) {
            return Err(ModelError::SpriteSize { found: data.len() });
        }
        let mc1 = ModelError::colour("sprite multicolor 1", multicolor1)?;
        let mc2 = ModelError::colour("sprite multicolor 2", multicolor2)?;
        let (blocks, _) = data.as_chunks::<{ Sprite::BYTES }>();
        let sprites = blocks.iter().map(|&b| Sprite::from_block(b)).collect();
        Ok(Self::new(sprites, mc1, mc2))
    }

    /// The set with these positions, one per sprite.
    ///
    /// # Errors
    /// [`ModelError::WrongLength`] unless there is exactly one position per sprite.
    pub fn with_positions(mut self, positions: Vec<SpritePos>) -> Result<Self, ModelError> {
        ModelError::check_len("positions", self.sprites.len(), positions.len())?;
        self.positions = positions;
        Ok(self)
    }

    /// The set with these overlay pairs (checked by the validator, rule 4).
    #[must_use]
    pub fn with_overlays(mut self, overlays: Vec<OverlayPair>) -> Self {
        self.overlays = overlays;
        self
    }

    /// The sprites.
    pub fn sprites(&self) -> &[Sprite] {
        &self.sprites
    }

    /// Mutable access to the sprites (block contents only; the count is fixed).
    pub fn sprites_mut(&mut self) -> &mut [Sprite] {
        &mut self.sprites
    }

    /// One position per sprite.
    pub fn positions(&self) -> &[SpritePos] {
        &self.positions
    }

    /// Mutable access to the positions.
    pub fn positions_mut(&mut self) -> &mut [SpritePos] {
        &mut self.positions
    }

    /// Shared multicolor 1 (`$D025`, bit pair `01`).
    pub const fn multicolor1(&self) -> ColourIndex {
        self.multicolor1
    }

    /// Shared multicolor 2 (`$D026`, bit pair `11`).
    pub const fn multicolor2(&self) -> ColourIndex {
        self.multicolor2
    }

    /// The overlay pairs.
    pub fn overlays(&self) -> &[OverlayPair] {
        &self.overlays
    }

    /// The colour of pixel `x`, `y` of sprite `index`, or `None` where it is transparent.
    /// Multicolor: `00` transparent, `01` MC1, `10` the sprite's colour, `11` MC2.
    ///
    /// # Panics
    /// If `index`, `x` or `y` is out of range.
    pub fn pixel(&self, index: usize, x: u8, y: u8) -> Option<ColourIndex> {
        let s = &self.sprites[index];
        match (s.is_multicolor(), s.raw_pixel(x, y)) {
            (_, 0) => None,
            (false, _) | (true, 0b10) => Some(s.colour()),
            (true, 0b01) => Some(self.multicolor1),
            (true, _) => Some(self.multicolor2),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Sprite, SpritePos, SpriteSet};
    use crate::colour::ColourIndex;
    use crate::model::ModelError;

    #[test]
    fn from_bytes_needs_whole_blocks() {
        assert_eq!(
            SpriteSet::from_bytes(&[0; 63], 0, 0),
            Err(ModelError::SpriteSize { found: 63 })
        );
        let err = SpriteSet::from_bytes(&[0; 65], 0, 0).expect_err("65 bytes");
        assert_eq!(err.rule().map(|r| r.number()), Some(4));
        assert_eq!(
            SpriteSet::from_bytes(&[0; 128], 1, 2).map(|s| s.sprites().len()),
            Ok(2)
        );
        assert!(matches!(
            SpriteSet::from_bytes(&[0; 64], 16, 0),
            Err(ModelError::Colour { .. })
        ));
    }

    #[test]
    fn attribute_byte_and_pixels() {
        let mut px = [0_u8; 63];
        px[0] = 0b01_10_11_00;
        let s = Sprite::new(px, ColourIndex::CYAN, true);
        assert_eq!(s.bytes()[63], 0x83);
        assert!(s.is_multicolor());
        let set = SpriteSet::new(vec![s], ColourIndex::RED, ColourIndex::GREEN);
        assert_eq!(set.pixel(0, 0, 0), Some(ColourIndex::RED));
        assert_eq!(set.pixel(0, 1, 0), Some(ColourIndex::CYAN));
        assert_eq!(set.pixel(0, 2, 0), Some(ColourIndex::GREEN));
        assert_eq!(set.pixel(0, 3, 0), None);

        let mut px = [0_u8; 63];
        px[62] = 0b0000_0001;
        let h = Sprite::new(px, ColourIndex::WHITE, false);
        let set = SpriteSet::new(vec![h], ColourIndex::RED, ColourIndex::GREEN);
        assert_eq!(set.pixel(0, 23, 20), Some(ColourIndex::WHITE));
        assert_eq!(set.pixel(0, 22, 20), None);
    }

    #[test]
    fn positions_must_match_sprite_count() {
        let set = SpriteSet::from_bytes(&[0; 128], 0, 0).expect("ok");
        assert!(
            set.clone()
                .with_positions(vec![SpritePos::default()])
                .is_err()
        );
        let set = set
            .with_positions(vec![SpritePos { x: 24, y: 50 }, SpritePos { x: 48, y: 50 }])
            .expect("ok");
        assert_eq!(set.positions()[1].x, 48);
    }
}
