//! The mip chain a large shrink samples (#235, ADR-012 amendment).
//!
//! png2c64's Mitchell bicubic reads four source pixels per output pixel along each axis,
//! however far it shrinks, so a source shrunk by much more than that aliases. Under
//! [`Downscale::Auto`](crate::layer::Downscale) a layer shrunk by more than
//! [`AUTO_THRESHOLD`] along an axis samples, with the same kernel, a box-reduced snapshot
//! of its source instead: **level** `L = ⌈log2(shrink / 2)⌉` on that axis, the source
//! halved `L` times, so what is left for the kernel is a shrink of at most 2.
//!
//! Levels are kept per layer axis (a rip-map, as ADR-011's #198 amendment §4): `(lx, ly)`
//! is the source halved `lx` times across and `ly` times down. Each is built once, when a
//! composite first needs it, and kept for as long as the source lives: the colour's in the
//! [`SourceImage`](crate::layer::SourceImage) itself (so every compositor and every layer
//! sharing the pixels shares it), a mask's with its layer's cache. A scale, rotation or
//! nudge reuses what is there; nothing is saved in a project, and the original is never
//! replaced.
//!
//! **Canonical construction.** A level's values must not depend on the order in which
//! levels were asked for (a drag through two scales would otherwise give a different
//! picture from opening the project at the second). Level `(lx, ly)` is therefore always
//! made the same way: the diagonal `(k, k)` by 2 × 2 box reductions of `(k − 1, k − 1)`
//! up to `k = min(lx, ly)`, then 1-D halvings along the remaining axis. A request starts
//! from the furthest level on that path already built. Only levels `0` and `≥ 2` are ever
//! sampled (`L` jumps from 0 to 2 at the threshold), so a level with an axis at 1 is an
//! intermediate and is not kept: `(1, 1)` is a quarter of the source.
//!
//! A reduction averages in linear light, on premultiplied colour (or a mask's values); an
//! odd last row or column is averaged with itself, the edge clamp.

use super::resample::Px;
use std::sync::{Arc, Mutex, PoisonError};

/// The shrink (source pixels per output pixel, per axis) above which
/// [`Downscale::Auto`](crate::layer::Downscale) samples a mip level. Up to it, png2c64's
/// four taps see every source pixel the output pixel covers, and every source up to
/// 1280 px wide fitted to the 320 grid is sampled as png2c64 samples it.
pub(crate) const AUTO_THRESHOLD: f64 = 4.0;

/// The level an axis shrunk by `shrink` samples, for a source axis of `len` pixels: 0 up
/// to [`AUTO_THRESHOLD`], else the least `L` with `shrink / 2^L ≤ 2`, at most the level
/// one pixel long.
pub(crate) fn level_for(shrink: f64, len: u32) -> u32 {
    if shrink.is_nan() || shrink <= AUTO_THRESHOLD {
        return 0;
    }
    // ⌈log2(len)⌉: the level at which the axis is one pixel.
    let top = u32::BITS - len.max(1).saturating_sub(1).leading_zeros();
    let mut l = 0;
    // Exact in f64 (powers of two), unlike a platform log2.
    while l < top && f64::from(1u32 << l) * 2.0 < shrink {
        l += 1;
    }
    l
}

/// The length of an axis of `len` pixels at level `l`: halved `l` times, rounding up.
pub(crate) fn level_len(len: u32, l: u32) -> u32 {
    (0..l).fold(len, |n, _| n.div_ceil(2))
}

/// One level of a mip chain: its pixels, row-major, and its size.
#[derive(Debug)]
pub(crate) struct MipLevel<T> {
    pub px: Vec<T>,
    pub size: (u32, u32),
}

/// A lazily built rip-map: the levels built so far, by `(lx, ly)`.
///
/// Cloning gives an empty cache, and any two compare equal: a cache is derived data, not
/// part of the value that holds it.
pub(crate) struct MipCache<T> {
    levels: Mutex<Vec<((u32, u32), Arc<MipLevel<T>>)>>,
}

impl<T> Default for MipCache<T> {
    fn default() -> Self {
        Self {
            levels: Mutex::new(Vec::new()),
        }
    }
}

impl<T> Clone for MipCache<T> {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl<T> PartialEq for MipCache<T> {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl<T> std::fmt::Debug for MipCache<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let levels = self.levels.lock().unwrap_or_else(PoisonError::into_inner);
        let keys: Vec<(u32, u32)> = levels.iter().map(|(k, _)| *k).collect();
        f.debug_struct("MipCache").field("levels", &keys).finish()
    }
}

