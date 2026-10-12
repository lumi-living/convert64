//! Bitpair assignment on export (#124, after png2prg).
//!
//! Which of a cell's colours goes on which bit pattern is free: in multicolor the three
//! cell colours can take bit pairs `01` (Screen RAM high nybble), `10` (Screen RAM low
//! nybble) and `11` (Colour RAM) in any order, and in hi-res the two colours can swap
//! between bit `1` (high nybble) and bit `0` (low nybble). The picture looks the same
//! either way; the bytes do not. [`reassign`] chooses the assignment by a
//! [`BitpairPolicy`], rewriting Screen RAM, Colour RAM and the cell's bitmap bits
//! together, so the displayed picture never changes.
//!
//! [`BitpairPolicy::Stable`] keeps a colour on the bit pattern it had in the cell to the
//! left, else the cell above, else the one it has used most so far, as png2prg does.
//! Screen and Colour RAM then repeat more, which packs smaller and avoids a colour
//! switching bit pair between neighbouring cells. Slots a cell does not use copy the
//! neighbour's value for the same reason.
//!
//! "Packs smaller" is measured by [`packed_size_estimate`]: the size of Screen and
//! Colour RAM compressed as fixed-Huffman DEFLATE ([`deflate_fixed_size`]), computed
//! here without a compression crate (TAD §4). On the six png2c64 baselines `Stable`
//! is smaller than `Frequency` on every one.

use convert64_core::ColourIndex;
use convert64_core::model::{BitmapImage, CellPos, ScreenCell};

/// How [`reassign`] puts a cell's colours on its bit patterns.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum BitpairPolicy {
    /// Keep each colour on the bit pattern it had in the cell to the left, else above,
    /// else the one it has used most so far; unused slots repeat the left (else above)
    /// neighbour's colour. Deterministic.
    #[default]
    Stable,
    /// Per cell, the most used colour first (ties: lower index): bit pairs `01`, `10`,
    /// `11` in multicolor, bit `0` then `1` in hi-res. Unused slots are black. No
    /// regard for neighbours.
    Frequency,
    /// Start from [`Stable`](Self::Stable), then, over two passes, try every
    /// assignment of every cell (with the neighbours' values or black in unused slots)
    /// and keep the one an adaptive order-1 model of Screen and Colour RAM codes
    /// shortest. Keep the result only if [`packed_size_estimate`] says it packs smaller
    /// than `Stable`'s, else return `Stable`'s. Slower (tens of milliseconds); never
    /// larger than `Stable` by that estimate.
    BruteForcePacked,
}

impl BitpairPolicy {
    /// Every policy.
    pub const ALL: [Self; 3] = [Self::Stable, Self::Frequency, Self::BruteForcePacked];

    /// The short id for command lines: `stable`, `frequency`, `packed`.
    pub const fn id(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Frequency => "frequency",
            Self::BruteForcePacked => "packed",
        }
    }

    /// The policy an id names, case-insensitively.
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|p| p.id().eq_ignore_ascii_case(id))
    }
}

/// An adaptive order-1 model of one byte stream, the way an arithmetic coder would
/// learn it: each byte is coded given the one before, with Krichevsky–Trofimov counts
/// (every symbol starts at ½) over an alphabet of `symbols`. [`bits`](Self::bits) is
/// the total code length. A byte seen often after the same predecessor (a run, a
/// repeated neighbour) costs little; a new byte or a new predecessor costs about
/// `log2(2·symbols)` bits, which keeps a search from scattering values.
struct Order1 {
    symbols: f64,
    pairs: Vec<u32>,
    contexts: Vec<u32>,
    bits: f64,
}

impl Order1 {
    fn new(stream: &[u8], symbols: usize) -> Self {
        let mut m = Self {
            symbols: symbols as f64,
            pairs: vec![0; 256 * 256],
            contexts: vec![0; 256],
            bits: 0.0,
        };
        for w in stream.windows(2) {
            m.pair(w[0], w[1], 1);
        }
        m
    }

