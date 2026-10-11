//! Display modes and their geometry: the single place the numbers in `docs/TAD.md` §1.2
//! and `docs/reference/vic-ii-constraints.md` §2–§3 live in code.
//!
//! The validator, solvers, format readers and writers, and the UI all read [`ModeSpec`]
//! instead of hard-coding sizes.

use serde::{Deserialize, Serialize};
use std::fmt;

/// A VIC-II output mode Convert64 can target.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    /// Multicolor bitmap (Koala): 160×200, 4 colours per 4×8 cell.
    MulticolorBitmap,
    /// Hi-res bitmap (Art Studio): 320×200, 2 colours per 8×8 cell.
    HiresBitmap,
    /// Multicolor character set: 160×200, bg/MC1/MC2 global plus a per-char colour 0–7.
    MulticolorCharset,
    /// Hi-res (standard) character set: 320×200, bg global plus a per-char colour.
    HiresCharset,
    /// Extended-colour-mode character set: one of four backgrounds per char, 64 chars.
    EcmCharset,
    /// Hi-res sprite: 24×21, one colour plus transparent.
    HiresSprite,
    /// Multicolor sprite: 12×21, three colours plus transparent; MC1/MC2 shared.
    MulticolorSprite,
    /// Hi-res sprite overlaid on a multicolor sprite at the same coordinates.
    OverlaySprite,
    /// FLI: reserved for M3, not implemented.
    Fli,
    /// IFLI (interlaced FLI): reserved for M3+, not implemented.
    Ifli,
    /// MCI (multicolor interlace): reserved for M3+, not implemented.
    Mci,
    /// NUFLI: reserved for M3+, not implemented.
    Nufli,
}

/// A mode that exists in the enum but has no specification yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReservedMode {
    /// The reserved mode.
    pub mode: Mode,
    /// Milestone that will implement it.
    pub milestone: &'static str,
}

impl fmt::Display for ReservedMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} is reserved for milestone {} and not implemented yet",
            self.mode.name(),
            self.milestone
        )
    }
}

impl std::error::Error for ReservedMode {}

/// How wide a logical pixel is relative to its height on screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PixelAspect {
    /// 1:1, hi-res modes.
    Square,
    /// 2:1, multicolor modes (double-wide pixels).
    DoubleWide,
    /// Overlay sprites: a 1:1 layer over a 2:1 layer.
    Mixed,
}

/// A colour register shared by every cell (or every sprite) in a mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum GlobalColour {
    /// Background colour 0, `$D021`.
    Background,
    /// Character multicolor 1, `$D022`.
    Multicolor1,
    /// Character multicolor 2, `$D023`.
    Multicolor2,
    /// ECM background 1, `$D022`.
    Background1,
    /// ECM background 2, `$D023`.
    Background2,
    /// ECM background 3, `$D024`.
    Background3,
    /// Sprite multicolor 1, `$D025`, shared by all sprites.
    SpriteMulticolor1,
    /// Sprite multicolor 2, `$D026`, shared by all sprites.
    SpriteMulticolor2,
}

impl GlobalColour {
    /// The VIC-II register holding this colour.
    pub const fn register(self) -> u16 {
        match self {
            Self::Background => 0xD021,
            Self::Multicolor1 | Self::Background1 => 0xD022,
            Self::Multicolor2 | Self::Background2 => 0xD023,
            Self::Background3 => 0xD024,
            Self::SpriteMulticolor1 => 0xD025,
            Self::SpriteMulticolor2 => 0xD026,
        }
    }
}

/// A kind of data block a mode's output consists of.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum BlockKind {
    /// Bitmap data, organised by cell.
    Bitmap,
    /// Screen RAM: colour nybbles (bitmap modes) or char codes (charset modes).
    ScreenRam,
    /// Colour RAM, one nybble per cell, fixed at `$D800`.
    ColourRam,
    /// The background colour byte.
    Background,
    /// Character set, 8 bytes per char.
    Charset,
    /// One sprite block, 63 bytes of pixels plus an attribute byte.
    Sprite,
}

