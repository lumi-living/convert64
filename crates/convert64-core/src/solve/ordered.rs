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
use crate::layer::{CustomMatrix, DitherKind, Orientation, PatternPhase};
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
    assert!(matches!(n, 1 | 2 | 4 | 8 | 16), "Bayer size {n}");
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

/// A threshold matrix: `w × h` ranks, row-major; the cell of rank `r` has threshold
/// `(r + offset) / levels`.
#[derive(Debug)]
struct Matrix {
    w: usize,
    h: usize,
    ranks: Vec<u16>,
    levels: f32,
    offset: f32,
    /// The thresholds, sorted (for [`shown_share`]).
    sorted: Vec<f32>,
}

impl Matrix {
    fn new(w: usize, h: usize, ranks: Vec<u16>, levels: f32, offset: f32) -> Self {
        assert_eq!(ranks.len(), w * h);
        let mut sorted: Vec<f32> = ranks
            .iter()
            .map(|&r| (f32::from(r) + offset) / levels)
            .collect();
        sorted.sort_by(f32::total_cmp);
        Self {
            w,
            h,
            ranks,
            levels,
            offset,
            sorted,
        }
    }

    /// Ranks `0..w·h` with the usual `(r + ½) / (w·h)` thresholds.
    fn ranked(w: usize, h: usize, ranks: &[u16]) -> Self {
        Self::new(w, h, ranks.to_vec(), (w * h) as f32, 0.5)
    }

    fn at(&self, x: i64, y: i64) -> f32 {
        let i =
            y.rem_euclid(self.h as i64) as usize * self.w + x.rem_euclid(self.w as i64) as usize;
        (f32::from(self.ranks[i]) + self.offset) / self.levels
    }
}

/// Ranks `0..n²` of an `n × n` tile ordered by `key` of the cell's offset from the tile
/// centre in doubled coordinates (`2x − (n − 1)`), ties by position: one dot that grows
/// in the key's shape.
fn dot_tile(n: usize, key: fn(i64, i64) -> i64) -> Matrix {
    let c = n as i64 - 1;
    let mut cells: Vec<(i64, usize)> = (0..n * n)
        .map(|i| {
            let (x, y) = ((i % n) as i64, (i / n) as i64);
            (key(2 * x - c, 2 * y - c), i)
        })
        .collect();
    cells.sort_unstable();
    let mut ranks = vec![0u16; n * n];
    for (r, (_, i)) in cells.into_iter().enumerate() {
        ranks[i] = r as u16;
    }
    Matrix::ranked(n, n, &ranks)
}

/// 45° lines: the cells of one diagonal (`line(x, y) mod 8`) share the low bits of their
/// rank, so the lines thicken one diagonal at a time.
fn diagonal_lines(line: fn(i64, i64) -> i64) -> Matrix {
    let ranks: Vec<u16> = (0..64)
        .map(|i| {
            let (x, y) = (i % 8, i / 8);
            (line(x, y).rem_euclid(8) * 8 + x) as u16
        })
        .collect();
    Matrix::ranked(8, 8, &ranks)
}

/// The transpose of a one-column matrix: lines become columns.
fn transposed(m: &Matrix) -> Matrix {
    Matrix::new(m.h, m.w, m.ranks.clone(), m.levels, m.offset)
}