    /// Adds (`sign` 1) or removes (`sign` −1) one `prev → next` pair; returns the
    /// change in bits.
    fn pair(&mut self, prev: u8, next: u8, sign: i32) -> f64 {
        let p = &mut self.pairs[usize::from(prev) * 256 + usize::from(next)];
        let t = &mut self.contexts[usize::from(prev)];
        if sign < 0 {
            *p -= 1;
            *t -= 1;
        }
        // The cost of coding `next` after `prev` with the counts before it was added.
        let cost = (f64::from(*t) + 0.5 * self.symbols).log2() - (f64::from(*p) + 0.5).log2();
        if sign > 0 {
            *p += 1;
            *t += 1;
        }
        let delta = if sign > 0 { cost } else { -cost };
        self.bits += delta;
        delta
    }

    /// Replaces the byte at `i` in `stream` (`old` there now) with `new`, keeping the
    /// pairs with its neighbours; returns the change in bits. `stream` is not changed.
    fn replace(&mut self, stream: &[u8], i: usize, old: u8, new: u8) -> f64 {
        let mut d = 0.0;
        if i > 0 {
            d += self.pair(stream[i - 1], old, -1) + self.pair(stream[i - 1], new, 1);
        }
        if i + 1 < stream.len() {
            d += self.pair(old, stream[i + 1], -1) + self.pair(new, stream[i + 1], 1);
        }
        d
    }
}

/// Length code bases and extra bits (RFC 1951 §3.2.5), codes 257–285.
const LEN_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LEN_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
/// Distance code bases and extra bits, codes 0–29.
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];

/// The size in bytes of `bytes` compressed as one DEFLATE block with the fixed Huffman
/// codes of RFC 1951 (§3.2.6), after a greedy LZ77 parse: at each position the longest
/// earlier match of 3 to 258 bytes within 32 KB (the nearest of equal length), else a
/// literal. That is a valid stream a real inflater accepts, so the figure is a size
/// that is actually achievable, without a compression crate. Add 6 bytes for a zlib
/// wrapper. Quadratic in the input; meant for the 2000 bytes of Screen and Colour RAM.
pub fn deflate_fixed_size(bytes: &[u8]) -> usize {
    let n = bytes.len();
    // Block header (BFINAL, BTYPE) and the end-of-block code (256: 7 bits).
    let mut bits = 3 + 7;
    let mut i = 0;
    while i < n {
        let (mut best, mut dist) = (0, 0);
        for j in (i.saturating_sub(32_768)..i).rev() {
            let max = (n - i).min(258);
            let len = (0..max)
                .take_while(|&k| bytes[j + k] == bytes[i + k])
                .count();
            if len > best {
                (best, dist) = (len, i - j);
                if len == max {
                    break;
                }
            }
        }
        if best >= 3 {
            // Lengths 3–257 and 258: codes 257–279 are 7 bits, 280–285 8 bits.
            let l = LEN_BASE
                .iter()
                .rposition(|&b| usize::from(b) <= best)
                .unwrap_or(0);
            bits += if l < 23 { 7 } else { 8 } + usize::from(LEN_EXTRA[l]);
            let d = DIST_BASE
                .iter()
                .rposition(|&b| usize::from(b) <= dist)
                .unwrap_or(0);
            bits += 5 + usize::from(DIST_EXTRA[d]);
            i += best;
        } else {
            // Literals 0–143 are 8 bits, 144–255 9 bits.
            bits += if bytes[i] < 144 { 8 } else { 9 };
            i += 1;
        }
    }
    bits.div_ceil(8)
}

/// How small Screen and Colour RAM pack, as [`BitpairPolicy`] compares assignments:
/// [`deflate_fixed_size`] of the 1000 Screen RAM bytes followed by the 1000 Colour RAM
/// bytes (Screen RAM only for hi-res). Bitmap bytes are left out: every policy writes
/// the same pixels, so a policy changes the bitmap only by which bit pattern a colour
/// gets.
pub fn packed_size_estimate(image: &BitmapImage) -> usize {
    let mut bytes = image.screen_bytes();
    bytes.extend(image.colour_bytes());
    deflate_fixed_size(&bytes)
}

