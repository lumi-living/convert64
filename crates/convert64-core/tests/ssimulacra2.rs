//! SSIMULACRA2 against png2amiga's own (ADR-013 §4): every pair below must score what
//! png2amiga's `ssimulacra2::compute` scores it, recorded in
//! `tests/data/ssimulacra2/reference.toml`.
//!
//! The pairs are built from committed fixtures, so nothing large is checked in:
//! png2c64's (archived, `fixtures/archive/png2c64/`) baselines against our renders, distortions of a baseline (blur, noise, a
//! crop, the identical image), and png2amiga's own C64 pairing, a prepared source against
//! a C64 render at the logical raster. To regenerate the reference scores, dump the
//! pairs and score them with the harness (`tests/data/ssimulacra2/README.md`):
//!
//! ```sh
//! CONVERT64_S2_DUMP=/tmp/s2 cargo test -p convert64-core --test ssimulacra2
//! ```
//!
//! The timing test (`--ignored`, release) prints what one 320 × 200 and one 160 × 200
//! score costs.

use convert64_core::palette::srgb8_to_linear;
use convert64_core::quality::ssimulacra2;
use convert64_core::solve::png2c64_ref::quantize::Mode;
use convert64_core::solve::png2c64_ref::{prepare, preprocess};
use std::path::PathBuf;

/// The largest difference allowed from png2amiga's score. The port is bit-exact against
/// png2amiga's scalar build; the margin is for a platform `pow` a last bit apart.
const TOLERANCE: f64 = 1e-4;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A PNG as 8-bit sRGB: width, height, pixels.
fn rgb8(path: &str) -> (usize, usize, Vec<[u8; 3]>) {
    let img = image::open(repo().join(path))
        .unwrap_or_else(|e| panic!("{path}: {e}"))
        .to_rgb8();
    let px = img.pixels().map(|p| p.0).collect();
    (img.width() as usize, img.height() as usize, px)
}

fn linear(px: &[[u8; 3]]) -> Vec<[f32; 3]> {
    px.iter().map(|c| c.map(srgb8_to_linear)).collect()
}

/// A 3 × 3 box blur in 8-bit, edges repeated, rounded to nearest.
fn box_blur(w: usize, h: usize, px: &[[u8; 3]]) -> Vec<[u8; 3]> {
    let mut out = Vec::with_capacity(px.len());
    for y in 0..h {
        for x in 0..w {
            let mut sum = [0u32; 3];
            for dy in [-1i64, 0, 1] {
                for dx in [-1i64, 0, 1] {
                    let sx = (x as i64 + dx).clamp(0, w as i64 - 1) as usize;
                    let sy = (y as i64 + dy).clamp(0, h as i64 - 1) as usize;
                    for c in 0..3 {
                        sum[c] += u32::from(px[sy * w + sx][c]);
                    }
                }
            }
            out.push(sum.map(|s| ((s + 4) / 9) as u8));
        }
    }
    out
}

/// Uniform noise of ±24 per channel from a fixed xorshift32 stream, clamped.
fn noise(px: &[[u8; 3]]) -> Vec<[u8; 3]> {
    let mut state = 0x2545_F491u32;
    px.iter()
        .map(|c| {
            c.map(|v| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                (i32::from(v) + (state % 49) as i32 - 24).clamp(0, 255) as u8
            })
        })
        .collect()
}

/// The `(x0, y0)`-based `cw × ch` crop of a `w`-wide image.
fn crop(
    w: usize,
    px: &[[u8; 3]],
    (x0, y0): (usize, usize),
    (cw, ch): (usize, usize),
) -> Vec<[u8; 3]> {
    (0..cw * ch)
        .map(|i| px[(y0 + i / cw) * w + x0 + i % cw])
        .collect()
}

/// Every other column: a 320-wide render at multicolor's 160 × 200 raster.
fn wide(px: &[[u8; 3]]) -> Vec<[u8; 3]> {
    px.iter().step_by(2).copied().collect()
}

