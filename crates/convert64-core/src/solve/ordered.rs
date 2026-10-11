//! Ordered and noise dithering (`docs/TAD.md` §3.1): per-pixel thresholds from Bayer
//! matrices or a blue-noise tile, scaled by the pixel's strength and anchored to the
//! screen or the layer (UX §6).
//!
//! A pixel chooses between the two colours of its cell's set nearest to it in its own
//! metric: with `t` its position along the segment from the nearest colour (`t = 0`) to
//! the second (`t = 1`), it takes the second colour when `t` exceeds the threshold. The
//! nearest colour always has `t ≤ ½`, and strength pulls thresholds towards ½, so at
//! strength 0 every pixel takes its nearest colour exactly. Thresholds are sampled in
//! target pixels, so on multicolor screens the pattern follows the 2:1 pixel grid.

use super::metric::Metric;
use crate::colour::ColourIndex;
use crate::layer::{DitherKind, Orientation, PatternPhase};
use std::sync::OnceLock;

/// Side of the blue-noise tile.
pub const BLUE_NOISE_SIZE: usize = 64;

/// The seed `cargo xtask blue-noise` generated the committed tile with.
pub const BLUE_NOISE_SEED: u64 = 0x0C64_B10E;

/// The committed 64 × 64 blue-noise ranks (void-and-cluster, `cargo xtask blue-noise`),
/// 16-bit little-endian, row-major.
static BLUE_NOISE_BYTES: &[u8; BLUE_NOISE_SIZE * BLUE_NOISE_SIZE * 2] =
    include_bytes!("blue_noise_64.bin");

/// The blue-noise ranks 0–4095.
pub fn blue_noise() -> &'static [u16] {
    static TILE: OnceLock<Vec<u16>> = OnceLock::new();
    TILE.get_or_init(|| {
        BLUE_NOISE_BYTES
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| u16::from_le_bytes(*b))
            .collect()
    })
}

/// The `n × n` Bayer index matrix (values `0..n²`), row-major, for `n` = 2, 4 or 8.
pub fn bayer(n: usize) -> Vec<u16> {
    assert!(matches!(n, 1 | 2 | 4 | 8), "Bayer size {n}");
    let mut m = vec![0u16];
    let mut size = 1;
    while size < n {
        let mut next = vec![0u16; 4 * size * size];
        for y in 0..size {
            for x in 0..size {
                let v = 4 * m[y * size + x];
                let s2 = 2 * size;
                next[y * s2 + x] = v;
                next[y * s2 + x + size] = v + 2;
                next[(y + size) * s2 + x] = v + 3;
                next[(y + size) * s2 + x + size] = v + 1;
            }
        }
        m = next;
        size *= 2;
    }
    m
}

fn bayer_table(n: usize) -> &'static [u16] {
    static B2: OnceLock<Vec<u16>> = OnceLock::new();
    static B4: OnceLock<Vec<u16>> = OnceLock::new();
    static B8: OnceLock<Vec<u16>> = OnceLock::new();
    match n {
        2 => B2.get_or_init(|| bayer(2)),
        4 => B4.get_or_init(|| bayer(4)),
        _ => B8.get_or_init(|| bayer(8)),
    }
}

/// The threshold `0 < t < 1` of an ordered or noise method at pattern position `(x, y)`
/// (target pixels, already offset by the phase), or `None` for diffusion and no-dither
/// methods.
pub fn threshold(kind: DitherKind, orientation: Orientation, x: i64, y: i64) -> Option<f32> {
    let (table, n): (&[u16], usize) = match kind {
        DitherKind::Bayer2 => (bayer_table(2), 2),
        DitherKind::Bayer4 => (bayer_table(4), 4),
        DitherKind::Bayer8 => (bayer_table(8), 8),
        DitherKind::BlueNoise => (blue_noise(), BLUE_NOISE_SIZE),
        _ => return None,
    };
    let ni = n as i64;
    let (px, py) = match orientation {
        Orientation::Auto => (x, y),
        // Lines: the column's threshold holds for n rows, then the row phase steps.
        Orientation::VerticalBiased => (x, y.div_euclid(ni)),
        // Square on a 2:1 screen: every matrix row covers two target rows.
        Orientation::TwoToOne => (x, y.div_euclid(2)),
    };
    let i = py.rem_euclid(ni) as usize * n + px.rem_euclid(ni) as usize;
    Some((f32::from(table[i]) + 0.5) / (n * n) as f32)
}