const CELLS_X: usize = 40;
const CELLS_Y: usize = 25;

/// One cell's slots: multicolor `[01, 10, 11]`, hi-res `[bit 0, bit 1]`.
type Slots = [ColourIndex; 3];

/// A cell's own colours (not the background) with their pixel counts, most used first,
/// ties by index.
fn own_colours(image: &BitmapImage, cell: CellPos, bg: Option<ColourIndex>) -> Vec<ColourIndex> {
    let (w, x0) = if bg.is_some() {
        (4, cell.x * 4)
    } else {
        (8, cell.x * 8)
    };
    let mut count = [0_u32; 16];
    for y in cell.y * 8..cell.y * 8 + 8 {
        for x in x0..x0 + w {
            count[usize::from(image.pixel(x, y).get())] += 1;
        }
    }
    let mut own: Vec<ColourIndex> = ColourIndex::ALL
        .into_iter()
        .filter(|&c| count[usize::from(c.get())] > 0 && Some(c) != bg)
        .collect();
    own.sort_by_key(|c| (std::cmp::Reverse(count[usize::from(c.get())]), c.get()));
    own
}

/// The Screen RAM byte and Colour RAM value of a cell's slots.
fn cell_bytes(slots: &Slots, multicolor: bool) -> (u8, Option<u8>) {
    if multicolor {
        (
            ScreenCell {
                high: slots[0],
                low: slots[1],
            }
            .to_byte(),
            Some(slots[2].get()),
        )
    } else {
        (
            ScreenCell {
                high: slots[1],
                low: slots[0],
            }
            .to_byte(),
            None,
        )
    }
}

/// Every way to put `own` on `n` slots, with each unused slot taking one of `fillers`.
fn candidates(own: &[ColourIndex], n: usize, fillers: &[ColourIndex]) -> Vec<Slots> {
    let mut out = Vec::new();
    let mut slots = [None; 3];
    fn place(
        own: &[ColourIndex],
        n: usize,
        slots: &mut [Option<ColourIndex>; 3],
        fillers: &[ColourIndex],
        out: &mut Vec<Slots>,
    ) {
        match own.split_first() {
            Some((&c, rest)) => {
                for i in 0..n {
                    if slots[i].is_none() {
                        slots[i] = Some(c);
                        place(rest, n, slots, fillers, out);
                        slots[i] = None;
                    }
                }
            }
            None => fill(0, n, slots, fillers, out),
        }
    }
    fn fill(
        i: usize,
        n: usize,
        slots: &mut [Option<ColourIndex>; 3],
        fillers: &[ColourIndex],
        out: &mut Vec<Slots>,
    ) {
        if i == n {
            out.push(slots.map(|s| s.unwrap_or(ColourIndex::BLACK)));
        } else if slots[i].is_some() {
            fill(i + 1, n, slots, fillers, out);
        } else {
            for &f in fillers {
                slots[i] = Some(f);
                fill(i + 1, n, slots, fillers, out);
            }
            slots[i] = None;
        }
    }
    place(own, n, &mut slots, fillers, &mut out);
    out
}

