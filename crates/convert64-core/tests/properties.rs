//! Property tests: random **legal** images of every implemented mode must validate, and
//! each targeted **mutation** must be rejected with the right rule at the right place.
//!
//! Cases are small descriptions (mode, sizes, seeds, the cell to break) that build the
//! image deterministically, so shrinking output stays readable.
//!
//! When the bitmap solver lands (#5, E5 Bitmap solver), add a property here: for random
//! source images and recipes, `validate(&solve(..))` is `Ok`. Until then these
//! properties exercise the validator itself.

use convert64_core::mode::{BlockKind, Mode};
use convert64_core::model::{
    CellPos, CharColour, CharsetGlobals, CharsetImage, IndexedBitmap, OverlayPair, Sprite,
    SpritePos, SpriteSet, TileSize,
};
use convert64_core::{
    ColourIndex, ColourSet, Location, Placement, Rule, Violation, validate, validate_placement,
};
use proptest::prelude::*;

/// SplitMix64: a tiny deterministic generator so cases are described by a seed.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    fn colour(&mut self) -> ColourIndex {
        ColourIndex::from_low_nybble(self.below(16) as u8)
    }
}

fn single(result: Result<(), Vec<Violation>>) -> (Rule, Location) {
    let v = result.expect_err("expected a violation");
    assert_eq!(v.len(), 1, "expected exactly one violation, got {v:#?}");
    (v[0].rule, v[0].location)
}

fn at_cell(x: u16, y: u16) -> Location {
    Location::Cell {
        cell: CellPos::new(x, y),
    }
}

/// Every mode needs a generator below; adding a mode breaks this match until it has one.
#[test]
fn every_mode_is_covered() {
    for mode in Mode::all() {
        let covered_by = match mode {
            Mode::MulticolorBitmap | Mode::HiresBitmap => "bitmap_case",
            Mode::MulticolorCharset | Mode::HiresCharset | Mode::EcmCharset => "charset_case",
            Mode::HiresSprite | Mode::MulticolorSprite | Mode::OverlaySprite => "sprite_case",
            Mode::Fli | Mode::Ifli | Mode::Mci | Mode::Nufli => {
                assert!(
                    mode.spec().is_err(),
                    "{mode} gained a spec: add a generator"
                );
                "reserved"
            }
        };
        assert!(!covered_by.is_empty());
    }
}

// ---------------------------------------------------------------------------------------
// Bitmaps (rule 1).

#[derive(Clone, Debug)]
struct BitmapCase {
    mode: Mode,
    background: u8,
    seed: u64,
    /// The cell a mutation breaks.
    cell_x: u16,
    cell_y: u16,
}

fn bitmap_case() -> impl Strategy<Value = BitmapCase> {
    (
        prop_oneof![Just(Mode::MulticolorBitmap), Just(Mode::HiresBitmap)],
        0..16_u8,
        any::<u64>(),
        0..40_u16,
        0..25_u16,
    )
        .prop_map(|(mode, background, seed, cell_x, cell_y)| BitmapCase {
            mode,
            background,
            seed,
            cell_x,
            cell_y,
        })
}

/// A legal picture: each cell picks its own colours (3 plus background for multicolor,
/// 2 for hi-res, sometimes fewer) and every pixel uses one of them.
fn legal_bitmap(case: &BitmapCase) -> IndexedBitmap {
    let mut rng = Rng(case.seed);
    let spec = case.mode.spec().expect("bitmap modes are implemented");
    let background = ColourIndex::from_low_nybble(case.background);
    let multicolor = case.mode == Mode::MulticolorBitmap;
    let mut img = IndexedBitmap::filled(case.mode, background).expect("bitmap mode");
    let (cw, ch) = (u16::from(spec.cell_width), u16::from(spec.cell_height));
    for cy in 0..spec.cells_y() {
        for cx in 0..spec.cells_x() {
            let own = if multicolor { 3 } else { 2 };
            let mut palette: Vec<ColourIndex> = (0..own).map(|_| rng.colour()).collect();
            if multicolor {
                palette.push(background);
            }
            for y in cy * ch..(cy + 1) * ch {
                for x in cx * cw..(cx + 1) * cw {
                    let pick = palette[rng.below(palette.len() as u64) as usize];
                    img.set_pixel(x, y, pick);
                }
            }
        }
    }
    img
}