/// One block of a mode's data and its size in bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct DataBlock {
    /// What the block holds.
    pub kind: BlockKind,
    /// Size in bytes.
    pub bytes: u16,
}

const fn block(kind: BlockKind, bytes: u16) -> DataBlock {
    DataBlock { kind, bytes }
}

/// The geometry and colour rules of one implemented [`Mode`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct ModeSpec {
    /// The mode this describes.
    pub mode: Mode,
    /// Logical width in pixels (a whole screen, or one sprite).
    pub width: u16,
    /// Logical height in pixels.
    pub height: u16,
    /// Pixel aspect ratio.
    pub aspect: PixelAspect,
    /// Width of a colour cell in logical pixels (for sprites: the whole sprite).
    pub cell_width: u8,
    /// Height of a colour cell in logical pixels (for sprites: the whole sprite).
    pub cell_height: u8,
    /// Most distinct colours a cell may show, counting global colours (validator rule 1).
    /// For sprites: visible colours, transparency not counted.
    pub colours_per_cell: u8,
    /// Global colour registers this mode uses.
    pub globals: &'static [GlobalColour],
    /// Highest allowed per-char colour (charset modes only); 7 in multicolor charset mode.
    pub per_char_colour_max: Option<u8>,
    /// Unique-character budget (charset modes only): 256, or 64 in ECM.
    pub unique_char_budget: Option<u16>,
    /// The blocks a picture of this mode is exported as, with sizes in bytes.
    pub data: &'static [DataBlock],
}

impl ModeSpec {
    /// Cells across one screen or sprite (`width / cell_width`).
    pub const fn cells_x(&self) -> u16 {
        self.width / self.cell_width as u16
    }

    /// Cells down one screen or sprite (`height / cell_height`).
    pub const fn cells_y(&self) -> u16 {
        self.height / self.cell_height as u16
    }

    /// Size in bytes of `kind` in this mode, if the mode has such a block. Repeated
    /// blocks (the two sprites of an overlay) are summed.
    pub fn block_bytes(&self, kind: BlockKind) -> Option<u32> {
        let total: u32 = self
            .data
            .iter()
            .filter(|b| b.kind == kind)
            .map(|b| u32::from(b.bytes))
            .sum();
        (total > 0).then_some(total)
    }

    /// Total exported size in bytes.
    pub fn total_bytes(&self) -> u32 {
        self.data.iter().map(|b| u32::from(b.bytes)).sum()
    }
}

use BlockKind::{Background, Bitmap, Charset, ColourRam, ScreenRam, Sprite};
use GlobalColour as G;

const MULTICOLOR_BITMAP: ModeSpec = ModeSpec {
    mode: Mode::MulticolorBitmap,
    width: 160,
    height: 200,
    aspect: PixelAspect::DoubleWide,
    cell_width: 4,
    cell_height: 8,
    colours_per_cell: 4,
    globals: &[G::Background],
    per_char_colour_max: None,
    unique_char_budget: None,
    data: &[
        block(Bitmap, 8000),
        block(ScreenRam, 1000),
        block(ColourRam, 1000),
        block(Background, 1),
    ],
};

const HIRES_BITMAP: ModeSpec = ModeSpec {
    mode: Mode::HiresBitmap,
    width: 320,
    height: 200,
    aspect: PixelAspect::Square,
    cell_width: 8,
    cell_height: 8,
    colours_per_cell: 2,
    globals: &[],
    per_char_colour_max: None,
    unique_char_budget: None,
    data: &[block(Bitmap, 8000), block(ScreenRam, 1000)],
};

const CHARSET_DATA: &[DataBlock] = &[
    block(Charset, 2048),
    block(ScreenRam, 1000),
    block(ColourRam, 1000),
];

const MULTICOLOR_CHARSET: ModeSpec = ModeSpec {
    mode: Mode::MulticolorCharset,
    width: 160,
    height: 200,
    aspect: PixelAspect::DoubleWide,
    cell_width: 4,
    cell_height: 8,
    colours_per_cell: 4,
    globals: &[G::Background, G::Multicolor1, G::Multicolor2],
    per_char_colour_max: Some(7),
    unique_char_budget: Some(256),
    data: CHARSET_DATA,
};

