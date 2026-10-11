//! [`ColourIndex`]: one of the 16 fixed VIC-II colours.
//!
//! Index semantics never change between palettes (`docs/reference/vic-ii-constraints.md` §1):
//! a palette only decides what RGB value an index is shown as.

use std::fmt;

/// A VIC-II colour, `0..=15`. Every colour stored anywhere in Convert64 uses this type,
/// so an out-of-range value cannot exist once constructed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ColourIndex(u8);

/// Error for a value that is not a palette index `0..=15` (validator rule 2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidColourIndex(pub u8);

impl fmt::Display for InvalidColourIndex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "colour value {} (${:02X}) is not a palette index 0-15",
            self.0, self.0
        )
    }
}

impl std::error::Error for InvalidColourIndex {}

const NAMES: [&str; 16] = [
    "Black",
    "White",
    "Red",
    "Cyan",
    "Purple",
    "Green",
    "Blue",
    "Yellow",
    "Orange",
    "Brown",
    "Light Red",
    "Dark Grey",
    "Medium Grey",
    "Light Green",
    "Light Blue",
    "Light Grey",
];

impl ColourIndex {
    /// 0, Black.
    pub const BLACK: Self = Self(0);
    /// 1, White.
    pub const WHITE: Self = Self(1);
    /// 2, Red.
    pub const RED: Self = Self(2);
    /// 3, Cyan.
    pub const CYAN: Self = Self(3);
    /// 4, Purple.
    pub const PURPLE: Self = Self(4);
    /// 5, Green.
    pub const GREEN: Self = Self(5);
    /// 6, Blue.
    pub const BLUE: Self = Self(6);
    /// 7, Yellow.
    pub const YELLOW: Self = Self(7);
    /// 8, Orange.
    pub const ORANGE: Self = Self(8);
    /// 9, Brown.
    pub const BROWN: Self = Self(9);
    /// 10, Light Red.
    pub const LIGHT_RED: Self = Self(10);
    /// 11, Dark Grey.
    pub const DARK_GREY: Self = Self(11);
    /// 12, Medium Grey.
    pub const MEDIUM_GREY: Self = Self(12);
    /// 13, Light Green.
    pub const LIGHT_GREEN: Self = Self(13);
    /// 14, Light Blue.
    pub const LIGHT_BLUE: Self = Self(14);
    /// 15, Light Grey.
    pub const LIGHT_GREY: Self = Self(15);

    /// All 16 colours in index order.
    pub const ALL: [Self; 16] = [
        Self(0),
        Self(1),
        Self(2),
        Self(3),
        Self(4),
        Self(5),
        Self(6),
        Self(7),
        Self(8),
        Self(9),
        Self(10),
        Self(11),
        Self(12),
        Self(13),
        Self(14),
        Self(15),
    ];

    /// Checks that `value` is a palette index `0..=15`.
    ///
    /// # Errors
    /// [`InvalidColourIndex`] for any value above 15.
    pub const fn new(value: u8) -> Result<Self, InvalidColourIndex> {
        if value < 16 {
            Ok(Self(value))
        } else {
            Err(InvalidColourIndex(value))
        }
    }

    /// The colour in the low four bits of `byte`; the high bits are ignored.
    ///
    /// Use this only where the hardware itself ignores the high nybble, such as
    /// Colour RAM, which is four bits wide.
    pub const fn from_low_nybble(byte: u8) -> Self {
        Self(byte & 0x0F)
    }

    /// The colour in the high four bits of `byte` (Screen RAM in bitmap modes).
    pub const fn from_high_nybble(byte: u8) -> Self {
        Self(byte >> 4)
    }

    /// The index as a number `0..=15`.
    pub const fn get(self) -> u8 {
        self.0
    }

    /// The index as a `usize`, for table lookups.
    pub const fn as_usize(self) -> usize {
        self.0 as usize
    }

    /// The colour's conventional name, as in the reference table.
    pub const fn name(self) -> &'static str {
        NAMES[self.0 as usize]
    }
}

impl TryFrom<u8> for ColourIndex {
    type Error = InvalidColourIndex;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<ColourIndex> for u8 {
    fn from(c: ColourIndex) -> Self {
        c.0
    }
}

impl fmt::Display for ColourIndex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.0, self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::{ColourIndex, InvalidColourIndex};

    #[test]
    fn accepts_0_to_15_and_rejects_16() {
        for v in 0..16 {
            assert_eq!(ColourIndex::new(v).map(ColourIndex::get), Ok(v));
        }
        assert_eq!(ColourIndex::new(16), Err(InvalidColourIndex(16)));
        assert_eq!(ColourIndex::new(255), Err(InvalidColourIndex(255)));
    }

    #[test]
    fn nybble_helpers() {
        assert_eq!(ColourIndex::from_low_nybble(0xA7), ColourIndex::YELLOW);
        assert_eq!(ColourIndex::from_high_nybble(0xA7), ColourIndex::LIGHT_RED);
    }

    #[test]
    fn all_is_in_index_order() {
        for (i, c) in ColourIndex::ALL.iter().enumerate() {
            assert_eq!(c.as_usize(), i);
        }
        assert_eq!(ColourIndex::LIGHT_GREY.name(), "Light Grey");
    }
}