/// The image with every cell's colours put on bit patterns by `policy`. Displays
/// exactly as `image` does (the same colour at every pixel) and is as legal; only the
/// bytes change. Deterministic.
pub fn reassign(image: &BitmapImage, policy: BitpairPolicy) -> BitmapImage {
    let bg = image.background();
    let multicolor = bg.is_some();
    let n = if multicolor { 3 } else { 2 };
    let own: Vec<Vec<ColourIndex>> = (0..CELLS_Y)
        .flat_map(|y| (0..CELLS_X).map(move |x| (x, y)))
        .map(|(x, y)| own_colours(image, cell_pos(x, y), bg))
        .collect();

    let mut slots: Vec<Slots> = vec![[ColourIndex::BLACK; 3]; CELLS_X * CELLS_Y];
    match policy {
        BitpairPolicy::Frequency => {
            for (i, cell) in own.iter().enumerate() {
                for (s, &c) in cell.iter().enumerate() {
                    slots[i][s] = c;
                }
                if !multicolor && cell.len() == 1 {
                    slots[i][1] = cell[0];
                }
            }
        }
        BitpairPolicy::Stable | BitpairPolicy::BruteForcePacked => {
            stable(&own, n, &mut slots);
            if policy == BitpairPolicy::BruteForcePacked {
                let stable_image = rebuild(image, bg, &slots);
                packed(&own, n, multicolor, &mut slots);
                let packed_image = rebuild(image, bg, &slots);
                // The search's model is only a guide: keep its result only if it
                // really packs smaller.
                return if packed_size_estimate(&packed_image) < packed_size_estimate(&stable_image)
                {
                    packed_image
                } else {
                    stable_image
                };
            }
        }
    }
    rebuild(image, bg, &slots)
}

const fn cell_pos(x: usize, y: usize) -> CellPos {
    CellPos::new(x as u16, y as u16)
}

/// The neighbours' slot contents that a cell copies or prefers: left, then above.
fn neighbours(slots: &[Slots], i: usize) -> impl Iterator<Item = &Slots> {
    let left = (!i.is_multiple_of(CELLS_X)).then(|| &slots[i - 1]);
    let above = (i >= CELLS_X).then(|| &slots[i - CELLS_X]);
    left.into_iter().chain(above)
}

fn stable(own: &[Vec<ColourIndex>], n: usize, slots: &mut [Slots]) {
    // How often each colour has sat on each slot so far.
    let mut used = [[0_u32; 3]; 16];
    for i in 0..own.len() {
        let mut taken: [Option<ColourIndex>; 3] = [None; 3];
        let mut pending: Vec<ColourIndex> = own[i].clone();
        // 1 and 2: the slot the colour had to the left, then above.
        let near: Vec<Slots> = neighbours(slots, i).copied().collect();
        for nb in &near {
            pending.retain(
                |&c| match (0..n).find(|&s| nb[s] == c && taken[s].is_none()) {
                    Some(s) => {
                        taken[s] = Some(c);
                        false
                    }
                    None => true,
                },
            );
        }
        // 3: the slot it has used most so far; 4: the first free one.
        for c in pending {
            let counts = used[usize::from(c.get())];
            let s = (0..n)
                .filter(|&s| taken[s].is_none())
                .max_by_key(|&s| (counts[s], std::cmp::Reverse(s)))
                .unwrap_or_else(|| unreachable!("at most n own colours"));
            taken[s] = Some(c);
        }
        // Unused slots repeat the left (else above) neighbour, else hold black; a
        // one-colour hi-res cell with no neighbour uses its colour for both.
        for s in 0..n {
            if taken[s].is_none() {
                taken[s] = near
                    .first()
                    .map(|nb| nb[s])
                    .or_else(|| (n == 2 && own[i].len() == 1).then(|| own[i][0]));
            }
        }
        slots[i] = taken.map(|t| t.unwrap_or(ColourIndex::BLACK));
        for &c in &own[i] {
            if let Some(s) = (0..n).find(|&s| slots[i][s] == c) {
                used[usize::from(c.get())][s] += 1;
            }
        }
    }
}