/// One pair to score.
struct Pair {
    name: String,
    w: usize,
    h: usize,
    reference: Vec<[f32; 3]>,
    distorted: Vec<[f32; 3]>,
}

impl Pair {
    fn new(
        name: &str,
        w: usize,
        h: usize,
        reference: Vec<[f32; 3]>,
        distorted: Vec<[f32; 3]>,
    ) -> Self {
        assert_eq!(reference.len(), w * h, "{name}");
        assert_eq!(distorted.len(), w * h, "{name}");
        Self {
            name: name.to_owned(),
            w,
            h,
            reference,
            distorted,
        }
    }

    /// The pair file the harness reads.
    fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&(self.w as u32).to_le_bytes());
        out.extend_from_slice(&(self.h as u32).to_le_bytes());
        for px in self.reference.iter().chain(&self.distorted) {
            for v in px {
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
        out
    }
}

/// png2amiga's C64 pairing: `source` prepared for `mode` (scaled to the logical raster,
/// png2c64's preprocessing at the fixture's defaults) against png2c64's render of it at
/// the same raster.
fn prepared_pair(name: &str, fixture: &str, mode: Mode) -> Pair {
    let (sw, sh, src) = rgb8(&format!("fixtures/{fixture}/source.png"));
    let prepared = prepare(&src, (sw, sh), mode, &preprocess::Settings::default(), None);
    let (_, _, base) = rgb8(&format!("fixtures/archive/png2c64/{fixture}/baseline.png"));
    let render = match mode {
        Mode::Multicolor => wide(&base),
        Mode::Hires => base,
    };
    Pair::new(name, prepared.w, prepared.h, prepared.px, linear(&render))
}

fn pairs() -> Vec<Pair> {
    let mut out = Vec::new();
    for f in [
        "photo", "portrait", "title", "gradient", "lineart", "terrain",
    ] {
        let (w, h, base) = rgb8(&format!("fixtures/archive/png2c64/{f}/baseline.png"));
        let (_, _, ours) = rgb8(&format!("fixtures/{f}/expected_native.png"));
        out.push(Pair::new(
            &format!("{f}-baseline-vs-native"),
            w,
            h,
            linear(&base),
            linear(&ours),
        ));
    }
    let (w, h, photo) = rgb8("fixtures/archive/png2c64/photo/baseline.png");
    out.push(Pair::new(
        "photo-identical",
        w,
        h,
        linear(&photo),
        linear(&photo),
    ));
    out.push(Pair::new(
        "photo-blur3x3",
        w,
        h,
        linear(&photo),
        linear(&box_blur(w, h, &photo)),
    ));
    let (_, _, portrait) = rgb8("fixtures/archive/png2c64/portrait/baseline.png");
    out.push(Pair::new(
        "portrait-noise24",
        w,
        h,
        linear(&portrait),
        linear(&noise(&portrait)),
    ));
    let (_, _, fs) = rgb8("fixtures/portrait/expected_fs-unclamped.png");
    out.push(Pair::new(
        "portrait-baseline-vs-fs-unclamped",
        w,
        h,
        linear(&portrait),
        linear(&fs),
    ));
    let (_, _, native) = rgb8("fixtures/portrait/expected_native.png");
    out.push(Pair::new(
        "portrait-wide-baseline-vs-native",
        160,
        200,
        linear(&wide(&portrait)),
        linear(&wide(&native)),
    ));
    let (_, _, title) = rgb8("fixtures/archive/png2c64/title/baseline.png");
    let (_, _, title_ours) = rgb8("fixtures/title/expected.png");
    out.push(Pair::new(
        "title-baseline-vs-expected",
        w,
        h,
        linear(&title),
        linear(&title_ours),
    ));
    let (_, _, grad) = rgb8("fixtures/archive/png2c64/gradient/baseline.png");
    let (_, _, grad_ours) = rgb8("fixtures/gradient/expected_native.png");
    out.push(Pair::new(
        "gradient-crop-37x29",
        37,
        29,
        linear(&crop(w, &grad, (100, 50), (37, 29))),
        linear(&crop(w, &grad_ours, (100, 50), (37, 29))),
    ));
    out.push(prepared_pair(
        "photo-prepared-vs-baseline",
        "photo",
        Mode::Multicolor,
    ));
    out.push(prepared_pair(
        "portrait-prepared-vs-baseline",
        "portrait",
        Mode::Multicolor,
    ));
    out.push(prepared_pair(
        "lineart-prepared-vs-baseline",
        "lineart",
        Mode::Hires,
    ));
    out
}

