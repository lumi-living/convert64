//! VIC-II luminance levels and same-luma groups (`docs/reference/vic-ii-constraints.md` §1).
//!
//! These are properties of the chip, not of a palette: they are the same whichever RGB
//! preset is used for display.

use crate::colour::ColourIndex;

/// Luma level of each colour on the reference table's 0–31 scale, in index order.
const LUMA: [u8; 16] = [0, 31, 10, 20, 12, 16, 8, 24, 12, 8, 16, 10, 15, 24, 15, 20];

/// The nine distinct luma levels, darkest first.
pub const LUMA_LEVELS: [u8; 9] = [0, 8, 10, 12, 15, 16, 20, 24, 31];

/// Colours per luma level, darkest first (reference §1 "Luminance groups").
pub const LUMA_GROUPS: [(u8, &[ColourIndex]); 9] = [
    (0, &[ColourIndex::BLACK]),
    (8, &[ColourIndex::BLUE, ColourIndex::BROWN]),
    (10, &[ColourIndex::RED, ColourIndex::DARK_GREY]),
    (12, &[ColourIndex::PURPLE, ColourIndex::ORANGE]),
    (15, &[ColourIndex::MEDIUM_GREY, ColourIndex::LIGHT_BLUE]),
    (16, &[ColourIndex::GREEN, ColourIndex::LIGHT_RED]),
    (20, &[ColourIndex::CYAN, ColourIndex::LIGHT_GREY]),
    (24, &[ColourIndex::YELLOW, ColourIndex::LIGHT_GREEN]),
    (31, &[ColourIndex::WHITE]),
];

/// The seven pairs of different colours sharing a luma level. They blend cleanly when
/// dithered on PAL, so the quantiser may reward them (`docs/TAD.md` §1.1).
pub const SAME_LUMA_PAIRS: [(ColourIndex, ColourIndex); 7] = [
    (ColourIndex::BLUE, ColourIndex::BROWN),
    (ColourIndex::RED, ColourIndex::DARK_GREY),
    (ColourIndex::PURPLE, ColourIndex::ORANGE),
    (ColourIndex::MEDIUM_GREY, ColourIndex::LIGHT_BLUE),
    (ColourIndex::GREEN, ColourIndex::LIGHT_RED),
    (ColourIndex::CYAN, ColourIndex::LIGHT_GREY),
    (ColourIndex::YELLOW, ColourIndex::LIGHT_GREEN),
];

/// Luma level of `c` on the reference table's 0–31 scale.
pub const fn luma(c: ColourIndex) -> u8 {
    LUMA[c.as_usize()]
}

/// Index of `c`'s luma level among [`LUMA_LEVELS`] (0 = black … 8 = white): the unit of
/// `max_luma_diff`.
pub const fn level(c: ColourIndex) -> u8 {
    let l = luma(c);
    let mut i = 0;
    while i < LUMA_LEVELS.len() {
        if LUMA_LEVELS[i] == l {
            return i as u8;
        }
        i += 1;
    }
    unreachable!()
}

/// How many luma levels apart two colours are (0–8).
pub const fn level_diff(a: ColourIndex, b: ColourIndex) -> u8 {
    level(a).abs_diff(level(b))
}

/// `true` when `a` and `b` are different colours with the same luma level.
pub const fn is_same_luma_pair(a: ColourIndex, b: ColourIndex) -> bool {
    a.get() != b.get() && luma(a) == luma(b)
}

#[cfg(test)]
mod tests {
    use super::{LUMA_GROUPS, LUMA_LEVELS, SAME_LUMA_PAIRS, is_same_luma_pair, luma};
    use crate::colour::ColourIndex;

    #[test]
    fn luma_column_matches_reference_table() {
        // docs/reference/vic-ii-constraints.md §1, "Luma" column, index order.
        let want = [0, 31, 10, 20, 12, 16, 8, 24, 12, 8, 16, 10, 15, 24, 15, 20];
        let got: Vec<u8> = ColourIndex::ALL.iter().map(|&c| luma(c)).collect();
        assert_eq!(got, want);
    }

    #[test]
    fn groups_are_exactly_the_colours_at_each_level() {
        let levels: Vec<u8> = LUMA_GROUPS.iter().map(|(l, _)| *l).collect();
        assert_eq!(levels, LUMA_LEVELS);
        for (level, members) in LUMA_GROUPS {
            let derived: Vec<ColourIndex> = ColourIndex::ALL
                .iter()
                .copied()
                .filter(|&c| luma(c) == level)
                .collect();
            assert_eq!(derived, members, "level {level}");
        }
    }

    #[test]
    fn pairs_are_the_two_member_groups() {
        let from_groups: Vec<(ColourIndex, ColourIndex)> = LUMA_GROUPS
            .iter()
            .filter(|(_, m)| m.len() == 2)
            .map(|(_, m)| (m[0], m[1]))
            .collect();
        assert_eq!(from_groups, SAME_LUMA_PAIRS);
        for (a, b) in SAME_LUMA_PAIRS {
            assert!(is_same_luma_pair(a, b) && is_same_luma_pair(b, a));
        }
        assert!(!is_same_luma_pair(ColourIndex::BLUE, ColourIndex::BLUE));
        assert!(!is_same_luma_pair(ColourIndex::BLACK, ColourIndex::WHITE));
    }
}
