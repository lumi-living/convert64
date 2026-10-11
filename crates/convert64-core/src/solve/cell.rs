//! Cell colour-set search (`docs/TAD.md` §3.1): for one cell and one background, the
//! legal set of colours with the lowest weighted error, by brute force.
//!
//! Multicolor: the background plus 3 of the other 15 colours (455 sets). Hi-res: 2 of
//! 16 (120 pairs), no background. The search is generic over `k` free colours. Each
//! pixel's distance to every colour is precomputed per cell ([`super::input::Input`]),
//! and the search keeps a running per-pixel minimum through its `k` nested picks, so a
//! set costs one min-and-sum over the cell's pixels.
//!
//! Adding a colour never increases a set's error, so sets of exactly `k` colours (or as
//! many as the constraints leave) are enough: a smaller set is never better.

use super::metric::luma_pair_bias;
use crate::colour::{ColourIndex, ColourSet};

/// Most pixels in a cell (hi-res 8 × 8).
pub(crate) const MAX_PIXELS: usize = 64;

/// What a cell's set may and must contain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Constraints {
    /// Colours the set may use: the attribute map's allowed sets, steering allow/deny.
    /// A filter, never a request.
    pub candidates: ColourSet,
    /// Colours the set must contain (pins; a full pin requires the whole set).
    pub required: ColourSet,
}

impl Constraints {
    /// No restriction.
    pub const FREE: Self = Self {
        candidates: ColourSet::ALL,
        required: ColourSet::EMPTY,
    };
}

/// The best set for a cell.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CellChoice {
    /// The cell's own colours (the background excluded).
    pub set: ColourSet,
    /// Weighted error of the set (plus the same-luma bias).
    pub score: f32,
}

/// The cell's free colours to choose from and those already fixed, for background `bg`.
fn pools(
    bg: Option<ColourIndex>,
    k: usize,
    c: &Constraints,
) -> (Vec<ColourIndex>, Vec<ColourIndex>, usize) {
    let bg_set = bg.map_or(ColourSet::EMPTY, |b| ColourSet::EMPTY.with(b));
    let required: Vec<ColourIndex> = c.required.difference(bg_set).iter().take(k).collect();
    let free: Vec<ColourIndex> = c
        .candidates
        .difference(c.required)
        .difference(bg_set)
        .iter()
        .collect();
    let picks = (k - required.len()).min(free.len());
    (free, required, picks)
}

/// Calls `visit` for every set the search considers, in search order, with the per-pixel
/// minimum over the set (and the background). `dist` is the cell's `16 × p` table.
fn walk(
    dist: &[f32],
    p: usize,
    bg: Option<ColourIndex>,
    k: usize,
    c: &Constraints,
    visit: &mut impl FnMut(ColourSet, &[f32]),
) {
    let (free, required, picks) = pools(bg, k, c);
    let mut base = [f32::INFINITY; MAX_PIXELS];
    let mut base_set = ColourSet::EMPTY;
    for col in bg.into_iter().chain(required.iter().copied()) {
        let row = &dist[col.as_usize() * p..][..p];
        for (m, d) in base[..p].iter_mut().zip(row) {
            *m = m.min(*d);
        }
        if Some(col) != bg {
            base_set.insert(col);
        }
    }
    rec(dist, p, &free, 0, picks, base_set, &base, visit);
}

#[expect(clippy::too_many_arguments, reason = "recursion state")]
fn rec(
    dist: &[f32],
    p: usize,
    free: &[ColourIndex],
    from: usize,
    left: usize,
    set: ColourSet,
    mins: &[f32; MAX_PIXELS],
    visit: &mut impl FnMut(ColourSet, &[f32]),
) {
    if left == 0 {
        visit(set, &mins[..p]);
        return;
    }
    for i in from..=free.len() - left {
        let col = free[i];
        let row = &dist[col.as_usize() * p..][..p];
        let mut next = [0.0f32; MAX_PIXELS];
        for ((n, m), d) in next[..p].iter_mut().zip(&mins[..p]).zip(row) {
            *n = m.min(*d);
        }
        rec(dist, p, free, i + 1, left - 1, set.with(col), &next, visit);
    }
}

/// The lowest-error set for a cell: `k` free colours (3 multicolor, 2 hi-res) besides
/// background `bg` (multicolor only), within `constraints`, scored as the sum over
/// pixels of the weighted distance to the nearest colour in the set, plus `bias` per
/// same-luma pair in the set. Ties go to the first set in search order (lowest colour
/// indices), so the result is deterministic.
pub fn search(
    dist: &[f32],
    p: usize,
    bg: Option<ColourIndex>,
    k: usize,
    constraints: &Constraints,
    bias: f32,
) -> CellChoice {
    debug_assert_eq!(dist.len(), 16 * p);
    let mut best = CellChoice {
        set: ColourSet::EMPTY,
        score: f32::INFINITY,
    };
    walk(dist, p, bg, k, constraints, &mut |set, mins| {
        let full = bg.map_or(set, |b| set.with(b));
        let score = mins.iter().sum::<f32>() + luma_pair_bias(full, bias);
        if score < best.score {
            best = CellChoice { set, score };
        }
    });
    if best.score.is_infinite() {
        // No colour available at all (everything denied): keep the background only.
        best.score = 0.0;
    }
    best
}