fn packed(own: &[Vec<ColourIndex>], n: usize, multicolor: bool, slots: &mut [Slots]) {
    let bytes = |s: &Slots| cell_bytes(s, multicolor);
    let mut screen: Vec<u8> = slots.iter().map(|s| bytes(s).0).collect();
    let mut colour: Vec<u8> = slots.iter().filter_map(|s| bytes(s).1).collect();
    let mut screen_model = Order1::new(&screen, 256);
    let mut colour_model = Order1::new(&colour, 16);
    for _pass in 0..2 {
        for i in 0..own.len() {
            let mut fillers: Vec<ColourIndex> = Vec::new();
            for nb in neighbours(slots, i) {
                fillers.extend_from_slice(&nb[..n]);
            }
            fillers.push(ColourIndex::BLACK);
            fillers.extend_from_slice(&slots[i][..n]);
            fillers.sort_by_key(|c| c.get());
            fillers.dedup();
            let (sc, co) = bytes(&slots[i]);
            let mut best = (0.0, slots[i]);
            for cand in candidates(&own[i], n, &fillers) {
                let (csc, cco) = bytes(&cand);
                // Try the candidate in the models, then undo.
                let mut d = screen_model.replace(&screen, i, sc, csc);
                screen_model.replace(&screen, i, csc, sc);
                if let (Some(old), Some(new)) = (co, cco) {
                    d += colour_model.replace(&colour, i, old, new);
                    colour_model.replace(&colour, i, new, old);
                }
                if d < best.0 - 1e-9 {
                    best = (d, cand);
                }
            }
            let (nsc, nco) = bytes(&best.1);
            screen_model.replace(&screen, i, sc, nsc);
            screen[i] = nsc;
            if let (Some(old), Some(new)) = (co, nco) {
                colour_model.replace(&colour, i, old, new);
                colour[i] = new;
            }
            slots[i] = best.1;
        }
    }
}