/// png2c64's matrices (`src/dither.cpp`), as ranks.
fn png2c64_matrix(kind: DitherKind) -> Option<Matrix> {
    Some(match kind {
        DitherKind::Checker => Matrix::new(2, 2, vec![0, 1, 1, 0], 2.0, 0.5),
        DitherKind::H2x4 => Matrix::ranked(2, 4, &[0, 4, 6, 2, 1, 5, 7, 3]),
        DitherKind::Clustered4 => Matrix::ranked(
            4,
            4,
            &[12, 5, 6, 13, 4, 0, 1, 7, 11, 3, 2, 8, 15, 10, 9, 14],
        ),
        DitherKind::LineH2 => Matrix::ranked(1, 2, &[0, 1]),
        DitherKind::LineH4 => Matrix::ranked(1, 4, &[0, 1, 2, 3]),
        DitherKind::LineH8 => Matrix::ranked(1, 8, &[0, 4, 2, 6, 1, 5, 3, 7]),
        // −0.35, −0.15 / 0.15, 0.35 around ½.
        DitherKind::LineChecker => Matrix::new(2, 2, vec![3, 7, 13, 17], 20.0, 0.0),
        // png2c64 numbers the halftone cells 1–32, twice: (raw − ½) / 32.
        DitherKind::Halftone8 => Matrix::new(
            8,
            8,
            vec![
                12, 6, 7, 13, 16, 20, 21, 17, 5, 0, 2, 8, 27, 30, 28, 22, 4, 1, 3, 9, 26, 31, 29,
                23, 15, 11, 10, 14, 19, 25, 24, 18, 16, 20, 21, 17, 12, 6, 7, 13, 27, 30, 28, 22,
                5, 0, 2, 8, 26, 31, 29, 23, 4, 1, 3, 9, 19, 25, 24, 18, 15, 11, 10, 14,
            ],
            32.0,
            0.5,
        ),
        DitherKind::Diagonal8 => Matrix::ranked(
            8,
            8,
            &[
                24, 10, 12, 26, 35, 47, 49, 37, 8, 0, 2, 14, 45, 59, 61, 51, 22, 6, 4, 16, 43, 57,
                63, 53, 30, 20, 18, 28, 33, 41, 55, 39, 34, 46, 48, 36, 25, 11, 13, 27, 44, 58, 60,
                50, 9, 1, 3, 15, 42, 56, 62, 52, 23, 7, 5, 17, 32, 40, 54, 38, 31, 21, 19, 29,
            ],
        ),
        DitherKind::Spiral5 => Matrix::ranked(
            5,
            5,
            &[
                20, 21, 22, 23, 24, 19, 6, 7, 8, 9, 18, 5, 0, 1, 10, 17, 4, 3, 2, 11, 16, 15, 14,
                13, 12,
            ],
        ),
        DitherKind::Hex8 => Matrix::new(
            8,
            8,
            vec![
                3, 4, 2, 7, 1, 6, 0, 5, 6, 0, 5, 3, 4, 2, 7, 1, 2, 7, 1, 6, 0, 5, 3, 4, 5, 3, 4, 2,
                7, 1, 6, 0, 1, 6, 0, 5, 3, 4, 2, 7, 4, 2, 7, 1, 6, 0, 5, 3, 0, 5, 3, 4, 2, 7, 1, 6,
                7, 1, 6, 0, 5, 3, 4, 2,
            ],
            8.0,
            0.5,
        ),
        DitherKind::Hex5 => Matrix::new(
            5,
            5,
            vec![
                4, 3, 0, 1, 2, 0, 1, 2, 4, 3, 2, 4, 3, 0, 1, 3, 0, 1, 2, 4, 1, 2, 4, 3, 0,
            ],
            5.0,
            0.5,
        ),
        _ => return None,
    })
}

/// Every kind's matrix, by discriminant; `None` for analytical patterns, the blue-noise
/// tile, custom matrices, diffusion and no dither.
fn matrices() -> &'static [Option<Matrix>] {
    static TABLE: OnceLock<Vec<Option<Matrix>>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut table: Vec<Option<Matrix>> = (0..DitherKind::ALL.len()).map(|_| None).collect();
        let bayer_m = |n: usize| Matrix::ranked(n, n, &bayer(n));
        for kind in DitherKind::ALL {
            table[kind as usize] = match kind {
                DitherKind::Bayer2 | DitherKind::Bayer2x2Mc => Some(bayer_m(2)),
                DitherKind::Bayer4 => Some(bayer_m(4)),
                DitherKind::Bayer8 => Some(bayer_m(8)),
                DitherKind::Bayer16 => Some(bayer_m(16)),
                // One round dot per tile (Euclidean), a smaller round dot, a diamond
                // (Manhattan) and a star (along the axes first).
                DitherKind::Clustered8 => Some(dot_tile(8, |x, y| x * x + y * y)),
                DitherKind::Dot => Some(dot_tile(6, |x, y| x * x + y * y)),
                DitherKind::Diamond => Some(dot_tile(8, |x, y| x.abs() + y.abs())),
                DitherKind::Star => Some(dot_tile(8, |x, y| {
                    4 * x.abs().min(y.abs()) + x.abs().max(y.abs())
                })),
                DitherKind::DiagDown => Some(diagonal_lines(|x, y| x + y)),
                DitherKind::DiagUp => Some(diagonal_lines(|x, y| x - y)),
                DitherKind::LineV2 => png2c64_matrix(DitherKind::LineH2).map(|m| transposed(&m)),
                DitherKind::LineV4 => png2c64_matrix(DitherKind::LineH4).map(|m| transposed(&m)),
                DitherKind::LineV8 => png2c64_matrix(DitherKind::LineH8).map(|m| transposed(&m)),
                k => png2c64_matrix(k),
            };
        }
        table
    })
}

