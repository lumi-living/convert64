//! Hand-built legal and illegal images, one test per rule per applicable mode.
//!
//! Names follow `rule<N>_<mode>_rejects_<what>` (and `…_accepts_…` for the legal
//! baseline). They are listed next to each rule in
//! `docs/reference/vic-ii-constraints.md` §6. This file is the regression net the
//! validator is judged by: never weaken a test here to make a change pass.

mod common;

use common::{
    CharsetBuilder, SpriteSetBuilder, bank1_placement, c, encoded_hires_bitmap, encoded_mc_bitmap,
    hires_bitmap, mc_bitmap,
};
use convert64_core::mode::BlockKind;
use convert64_core::model::{BitmapImage, CellPos, CharColour, OverlayPair, SpriteSet};
use convert64_core::{
    C64Image, ColourIndex, Location, Placement, Rule, Violation, validate, validate_placement,
};

/// Asserts exactly one violation of `rule` at `location`.
fn assert_single(result: Result<(), Vec<Violation>>, rule: Rule, location: Location) {
    let v = result.expect_err("expected a violation");
    assert_eq!(v.len(), 1, "expected one violation, got {v:#?}");
    assert_eq!((v[0].rule, v[0].location), (rule, location), "{}", v[0]);
}

fn cell(x: u16, y: u16) -> Location {
    Location::Cell {
        cell: CellPos::new(x, y),
    }
}

fn address(block: BlockKind, address: u16) -> Location {
    Location::Address { block, address }
}

// ---------------------------------------------------------------------------------------
// Legal baselines: every builder's default passes every rule.

#[test]
fn every_builder_default_is_legal() {
    let images: Vec<Box<dyn C64Image>> = vec![
        Box::new(mc_bitmap()),
        Box::new(hires_bitmap()),
        Box::new(encoded_mc_bitmap()),
        Box::new(encoded_hires_bitmap()),
        Box::new(CharsetBuilder::hires().build()),
        Box::new(CharsetBuilder::multicolor().build()),
        Box::new(CharsetBuilder::ecm().build()),
        Box::new(SpriteSetBuilder::new().build()),
    ];
    for img in &images {
        assert_eq!(validate(img.as_ref()), Ok(()), "{}", img.mode());
    }
    assert_eq!(validate_placement(&bank1_placement()), Ok(()));
}

// ---------------------------------------------------------------------------------------
// Rule 1: colours per cell, counting global colours.

#[test]
fn rule1_mc_bitmap_rejects_five_colours_in_a_cell() {
    let mut img = mc_bitmap();
    // Cell (7, 3) already shows background + 3; a fourth own colour makes 5.
    let (x, y) = (7 * 4, 3 * 8 + 5);
    let used = img.cell_colours(CellPos::new(7, 3));
    let extra = ColourIndex::ALL
        .into_iter()
        .find(|&k| k != ColourIndex::BLACK && !used.contains(k))
        .expect("a colour not yet in the cell");
    img.set_pixel(x, y, extra);
    // The replaced pixel was background, so the cell now uses background + 4.
    assert_single(validate(&img), Rule::ColoursPerCell, cell(7, 3));
}

#[test]
fn rule1_mc_bitmap_rejects_four_own_colours_without_background_pixels() {
    // Four colours and no background pixel still counts the background: 5 > 4.
    let mut img = mc_bitmap();
    for (i, k) in [2_u8, 3, 4, 5].into_iter().enumerate() {
        for y in 0..8 {
            img.set_pixel(u16::try_from(i).expect("0..4"), y, c(k));
        }
    }
    assert_single(validate(&img), Rule::ColoursPerCell, cell(0, 0));
}

#[test]
fn rule1_hires_bitmap_rejects_three_colours_in_a_cell() {
    let mut img = hires_bitmap();
    img.set_pixel(319, 199, ColourIndex::WHITE);
    img.set_pixel(318, 199, ColourIndex::RED);
    let v = validate(&img).expect_err("3 colours");
    assert_eq!(
        (v[0].rule, v[0].location),
        (Rule::ColoursPerCell, cell(39, 24))
    );
}

#[test]
fn rule1_encoded_bitmaps_accept_any_bytes() {
    // Bit patterns can only select 2 (hi-res) or 4 (multicolor) colours per cell.
    assert_eq!(validate(&encoded_mc_bitmap()), Ok(()));
    assert_eq!(validate(&encoded_hires_bitmap()), Ok(()));
}