/// The image with `slots` as each cell's colours and its bitmap bits rewritten to them.
fn rebuild(image: &BitmapImage, bg: Option<ColourIndex>, slots: &[Slots]) -> BitmapImage {
    let multicolor = bg.is_some();
    let mut bitmap = vec![0_u8; 8000];
    for (i, cell_slots) in slots.iter().enumerate() {
        let (cx, cy) = (i % CELLS_X, i / CELLS_X);
        for row in 0..8 {
            let y = (cy * 8 + row) as u16;
            let mut byte = 0_u8;
            if multicolor {
                for px in 0..4 {
                    let c = image.pixel((cx * 4 + px) as u16, y);
                    let v = if Some(c) == bg {
                        0
                    } else {
                        1 + (0..3)
                            .find(|&s| cell_slots[s] == c)
                            .unwrap_or_else(|| unreachable!("own colour has a slot"))
                            as u8
                    };
                    byte |= v << (6 - 2 * px);
                }
            } else {
                for px in 0..8 {
                    let c = image.pixel((cx * 8 + px) as u16, y);
                    // Bit 0 is slot 0 (low nybble); a colour on both slots uses bit 0.
                    let v = u8::from(cell_slots[0] != c);
                    byte |= v << (7 - px);
                }
            }
            bitmap[cy * 320 + cx * 8 + row] = byte;
        }
    }
    let screen = slots
        .iter()
        .map(|s| cell_bytes(s, multicolor).0)
        .map(ScreenCell::from_byte)
        .collect();
    match bg {
        Some(bg) => {
            BitmapImage::multicolor(bitmap, screen, slots.iter().map(|s| s[2]).collect(), bg)
        }
        None => BitmapImage::hires(bitmap, screen),
    }
    .unwrap_or_else(|_| unreachable!("sizes are fixed"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use convert64_core::mode::Mode;
    use convert64_core::model::IndexedBitmap;

    fn c(i: u8) -> ColourIndex {
        ColourIndex::new(i).expect("0-15")
    }

    /// A multicolor picture of vertical stripes in colours 2, 5 and 7: in each cell one
    /// of them, rotating from cell to cell, fills two of the four columns, so the most
    /// used colour changes between neighbours.
    fn stripes() -> IndexedBitmap {
        let colours = [c(2), c(5), c(7)];
        let pixels = (0..200_usize)
            .flat_map(|y| (0..160_usize).map(move |x| (x, y)))
            .map(|(x, y)| {
                let cell = x / 4 + (y / 8) * 40;
                let column = [0, 0, 1, 2][x % 4];
                colours[(column + cell) % 3]
            })
            .collect();
        IndexedBitmap::new(Mode::MulticolorBitmap, pixels, Some(c(0))).expect("legal")
    }

    #[test]
    fn ids_round_trip() {
        for p in BitpairPolicy::ALL {
            assert_eq!(BitpairPolicy::from_id(p.id()), Some(p));
        }
        assert_eq!(
            BitpairPolicy::from_id("PACKED"),
            Some(BitpairPolicy::BruteForcePacked)
        );
        assert_eq!(BitpairPolicy::from_id("random"), None);
        assert_eq!(BitpairPolicy::default(), BitpairPolicy::Stable);
    }

    /// Hand-counted fixed-Huffman sizes.
    #[test]
    fn deflate_fixed_sizes() {
        // Header (3 bits) and end of block (7 bits).
        assert_eq!(deflate_fixed_size(&[]), 2);
        // + one 8-bit literal.
        assert_eq!(deflate_fixed_size(b"a"), 3);
        // + one 9-bit literal.
        assert_eq!(deflate_fixed_size(&[0xFF; 1]), 3);
        // A literal, three 258-byte matches (8 + 5 bits each) and a 225-byte match
        // (8 + 5 extra + 5 bits): 10 + 8 + 39 + 18 = 75 bits.
        assert_eq!(deflate_fixed_size(&[0; 1000]), 10);
        // Repetition packs smaller than no repetition.
        let varied: Vec<u8> = (0..=255).collect();
        assert!(deflate_fixed_size(&[7; 256]) < deflate_fixed_size(&varied));
    }

    #[test]
    fn every_policy_displays_the_same_picture() {
        let img = BitmapImage::encode(&stripes()).expect("legal");
        for p in BitpairPolicy::ALL {
            let r = reassign(&img, p);
            assert_eq!(r.to_indexed(), img.to_indexed(), "{p:?}");
            assert_eq!(r.background(), img.background());
            assert_eq!(reassign(&img, p), r, "{p:?} is deterministic");
        }
    }

    /// Stable keeps every colour on one bit pair across the stripes, so Screen and
    /// Colour RAM are one value each; Frequency follows the per-cell counts.
    #[test]
    fn stable_keeps_colours_on_their_bit_pairs() {
        let img = BitmapImage::encode(&stripes()).expect("legal");
        let distinct = |bytes: Vec<u8>| {
            let mut b = bytes;
            b.sort_unstable();
            b.dedup();
            b.len()
        };
        let stable = reassign(&img, BitpairPolicy::Stable);
        let frequency = reassign(&img, BitpairPolicy::Frequency);
        assert_eq!(distinct(stable.screen_bytes()), 1);
        assert_eq!(distinct(stable.colour_bytes()), 1);
        assert!(distinct(frequency.screen_bytes()) > 1);
        assert!(packed_size_estimate(&stable) < packed_size_estimate(&frequency));
        let packed = reassign(&img, BitpairPolicy::BruteForcePacked);
        assert!(packed_size_estimate(&packed) <= packed_size_estimate(&stable));
    }

    /// Hi-res: swapping a cell's two colours inverts its bitmap bits.
    #[test]
    fn hires_swap_inverts_the_cell() {
        let mut screen = vec![ScreenCell::from_byte(0x10); 1000];
        // Cell 1: the same two colours the other way round.
        screen[1] = ScreenCell::from_byte(0x01);
        let mut bitmap = vec![0b1111_0000_u8; 8000];
        for b in &mut bitmap[8..16] {
            *b = 0b0000_1111;
        }
        let img = BitmapImage::hires(bitmap, screen).expect("legal");
        let stable = reassign(&img, BitpairPolicy::Stable);
        assert_eq!(stable.to_indexed(), img.to_indexed());
        let first = stable.screen_bytes()[0];
        assert!(stable.screen_bytes().iter().all(|&b| b == first));
        assert_eq!(stable.bitmap()[8], stable.bitmap()[0]);
    }
}