/// A pixel's position in its pattern: target pixel `(x, y)` for a screen-locked
/// pattern, or relative to `layer_origin` (the layer's top-left corner in whole target
/// pixels) for a layer-locked one, so the pattern moves with the layer.
pub const fn pattern_position(
    x: i64,
    y: i64,
    phase: PatternPhase,
    layer_origin: (i64, i64),
) -> (i64, i64) {
    match phase {
        PatternPhase::Screen => (x, y),
        PatternPhase::Layer => (x - layer_origin.0, y - layer_origin.1),
    }
}

/// The colour of `set` nearest to `x` (coordinates in `metric`'s space); `colours` are
/// the 16 palette colours in that space. Ties go to the lower index.
pub fn nearest(
    metric: Metric,
    x: [f32; 3],
    set: &[ColourIndex],
    colours: &[[f32; 3]; 16],
) -> ColourIndex {
    nearest_two(metric, x, set, colours).0
}

/// The nearest and second-nearest colours of `set` to `x`; the second is the first
/// again when the set has one colour.
pub(crate) fn nearest_two(
    metric: Metric,
    x: [f32; 3],
    set: &[ColourIndex],
    colours: &[[f32; 3]; 16],
) -> (ColourIndex, ColourIndex) {
    let mut best = (f32::INFINITY, set[0]);
    let mut second = (f32::INFINITY, set[0]);
    for &c in set {
        let d = metric.distance(x, colours[c.as_usize()]);
        if d < best.0 {
            second = best;
            best = (d, c);
        } else if d < second.0 {
            second = (d, c);
        }
    }
    if second.0.is_infinite() {
        second = best;
    }
    (best.1, second.1)
}

