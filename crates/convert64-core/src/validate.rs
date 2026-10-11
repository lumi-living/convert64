//! The validator: the executable form of `docs/reference/vic-ii-constraints.md` §6,
//! rules 1–5, numbered as there.
//!
//! What the model types already guarantee is not re-checked here: every stored colour is
//! 0–15 (rule 2, [`ColourIndex`](crate::ColourIndex)), every sprite block is 64 bytes and a set has one
//! MC1/MC2 pair (rule 4, [`SpriteSet`]), and an encoded [`BitmapImage`] cannot show more
//! colours per cell than its mode allows (rule 1). This module checks the rest:
//!
//! | Rule | Checked on | What |
//! |---|---|---|
//! | 1 | [`IndexedBitmap`] | distinct colours per cell, counting the multicolor background |
//! | 2 | [`CharsetImage`] | multicolor-charset per-char colour is 0–7 |
//! | 3 | [`CharsetImage`] | char count within budget; every map cell refers to an existing char |
//! | 4 | [`SpriteSet`] | overlay pairs: indices exist, hi-res over multicolor, same coordinates |
//! | 5 | [`Placement`] | alignment of every block; no VIC-read block in the char-ROM shadow |

use crate::colour::ColourSet;
use crate::mode::{BlockKind, Mode};
use crate::model::{BitmapImage, CellPos, CharsetGlobals, CharsetImage, IndexedBitmap, SpriteSet};
use crate::rule::Rule;
use serde::Serialize;
use std::fmt;

/// Where a violation is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum Location {
    /// The image as a whole.
    Image,
    /// A cell (character position).
    Cell {
        /// The cell.
        cell: CellPos,
    },
    /// A char in the character set.
    Char {
        /// Char index.
        index: usize,
    },
    /// A sprite in the set.
    Sprite {
        /// Sprite index.
        index: usize,
    },
    /// A data block placed at an address.
    Address {
        /// What the block is.
        block: BlockKind,
        /// Its start address.
        address: u16,
    },
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Image => f.write_str("image"),
            Self::Cell { cell } => write!(f, "cell {cell}"),
            Self::Char { index } => write!(f, "char {index}"),
            Self::Sprite { index } => write!(f, "sprite {index}"),
            Self::Address { block, address } => {
                write!(f, "{} at ${address:04X}", block_name(*block))
            }
        }
    }
}

/// One broken rule, where, and why.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Violation {
    /// The rule broken.
    pub rule: Rule,
    /// Where.
    pub location: Location,
    /// Human-readable explanation.
    pub message: String,
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}: {}", self.rule, self.location, self.message)
    }
}

mod sealed {
    pub trait Sealed {}
    impl Sealed for crate::model::IndexedBitmap {}
    impl Sealed for crate::model::BitmapImage {}
    impl Sealed for crate::model::CharsetImage {}
    impl Sealed for crate::model::SpriteSet {}
}

/// Anything the validator can check: the four image-model types.
pub trait C64Image: sealed::Sealed {
    /// The image's mode.
    fn mode(&self) -> Mode;
    /// Appends every violation of rules 1–4 to `out`.
    fn collect_violations(&self, out: &mut Vec<Violation>);
}

/// Checks `image` against rules 1–4.
///
/// # Errors
/// Every violation found, in a stable order (rule, then position).
pub fn validate(image: &dyn C64Image) -> Result<(), Vec<Violation>> {
    let mut out = Vec::new();
    image.collect_violations(&mut out);
    finish(out)
}

/// Checks where an export is placed in memory against rule 5.
///
/// # Errors
/// Every violation found.
pub fn validate_placement(placement: &Placement) -> Result<(), Vec<Violation>> {
    let mut out = Vec::new();
    placement.collect_violations(&mut out);
    finish(out)
}

/// Checks `image` (rules 1–4) and its placement (rule 5) together.
///
/// # Errors
/// Every violation found, image rules first.
pub fn validate_export(image: &dyn C64Image, placement: &Placement) -> Result<(), Vec<Violation>> {
    let mut out = Vec::new();
    image.collect_violations(&mut out);
    placement.collect_violations(&mut out);
    finish(out)
}

fn finish(mut out: Vec<Violation>) -> Result<(), Vec<Violation>> {
    out.sort_by_key(|v| v.rule);
    if out.is_empty() { Ok(()) } else { Err(out) }
}