/// Adds colours to one cell until it holds one more than the mode allows.
fn overfill_cell(img: &mut IndexedBitmap, cell: CellPos) {
    let spec = img.spec();
    let limit = u32::from(spec.colours_per_cell);
    let globals = img
        .background()
        .map_or(ColourSet::EMPTY, |bg| ColourSet::EMPTY.with(bg));
    let (cw, ch) = (u16::from(spec.cell_width), u16::from(spec.cell_height));
    let mut used = img.cell_colours(cell).union(globals);
    let initial = used;
    let mut free = ColourIndex::ALL
        .into_iter()
        .filter(move |&k| !initial.contains(k));
    let mut slot = 0;
    while used.len() <= limit {
        let colour = free.next().expect("16 colours exceed any cell limit");
        // Overwrite pixels one by one, but only ones whose colour stays present
        // elsewhere in the cell, so the count really grows.
        loop {
            let (x, y) = (cell.x * cw + slot % cw, cell.y * ch + slot / cw);
            slot += 1;
            let old = img.pixel(x, y);
            img.set_pixel(x, y, colour);
            let now = img.cell_colours(cell).union(globals);
            if now.len() > used.len() {
                used = now;
                break;
            }
            img.set_pixel(x, y, old);
            assert!(slot < cw * ch, "cell too small to overfill");
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    #[test]
    fn legal_bitmaps_validate(case in bitmap_case()) {
        let img = legal_bitmap(&case);
        prop_assert_eq!(validate(&img), Ok(()));
    }

    #[test]
    fn overfilled_cell_is_rejected_at_that_cell(case in bitmap_case()) {
        let mut img = legal_bitmap(&case);
        overfill_cell(&mut img, CellPos::new(case.cell_x, case.cell_y));
        prop_assert_eq!(
            single(validate(&img)),
            (Rule::ColoursPerCell, at_cell(case.cell_x, case.cell_y))
        );
    }
}

// ---------------------------------------------------------------------------------------
// Charsets (rules 2 and 3), any map size including odd ones.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CharsetKind {
    Hires,
    Multicolor,
    Ecm,
}

#[derive(Clone, Debug)]
struct CharsetCase {
    kind: CharsetKind,
    width: u16,
    height: u16,
    /// Chars in the charset, 1..=budget.
    chars: usize,
    seed: u64,
    /// Cell a mutation breaks, as a fraction of the map (0..1000 per mille).
    target: (u16, u16),
}

impl CharsetCase {
    fn budget(&self) -> usize {
        if self.kind == CharsetKind::Ecm {
            64
        } else {
            256
        }
    }

    fn target_cell(&self) -> (u16, u16) {
        let scale =
            |t: u16, n: u16| u16::try_from(u32::from(t) * u32::from(n) / 1000).expect("below n");
        (
            scale(self.target.0, self.width),
            scale(self.target.1, self.height),
        )
    }

    fn globals(&self, rng: &mut Rng) -> CharsetGlobals {
        match self.kind {
            CharsetKind::Hires => CharsetGlobals::Hires {
                background: rng.colour(),
            },
            CharsetKind::Multicolor => CharsetGlobals::Multicolor {
                background: rng.colour(),
                multicolor1: rng.colour(),
                multicolor2: rng.colour(),
            },
            CharsetKind::Ecm => CharsetGlobals::Ecm {
                backgrounds: [rng.colour(), rng.colour(), rng.colour(), rng.colour()],
            },
        }
    }
}

fn charset_case() -> impl Strategy<Value = CharsetCase> {
    (
        prop_oneof![
            Just(CharsetKind::Hires),
            Just(CharsetKind::Multicolor),
            Just(CharsetKind::Ecm)
        ],
        1..=81_u16,
        1..=51_u16,
        1..=256_usize,
        any::<u64>(),
        (0..1000_u16, 0..1000_u16),
    )
        .prop_map(|(kind, width, height, chars, seed, target)| {
            let budget = if kind == CharsetKind::Ecm { 64 } else { 256 };
            CharsetCase {
                kind,
                width,
                height,
                chars: 1 + (chars - 1) % budget,
                seed,
                target,
            }
        })
}

/// The parts of a legal charset picture, so mutations can change one thing.
struct CharsetParts {
    charset: Vec<[u8; 8]>,
    map: Vec<u8>,
    colours: Vec<CharColour>,
    globals: CharsetGlobals,
}

impl CharsetParts {
    fn legal(case: &CharsetCase) -> Self {
        let mut rng = Rng(case.seed);
        let globals = case.globals(&mut rng);
        let cells = usize::from(case.width) * usize::from(case.height);
        let charset = (0..case.chars).map(|_| rng.next().to_le_bytes()).collect();
        let map = (0..cells)
            .map(|_| {
                let index = rng.below(case.chars as u64) as u8;
                match case.kind {
                    CharsetKind::Ecm => index | ((rng.below(4) as u8) << 6),
                    _ => index,
                }
            })
            .collect();
        let colours = (0..cells)
            .map(|_| match case.kind {
                CharsetKind::Multicolor => {
                    let colour = ColourIndex::from_low_nybble(rng.below(8) as u8);
                    if rng.below(2) == 0 {
                        CharColour::multicolor(colour)
                    } else {
                        CharColour::hires(colour)
                    }
                }
                _ => CharColour::hires(rng.colour()),
            })
            .collect();
        Self {
            charset,
            map,
            colours,
            globals,
        }
    }

    fn build(self, case: &CharsetCase) -> CharsetImage {
        CharsetImage::new(
            self.charset,
            self.map,
            case.width,
            case.height,
            self.colours,
            self.globals,
            TileSize::SINGLE,
        )
        .expect("sizes are consistent")
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]

    #[test]
    fn legal_charsets_validate(case in charset_case()) {
        let img = CharsetParts::legal(&case).build(&case);
        prop_assert_eq!(validate(&img), Ok(()));
    }

    #[test]
    fn one_char_over_budget_is_rejected(case in charset_case(), extra in 1..=8_usize) {
        let mut parts = CharsetParts::legal(&case);
        let budget = case.budget();
        parts.charset.resize(budget + extra, [0; 8]);
        prop_assert_eq!(
            single(validate(&parts.build(&case))),
            (Rule::CharsetBudget, Location::Char { index: budget })
        );
    }

    #[test]
    fn code_past_the_charset_is_rejected_at_its_cell(case in charset_case()) {
        prop_assume!(case.chars < case.budget());
        let (x, y) = case.target_cell();
        let mut parts = CharsetParts::legal(&case);
        let i = usize::from(y) * usize::from(case.width) + usize::from(x);
        // Point the char index just past the end; ECM keeps its background bits.
        let background_bits = if case.kind == CharsetKind::Ecm { parts.map[i] & 0xC0 } else { 0 };
        parts.map[i] = background_bits | case.chars as u8;
        prop_assert_eq!(
            single(validate(&parts.build(&case))),
            (Rule::CharsetBudget, at_cell(x, y))
        );
    }

    #[test]
    fn multicolor_char_colour_above_7_is_rejected_at_its_cell(
        case in charset_case(),
        colour in 8..16_u8,
        multicolor in any::<bool>(),
    ) {
        prop_assume!(case.kind == CharsetKind::Multicolor);
        let (x, y) = case.target_cell();
        let mut parts = CharsetParts::legal(&case);
        let i = usize::from(y) * usize::from(case.width) + usize::from(x);
        let colour = ColourIndex::from_low_nybble(colour);
        parts.colours[i] = if multicolor {
            CharColour::multicolor(colour)
        } else {
            CharColour::hires(colour)
        };
        prop_assert_eq!(
            single(validate(&parts.build(&case))),
            (Rule::ColourRange, at_cell(x, y))
        );
    }
}

// ---------------------------------------------------------------------------------------
// Sprites (rule 4): hi-res-only, multicolor-only and overlay sets of any size.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SpriteKind {
    Hires,
    Multicolor,
    Overlay,
}