/// The colour an ordered pixel takes: between its nearest and second-nearest colour of
/// `set`, by threshold `thr` pulled towards ½ by `1 − strength`.
pub fn ordered_pick(
    metric: Metric,
    x: [f32; 3],
    set: &[ColourIndex],
    colours: &[[f32; 3]; 16],
    thr: f32,
    strength: f32,
) -> ColourIndex {
    let (c1, c2) = nearest_two(metric, x, set, colours);
    if c1 == c2 || strength <= 0.0 {
        return c1;
    }
    let (a, b) = (colours[c1.as_usize()], colours[c2.as_usize()]);
    let seg = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let len2 = metric.dot(seg, seg);
    if len2 <= 0.0 {
        return c1;
    }
    let t = metric.dot([x[0] - a[0], x[1] - a[1], x[2] - a[2]], seg) / len2;
    let thr = 0.5 + (thr - 0.5) * strength.min(1.0);
    if t > thr { c2 } else { c1 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::palette::Palette;
    use crate::solve::MetricTables;
    use crate::solve::metric::Space;

    #[test]
    fn bayer_matrices_are_permutations() {
        for n in [2, 4, 8] {
            let mut m = bayer(n);
            assert_eq!(m.len(), n * n);
            m.sort_unstable();
            assert!(
                m.iter().enumerate().all(|(i, &v)| usize::from(v) == i),
                "{n}"
            );
        }
        assert_eq!(bayer(2), vec![0, 2, 3, 1]);
    }

    #[test]
    fn blue_noise_tile_is_a_permutation() {
        let mut t = blue_noise().to_vec();
        assert_eq!(t.len(), 4096);
        t.sort_unstable();
        assert!(t.iter().enumerate().all(|(i, &v)| usize::from(v) == i));
    }

    #[test]
    fn thresholds_are_in_the_open_unit_interval_and_follow_orientation() {
        for kind in [
            DitherKind::Bayer2,
            DitherKind::Bayer4,
            DitherKind::Bayer8,
            DitherKind::BlueNoise,
        ] {
            for o in [
                Orientation::Auto,
                Orientation::VerticalBiased,
                Orientation::TwoToOne,
            ] {
                for (x, y) in [(0, 0), (3, 5), (-7, 2), (100, -33)] {
                    let t = threshold(kind, o, x, y).unwrap();
                    assert!(t > 0.0 && t < 1.0);
                }
            }
        }
        assert_eq!(
            threshold(DitherKind::FloydSteinberg, Orientation::Auto, 0, 0),
            None
        );
        // Line variant: the same column keeps its threshold for n rows.
        let a = threshold(DitherKind::Bayer4, Orientation::VerticalBiased, 1, 0);
        assert_eq!(
            a,
            threshold(DitherKind::Bayer4, Orientation::VerticalBiased, 1, 3)
        );
        // 2:1 variant: rows come in pairs.
        assert_eq!(
            threshold(DitherKind::Bayer8, Orientation::TwoToOne, 3, 4),
            threshold(DitherKind::Bayer8, Orientation::TwoToOne, 3, 5)
        );
    }

    #[test]
    fn layer_locked_patterns_move_with_the_layer() {
        let at = |phase, origin| {
            let (px, py) = pattern_position(13, 7, phase, origin);
            threshold(DitherKind::Bayer4, Orientation::Auto, px, py)
        };
        assert_eq!(
            at(PatternPhase::Screen, (0, 0)),
            at(PatternPhase::Screen, (3, 1))
        );
        // Moving the layer by one pixel moves its pattern by one pixel.
        assert_eq!(at(PatternPhase::Layer, (1, 0)), {
            let (px, py) = pattern_position(12, 7, PatternPhase::Layer, (0, 0));
            threshold(DitherKind::Bayer4, Orientation::Auto, px, py)
        });
    }

    #[test]
    fn strength_zero_is_nearest_in_set() {
        let t = MetricTables::new(&Palette::pepto_pal());
        let colours = t.colours(Space::OkLab);
        let set = [
            ColourIndex::BLACK,
            ColourIndex::WHITE,
            ColourIndex::RED,
            ColourIndex::BLUE,
        ];
        for i in 0..200 {
            let v = i as f32 / 200.0;
            let x = Metric::OkLab.coords([v, v * 0.5, 1.0 - v]);
            for thr in [0.01, 0.3, 0.5, 0.99] {
                assert_eq!(
                    ordered_pick(Metric::OkLab, x, &set, colours, thr, 0.0),
                    nearest(Metric::OkLab, x, &set, colours)
                );
            }
        }
    }

    #[test]
    fn a_midway_colour_mixes_half_and_half() {
        // A colour exactly between black and white in OKLab: Bayer 8 gives 32 of 64 each.
        let t = MetricTables::new(&Palette::pepto_pal());
        let colours = t.colours(Space::OkLab);
        let (b, w) = (colours[0], colours[1]);
        let mid = [
            (b[0] + w[0]) / 2.0,
            (b[1] + w[1]) / 2.0,
            (b[2] + w[2]) / 2.0,
        ];
        let set = [ColourIndex::BLACK, ColourIndex::WHITE];
        let whites = (0..64)
            .filter(|i| {
                let thr = threshold(DitherKind::Bayer8, Orientation::Auto, i % 8, i / 8).unwrap();
                ordered_pick(Metric::OkLab, mid, &set, colours, thr, 1.0) == ColourIndex::WHITE
            })
            .count();
        assert!((31..=33).contains(&whites), "{whites}");
    }
}