/// The levels on the canonical path to `(lx, ly)`, from `(0, 0)` (excluded) to it.
fn path(lx: u32, ly: u32) -> Vec<(u32, u32)> {
    let m = lx.min(ly);
    let mut p: Vec<(u32, u32)> = (1..=m).map(|k| (k, k)).collect();
    p.extend((m + 1..=lx).map(|x| (x, m)));
    p.extend((m + 1..=ly).map(|y| (m, y)));
    p
}

/// Whether a level is kept once built: one with an axis at 1 is never sampled.
const fn kept((lx, ly): (u32, u32)) -> bool {
    lx != 1 && ly != 1
}

impl<T: Px> MipCache<T> {
    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<((u32, u32), Arc<MipLevel<T>>)>> {
        self.levels.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The levels built so far.
    #[cfg(test)]
    pub fn built(&self) -> Vec<(u32, u32)> {
        self.lock().iter().map(|(k, _)| *k).collect()
    }

    /// Level `(lx, ly)` (not `(0, 0)`) of the `w × h` image whose pixel `i` is `base(i)`,
    /// and how many reductions it took to make it (0 when it was already built).
    ///
    /// The lock is not held while building: a parallel build may run another layer's
    /// request on the same thread, and two layers can share a source. Two requests racing
    /// for the same level build it twice, with the same values; the first one kept wins.
    pub fn level(
        &self,
        (w, h): (u32, u32),
        base: impl Fn(usize) -> T + Sync,
        (lx, ly): (u32, u32),
    ) -> (Arc<MipLevel<T>>, u32) {
        debug_assert!((lx, ly) != (0, 0), "level 0 is the source itself");
        let path = path(lx, ly);
        let start = {
            let levels = self.lock();
            path.iter()
                .enumerate()
                .rev()
                .find_map(|(i, k)| {
                    levels
                        .iter()
                        .find(|(l, _)| l == k)
                        .map(|(_, v)| (i + 1, Arc::clone(v)))
                })
        };
        if let Some((i, level)) = &start
            && *i == path.len()
        {
            return (Arc::clone(level), 0);
        }
        let (from, mut cur) = match start {
            Some((i, level)) => (i, Some(level)),
            None => (0, None),
        };
        let mut prev = if from == 0 { (0, 0) } else { path[from - 1] };
        let mut made = Vec::new();
        for &next in &path[from..] {
            let level = match &cur {
                None => Arc::new(reduce(&base, (w, h), prev, next)),
                Some(c) => {
                    let px = &c.px;
                    Arc::new(reduce(&|i| px[i], c.size, prev, next))
                }
            };
            if kept(next) || next == (lx, ly) {
                made.push((next, Arc::clone(&level)));
            }
            cur = Some(level);
            prev = next;
        }
        let built = u32::try_from(path.len() - from).expect("at most 64 levels");
        let mut levels = self.lock();
        for (k, v) in made {
            if !levels.iter().any(|(l, _)| *l == k) {
                levels.push((k, v));
            }
        }
        let (_, level) = levels
            .iter()
            .find(|(l, _)| *l == (lx, ly))
            .expect("inserted above");
        (Arc::clone(level), built)
    }
}

/// One step of the canonical path, `from` to `to`: a 2 × 2 box when both axes halve,
/// else a 1-D halving.
fn reduce<T: Px>(
    get: &(impl Fn(usize) -> T + Sync),
    (w, h): (u32, u32),
    from: (u32, u32),
    to: (u32, u32),
) -> MipLevel<T> {
    let (hx, hy) = (to.0 > from.0, to.1 > from.1);
    let (w, h) = (w as usize, h as usize);
    let (nw, nh) = (
        if hx { w.div_ceil(2) } else { w },
        if hy { h.div_ceil(2) } else { h },
    );
    let mut px = vec![T::ZERO; nw * nh];
    let mut rows: Vec<(usize, &mut [T])> = px.chunks_mut(nw).enumerate().collect();
    crate::par::for_each(&mut rows, |(y, row)| {
        let (r0, r1) = if hy {
            (2 * *y, (2 * *y + 1).min(h - 1))
        } else {
            (*y, *y)
        };
        let (a, b) = (r0 * w, r1 * w);
        for (x, o) in row.iter_mut().enumerate() {
            let (c0, c1) = if hx { (2 * x, (2 * x + 1).min(w - 1)) } else { (x, x) };
            *o = match (hx, hy) {
                // ((a + b) + (c + d)) / 4.
                (true, true) => {
                    let mut top = T::ZERO;
                    top.madd(1.0, get(a + c0));
                    top.madd(1.0, get(a + c1));
                    let mut bottom = T::ZERO;
                    bottom.madd(1.0, get(b + c0));
                    bottom.madd(1.0, get(b + c1));
                    top.madd(1.0, bottom);
                    top.scaled(0.25)
                }
                (true, false) | (false, true) => {
                    let (i, j) = if hx { (a + c0, a + c1) } else { (a + x, b + x) };
                    let mut s = T::ZERO;
                    s.madd(1.0, get(i));
                    s.madd(1.0, get(j));
                    s.scaled(0.5)
                }
                (false, false) => get(a + x),
            };
        }
    });
    let size = (
        u32::try_from(nw).expect("a level of a u32 size"),
        u32::try_from(nh).expect("a level of a u32 size"),
    );
    MipLevel { px, size }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_follow_the_rule() {
        // ≤ 4×: the original; above, the remaining shrink is at most 2.
        for (shrink, want) in [
            (1.0, 0),
            (3.2, 0),
            (4.0, 0),
            (4.01, 2),
            (8.0, 2),
            (8.5, 3),
            (12.8, 3),
            (16.0, 3),
            (25.6, 4),
        ] {
            assert_eq!(level_for(shrink, 4096), want, "{shrink}");
        }
        // 4096 → 320 samples the 512 px level.
        let l = level_for(4096.0 / 320.0, 4096);
        assert_eq!(level_len(4096, l), 512);
        // Never below one pixel.
        assert_eq!(level_for(1000.0, 5), 3);
        assert_eq!(level_len(5, 3), 1);
    }

    #[test]
    fn the_path_is_the_diagonal_then_one_axis() {
        assert_eq!(path(3, 3), vec![(1, 1), (2, 2), (3, 3)]);
        assert_eq!(path(4, 2), vec![(1, 1), (2, 2), (3, 2), (4, 2)]);
        assert_eq!(path(0, 2), vec![(0, 1), (0, 2)]);
    }

    fn ramp(w: u32, h: u32) -> Vec<f32> {
        (0..w * h)
            .map(|i| ((i % w) as f32 * 0.37 + (i / w) as f32 * 1.91).sin())
            .collect()
    }

    /// A level's values do not depend on which levels were built before it.
    #[test]
    fn a_level_is_the_same_whatever_was_built_first() {
        let (w, h) = (97, 61);
        let src = ramp(w, h);
        let get = |i: usize| src[i];
        let fresh = MipCache::<f32>::default();
        let (want, built) = fresh.level((w, h), get, (4, 3));
        assert_eq!(built, 4);
        let warm = MipCache::<f32>::default();
        for l in [(2, 2), (3, 3), (2, 0), (3, 2)] {
            warm.level((w, h), get, l);
        }
        let (got, built) = warm.level((w, h), get, (4, 3));
        assert_eq!(built, 1, "from (3, 3)");
        assert_eq!(got.size, (level_len(w, 4), level_len(h, 3)));
        assert!(
            want.px
                .iter()
                .zip(&got.px)
                .all(|(a, b)| a.to_bits() == b.to_bits())
        );
        // Asking again builds nothing; (1, 1) was not kept.
        assert_eq!(warm.level((w, h), get, (4, 3)).1, 0);
        assert!(!warm.built().contains(&(1, 1)));
    }

    #[test]
    fn a_level_keeps_flat_fields_and_averages_boxes() {
        let flat = MipCache::<[f32; 4]>::default();
        let (l, _) = flat.level((7, 5), |_| [0.5, 0.25, 1.0, 1.0], (2, 2));
        assert_eq!(l.size, (2, 2));
        assert!(l.px.iter().all(|p| *p == [0.5, 0.25, 1.0, 1.0]));
        let src: Vec<f32> = (0..16).map(|i| i as f32).collect();
        let c = MipCache::<f32>::default();
        let (l, _) = c.level((4, 4), |i| src[i], (2, 2));
        assert_eq!(l.px, vec![7.5]);
        let (l, _) = c.level((4, 4), |i| src[i], (2, 0));
        assert_eq!(l.px, vec![1.5, 5.5, 9.5, 13.5]);
    }
}