const HIRES_CHARSET: ModeSpec = ModeSpec {
    mode: Mode::HiresCharset,
    width: 320,
    height: 200,
    aspect: PixelAspect::Square,
    cell_width: 8,
    cell_height: 8,
    colours_per_cell: 2,
    globals: &[G::Background],
    per_char_colour_max: Some(15),
    unique_char_budget: Some(256),
    data: CHARSET_DATA,
};

const ECM_CHARSET: ModeSpec = ModeSpec {
    mode: Mode::EcmCharset,
    width: 320,
    height: 200,
    aspect: PixelAspect::Square,
    cell_width: 8,
    cell_height: 8,
    colours_per_cell: 2,
    globals: &[
        G::Background,
        G::Background1,
        G::Background2,
        G::Background3,
    ],
    per_char_colour_max: Some(15),
    unique_char_budget: Some(64),
    data: CHARSET_DATA,
};

const HIRES_SPRITE: ModeSpec = ModeSpec {
    mode: Mode::HiresSprite,
    width: 24,
    height: 21,
    aspect: PixelAspect::Square,
    cell_width: 24,
    cell_height: 21,
    colours_per_cell: 1,
    globals: &[],
    per_char_colour_max: None,
    unique_char_budget: None,
    data: &[block(Sprite, 64)],
};

const MULTICOLOR_SPRITE: ModeSpec = ModeSpec {
    mode: Mode::MulticolorSprite,
    width: 12,
    height: 21,
    aspect: PixelAspect::DoubleWide,
    cell_width: 12,
    cell_height: 21,
    colours_per_cell: 3,
    globals: &[G::SpriteMulticolor1, G::SpriteMulticolor2],
    per_char_colour_max: None,
    unique_char_budget: None,
    data: &[block(Sprite, 64)],
};

const OVERLAY_SPRITE: ModeSpec = ModeSpec {
    mode: Mode::OverlaySprite,
    width: 24,
    height: 21,
    aspect: PixelAspect::Mixed,
    cell_width: 24,
    cell_height: 21,
    // "up to 7 colours" (reference §3); TAD §1.2 gives 6–7.
    colours_per_cell: 7,
    globals: &[G::SpriteMulticolor1, G::SpriteMulticolor2],
    per_char_colour_max: None,
    unique_char_budget: None,
    data: &[block(Sprite, 64), block(Sprite, 64)],
};

impl Mode {
    /// Every mode, implemented and reserved, in declaration order.
    pub const ALL: [Self; 12] = [
        Self::MulticolorBitmap,
        Self::HiresBitmap,
        Self::MulticolorCharset,
        Self::HiresCharset,
        Self::EcmCharset,
        Self::HiresSprite,
        Self::MulticolorSprite,
        Self::OverlaySprite,
        Self::Fli,
        Self::Ifli,
        Self::Mci,
        Self::Nufli,
    ];

    /// Iterates over every mode, including reserved ones.
    pub fn all() -> impl Iterator<Item = Self> {
        Self::ALL.into_iter()
    }

    /// Iterates over the modes that have a [`ModeSpec`].
    pub fn implemented() -> impl Iterator<Item = Self> {
        Self::all().filter(|m| m.spec().is_ok())
    }

