//! `cargo xtask blue-noise`: generate the solver's 64 × 64 blue-noise threshold tile by
//! void-and-cluster (Ulichney 1993) from a fixed seed, and write it where
//! `convert64-core` includes it (#104).
//!
//! The tile is committed; this task exists so it can be regenerated and checked. Every
//! step uses basic IEEE operations only (the Gaussian kernel's `exp` is a fixed Taylor
//! series), so the same seed gives the same bytes on every platform.

use std::path::Path;

/// Tile side.
pub(crate) const SIZE: usize = 64;
/// The seed the committed tile was generated with.
pub(crate) const SEED: u64 = 0x0C64_B10E;
/// Where the tile lives, relative to the repository root.
pub(crate) const TILE_PATH: &str = "crates/convert64-core/src/solve/blue_noise_64.bin";

/// Gaussian σ of the energy filter, in pixels (Ulichney's 1.5).
const SIGMA: f64 = 1.5;
/// Kernel radius beyond which weights are treated as zero.
const RADIUS: i64 = 9;

/// `e^(-x)` for `x ≥ 0` with basic operations only: a Taylor series for `e^(-x/64)`,
/// squared six times.
fn exp_neg(x: f64) -> f64 {
    let y = -x / 64.0;
    let mut term = 1.0;
    let mut sum = 1.0;
    for n in 1..=20 {
        term *= y / f64::from(n);
        sum += term;
    }
    for _ in 0..6 {
        sum *= sum;
    }
    sum
}

/// The toroidal Gaussian kernel as a `SIZE × SIZE` table of weights by offset.
fn kernel() -> Vec<f64> {
    let n = SIZE as i64;
    let mut k = vec![0.0; SIZE * SIZE];
    for dy in -RADIUS..=RADIUS {
        for dx in -RADIUS..=RADIUS {
            let w = exp_neg(((dx * dx + dy * dy) as f64) / (2.0 * SIGMA * SIGMA));
            let (x, y) = (dx.rem_euclid(n) as usize, dy.rem_euclid(n) as usize);
            k[y * SIZE + x] += w;
        }
    }
    k
}

struct Field {
    ones: Vec<bool>,
    energy: Vec<f64>,
    kernel: Vec<f64>,
}

impl Field {
    fn new(ones: Vec<bool>) -> Self {
        let mut f = Self {
            ones: vec![false; SIZE * SIZE],
            energy: vec![0.0; SIZE * SIZE],
            kernel: kernel(),
        };
        for (i, on) in ones.into_iter().enumerate() {
            if on {
                f.set(i, true);
            }
        }
        f
    }

    fn set(&mut self, i: usize, on: bool) {
        self.ones[i] = on;
        let (px, py) = (i % SIZE, i / SIZE);
        let s = if on { 1.0 } else { -1.0 };
        for y in 0..SIZE {
            for x in 0..SIZE {
                let k = self.kernel[((y + SIZE - py) % SIZE) * SIZE + (x + SIZE - px) % SIZE];
                self.energy[y * SIZE + x] += s * k;
            }
        }
    }

    /// The densest one (highest energy among ones); first in scan order on ties.
    fn tightest_cluster(&self, value: bool) -> usize {
        let mut best = usize::MAX;
        for i in 0..SIZE * SIZE {
            if self.ones[i] == value && (best == usize::MAX || self.energy[i] > self.energy[best]) {
                best = i;
            }
        }
        best
    }

    /// The emptiest zero (lowest energy among zeros); first in scan order on ties.
    fn largest_void(&self, value: bool) -> usize {
        let mut best = usize::MAX;
        for i in 0..SIZE * SIZE {
            if self.ones[i] != value && (best == usize::MAX || self.energy[i] < self.energy[best]) {
                best = i;
            }
        }
        best
    }
}

/// A seeded 64-bit linear congruential generator (Knuth's MMIX constants).
fn lcg(state: &mut u64) -> u64 {
    *state = state
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407);
    *state >> 33
}

/// The tile: rank 0–4095 of every position, row-major.
pub(crate) fn generate(seed: u64) -> Vec<u16> {
    let n = SIZE * SIZE;
    // Initial binary pattern: 10 % ones at seeded random positions.
    let mut state = seed;
    let mut initial = vec![false; n];
    let mut placed = 0;
    while placed < n / 10 {
        let i = (lcg(&mut state) % n as u64) as usize;
        if !initial[i] {
            initial[i] = true;
            placed += 1;
        }
    }
    // Relax it: move the tightest cluster into the largest void until stable.
    let mut f = Field::new(initial);
    loop {
        let c = f.tightest_cluster(true);
        f.set(c, false);
        let v = f.largest_void(true);
        if v == c {
            f.set(c, true);
            break;
        }
        f.set(v, true);
    }
    let prototype = f.ones.clone();
    let ones = prototype.iter().filter(|&&b| b).count();
    let mut rank = vec![0u16; n];
    // Phase 1: rank the prototype's ones by removing tightest clusters.
    let mut g = Field::new(prototype.clone());
    for r in (0..ones).rev() {
        let c = g.tightest_cluster(true);
        g.set(c, false);
        rank[c] = r as u16;
    }
    // Phase 2: fill the largest voids up to half.
    let mut g = Field::new(prototype);
    for r in ones..n / 2 {
        let v = g.largest_void(true);
        g.set(v, true);
        rank[v] = r as u16;
    }
    // Phase 3: the remaining zeros, densest-zero-cluster first (energy of the zeros).
    let zeros: Vec<bool> = g.ones.iter().map(|&b| !b).collect();
    let mut z = Field::new(zeros);
    for r in n / 2..n {
        let c = z.tightest_cluster(true);
        z.set(c, false);
        rank[c] = r as u16;
    }
    rank
}

/// Little-endian bytes of the tile.
pub(crate) fn to_bytes(tile: &[u16]) -> Vec<u8> {
    tile.iter().flat_map(|v| v.to_le_bytes()).collect()
}

/// `cargo xtask blue-noise`: regenerate the committed tile.
pub(crate) fn run(root: &Path) -> Result<(), String> {
    let path = root.join(TILE_PATH);
    let bytes = to_bytes(&generate(SEED));
    std::fs::write(&path, &bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    println!(
        "wrote {} ({} bytes, seed {SEED:#x})",
        path.display(),
        bytes.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exp_matches_the_library_closely() {
        for x in [0.0, 0.1, 1.0, 5.5, 20.0, 36.0] {
            let (a, b) = (exp_neg(x), (-x).exp());
            assert!((a - b).abs() <= b * 1e-12, "{x}: {a} vs {b}");
        }
    }

    #[test]
    fn the_committed_tile_regenerates_from_its_seed() {
        let committed = std::fs::read(crate::repo_root().join(TILE_PATH)).expect("tile");
        assert_eq!(committed, to_bytes(&generate(SEED)));
    }
}