fn matrix(kind: DitherKind) -> Option<&'static Matrix> {
    matrices()[kind as usize].as_ref()
}

/// png2c64's analytical thresholds (`ordered_threshold` in `src/dither.cpp`), in
/// `0..=1`. Only basic IEEE operations (`%`, `sqrt`, `floor`), so every platform gets the
/// same bits.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "hash arithmetic on pattern coordinates"
)]
fn analytical(kind: DitherKind, x: i64, y: i64) -> Option<f32> {
    let (fx, fy) = (x as f32, y as f32);
    Some(match kind {
        DitherKind::Ign => {
            let v = 52.982_918_9 * (0.067_110_56 * fx + 0.005_837_15 * fy).rem_euclid(1.0);
            v.rem_euclid(1.0)
        }
        DitherKind::R2 => (fx * 0.754_877_7 + fy * 0.569_840_3 + 0.5).rem_euclid(1.0),
        DitherKind::WhiteNoise => {
            let mut s = (y as u32).wrapping_mul(65_537).wrapping_add(x as u32);
            s = (s ^ 61) ^ (s >> 16);
            s = s.wrapping_mul(9);
            s ^= s >> 4;
            s = s.wrapping_mul(0x27d4_eb2d);
            s ^= s >> 15;
            f32::from((s & 0xFFFF) as u16) / 65_536.0
        }
        DitherKind::Crosshatch => {
            let tri = |v: f32| 1.0 - (v.rem_euclid(8.0) / 8.0 * 2.0 - 1.0).abs();
            let d0 = tri(fy);
            let d1 = tri(fx);
            let d2 = tri((fx + fy) * 0.7071);
            let d3 = tri((fx - fy + 512.0) * 0.7071);
            d0.min(d0 * 0.5 + d1 * 0.5)
                .min(d0 * 0.3 + d1 * 0.3 + d2 * 0.4)
                .min(d0 * 0.25 + d1 * 0.25 + d2 * 0.25 + d3 * 0.25)
        }
        DitherKind::Radial => {
            let (dx, dy) = (fx - 160.0, fy - 100.0);
            let v = ((dx * dx + dy * dy).sqrt() * 0.15).rem_euclid(1.0);
            1.0 - (v * 2.0 - 1.0).abs()
        }
        DitherKind::ValueNoise => {
            let hash = |ix: i64, iy: i64| {
                let s = (ix as u32)
                    .wrapping_mul(374_761_393)
                    .wrapping_add((iy as u32).wrapping_mul(668_265_263))
                    .wrapping_add(1_013_904_223);
                let s = (s ^ (s >> 13)).wrapping_mul(1_274_126_177);
                f32::from((s & 0xFFFF) as u16) / 65_536.0
            };
            let (sx, sy) = (fx * 0.125, fy * 0.125);
            let (ix, iy) = (sx.floor(), sy.floor());
            let (tx, ty) = (sx - ix, sy - iy);
            let (tx, ty) = (tx * tx * (3.0 - 2.0 * tx), ty * ty * (3.0 - 2.0 * ty));
            let (ix, iy) = (ix as i64, iy as i64);
            hash(ix, iy) * (1.0 - tx) * (1.0 - ty)
                + hash(ix + 1, iy) * tx * (1.0 - ty)
                + hash(ix, iy + 1) * (1.0 - tx) * ty
                + hash(ix + 1, iy + 1) * tx * ty
        }
        _ => return None,
    })
}

/// A pattern position after `orientation`: the line variant holds a column's threshold
/// for `period` rows; the 2:1 variant covers two target rows with each pattern row.
const fn oriented(orientation: Orientation, x: i64, y: i64, period: i64) -> (i64, i64) {
    match orientation {
        Orientation::Auto => (x, y),
        Orientation::VerticalBiased => (x, y.div_euclid(period)),
        Orientation::TwoToOne => (x, y.div_euclid(2)),
    }
}