    /// The mode's specification.
    ///
    /// # Errors
    /// [`ReservedMode`] for FLI and the interlace modes, which are not implemented yet.
    pub const fn spec(self) -> Result<&'static ModeSpec, ReservedMode> {
        match self {
            Self::MulticolorBitmap => Ok(&MULTICOLOR_BITMAP),
            Self::HiresBitmap => Ok(&HIRES_BITMAP),
            Self::MulticolorCharset => Ok(&MULTICOLOR_CHARSET),
            Self::HiresCharset => Ok(&HIRES_CHARSET),
            Self::EcmCharset => Ok(&ECM_CHARSET),
            Self::HiresSprite => Ok(&HIRES_SPRITE),
            Self::MulticolorSprite => Ok(&MULTICOLOR_SPRITE),
            Self::OverlaySprite => Ok(&OVERLAY_SPRITE),
            Self::Fli => Err(ReservedMode {
                mode: self,
                milestone: "M3",
            }),
            Self::Ifli | Self::Mci | Self::Nufli => Err(ReservedMode {
                mode: self,
                milestone: "M3+",
            }),
        }
    }

    /// Human-readable name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::MulticolorBitmap => "multicolor bitmap",
            Self::HiresBitmap => "hi-res bitmap",
            Self::MulticolorCharset => "multicolor charset",
            Self::HiresCharset => "hi-res charset",
            Self::EcmCharset => "ECM charset",
            Self::HiresSprite => "hi-res sprite",
            Self::MulticolorSprite => "multicolor sprite",
            Self::OverlaySprite => "overlay sprite",
            Self::Fli => "FLI",
            Self::Ifli => "IFLI",
            Self::Mci => "MCI",
            Self::Nufli => "NUFLI",
        }
    }

    /// `true` for bitmap modes.
    pub const fn is_bitmap(self) -> bool {
        matches!(self, Self::MulticolorBitmap | Self::HiresBitmap)
    }

    /// `true` for character-set modes.
    pub const fn is_charset(self) -> bool {
        matches!(
            self,
            Self::MulticolorCharset | Self::HiresCharset | Self::EcmCharset
        )
    }

    /// `true` for sprite modes.
    pub const fn is_sprite(self) -> bool {
        matches!(
            self,
            Self::HiresSprite | Self::MulticolorSprite | Self::OverlaySprite
        )
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::{BlockKind, GlobalColour, Mode, PixelAspect, ReservedMode};

    /// One row per implemented mode, transcribed from `docs/TAD.md` §1.2:
    /// (mode, width, height, aspect, cell w, cell h, colours per cell, total bytes).
    /// (mode, width, height, aspect, cell width, cell height, colours per cell, total bytes)
    type Row = (Mode, u16, u16, PixelAspect, u8, u8, u8, u32);

    const TAD_TABLE: [Row; 8] = [
        (
            Mode::MulticolorBitmap,
            160,
            200,
            PixelAspect::DoubleWide,
            4,
            8,
            4,
            10_001,
        ),
        (
            Mode::HiresBitmap,
            320,
            200,
            PixelAspect::Square,
            8,
            8,
            2,
            9000,
        ),
        (
            Mode::MulticolorCharset,
            160,
            200,
            PixelAspect::DoubleWide,
            4,
            8,
            4,
            4048,
        ),
        (
            Mode::HiresCharset,
            320,
            200,
            PixelAspect::Square,
            8,
            8,
            2,
            4048,
        ),
        (
            Mode::EcmCharset,
            320,
            200,
            PixelAspect::Square,
            8,
            8,
            2,
            4048,
        ),
        (
            Mode::HiresSprite,
            24,
            21,
            PixelAspect::Square,
            24,
            21,
            1,
            64,
        ),
        (
            Mode::MulticolorSprite,
            12,
            21,
            PixelAspect::DoubleWide,
            12,
            21,
            3,
            64,
        ),
        (
            Mode::OverlaySprite,
            24,
            21,
            PixelAspect::Mixed,
            24,
            21,
            7,
            128,
        ),
    ];

    #[test]
    fn every_tad_row_is_represented() {
        for (mode, w, h, aspect, cw, ch, colours, bytes) in TAD_TABLE {
            let s = mode.spec().expect("implemented");
            assert_eq!(s.mode, mode);
            assert_eq!((s.width, s.height), (w, h), "{mode} resolution");
            assert_eq!(s.aspect, aspect, "{mode} aspect");
            assert_eq!((s.cell_width, s.cell_height), (cw, ch), "{mode} cell");
            assert_eq!(s.colours_per_cell, colours, "{mode} colours");
            assert_eq!(s.total_bytes(), bytes, "{mode} bytes");
        }
        assert_eq!(Mode::implemented().count(), TAD_TABLE.len());
    }

    #[test]
    fn data_block_sizes_match_tad() {
        let mc = Mode::MulticolorBitmap.spec().expect("spec");
        assert_eq!(mc.block_bytes(BlockKind::Bitmap), Some(8000));
        assert_eq!(mc.block_bytes(BlockKind::ScreenRam), Some(1000));
        assert_eq!(mc.block_bytes(BlockKind::ColourRam), Some(1000));
        assert_eq!(mc.block_bytes(BlockKind::Background), Some(1));
        let hr = Mode::HiresBitmap.spec().expect("spec");
        assert_eq!(hr.block_bytes(BlockKind::ColourRam), None);
        for m in [
            Mode::MulticolorCharset,
            Mode::HiresCharset,
            Mode::EcmCharset,
        ] {
            assert_eq!(
                m.spec().expect("spec").block_bytes(BlockKind::Charset),
                Some(2048)
            );
        }
        let ov = Mode::OverlaySprite.spec().expect("spec");
        assert_eq!(ov.block_bytes(BlockKind::Sprite), Some(128));
    }

    #[test]
    fn charset_rules() {
        let get = |m: Mode| m.spec().expect("spec");
        assert_eq!(get(Mode::MulticolorCharset).per_char_colour_max, Some(7));
        assert_eq!(get(Mode::HiresCharset).per_char_colour_max, Some(15));
        assert_eq!(get(Mode::EcmCharset).unique_char_budget, Some(64));
        assert_eq!(get(Mode::MulticolorCharset).unique_char_budget, Some(256));
        assert_eq!(get(Mode::HiresCharset).unique_char_budget, Some(256));
        assert_eq!(get(Mode::HiresBitmap).unique_char_budget, None);
        assert_eq!(get(Mode::EcmCharset).globals.len(), 4);
        assert_eq!(get(Mode::MulticolorBitmap).cells_x(), 40);
        assert_eq!(get(Mode::HiresCharset).cells_y(), 25);
    }

    #[test]
    fn global_registers() {
        assert_eq!(GlobalColour::Background.register(), 0xD021);
        assert_eq!(GlobalColour::Multicolor2.register(), 0xD023);
        assert_eq!(GlobalColour::Background3.register(), 0xD024);
        assert_eq!(GlobalColour::SpriteMulticolor2.register(), 0xD026);
    }

    #[test]
    fn reserved_modes_explain_themselves() {
        let err = Mode::Fli.spec().expect_err("reserved");
        assert_eq!(
            err,
            ReservedMode {
                mode: Mode::Fli,
                milestone: "M3"
            }
        );
        assert_eq!(
            err.to_string(),
            "FLI is reserved for milestone M3 and not implemented yet"
        );
        for m in [Mode::Ifli, Mode::Mci, Mode::Nufli] {
            assert!(m.spec().is_err(), "{m}");
        }
        assert_eq!(Mode::all().count(), 12);
    }

    #[test]
    fn spec_is_copy_and_serialisable() {
        fn assert_traits<T: Copy + serde::Serialize>() {}
        assert_traits::<super::ModeSpec>();
        assert_traits::<Mode>();
    }

    #[test]
    fn deserialises_from_kebab_case_name() {
        use serde::de::IntoDeserializer;
        use serde::de::value::{Error, StrDeserializer};
        let d: StrDeserializer<'_, Error> = "multicolor-bitmap".into_deserializer();
        let m: Mode = serde::Deserialize::deserialize(d).expect("deserialises");
        assert_eq!(m, Mode::MulticolorBitmap);
    }

    #[test]
    fn mode_kind_predicates() {
        assert!(Mode::HiresBitmap.is_bitmap() && !Mode::HiresBitmap.is_charset());
        assert!(Mode::EcmCharset.is_charset());
        assert!(Mode::OverlaySprite.is_sprite());
    }
}
