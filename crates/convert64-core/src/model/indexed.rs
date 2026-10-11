//! [`IndexedBitmap`]: a bitmap-mode picture as one colour per logical pixel.

use super::{CellPos, ModelError};
use crate::colour::{ColourIndex, ColourSet};
use crate::mode::{Mode, ModeSpec};
use serde::{Deserialize, Serialize};
use std::fmt;

/// A multicolor (160×200) or hi-res (320×200) bitmap picture, one [`ColourIndex`] per
/// logical pixel, row-major from the top-left. Multicolor pictures also carry the global
/// background colour (`$D021`), which every cell may use in addition to its own three.
///
/// This is what the bitmap solver produces and what the preview shows. It may break
/// rule 1; [`crate::model::BitmapImage`] cannot.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "IndexedBitmapData")]
pub struct IndexedBitmap {
    mode: Mode,
    background: Option<ColourIndex>,
    pixels: Vec<ColourIndex>,
}

#[derive(Deserialize)]
struct IndexedBitmapData {
    mode: Mode,
    background: Option<ColourIndex>,
    pixels: Vec<ColourIndex>,
}

impl TryFrom<IndexedBitmapData> for IndexedBitmap {
    type Error = ModelError;

    fn try_from(d: IndexedBitmapData) -> Result<Self, Self::Error> {
        Self::new(d.mode, d.pixels, d.background)
    }
}

fn bitmap_spec(mode: Mode) -> Result<&'static ModeSpec, ModelError> {
    if mode.is_bitmap() {
        mode.spec().map_err(|_| ModelError::WrongMode {
            what: "indexed bitmap",
            mode,
        })
    } else {
        Err(ModelError::WrongMode {
            what: "indexed bitmap",
            mode,
        })
    }
}

impl IndexedBitmap {
    /// A picture from its pixels.
    ///
    /// # Errors
    /// [`ModelError`] if `mode` is not a bitmap mode, `pixels` is not exactly
    /// width × height long, or `background` is missing (multicolor) or present (hi-res).
    pub fn new(
        mode: Mode,
        pixels: Vec<ColourIndex>,
        background: Option<ColourIndex>,
    ) -> Result<Self, ModelError> {
        let spec = bitmap_spec(mode)?;
        ModelError::check_len(
            "pixels",
            usize::from(spec.width) * usize::from(spec.height),
            pixels.len(),
        )?;
        let wants_background = mode == Mode::MulticolorBitmap;
        if wants_background != background.is_some() {
            return Err(ModelError::Background { mode });
        }
        Ok(Self {
            mode,
            background,
            pixels,
        })
    }

    /// A picture filled with one colour. Multicolor pictures use `colour` as background.
    ///
    /// # Errors
    /// [`ModelError::WrongMode`] if `mode` is not a bitmap mode.
    pub fn filled(mode: Mode, colour: ColourIndex) -> Result<Self, ModelError> {
        let spec = bitmap_spec(mode)?;
        let n = usize::from(spec.width) * usize::from(spec.height);
        let background = (mode == Mode::MulticolorBitmap).then_some(colour);
        Self::new(mode, vec![colour; n], background)
    }

    /// The mode.
    pub const fn mode(&self) -> Mode {
        self.mode
    }