/// The threshold `0 ≤ t ≤ 1` of a threshold method (ordered, noise or effect) at pattern
/// position `(x, y)` (target pixels, already offset by the phase), or `None` for
/// diffusion and no-dither methods. [`DitherKind::Custom`] is Bayer 8 here; its own
/// matrix goes through [`custom_threshold`].
pub fn threshold(kind: DitherKind, orientation: Orientation, x: i64, y: i64) -> Option<f32> {
    if kind == DitherKind::Custom {
        return threshold(DitherKind::Bayer8, orientation, x, y);
    }
    if kind == DitherKind::BlueNoise {
        let n = BLUE_NOISE_SIZE as i64;
        let (px, py) = oriented(orientation, x, y, n);
        let i = py.rem_euclid(n) as usize * BLUE_NOISE_SIZE + px.rem_euclid(n) as usize;
        return Some((f32::from(blue_noise()[i]) + 0.5) / (n * n) as f32);
    }
    if let Some(m) = matrix(kind) {
        let (px, py) = oriented(orientation, x, y, m.h as i64);
        return Some(m.at(px, py));
    }
    let (px, py) = oriented(orientation, x, y, 8);
    analytical(kind, px, py)
}

/// The threshold of a user matrix at pattern position `(x, y)`.
pub fn custom_threshold(m: &CustomMatrix, orientation: Orientation, x: i64, y: i64) -> f32 {
    let n = m.size() as i64;
    let (px, py) = oriented(orientation, x, y, n);
    let r = m.rank(px.rem_euclid(n) as usize, py.rem_euclid(n) as usize);
    (f32::from(r) + 0.5) / (n * n) as f32
}