/// png2amiga's scores by pair name.
fn reference_scores() -> Vec<(String, f64)> {
    let path = repo().join("crates/convert64-core/tests/data/ssimulacra2/reference.toml");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
    let doc: toml::Table = text.parse().expect("reference.toml parses");
    doc["pair"]
        .as_array()
        .expect("[[pair]] entries")
        .iter()
        .map(|p| {
            let name = p["name"].as_str().expect("name").to_owned();
            let bits = u32::from_str_radix(p["bits"].as_str().expect("bits"), 16).expect("hex");
            (name, f64::from(f32::from_bits(bits)))
        })
        .collect()
}

#[test]
fn ssimulacra2_matches_png2amiga() {
    let pairs = pairs();
    if let Some(dir) = std::env::var_os("CONVERT64_S2_DUMP") {
        let dir = PathBuf::from(dir);
        std::fs::create_dir_all(&dir).expect("dump dir");
        for p in &pairs {
            std::fs::write(dir.join(format!("{}.bin", p.name)), p.to_bytes()).expect("dump");
        }
    }
    let reference = reference_scores();
    let mut worst = 0.0f64;
    let mut failures = Vec::new();
    for p in &pairs {
        let ours = ssimulacra2(&p.reference, &p.distorted, p.w, p.h);
        let theirs = reference
            .iter()
            .find(|(n, _)| *n == p.name)
            .unwrap_or_else(|| panic!("{} has no reference score", p.name))
            .1;
        let diff = (ours - theirs).abs();
        worst = worst.max(diff);
        println!(
            "{:<36} ours {ours:>12.6} png2amiga {theirs:>12.6} diff {diff:.2e}",
            p.name
        );
        if diff > TOLERANCE {
            failures.push(p.name.clone());
        }
    }
    println!("max abs diff {worst:.2e}");
    assert_eq!(reference.len(), pairs.len(), "a reference score per pair");
    assert!(failures.is_empty(), "off png2amiga's score: {failures:?}");
}

#[test]
fn identical_pair_scores_100() {
    let (w, h, px) = rgb8("fixtures/archive/png2c64/portrait/baseline.png");
    let lin = linear(&px);
    assert_eq!(ssimulacra2(&lin, &lin, w, h), 100.0);
}

/// One score's cost, at both C64 rasters. Run in release:
/// `cargo test --release -p convert64-core --test ssimulacra2 -- --ignored --nocapture`.
#[test]
#[ignore = "timing; run in release with --nocapture"]
fn ssimulacra2_timing() {
    let (w, h, base) = rgb8("fixtures/archive/png2c64/photo/baseline.png");
    let (_, _, ours) = rgb8("fixtures/photo/expected_native.png");
    for (label, w, a, b) in [
        ("320 × 200", w, linear(&base), linear(&ours)),
        (
            "160 × 200",
            w / 2,
            linear(&wide(&base)),
            linear(&wide(&ours)),
        ),
    ] {
        let runs = 50;
        let mut times: Vec<f64> = (0..runs)
            .map(|_| {
                let t = std::time::Instant::now();
                std::hint::black_box(ssimulacra2(&a, &b, w, h));
                t.elapsed().as_secs_f64() * 1000.0
            })
            .collect();
        times.sort_by(f64::total_cmp);
        println!(
            "SSIMULACRA2 {label}: median {:.2} ms, min {:.2} ms",
            times[runs / 2],
            times[0]
        );
    }
}
