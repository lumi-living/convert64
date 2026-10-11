//! The C64 image model: what solvers produce, format readers decode into, writers encode
//! from and the validator checks (`docs/TAD.md` §4).
//!
//! Two layers:
//! - **Pixel level:** [`IndexedBitmap`], one [`ColourIndex`] per logical pixel. This is
//!   the only layer where validator rule 1 (colours per cell) can be broken, because the
//!   encoded formats cannot express more colours per cell than the hardware shows.
//! - **Encoded:** [`BitmapImage`], [`CharsetImage`] and [`SpriteSet`], the bytes the
//!   VIC-II reads, with colours lifted into [`ColourIndex`].
//!
//! Constructors enforce sizes (and the 0–15 range of every colour); everything else is
//! the validator's job. Deserialising goes through the same constructors.

mod bitmap;
mod charset;
mod indexed;
mod sprite;

pub use bitmap::{BitmapImage, ScreenCell};
pub use charset::{CharColour, CharsetGlobals, CharsetImage, TileSize};
pub use indexed::IndexedBitmap;
pub use sprite::{OverlayPair, Sprite, SpritePos, SpriteSet};

use crate::colour::{ColourIndex, InvalidColourIndex};
use crate::mode::Mode;
use crate::rule::Rule;
use serde::{Deserialize, Serialize};
use std::fmt;

/// A cell (character position) on a screen or map: column `x`, row `y`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CellPos {
    /// Column, from 0 at the left.
    pub x: u16,
    /// Row, from 0 at the top.
    pub y: u16,
}

impl CellPos {
    /// The cell at column `x`, row `y`.
    pub const fn new(x: u16, y: u16) -> Self {
        Self { x, y }
    }
}

impl fmt::Display for CellPos {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({}, {})", self.x, self.y)
    }
}

/// The colours a cell can show, in bit-pattern order.
///
/// Multicolor bitmap: `[bg (00), screen high (01), screen low (10), colour RAM (11)]`.
/// Hi-res bitmap: `[screen low (0), screen high (1)]`.
/// Charsets: `[bg, char colour]` (hi-res), `[bg, MC1, MC2, char colour]` (multicolor
/// char) or `[selected ECM bg, char colour]` (ECM).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellPalette {
    slots: [ColourIndex; 4],
    len: u8,
}

impl CellPalette {
    pub(crate) fn two(a: ColourIndex, b: ColourIndex) -> Self {
        Self {
            slots: [a, b, ColourIndex::BLACK, ColourIndex::BLACK],
            len: 2,
        }
    }

    pub(crate) const fn four(slots: [ColourIndex; 4]) -> Self {
        Self { slots, len: 4 }
    }

    /// The slots in bit-pattern order.
    pub fn as_slice(&self) -> &[ColourIndex] {
        &self.slots[..usize::from(self.len)]
    }

    /// The colour shown for bit pattern `value` (0–1 hi-res, 0–3 multicolor).
    pub fn get(&self, value: u8) -> Option<ColourIndex> {
        self.as_slice().get(usize::from(value)).copied()
    }
}

/// Why a model value could not be constructed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelError {
    /// A buffer has the wrong length.
    WrongLength {
        /// What the buffer is, for example `"bitmap"`.
        what: &'static str,
        /// Required length.
        expected: usize,
        /// Actual length.
        found: usize,
    },
    /// The mode does not fit this kind of image.
    WrongMode {
        /// What was being built.
        what: &'static str,
        /// The mode given.
        mode: Mode,
    },
    /// A colour value is outside 0–15 (validator rule 2).
    Colour {
        /// Which colour, for example `"background"`.
        what: &'static str,
        /// The underlying error.
        source: InvalidColourIndex,
    },
    /// A multicolor background is required but missing, or given in a hi-res mode.
    Background {
        /// The mode.
        mode: Mode,
    },
    /// Sprite data is not a whole number of 64-byte blocks (validator rule 4).
    SpriteSize {
        /// Bytes given.
        found: usize,
    },
    /// Tile width or height outside 1–4, or a map that is not a whole number of tiles.
    TileSize {
        /// Explanation.
        reason: String,
    },
    /// A charset map must be at least one cell in each direction.
    EmptyMap,
}

impl ModelError {
    /// The validator rule this error enforces, when it is one of them.
    pub const fn rule(&self) -> Option<Rule> {
        match self {
            Self::Colour { .. } => Some(Rule::ColourRange),
            Self::SpriteSize { .. } => Some(Rule::Sprites),
            _ => None,
        }
    }

    pub(crate) fn colour(what: &'static str, value: u8) -> Result<ColourIndex, Self> {
        ColourIndex::new(value).map_err(|source| Self::Colour { what, source })
    }

    pub(crate) fn check_len(what: &'static str, expected: usize, found: usize) -> Result<(), Self> {
        if expected == found {
            Ok(())
        } else {
            Err(Self::WrongLength {
                what,
                expected,
                found,
            })
        }
    }
}

impl fmt::Display for ModelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongLength {
                what,
                expected,
                found,
            } => write!(f, "{what} must be {expected} bytes, found {found}"),
            Self::WrongMode { what, mode } => write!(f, "{what} cannot use mode {mode}"),
            Self::Colour { what, source } => write!(f, "{what}: {source}"),
            Self::Background { mode } => {
                if *mode == Mode::MulticolorBitmap {
                    write!(f, "{mode} needs a background colour")
                } else {
                    write!(f, "{mode} has no background colour")
                }
            }
            Self::SpriteSize { found } => write!(
                f,
                "sprite data must be a multiple of 64 bytes, found {found}"
            ),
            Self::TileSize { reason } => f.write_str(reason),
            Self::EmptyMap => f.write_str("a charset map needs at least one cell"),
        }
    }
}

impl std::error::Error for ModelError {}