// ---------------------------------------------------------------------------------------
// Rule 2: palette indices 0-15; multicolor-charset per-char colour 0-7.

#[test]
fn rule2_mc_charset_rejects_char_colour_9() {
    let img = CharsetBuilder::multicolor()
        .colour_at(11, 6, CharColour::multicolor(ColourIndex::BROWN))
        .build();
    assert_single(validate(&img), Rule::ColourRange, cell(11, 6));
}

#[test]
fn rule2_mc_charset_rejects_hires_char_colour_above_7() {
    // Hi-res chars in multicolor mode also have only three colour bits.
    let img = CharsetBuilder::multicolor()
        .colour_at(0, 24, CharColour::hires(ColourIndex::LIGHT_GREY))
        .build();
    assert_single(validate(&img), Rule::ColourRange, cell(0, 24));
}

#[test]
fn rule2_mc_bitmap_rejects_background_16() {
    let err = BitmapImage::multicolor_from_bytes(&[0; 8000], &[0; 1000], &[0; 1000], 16)
        .expect_err("background 16");
    assert_eq!(err.rule(), Some(Rule::ColourRange));
}

#[test]
fn rule2_mc_sprite_rejects_multicolor_register_16() {
    let err = SpriteSet::from_bytes(&[0; 64], 16, 0).expect_err("MC1 16");
    assert_eq!(err.rule(), Some(Rule::ColourRange));
}

#[test]
fn rule2_any_mode_rejects_colour_index_16() {
    assert!(ColourIndex::new(16).is_err());
    assert!(ColourIndex::new(15).is_ok());
}

// ---------------------------------------------------------------------------------------
// Rule 3: unique-character budget and existing chars.

#[test]
fn rule3_hires_charset_rejects_257_chars() {
    let img = CharsetBuilder::hires().chars(257).build();
    assert_single(
        validate(&img),
        Rule::CharsetBudget,
        Location::Char { index: 256 },
    );
}

#[test]
fn rule3_mc_charset_rejects_257_chars() {
    let img = CharsetBuilder::multicolor().chars(257).build();
    assert_single(
        validate(&img),
        Rule::CharsetBudget,
        Location::Char { index: 256 },
    );
}

#[test]
fn rule3_ecm_charset_rejects_65_chars() {
    let img = CharsetBuilder::ecm().chars(65).build();
    assert_single(
        validate(&img),
        Rule::CharsetBudget,
        Location::Char { index: 64 },
    );
}

#[test]
fn rule3_hires_charset_rejects_a_missing_char() {
    let img = CharsetBuilder::hires().chars(10).code_at(39, 0, 10).build();
    assert_single(validate(&img), Rule::CharsetBudget, cell(39, 0));
}

#[test]
fn rule3_mc_charset_rejects_a_missing_char() {
    let img = CharsetBuilder::multicolor()
        .chars(100)
        .code_at(5, 5, 200)
        .build();
    assert_single(validate(&img), Rule::CharsetBudget, cell(5, 5));
}

#[test]
fn rule3_ecm_charset_rejects_a_missing_char() {
    // Code $C5 shows char 5 on background 3; the charset has only 5 chars.
    let img = CharsetBuilder::ecm().chars(5).code_at(2, 1, 0xC5).build();
    assert_single(validate(&img), Rule::CharsetBudget, cell(2, 1));
}

#[test]
fn rule3_ecm_charset_accepts_any_background_bits() {
    let img = CharsetBuilder::ecm().code_at(0, 0, 0xFF).build();
    assert_eq!(validate(&img), Ok(()));
}

// ---------------------------------------------------------------------------------------
// Rule 4: 64-byte sprites, one MC1/MC2 pair, overlay pairs share coordinates.

#[test]
fn rule4_hires_sprite_rejects_63_byte_block() {
    let err = SpriteSet::from_bytes(&[0; 63], 0, 0).expect_err("63 bytes");
    assert_eq!(err.rule(), Some(Rule::Sprites));
}

#[test]
fn rule4_mc_sprite_rejects_partial_second_block() {
    let err = SpriteSet::from_bytes(&[0x80; 100], 1, 2).expect_err("100 bytes");
    assert_eq!(err.rule(), Some(Rule::Sprites));
}