/// How many sets [`search`] evaluates under these constraints.
pub fn count_sets(bg: Option<ColourIndex>, k: usize, constraints: &Constraints) -> usize {
    let mut n = 0;
    let dist = [0.0f32; 16];
    walk(&dist, 1, bg, k, constraints, &mut |_, _| n += 1);
    n
}

/// The weighted error of one given set (the background included when `bg` is set).
pub fn score_set(
    dist: &[f32],
    p: usize,
    bg: Option<ColourIndex>,
    set: ColourSet,
    bias: f32,
) -> f32 {
    let full = bg.map_or(set, |b| set.with(b));
    let mut total = 0.0;
    for k in 0..p {
        let mut m = f32::INFINITY;
        for c in full.iter() {
            m = m.min(dist[c.as_usize() * p + k]);
        }
        total += if m.is_finite() { m } else { 0.0 };
    }
    total + luma_pair_bias(full, bias)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(p: usize, mut f: impl FnMut(usize, usize) -> f32) -> Vec<f32> {
        (0..16 * p).map(|i| f(i / p, i % p)).collect()
    }

    #[test]
    fn combination_counts() {
        assert_eq!(
            count_sets(Some(ColourIndex::BLACK), 3, &Constraints::FREE),
            455
        );
        assert_eq!(count_sets(None, 2, &Constraints::FREE), 120);
        for bg in ColourIndex::ALL {
            assert_eq!(count_sets(Some(bg), 3, &Constraints::FREE), 455, "{bg}");
        }
        let pin = Constraints {
            candidates: ColourSet::ALL,
            required: ColourSet::EMPTY.with(ColourIndex::RED),
        };
        assert_eq!(
            count_sets(Some(ColourIndex::BLACK), 3, &pin),
            91,
            "C(14, 2)"
        );
        assert_eq!(count_sets(None, 2, &pin), 15);
    }

    #[test]
    fn finds_the_exact_colours_of_a_four_colour_cell() {
        // Pixel k exactly matches colour (k % 4) * 3 + 1; everything else is far.
        let p = 32;
        let want = |k: usize| (k % 4) * 3 + 1;
        let d = table(p, |c, k| if c == want(k) { 0.0 } else { 1.0 });
        let bg = ColourIndex::ALL[1];
        let best = search(&d, p, Some(bg), 3, &Constraints::FREE, 0.0);
        assert_eq!(best.score, 0.0);
        let expect: ColourSet = [4, 7, 10]
            .map(|i| ColourIndex::ALL[i])
            .into_iter()
            .collect();
        assert_eq!(best.set, expect);
        assert_eq!(score_set(&d, p, Some(bg), best.set, 0.0), 0.0);
    }

    #[test]
    fn constraints_are_honoured() {
        let p = 4;
        let d = table(p, |c, _| c as f32);
        // Low indices are best; deny 1–3, pin 9.
        let c = Constraints {
            candidates: ColourSet::ALL
                .difference([1, 2, 3].map(|i| ColourIndex::ALL[i]).into_iter().collect()),
            required: ColourSet::EMPTY.with(ColourIndex::ALL[9]),
        };
        let best = search(&d, p, None, 2, &c, 0.0);
        assert!(best.set.contains(ColourIndex::ALL[9]));
        assert!(best.set.contains(ColourIndex::ALL[0]));
        assert_eq!(best.set.len(), 2);
        // A full pin: the set is exactly the pinned colours.
        let pinned: ColourSet = [5, 6, 7].map(|i| ColourIndex::ALL[i]).into_iter().collect();
        let full = Constraints {
            candidates: pinned,
            required: pinned,
        };
        assert_eq!(
            search(&d, p, Some(ColourIndex::BLACK), 3, &full, 0.0).set,
            pinned
        );
    }

    #[test]
    fn the_brute_force_minimum_is_the_true_minimum() {
        // Pseudo-random table: compare search() with scoring every set directly.
        let p = 32;
        let mut s = 12_345u32;
        let d = table(p, |_, _| {
            s = s.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            (s >> 16) as f32 / 65_536.0
        });
        for bg in [ColourIndex::BLACK, ColourIndex::ALL[7]] {
            let best = search(&d, p, Some(bg), 3, &Constraints::FREE, 0.0);
            let mut brute = f32::INFINITY;
            for a in 0..16 {
                for b in a + 1..16 {
                    for c in b + 1..16 {
                        let set: ColourSet =
                            [a, b, c].map(|i| ColourIndex::ALL[i]).into_iter().collect();
                        if !set.contains(bg) {
                            brute = brute.min(score_set(&d, p, Some(bg), set, 0.0));
                        }
                    }
                }
            }
            assert!(
                (best.score - brute).abs() < 1e-5,
                "{} vs {brute}",
                best.score
            );
        }
    }

    #[test]
    fn same_luma_bias_shifts_the_choice() {
        // Blue (6) and Brown (9) share a luma level; make them the best pair.
        let p = 2;
        let d = table(p, |c, k| match (c, k) {
            (6, 0) | (9, 1) => 0.0,
            (0, _) => 0.05,
            _ => 1.0,
        });
        let plain = search(&d, p, None, 2, &Constraints::FREE, 0.0);
        assert!(plain.set.contains(ColourIndex::BLUE) && plain.set.contains(ColourIndex::BROWN));
        let penalised = search(&d, p, None, 2, &Constraints::FREE, 0.5);
        assert!(
            !(penalised.set.contains(ColourIndex::BLUE)
                && penalised.set.contains(ColourIndex::BROWN))
        );
    }
}