/// The share of a pattern's pixels that take the second colour at `level` when its
/// thresholds are pulled towards ½ by `strength` (as [`ordered_pick`] does): what the
/// eye sees of an ordered mix. Exact for matrices; noise, effects and custom matrices
/// count as continuous.
pub fn shown_share(level: f32, kind: DitherKind, strength: f32) -> f32 {
    let s = strength.clamp(0.0, 1.0);
    if s <= 0.0 {
        return if level > 0.5 { 1.0 } else { 0.0 };
    }
    match matrix(kind) {
        Some(m) => {
            let below = m
                .sorted
                .iter()
                .filter(|&&t| 0.5 + (t - 0.5) * s < level)
                .count();
            below as f32 / m.sorted.len() as f32
        }
        None => (0.5 + (level - 0.5) / s).clamp(0.0, 1.0),
    }
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
    fn every_threshold_method_has_a_varied_pattern_in_the_unit_interval() {
        for kind in DitherKind::ALL {
            let family = kind.family();
            let t0 = threshold(kind, Orientation::Auto, 0, 0);
            assert_eq!(t0.is_some(), family.is_threshold(), "{kind:?}");
            if !family.is_threshold() {
                continue;
            }
            let mut seen = std::collections::BTreeSet::new();
            for o in [
                Orientation::Auto,
                Orientation::VerticalBiased,
                Orientation::TwoToOne,
            ] {
                for y in -20..40 {
                    for x in -20..40 {
                        let t = threshold(kind, o, x, y).unwrap();
                        assert!((0.0..=1.0).contains(&t), "{kind:?} {o:?} ({x}, {y}): {t}");
                        seen.insert(t.to_bits());
                    }
                }
            }
            assert!(seen.len() >= 2, "{kind:?} is flat");
        }
    }

    #[test]
    fn matrices_rank_every_cell_once() {
        for kind in DitherKind::ALL {
            let Some(m) = matrix(kind) else { continue };
            let mut r = m.ranks.clone();
            r.sort_unstable();
            r.dedup();
            // Hex tilings and the checker repeat ranks by design; the rest are
            // permutations.
            if !matches!(
                kind,
                DitherKind::Checker | DitherKind::Hex5 | DitherKind::Hex8 | DitherKind::Halftone8
            ) {
                assert_eq!(r.len(), m.w * m.h, "{kind:?}");
            }
        }
        assert_eq!(matrix(DitherKind::Bayer16).unwrap().ranks.len(), 256);
    }

    #[test]
    fn png2c64_matrices_keep_png2c64_s_thresholds() {
        let t = |k, x, y| threshold(k, Orientation::Auto, x, y).unwrap();
        // checker: ∓0.25 around ½; line-checker: −0.35, −0.15, 0.15, 0.35.
        assert_eq!(
            (t(DitherKind::Checker, 0, 0), t(DitherKind::Checker, 1, 0)),
            (0.25, 0.75)
        );
        let lc = [(0, 0), (1, 0), (0, 1), (1, 1)].map(|(x, y)| t(DitherKind::LineChecker, x, y));
        for (got, want) in lc.iter().zip([0.15, 0.35, 0.65, 0.85]) {
            assert!((got - want).abs() < 1e-6, "{lc:?}");
        }
        // halftone8 cell (1, 1) is png2c64's 1: (1 − ½) / 32.
        assert_eq!(t(DitherKind::Halftone8, 1, 1), 0.5 / 32.0);
        // Lines: horizontal ones vary down, vertical ones across.
        assert_eq!(t(DitherKind::LineH4, 0, 1), t(DitherKind::LineH4, 3, 1));
        assert_eq!(t(DitherKind::LineV4, 1, 0), t(DitherKind::LineV4, 1, 3));
        assert_eq!(t(DitherKind::LineH4, 0, 2), t(DitherKind::LineV4, 2, 0));
        // 45° lines: the lowest thresholds lie on one diagonal.
        let low: Vec<(i64, i64)> = (0..64)
            .map(|i| (i % 8, i / 8))
            .filter(|&(x, y)| t(DitherKind::DiagDown, x, y) < 8.0 / 64.0)
            .collect();
        assert!(low.iter().all(|(x, y)| (x + y) % 8 == 0), "{low:?}");
        // bayer2x2-mc is png2c64's bayer2x2, the same matrix as bayer2.
        for i in 0..4 {
            assert_eq!(
                t(DitherKind::Bayer2x2Mc, i % 2, i / 2),
                t(DitherKind::Bayer2, i % 2, i / 2)
            );
        }
        // Analytical patterns follow png2c64's formulas.
        assert!((t(DitherKind::R2, 0, 0) - 0.5).abs() < 1e-6);
        assert_eq!(t(DitherKind::Radial, 160, 100), 0.0);
    }

    #[test]
    fn custom_matrices_rank_their_values() {
        let values: Vec<u16> = (0..64).map(|i| 1000 - i * 7).collect();
        let m = CustomMatrix::new(&values).unwrap();
        // Descending values: the last cell has the lowest threshold.
        assert_eq!(custom_threshold(&m, Orientation::Auto, 7, 7), 0.5 / 64.0);
        assert_eq!(custom_threshold(&m, Orientation::Auto, 0, 0), 63.5 / 64.0);
        assert_eq!(custom_threshold(&m, Orientation::Auto, 8, 8), 63.5 / 64.0);
        assert!(CustomMatrix::new(&[1, 2, 3]).is_err());
        assert_eq!(CustomMatrix::new(&[0; 256]).unwrap().size(), 16);
        // Without a matrix, custom is Bayer 8.
        assert_eq!(
            threshold(DitherKind::Custom, Orientation::Auto, 3, 5),
            threshold(DitherKind::Bayer8, Orientation::Auto, 3, 5)
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
    fn shown_share_counts_the_pattern_s_thresholds() {
        // Bayer 2 at 70 %: the five levels show exactly 0, 1, 2, 3 and 4 of 4 pixels.
        for (level, want) in [
            (0.0, 0.0),
            (0.25, 0.25),
            (0.5, 0.5),
            (0.75, 0.75),
            (1.0, 1.0),
        ] {
            assert_eq!(shown_share(level, DitherKind::Bayer2, 0.7), want, "{level}");
        }
        // The count agrees with thresholding every pattern position.
        for kind in [
            DitherKind::Bayer4,
            DitherKind::LineChecker,
            DitherKind::Halftone8,
        ] {
            for level in [0.1, 0.3, 0.45, 0.6, 0.9] {
                for s in [0.3, 0.7, 1.0] {
                    let n = (0..64)
                        .filter(|i| {
                            let t = threshold(kind, Orientation::Auto, i % 8, i / 8).unwrap();
                            level > 0.5 + (t - 0.5) * s
                        })
                        .count();
                    assert_eq!(
                        shown_share(level, kind, s),
                        n as f32 / 64.0,
                        "{kind:?} {level} {s}"
                    );
                }
            }
        }
        assert_eq!(shown_share(0.4, DitherKind::Bayer8, 0.0), 0.0);
        assert_eq!(shown_share(0.6, DitherKind::Ign, 0.0), 1.0);
        assert!((shown_share(0.3, DitherKind::Ign, 1.0) - 0.3).abs() < 1e-6);
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