#[test]
fn rule4_overlay_sprite_rejects_different_coordinates() {
    let set = SpriteSetBuilder::new().position(1, 101, 120).build();
    assert_single(validate(&set), Rule::Sprites, Location::Sprite { index: 1 });
}

#[test]
fn rule4_overlay_sprite_rejects_hires_base() {
    // Sprite 0 (hi-res) as the base of an overlay; sprite 1 on top, same position.
    let set = SpriteSetBuilder::new()
        .position(0, 100, 120)
        .overlays(vec![OverlayPair {
            hires: 1,
            multicolor: 0,
        }])
        .build();
    assert_single(validate(&set), Rule::Sprites, Location::Sprite { index: 0 });
}

#[test]
fn rule4_overlay_sprite_rejects_a_missing_sprite() {
    let set = SpriteSetBuilder::new()
        .overlays(vec![OverlayPair {
            hires: 1,
            multicolor: 9,
        }])
        .build();
    assert_single(validate(&set), Rule::Sprites, Location::Sprite { index: 9 });
}

// ---------------------------------------------------------------------------------------
// Rule 5: alignment and the char-ROM shadow.

#[test]
fn rule5_sprite_rejects_misaligned_block() {
    let p = Placement::new().with(BlockKind::Sprite, 0x4020);
    assert_single(
        validate_placement(&p),
        Rule::Placement,
        address(BlockKind::Sprite, 0x4020),
    );
}

#[test]
fn rule5_charset_rejects_char_rom_shadow() {
    let p = Placement::new().with(BlockKind::Charset, 0x1000);
    assert_single(
        validate_placement(&p),
        Rule::Placement,
        address(BlockKind::Charset, 0x1000),
    );
}

#[test]
fn rule5_charset_rejects_misaligned_address() {
    let p = Placement::new().with(BlockKind::Charset, 0x4400);
    assert_single(
        validate_placement(&p),
        Rule::Placement,
        address(BlockKind::Charset, 0x4400),
    );
}

#[test]
fn rule5_bitmap_rejects_char_rom_shadow_in_bank_2() {
    // $8000-$9F3F overlaps $9000-$9FFF.
    let p = Placement::new().with(BlockKind::Bitmap, 0x8000);
    assert_single(
        validate_placement(&p),
        Rule::Placement,
        address(BlockKind::Bitmap, 0x8000),
    );
}

#[test]
fn rule5_bitmap_rejects_misaligned_address() {
    let p = Placement::new().with(BlockKind::Bitmap, 0x6400);
    assert_single(
        validate_placement(&p),
        Rule::Placement,
        address(BlockKind::Bitmap, 0x6400),
    );
}

#[test]
fn rule5_screen_rejects_misaligned_address() {
    let p = Placement::new().with(BlockKind::ScreenRam, 0x5C10);
    assert_single(
        validate_placement(&p),
        Rule::Placement,
        address(BlockKind::ScreenRam, 0x5C10),
    );
}

#[test]
fn rule5_colour_ram_rejects_relocation() {
    let p = Placement::new().with(BlockKind::ColourRam, 0x5800);
    assert_single(
        validate_placement(&p),
        Rule::Placement,
        address(BlockKind::ColourRam, 0x5800),
    );
}

#[test]
fn rule5_bank3_accepts_the_shadow_free_range() {
    // Banks 1 and 3 have no char-ROM shadow.
    let p = Placement::new()
        .with(BlockKind::Bitmap, 0xE000)
        .with(BlockKind::Charset, 0xD000)
        .with(BlockKind::ScreenRam, 0xCC00);
    assert_eq!(validate_placement(&p), Ok(()));
}

// ---------------------------------------------------------------------------------------
// Keep the reference document in step with this file.

#[test]
fn reference_lists_every_rule_test() {
    let reference = include_str!("../../../docs/reference/vic-ii-constraints.md");
    let source = include_str!("rules.rs");
    let names: Vec<&str> = source
        .lines()
        .filter_map(|l| l.strip_prefix("fn rule"))
        .filter_map(|rest| rest.split('(').next())
        .collect();
    assert!(names.len() >= 25, "found {} rule tests", names.len());
    for name in names {
        let full = format!("`rule{name}`");
        assert!(
            reference.contains(&full),
            "{full} is missing from docs/reference/vic-ii-constraints.md §6"
        );
    }
}