    /// The mode's specification.
    pub fn spec(&self) -> &'static ModeSpec {
        // Construction guarantees an implemented bitmap mode.
        bitmap_spec(self.mode).unwrap_or_else(|_| unreachable!("checked in new()"))
    }

    /// Width in logical pixels (160 or 320).
    pub fn width(&self) -> u16 {
        self.spec().width
    }

    /// Height in logical pixels (200).
    pub fn height(&self) -> u16 {
        self.spec().height
    }

    /// The global background colour (multicolor only).
    pub const fn background(&self) -> Option<ColourIndex> {
        self.background
    }

    /// All pixels, row-major.
    pub fn pixels(&self) -> &[ColourIndex] {
        &self.pixels
    }

    /// The pixel at logical `x`, `y`.
    ///
    /// # Panics
    /// If `x` or `y` is outside the picture.
    pub fn pixel(&self, x: u16, y: u16) -> ColourIndex {
        assert!(x < self.width() && y < self.height(), "pixel out of range");
        self.pixels[usize::from(y) * usize::from(self.width()) + usize::from(x)]
    }

    /// Sets the pixel at logical `x`, `y`.
    ///
    /// # Panics
    /// If `x` or `y` is outside the picture.
    pub fn set_pixel(&mut self, x: u16, y: u16, c: ColourIndex) {
        assert!(x < self.width() && y < self.height(), "pixel out of range");
        let w = usize::from(self.width());
        self.pixels[usize::from(y) * w + usize::from(x)] = c;
    }

    /// Every cell position, row by row.
    pub fn cells(&self) -> impl Iterator<Item = CellPos> + use<> {
        let spec = self.spec();
        let (cx, cy) = (spec.cells_x(), spec.cells_y());
        (0..cy).flat_map(move |y| (0..cx).map(move |x| CellPos::new(x, y)))
    }

    /// The distinct colours the pixels of `cell` use. The background is included only if
    /// a pixel in the cell actually shows it.
    ///
    /// # Panics
    /// If `cell` is outside the 40×25 grid.
    pub fn cell_colours(&self, cell: CellPos) -> ColourSet {
        let spec = self.spec();
        assert!(
            cell.x < spec.cells_x() && cell.y < spec.cells_y(),
            "cell out of range"
        );
        let (cw, ch) = (u16::from(spec.cell_width), u16::from(spec.cell_height));
        let mut set = ColourSet::EMPTY;
        for y in cell.y * ch..(cell.y + 1) * ch {
            for x in cell.x * cw..(cell.x + 1) * cw {
                set.insert(self.pixel(x, y));
            }
        }
        set
    }
}

impl fmt::Debug for IndexedBitmap {
    /// Compact form: mode, background and the colours used, not 32 000 pixels.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let used: ColourSet = self.pixels.iter().copied().collect();
        f.debug_struct("IndexedBitmap")
            .field("mode", &self.mode)
            .field("background", &self.background.map(ColourIndex::get))
            .field("colours_used", &format_args!("[{used}]"))
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::IndexedBitmap;
    use crate::colour::ColourIndex;
    use crate::mode::Mode;
    use crate::model::{CellPos, ModelError};

    #[test]
    fn enforces_size_mode_and_background() {
        let px = |n| vec![ColourIndex::BLACK; n];
        assert!(
            IndexedBitmap::new(Mode::MulticolorBitmap, px(32_000), Some(ColourIndex::BLUE)).is_ok()
        );
        assert!(IndexedBitmap::new(Mode::HiresBitmap, px(64_000), None).is_ok());
        assert_eq!(
            IndexedBitmap::new(Mode::HiresBitmap, px(32_000), None),
            Err(ModelError::WrongLength {
                what: "pixels",
                expected: 64_000,
                found: 32_000
            })
        );
        assert_eq!(
            IndexedBitmap::new(Mode::MulticolorBitmap, px(32_000), None),
            Err(ModelError::Background {
                mode: Mode::MulticolorBitmap
            })
        );
        assert_eq!(
            IndexedBitmap::new(Mode::HiresBitmap, px(64_000), Some(ColourIndex::BLUE)),
            Err(ModelError::Background {
                mode: Mode::HiresBitmap
            })
        );
        assert!(matches!(
            IndexedBitmap::filled(Mode::HiresCharset, ColourIndex::BLACK),
            Err(ModelError::WrongMode { .. })
        ));
    }

    #[test]
    fn cell_colours_cover_the_right_pixels() {
        let mut img = IndexedBitmap::filled(Mode::MulticolorBitmap, ColourIndex::BLUE).expect("ok");
        // Cell (1, 2) spans x 4..8, y 16..24 at 160×200.
        img.set_pixel(4, 16, ColourIndex::RED);
        img.set_pixel(7, 23, ColourIndex::WHITE);
        img.set_pixel(8, 16, ColourIndex::GREEN); // cell (2, 2)
        let c = img.cell_colours(CellPos::new(1, 2));
        assert_eq!(
            c.iter().collect::<Vec<_>>(),
            [ColourIndex::WHITE, ColourIndex::RED, ColourIndex::BLUE]
        );
        assert_eq!(img.cells().count(), 1000);
        assert_eq!(img.cells().last(), Some(CellPos::new(39, 24)));
    }

    #[test]
    fn debug_output_is_compact() {
        let img = IndexedBitmap::filled(Mode::HiresBitmap, ColourIndex::RED).expect("ok");
        let s = format!("{img:?}");
        assert!(s.len() < 120, "{s}");
        assert!(s.contains("colours_used: [2]"), "{s}");
    }
}