fn list(set: ColourSet) -> String {
    set.iter()
        .map(|c| format!("{} {}", c.get(), c.name()))
        .collect::<Vec<_>>()
        .join(", ")
}

// ---------------------------------------------------------------------------------------
// Rule 1: colours per cell, counting global colours.

impl C64Image for IndexedBitmap {
    fn mode(&self) -> Mode {
        IndexedBitmap::mode(self)
    }

    fn collect_violations(&self, out: &mut Vec<Violation>) {
        let spec = self.spec();
        let (width, cw, ch) = (
            usize::from(spec.width),
            usize::from(spec.cell_width),
            usize::from(spec.cell_height),
        );
        let cells_x = usize::from(spec.cells_x());
        // One pass over the pixels, OR-ing each into its cell's colour mask.
        let mut masks = vec![ColourSet::EMPTY; cells_x * usize::from(spec.cells_y())];
        for (row_index, row) in self.pixels().chunks_exact(width).enumerate() {
            let cell_row = (row_index / ch) * cells_x;
            for (x, &c) in row.iter().enumerate() {
                masks[cell_row + x / cw].insert(c);
            }
        }
        let globals = self
            .background()
            .map_or(ColourSet::EMPTY, |bg| ColourSet::EMPTY.with(bg));
        let limit = u32::from(spec.colours_per_cell);
        for (i, used) in masks.into_iter().enumerate() {
            let counted = used.union(globals);
            if counted.len() > limit {
                let cell = cell_pos(i, cells_x);
                let message = match self.background() {
                    Some(bg) => format!(
                        "uses {} colours counting the background {} ({}): {}; a {} cell allows {limit}",
                        counted.len(),
                        bg.get(),
                        bg.name(),
                        list(counted),
                        spec.mode
                    ),
                    None => format!(
                        "uses {} colours: {}; a {} cell allows {limit}",
                        counted.len(),
                        list(counted),
                        spec.mode
                    ),
                };
                out.push(Violation {
                    rule: Rule::ColoursPerCell,
                    location: Location::Cell { cell },
                    message,
                });
            }
        }
    }
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "cell grids are at most 65535 cells wide and tall"
)]
fn cell_pos(index: usize, cells_x: usize) -> CellPos {
    CellPos::new((index % cells_x) as u16, (index / cells_x) as u16)
}

impl C64Image for BitmapImage {
    fn mode(&self) -> Mode {
        BitmapImage::mode(self)
    }

    /// Rules 1–4 cannot be broken by an encoded bitmap: each cell's colours come from
    /// at most 2 (hi-res) or 4 (multicolor) nybbles, and every colour is a `ColourIndex`.
    fn collect_violations(&self, _out: &mut Vec<Violation>) {}
}

// ---------------------------------------------------------------------------------------
// Rules 2 and 3 on charsets.

impl C64Image for CharsetImage {
    fn mode(&self) -> Mode {
        CharsetImage::mode(self)
    }

    fn collect_violations(&self, out: &mut Vec<Violation>) {
        let spec = self
            .mode()
            .spec()
            .unwrap_or_else(|_| unreachable!("charset modes are implemented"));

        // Rule 2: multicolor-charset per-char colour is 0-7.
        if let (CharsetGlobals::Multicolor { .. }, Some(max)) =
            (self.globals(), spec.per_char_colour_max)
        {
            for cell in self.cells() {
                let c = self.colour(cell).colour;
                if c.get() > max {
                    out.push(Violation {
                        rule: Rule::ColourRange,
                        location: Location::Cell { cell },
                        message: format!(
                            "char colour {} ({}) is above {max}; multicolor charset per-char colour is 0-{max}",
                            c.get(),
                            c.name()
                        ),
                    });
                }
            }
        }

        // Rule 3: budget, then references.
        let chars = self.charset().len();
        if let Some(budget) = spec.unique_char_budget {
            let budget = usize::from(budget);
            if chars > budget {
                out.push(Violation {
                    rule: Rule::CharsetBudget,
                    location: Location::Char { index: budget },
                    message: format!(
                        "charset has {chars} chars; the {} budget is {budget}",
                        spec.mode
                    ),
                });
            }
        }
        for cell in self.cells() {
            let index = self.char_index(cell);
            if index >= chars {
                out.push(Violation {
                    rule: Rule::CharsetBudget,
                    location: Location::Cell { cell },
                    message: format!(
                        "refers to char {index}, but the charset has {chars} chars (0-{})",
                        chars.saturating_sub(1)
                    ),
                });
            }
        }
    }
}