#[derive(Clone, Debug)]
struct SpriteCase {
    kind: SpriteKind,
    /// Sprites (overlay: pairs) in the set.
    count: usize,
    seed: u64,
    /// Which overlay pair a mutation breaks, and by how much.
    pair: usize,
    dx: u16,
    dy: u16,
}

fn sprite_case() -> impl Strategy<Value = SpriteCase> {
    (
        prop_oneof![
            Just(SpriteKind::Hires),
            Just(SpriteKind::Multicolor),
            Just(SpriteKind::Overlay)
        ],
        1..=17_usize,
        any::<u64>(),
        any::<usize>(),
        (0..3_u16, 0..3_u16),
    )
        .prop_map(|(kind, count, seed, pair, (dx, dy))| SpriteCase {
            kind,
            count,
            seed,
            pair,
            dx,
            dy,
        })
}

fn random_sprite(rng: &mut Rng, multicolor: bool) -> Sprite {
    let mut px = [0_u8; 63];
    for b in &mut px {
        *b = rng.next() as u8;
    }
    Sprite::new(px, rng.colour(), multicolor)
}

fn legal_sprites(case: &SpriteCase) -> SpriteSet {
    let mut rng = Rng(case.seed);
    let mut sprites = Vec::new();
    let mut positions = Vec::new();
    let mut overlays = Vec::new();
    for _ in 0..case.count {
        let pos = SpritePos {
            x: rng.below(344) as u16,
            y: rng.below(250) as u16,
        };
        match case.kind {
            SpriteKind::Hires | SpriteKind::Multicolor => {
                sprites.push(random_sprite(&mut rng, case.kind == SpriteKind::Multicolor));
                positions.push(pos);
            }
            SpriteKind::Overlay => {
                let base = sprites.len();
                sprites.push(random_sprite(&mut rng, true));
                sprites.push(random_sprite(&mut rng, false));
                positions.extend([pos, pos]);
                overlays.push(OverlayPair {
                    hires: base + 1,
                    multicolor: base,
                });
            }
        }
    }
    SpriteSet::new(sprites, rng.colour(), rng.colour())
        .with_positions(positions)
        .expect("one position per sprite")
        .with_overlays(overlays)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]

    #[test]
    fn legal_sprite_sets_validate(case in sprite_case()) {
        let set = legal_sprites(&case);
        prop_assert_eq!(validate(&set), Ok(()));
        let expected = match case.kind {
            SpriteKind::Hires => Mode::HiresSprite,
            SpriteKind::Multicolor => Mode::MulticolorSprite,
            SpriteKind::Overlay => Mode::OverlaySprite,
        };
        prop_assert_eq!(convert64_core::C64Image::mode(&set), expected);
    }

    #[test]
    fn moved_overlay_sprite_is_rejected(case in sprite_case()) {
        prop_assume!(case.kind == SpriteKind::Overlay && (case.dx, case.dy) != (0, 0));
        let mut set = legal_sprites(&case);
        let pair = set.overlays()[case.pair % set.overlays().len()];
        let p = &mut set.positions_mut()[pair.hires];
        p.x += case.dx;
        p.y += case.dy;
        prop_assert_eq!(
            single(validate(&set)),
            (Rule::Sprites, Location::Sprite { index: pair.hires })
        );
    }
}

