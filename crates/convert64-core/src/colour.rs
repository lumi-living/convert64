//! [`ColourIndex`]: one of the 16 fixed VIC-II colours.
//!
//! Index semantics never change between palettes (`docs/reference/vic-ii-constraints.md` §1):
//! a palette only decides what RGB value an index is shown as.

use serde::{Deserialize, Serialize};
use std::fmt;

/// A VIC-II colour, `0..=15`. Every colour stored anywhere in Convert64 uses this type,
/// so an out-of-range value cannot exist once constructed. Serialises as a plain number;
/// deserialising rejects values above 15.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
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

/// A set of [`ColourIndex`] values, stored as a 16-bit mask. Used for "which colours
/// does this cell use" questions without allocating.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ColourSet(u16);

impl ColourSet {
    /// The empty set.
    pub const EMPTY: Self = Self(0);

    /// Adds `c`.
    pub const fn insert(&mut self, c: ColourIndex) {
        self.0 |= 1 << c.0;
    }

    /// The set with `c` added.
    #[must_use]
    pub const fn with(mut self, c: ColourIndex) -> Self {
        self.insert(c);
        self
    }

    /// Whether `c` is in the set.
    pub const fn contains(self, c: ColourIndex) -> bool {
        self.0 & (1 << c.0) != 0
    }

    /// Number of colours in the set.
    pub const fn len(self) -> u32 {
        self.0.count_ones()
    }

    /// Whether the set is empty.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Colours in either set.
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Colours in `self` but not in `other`.
    #[must_use]
    pub const fn difference(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }

    /// The colours in index order.
    pub fn iter(self) -> impl Iterator<Item = ColourIndex> {
        ColourIndex::ALL
            .into_iter()
            .filter(move |&c| self.contains(c))
    }
}

impl FromIterator<ColourIndex> for ColourSet {
    fn from_iter<I: IntoIterator<Item = ColourIndex>>(iter: I) -> Self {
        let mut set = Self::EMPTY;
        for c in iter {
            set.insert(c);
        }
        set
    }
}

impl fmt::Display for ColourSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        for c in self.iter() {
            if !first {
                f.write_str(", ")?;
            }
            first = false;
            write!(f, "{}", c.get())?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{ColourIndex, ColourSet, InvalidColourIndex};

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

    #[test]
    fn colour_set_operations() {
        let a: ColourSet = [ColourIndex::BLACK, ColourIndex::RED, ColourIndex::RED]
            .into_iter()
            .collect();
        assert_eq!(a.len(), 2);
        assert!(a.contains(ColourIndex::RED) && !a.contains(ColourIndex::WHITE));
        let b = ColourSet::EMPTY
            .with(ColourIndex::RED)
            .with(ColourIndex::WHITE);
        assert_eq!(a.union(b).len(), 3);
        assert_eq!(
            a.difference(b).iter().collect::<Vec<_>>(),
            [ColourIndex::BLACK]
        );
        assert_eq!(a.union(b).to_string(), "0, 1, 2");
        assert!(ColourSet::EMPTY.is_empty());
    }

    #[test]
    fn deserialising_checks_the_range() {
        use serde::Deserialize;
        use serde::de::IntoDeserializer;
        use serde::de::value::{Error, U8Deserializer};
        let ok: U8Deserializer<Error> = 9_u8.into_deserializer();
        assert_eq!(ColourIndex::deserialize(ok), Ok(ColourIndex::BROWN));
        let bad: U8Deserializer<Error> = 16_u8.into_deserializer();
        assert!(ColourIndex::deserialize(bad).is_err());
    }
}