// ---------------------------------------------------------------------------------------
// Rule 4 on sprites.

impl C64Image for SpriteSet {
    fn mode(&self) -> Mode {
        let any_multicolor = self.sprites().iter().any(|s| s.is_multicolor());
        if !self.overlays().is_empty() {
            Mode::OverlaySprite
        } else if any_multicolor {
            Mode::MulticolorSprite
        } else {
            Mode::HiresSprite
        }
    }

    /// Block size and the single MC1/MC2 pair are guaranteed by [`SpriteSet`]; this
    /// checks overlay pairs.
    fn collect_violations(&self, out: &mut Vec<Violation>) {
        let n = self.sprites().len();
        for pair in self.overlays() {
            let mut fail = |index: usize, message: String| {
                out.push(Violation {
                    rule: Rule::Sprites,
                    location: Location::Sprite { index },
                    message,
                });
            };
            if pair.hires >= n || pair.multicolor >= n {
                let index = pair.hires.max(pair.multicolor);
                fail(
                    index,
                    format!("overlay pair refers to sprite {index}, but the set has {n} sprites"),
                );
                continue;
            }
            if self.sprites()[pair.hires].is_multicolor() {
                fail(
                    pair.hires,
                    format!("overlay top sprite {} must be hi-res", pair.hires),
                );
            }
            if !self.sprites()[pair.multicolor].is_multicolor() {
                fail(
                    pair.multicolor,
                    format!("overlay base sprite {} must be multicolor", pair.multicolor),
                );
            }
            let (a, b) = (
                self.positions()[pair.hires],
                self.positions()[pair.multicolor],
            );
            if a != b {
                fail(
                    pair.hires,
                    format!(
                        "overlay pair must share coordinates: hi-res sprite {} is at ({}, {}), multicolor sprite {} at ({}, {})",
                        pair.hires, a.x, a.y, pair.multicolor, b.x, b.y
                    ),
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------------------
// Rule 5: placement.

/// One exported block placed at an address.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct PlacedBlock {
    /// What the block is.
    pub kind: BlockKind,
    /// Start address.
    pub address: u16,
    /// Length in bytes (used for the char-ROM shadow check).
    pub bytes: u16,
}

/// Where an export's blocks go in C64 memory (rule 5).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Placement {
    blocks: Vec<PlacedBlock>,
}

/// The standard size of a block (reference §5 and `docs/TAD.md` §1.2).
pub const fn standard_bytes(kind: BlockKind) -> u16 {
    match kind {
        BlockKind::Bitmap => 8000,
        BlockKind::ScreenRam | BlockKind::ColourRam => 1000,
        BlockKind::Charset => 2048,
        BlockKind::Sprite => 64,
        BlockKind::Background => 1,
    }
}

fn block_name(kind: BlockKind) -> &'static str {
    match kind {
        BlockKind::Bitmap => "bitmap",
        BlockKind::ScreenRam => "screen RAM",
        BlockKind::ColourRam => "colour RAM",
        BlockKind::Background => "background",
        BlockKind::Charset => "charset",
        BlockKind::Sprite => "sprite",
    }
}

/// Alignment required for `kind` (reference §5): `None` for blocks that are not placed
/// in a VIC bank.
const fn alignment(kind: BlockKind) -> Option<u32> {
    match kind {
        BlockKind::Charset => Some(0x0800),
        BlockKind::ScreenRam => Some(0x0400),
        BlockKind::Bitmap => Some(0x2000),
        BlockKind::Sprite => Some(64),
        BlockKind::ColourRam | BlockKind::Background => None,
    }
}

/// Address ranges where the VIC-II sees character ROM instead of RAM (banks 0 and 2).
pub const CHAR_ROM_SHADOWS: [(u16, u16); 2] = [(0x1000, 0x1FFF), (0x9000, 0x9FFF)];

/// Colour RAM's fixed address.
pub const COLOUR_RAM: u16 = 0xD800;

impl Placement {
    /// An empty placement.
    pub const fn new() -> Self {
        Self { blocks: Vec::new() }
    }

    /// Adds a block of its standard size at `address`.
    #[must_use]
    pub fn with(self, kind: BlockKind, address: u16) -> Self {
        self.with_sized(kind, address, standard_bytes(kind))
    }

    /// Adds a block of `bytes` bytes at `address` (for example a short charset).
    #[must_use]
    pub fn with_sized(mut self, kind: BlockKind, address: u16, bytes: u16) -> Self {
        self.blocks.push(PlacedBlock {
            kind,
            address,
            bytes,
        });
        self
    }

    /// The placed blocks.
    pub fn blocks(&self) -> &[PlacedBlock] {
        &self.blocks
    }

    fn collect_violations(&self, out: &mut Vec<Violation>) {
        for b in &self.blocks {
            let location = Location::Address {
                block: b.kind,
                address: b.address,
            };
            let mut fail = |message: String| {
                out.push(Violation {
                    rule: Rule::Placement,
                    location,
                    message,
                });
            };
            match b.kind {
                BlockKind::Background => {
                    fail("the background colour is a register, not a memory block".to_owned());
                    continue;
                }
                BlockKind::ColourRam => {
                    if b.address != COLOUR_RAM {
                        fail(format!(
                            "colour RAM is fixed at ${COLOUR_RAM:04X} and cannot be relocated"
                        ));
                    }
                    continue;
                }
                _ => {}
            }
            if let Some(align) = alignment(b.kind)
                && u32::from(b.address) % align != 0
            {
                fail(format!(
                    "{} must start on a ${align:04X}-byte boundary",
                    block_name(b.kind)
                ));
            }
            let start = u32::from(b.address);
            let end = start + u32::from(b.bytes.max(1)) - 1;
            for (lo, hi) in CHAR_ROM_SHADOWS {
                if start <= u32::from(hi) && end >= u32::from(lo) {
                    fail(format!(
                        "${start:04X}-${end:04X} overlaps the char-ROM shadow ${lo:04X}-${hi:04X}, where the VIC-II reads ROM"
                    ));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Location, Placement, Violation, validate, validate_export, validate_placement};
    use crate::colour::ColourIndex;
    use crate::mode::{BlockKind, Mode};
    use crate::model::{
        BitmapImage, CellPos, CharColour, CharsetGlobals, CharsetImage, IndexedBitmap, OverlayPair,
        Sprite, SpritePos, SpriteSet, TileSize,
    };
    use crate::rule::Rule;

    fn rules(r: Result<(), Vec<Violation>>) -> Vec<(Rule, Location)> {
        r.err()
            .unwrap_or_default()
            .into_iter()
            .map(|v| (v.rule, v.location))
            .collect()
    }

    // Rule 1 ---------------------------------------------------------------------------

    #[test]
    fn rule1_passes_a_full_multicolor_cell() {
        let mut img =
            IndexedBitmap::filled(Mode::MulticolorBitmap, ColourIndex::BLACK).expect("ok");
        img.set_pixel(0, 0, ColourIndex::RED);
        img.set_pixel(1, 0, ColourIndex::GREEN);
        img.set_pixel(2, 0, ColourIndex::BLUE);
        assert_eq!(validate(&img), Ok(()));
    }

    #[test]
    fn rule1_reports_the_exact_cell_of_an_injected_clash() {
        let mut img =
            IndexedBitmap::filled(Mode::MulticolorBitmap, ColourIndex::BLACK).expect("ok");
        // Cell (12, 5) spans x 48..52, y 40..48. Four non-background colours = 5 total.
        for (x, c) in [
            (48, ColourIndex::RED),
            (49, ColourIndex::GREEN),
            (50, ColourIndex::BLUE),
            (51, ColourIndex::WHITE),
        ] {
            img.set_pixel(x, 45, c);
        }
        let err = validate(&img).expect_err("5 colours");
        assert_eq!(err.len(), 1);
        assert_eq!(err[0].rule, Rule::ColoursPerCell);
        assert_eq!(
            err[0].location,
            Location::Cell {
                cell: CellPos::new(12, 5)
            }
        );
        assert!(
            err[0]
                .message
                .contains("uses 5 colours counting the background 0"),
            "{}",
            err[0]
        );
    }

    #[test]
    fn rule1_hires_allows_two() {
        let mut img = IndexedBitmap::filled(Mode::HiresBitmap, ColourIndex::BLACK).expect("ok");
        img.set_pixel(319, 199, ColourIndex::WHITE);
        assert_eq!(validate(&img), Ok(()));
        img.set_pixel(318, 199, ColourIndex::RED);
        assert_eq!(
            rules(validate(&img)),
            [(
                Rule::ColoursPerCell,
                Location::Cell {
                    cell: CellPos::new(39, 24)
                }
            )]
        );
    }

    #[test]
    fn encoded_bitmaps_always_pass_rules_1_to_4() {
        let img = BitmapImage::multicolor_from_bytes(&[0xE4; 8000], &[0x12; 1000], &[3; 1000], 4)
            .expect("ok");
        assert_eq!(validate(&img), Ok(()));
    }

    // Rules 2 and 3 --------------------------------------------------------------------

    fn mc_globals() -> CharsetGlobals {
        CharsetGlobals::Multicolor {
            background: ColourIndex::BLACK,
            multicolor1: ColourIndex::RED,
            multicolor2: ColourIndex::GREEN,
        }
    }

    #[test]
    fn rule2_multicolor_char_colour_must_be_0_to_7() {
        let mut colours = vec![CharColour::multicolor(ColourIndex::YELLOW); 4];
        let ok = CharsetImage::new(
            vec![[0; 8]],
            vec![0; 4],
            2,
            2,
            colours.clone(),
            mc_globals(),
            TileSize::SINGLE,
        )
        .expect("ok");
        assert_eq!(validate(&ok), Ok(()));
        colours[3] = CharColour::multicolor(ColourIndex::BROWN);
        let bad = CharsetImage::new(
            vec![[0; 8]],
            vec![0; 4],
            2,
            2,
            colours,
            mc_globals(),
            TileSize::SINGLE,
        )
        .expect("ok");
        assert_eq!(
            rules(validate(&bad)),
            [(
                Rule::ColourRange,
                Location::Cell {
                    cell: CellPos::new(1, 1)
                }
            )]
        );
    }

    #[test]
    fn rule3_budget_and_missing_chars() {
        let hires = CharsetGlobals::Hires {
            background: ColourIndex::BLACK,
        };
        let col = vec![CharColour::hires(ColourIndex::WHITE); 2];
        let ok = CharsetImage::new(
            vec![[0; 8]; 256],
            vec![0, 255],
            2,
            1,
            col.clone(),
            hires,
            TileSize::SINGLE,
        )
        .expect("ok");
        assert_eq!(validate(&ok), Ok(()));
        let over = CharsetImage::new(
            vec![[0; 8]; 257],
            vec![0, 1],
            2,
            1,
            col.clone(),
            hires,
            TileSize::SINGLE,
        )
        .expect("ok");
        assert_eq!(
            rules(validate(&over)),
            [(Rule::CharsetBudget, Location::Char { index: 256 })]
        );
        let missing = CharsetImage::new(
            vec![[0; 8]; 10],
            vec![0, 10],
            2,
            1,
            col,
            hires,
            TileSize::SINGLE,
        )
        .expect("ok");
        assert_eq!(
            rules(validate(&missing)),
            [(
                Rule::CharsetBudget,
                Location::Cell {
                    cell: CellPos::new(1, 0)
                }
            )]
        );
    }

    #[test]
    fn rule3_ecm_budget_is_64_and_codes_use_six_bits() {
        let ecm = CharsetGlobals::Ecm {
            backgrounds: [ColourIndex::BLACK; 4],
        };
        let col = vec![CharColour::hires(ColourIndex::WHITE)];
        // Code 0xFF selects background 3 and char 63.
        let ok = CharsetImage::new(
            vec![[0; 8]; 64],
            vec![0xFF],
            1,
            1,
            col.clone(),
            ecm,
            TileSize::SINGLE,
        )
        .expect("ok");
        assert_eq!(validate(&ok), Ok(()));
        let over = CharsetImage::new(vec![[0; 8]; 65], vec![0], 1, 1, col, ecm, TileSize::SINGLE)
            .expect("ok");
        assert_eq!(
            rules(validate(&over)),
            [(Rule::CharsetBudget, Location::Char { index: 64 })]
        );
    }

    // Rule 4 ---------------------------------------------------------------------------

    fn overlay_set(hires_pos: SpritePos) -> SpriteSet {
        let hires = Sprite::new([0; 63], ColourIndex::WHITE, false);
        let multi = Sprite::new([0; 63], ColourIndex::RED, true);
        SpriteSet::new(vec![hires, multi], ColourIndex::BLUE, ColourIndex::GREEN)
            .with_positions(vec![hires_pos, SpritePos { x: 100, y: 80 }])
            .expect("ok")
            .with_overlays(vec![OverlayPair {
                hires: 0,
                multicolor: 1,
            }])
    }

    #[test]
    fn rule4_overlay_pairs_share_coordinates() {
        let ok = overlay_set(SpritePos { x: 100, y: 80 });
        assert_eq!(validate(&ok), Ok(()));
        assert_eq!(super::C64Image::mode(&ok), Mode::OverlaySprite);
        let bad = overlay_set(SpritePos { x: 101, y: 80 });
        assert_eq!(
            rules(validate(&bad)),
            [(Rule::Sprites, Location::Sprite { index: 0 })]
        );
    }

    #[test]
    fn rule4_overlay_kinds_and_indices() {
        let swapped = overlay_set(SpritePos { x: 100, y: 80 }).with_overlays(vec![OverlayPair {
            hires: 1,
            multicolor: 0,
        }]);
        assert_eq!(
            rules(validate(&swapped)),
            [
                (Rule::Sprites, Location::Sprite { index: 1 }),
                (Rule::Sprites, Location::Sprite { index: 0 })
            ]
        );
        let dangling = overlay_set(SpritePos { x: 100, y: 80 }).with_overlays(vec![OverlayPair {
            hires: 0,
            multicolor: 5,
        }]);
        assert_eq!(
            rules(validate(&dangling)),
            [(Rule::Sprites, Location::Sprite { index: 5 })]
        );
    }

    // Rule 5 ---------------------------------------------------------------------------

    #[test]
    fn rule5_accepts_the_recommended_bank_1_layout() {
        let p = Placement::new()
            .with(BlockKind::Bitmap, 0x6000)
            .with(BlockKind::ScreenRam, 0x5C00)
            .with(BlockKind::Charset, 0x4800)
            .with(BlockKind::Sprite, 0x4000)
            .with(BlockKind::ColourRam, 0xD800);
        assert_eq!(validate_placement(&p), Ok(()));
    }

    #[test]
    fn rule5_rejects_misalignment_and_the_char_rom_shadow() {
        let at = |kind, address| Location::Address {
            block: kind,
            address,
        };
        let p = Placement::new()
            .with(BlockKind::Sprite, 0x4010)
            .with(BlockKind::Charset, 0x1800)
            .with(BlockKind::Bitmap, 0x8000)
            .with(BlockKind::ScreenRam, 0x0C00)
            .with(BlockKind::ColourRam, 0xC000);
        assert_eq!(
            rules(validate_placement(&p)),
            [
                (Rule::Placement, at(BlockKind::Sprite, 0x4010)),
                (Rule::Placement, at(BlockKind::Charset, 0x1800)),
                (Rule::Placement, at(BlockKind::Bitmap, 0x8000)),
                (Rule::Placement, at(BlockKind::ColourRam, 0xC000)),
            ]
        );
        // The screen at $0C00-$0FE7 ends just below the shadow and is fine.
    }

    #[test]
    fn validate_export_combines_image_and_placement() {
        let img = IndexedBitmap::filled(Mode::HiresBitmap, ColourIndex::BLACK).expect("ok");
        let p = Placement::new().with(BlockKind::Bitmap, 0x2100);
        assert_eq!(
            rules(validate_export(&img, &p)),
            [(
                Rule::Placement,
                Location::Address {
                    block: BlockKind::Bitmap,
                    address: 0x2100
                }
            )]
        );
    }

    #[test]
    fn violations_display_rule_location_and_message() {
        let p = Placement::new().with(BlockKind::Charset, 0x1000);
        let err = validate_placement(&p).expect_err("shadow");
        assert_eq!(
            err[0].to_string(),
            "rule 5: charset at $1000: $1000-$17FF overlaps the char-ROM shadow $1000-$1FFF, where the VIC-II reads ROM"
        );
    }

    /// Rule-1 check of a full 320×200 picture must stay well under 1 ms. Timing is only
    /// meaningful in an optimised build: `cargo test --release -p convert64-core`.
    #[test]
    #[cfg_attr(debug_assertions, ignore = "timing needs a release build")]
    fn full_hires_bitmap_validates_under_1ms() {
        let mut img = IndexedBitmap::filled(Mode::HiresBitmap, ColourIndex::BLACK).expect("ok");
        for x in 0..320 {
            img.set_pixel(x, x % 200, ColourIndex::WHITE);
        }
        let best = (0..50)
            .map(|_| {
                let t = std::time::Instant::now();
                let r = validate(&img);
                let e = t.elapsed();
                assert_eq!(r, Ok(()));
                e
            })
            .min()
            .unwrap_or_default();
        println!("best of 50: {best:?}");
        assert!(best.as_micros() < 1000, "took {best:?}");
    }
}