// ---------------------------------------------------------------------------------------
// Placement (rule 5).

fn align(kind: BlockKind) -> u16 {
    match kind {
        BlockKind::Charset => 0x0800,
        BlockKind::ScreenRam => 0x0400,
        BlockKind::Bitmap => 0x2000,
        _ => 64,
    }
}

fn size(kind: BlockKind) -> u32 {
    match kind {
        BlockKind::Charset => 2048,
        BlockKind::ScreenRam => 1000,
        BlockKind::Bitmap => 8000,
        _ => 64,
    }
}

fn in_shadow(kind: BlockKind, address: u16) -> bool {
    let (start, end) = (u32::from(address), u32::from(address) + size(kind) - 1);
    [(0x1000_u32, 0x1FFF_u32), (0x9000, 0x9FFF)]
        .iter()
        .any(|&(lo, hi)| start <= hi && end >= lo)
}

fn placed_block() -> impl Strategy<Value = (BlockKind, u16)> {
    prop_oneof![
        Just(BlockKind::Charset),
        Just(BlockKind::ScreenRam),
        Just(BlockKind::Bitmap),
        Just(BlockKind::Sprite)
    ]
    .prop_flat_map(|kind| {
        let slots = 0x1_0000 / u32::from(align(kind));
        (Just(kind), 0..slots)
    })
    .prop_map(|(kind, slot)| (kind, (slot * u32::from(align(kind))) as u16))
    .prop_filter("legal blocks avoid the shadow", |&(kind, address)| {
        !in_shadow(kind, address)
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    #[test]
    fn aligned_shadow_free_placements_validate(
        blocks in proptest::collection::vec(placed_block(), 1..8)
    ) {
        let p = blocks
            .iter()
            .fold(Placement::new(), |p, &(kind, address)| p.with(kind, address));
        prop_assert_eq!(validate_placement(&p), Ok(()));
    }

    #[test]
    fn misaligned_block_is_rejected_at_its_address(
        (kind, address) in placed_block(),
        offset in 1..64_u16,
    ) {
        let bad = address + offset;
        prop_assume!(!in_shadow(kind, bad));
        let p = Placement::new().with(kind, bad);
        prop_assert_eq!(
            single(validate_placement(&p)),
            (Rule::Placement, Location::Address { block: kind, address: bad })
        );
    }

    #[test]
    fn block_in_the_char_rom_shadow_is_rejected(
        kind in prop_oneof![Just(BlockKind::Charset), Just(BlockKind::ScreenRam), Just(BlockKind::Sprite)],
        bank2 in any::<bool>(),
        slot in 0..2_u16,
    ) {
        // Aligned addresses inside $1000-$1FFF or $9000-$9FFF.
        let address = if bank2 { 0x9000 } else { 0x1000 } + slot * 0x0800;
        let p = Placement::new().with(kind, address);
        prop_assert_eq!(
            single(validate_placement(&p)),
            (Rule::Placement, Location::Address { block: kind, address })
        );
    }
}
