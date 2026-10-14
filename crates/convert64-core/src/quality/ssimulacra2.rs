//! SSIMULACRA2 (Jon Sneyers, Cloudinary 2022–2023): a full-reference perceptual image
//! metric. Scores run to 100: 30 is low quality, 50 fair, 70 high and 90 visually
//! lossless; heavy damage goes below 0.
//!
//! A port of png2amiga's `src/ssimulacra2.cpp` (MIT, commit `4a87776`, v1.111.2), which
//! ports Cloudinary's `ssimulacra2` (BSD-3-Clause), itself built on libjxl's
//! `tools/ssimulacra2.cc` (BSD-3-Clause). The ADR-013 reference is png2amiga, so its
//! choices are kept, including where it departs from libjxl:
//!
//! - **Blur.** A separable finite Gaussian (σ = 1.5, 11 taps, zero padding without
//!   renormalisation) instead of libjxl's recursive `FastGaussian`.
//! - **Cube root.** A bit-trick seed and two Newton steps (`fast_cbrt`, within 2e-6
//!   relative error) instead of `std::cbrt`.
//! - **Scales.** Up to six, stopping before an image smaller than 8 × 8; 320 × 200 and
//!   160 × 200 both get five. As in libjxl, the 108 weights are then read in order, so
//!   with fewer than six scales the Y and B channels use weights laid out for earlier
//!   slots. That is upstream's behaviour and the score's calibration includes it.
//!
//! **Arithmetic.** This follows png2amiga's scalar backend exactly: `f32` per pixel,
//! `f64` for the sums and the final score, taps in kernel order, and no fused
//! multiply-add (Rust never contracts). Built with `-ffp-contract=off` and its scalar
//! backend, png2amiga gives bit-identical scores (`tests/ssimulacra2.rs`). Its NEON and
//! AVX2 builds fuse the blur's multiply-adds and differ in the last digits. The two
//! constants png2amiga computes with the C library (`cbrtf`, `expf`) are stored as bits
//! here, so the score is the same on every platform, WASM included.

/// The bias of the opsin absorbance before the cube root (libjxl `kOpsinAbsorbanceBias`).
const BIAS: f32 = 0.003_793_073_3;
/// `cbrtf(BIAS)` as png2amiga computes it (Apple libm, GCC 15 and 16 and clang agree).
const BIAS_CBRT: f32 = f32::from_bits(0x3E1F_B275);

/// Half the blur kernel's width: 11 taps.
const BLUR_HALF: usize = 5;
/// The normalised σ = 1.5 Gaussian, `expf(-0.5 x² / σ²) / Σ` in `f32` as png2amiga
/// computes it, by tap.
const BLUR_KERNEL: [f32; 2 * BLUR_HALF + 1] = [
    f32::from_bits(0x3A86_CAB8),
    f32::from_bits(0x3BF8_FF02),
    f32::from_bits(0x3D13_758C),
    f32::from_bits(0x3DDF_F880),
    f32::from_bits(0x3E5A_1E20),
    f32::from_bits(0x3E88_32B1),
    f32::from_bits(0x3E5A_1E20),
    f32::from_bits(0x3DDF_F880),
    f32::from_bits(0x3D13_758C),
    f32::from_bits(0x3BF8_FF02),
    f32::from_bits(0x3A86_CAB8),
];

/// SSIM's stabilising constant (only C2; the metric drops the luminance term's C1).
const C2: f32 = 0.0009;

/// At most this many scales: 1:1 down to 1:32.
const SCALES: usize = 6;

/// The 108 calibrated weights (Cloudinary, April 2023), per channel (X, Y, B), per
/// scale, per norm (1, 4): SSIM, ringing, blurring.
const WEIGHTS: [f64; 108] = [
    0.0,
    0.000_737_660_670_740_658_6,
    0.0,
    0.0,
    0.000_779_348_168_286_730_9,
    0.0,
    0.0,
    0.000_437_115_573_010_737_9,
    0.0,
    1.104_172_642_665_734_6,
    0.000_662_848_341_292_71,
    0.000_152_316_327_837_187_52,
    0.0,
    0.001_640_643_745_659_975_4,
    0.0,
    1.842_245_552_053_929_8,
    11.441_172_603_757_666,
    0.0,
    0.000_798_910_943_601_516_3,
    0.000_176_816_438_078_653,
    0.0,
    1.878_759_497_954_638_7,
    10.949_069_906_051_42,
    0.0,
    0.000_728_934_699_150_807_2,
    0.967_793_708_062_683_3,
    0.0,
    0.000_140_034_242_854_358_84,
    0.998_176_697_785_496_7,
    0.000_319_497_559_344_350_53,
    0.000_455_099_211_379_206_3,
    0.0,
    0.0,
    0.001_364_876_616_324_339_8,
    0.0,
    0.0,
    0.0,
    0.0,
    0.0,
    7.466_890_328_078_848,
    0.0,
    17.445_833_984_131_262,
    0.000_623_560_163_404_146_6,
    0.0,
    0.0,
    6.683_678_146_179_332,
    0.000_377_244_079_796_112_96,
    1.027_889_937_768_264,
    225.205_153_008_492_74,
    0.0,
    0.0,
    19.213_238_186_143_016,
    0.001_140_152_458_661_836_1,
    0.001_237_755_635_509_985,
    176.393_175_984_506_94,
    0.0,
    0.0,
    24.433_009_998_704_76,
    0.285_208_026_121_177_57,
    0.000_448_543_692_383_340_8,
    0.0,
    0.0,
    0.0,
    34.779_063_444_837_72,
    44.835_625_328_877_896,
    0.0,
    0.0,
    0.0,
    0.0,
    0.0,
    0.0,
    0.0,
    0.0,
    0.000_868_055_657_329_169_8,
    0.0,
    0.0,
    0.0,
    0.0,
    0.0,
    0.000_531_319_187_435_874_7,
    0.0,
    0.000_165_338_141_613_791_12,
    0.0,
    0.0,
    0.0,
    0.0,
    0.0,
    0.000_417_917_180_325_133_6,
    0.001_729_082_823_472_283_3,
    0.0,
    0.002_082_700_584_663_643_7,
    0.0,
    0.0,
    8.826_982_764_996_862,
    23.192_433_439_989_26,
    0.0,
    95.108_049_881_108_6,
    0.986_397_803_440_068_2,
    0.983_438_279_246_535_3,
    0.001_228_640_504_827_849_3,
    171.266_725_589_730_7,
    0.980_785_887_243_537_9,
    0.0,
    0.0,
    0.0,
    0.000_513_006_458_899_067_9,
    0.0,
    0.000_108_540_578_584_115_37,
];

/// SSIMULACRA2 of `distorted` against `reference`, both row-major linear-light sRGB
/// (`0..=1` per channel, not clamped), `width × height` pixels.
///
/// The score is png2amiga's `ssimulacra2::compute`, which returns `f32`; it is widened
/// unchanged, so it compares bit for bit with png2amiga's. An empty image scores 0, as
/// upstream; an image narrower or lower than 8 pixels has no scales and scores 100.
///
/// # Panics
/// If either slice is shorter than `width × height`.
pub fn ssimulacra2(
    reference: &[[f32; 3]],
    distorted: &[[f32; 3]],
    width: usize,
    height: usize,
) -> f64 {
    let n = width * height;
    if n == 0 {
        return 0.0;
    }
    assert!(
        reference.len() >= n && distorted.len() >= n,
        "SSIMULACRA2 needs {width} × {height} pixels on both sides"
    );
    let mut lin1 = reference[..n].to_vec();
    let mut lin2 = distorted[..n].to_vec();
    let mut planes = Planes::new(n);
    let mut scratch = Scratch::new(n);
    let mut scales: Vec<ScaleScores> = Vec::with_capacity(SCALES);
    let (mut w, mut h) = (width, height);
    for scale in 0..SCALES {
        if w < 8 || h < 8 {
            break;
        }
        if scale > 0 {
            lin1 = downsample_2x(&lin1, w, h);
            lin2 = downsample_2x(&lin2, w, h);
            w = w.div_ceil(2);
            h = h.div_ceil(2);
        }
        let len = w * h;
        to_xyb_planes(&lin1, &mut planes.a, len);
        to_xyb_planes(&lin2, &mut planes.b, len);
        scales.push(one_scale(&planes, w, h, &mut scratch));
    }
    // Upstream returns the score as `f32`.
    f64::from(final_score(&scales) as f32)
}

/// XYB planes (X, Y, B) of both images at the current scale.
struct Planes {
    a: [Vec<f32>; 3],
    b: [Vec<f32>; 3],
}

impl Planes {
    fn new(n: usize) -> Self {
        let plane = || [vec![0.0; n], vec![0.0; n], vec![0.0; n]];
        Self {
            a: plane(),
            b: plane(),
        }
    }
}

/// One scale's working buffers, sized for scale 0 and reused by the smaller ones.
struct Scratch {
    mu1: Vec<f32>,
    mu2: Vec<f32>,
    s11: Vec<f32>,
    s22: Vec<f32>,
    s12: Vec<f32>,
    prod: Vec<f32>,
    tmp: Vec<f32>,
}

impl Scratch {
    fn new(n: usize) -> Self {
        Self {
            mu1: vec![0.0; n],
            mu2: vec![0.0; n],
            s11: vec![0.0; n],
            s22: vec![0.0; n],
            s12: vec![0.0; n],
            prod: vec![0.0; n],
            tmp: vec![0.0; n],
        }
    }
}

/// The cube root png2amiga uses: a bit-trick seed (Lavrentyev's constant) and two
/// Newton steps, within 2e-6 relative error on `0..=1.5` (upstream says 5e-7).
fn fast_cbrt(x: f32) -> f32 {
    let mut y = f32::from_bits(x.to_bits() / 3 + 0x2A51_0554);
    y = (2.0 * y + x / (y * y)) * (1.0 / 3.0);
    y = (2.0 * y + x / (y * y)) * (1.0 / 3.0);
    y
}

/// Linear sRGB to XYB (libjxl's opsin absorbance, cube root with bias), then range-shifted
/// to be roughly `0..=1` per channel (`MakePositiveXYB`).
fn linear_to_xyb([r, g, b]: [f32; 3]) -> [f32; 3] {
    let l = 0.30 * r + 0.622 * g + 0.078 * b;
    let m = 0.23 * r + 0.692 * g + 0.078 * b;
    let s = 0.243_422 * r + 0.204_162 * g + 0.552_416 * b;
    let lp = fast_cbrt(l + BIAS) - BIAS_CBRT;
    let mp = fast_cbrt(m + BIAS) - BIAS_CBRT;
    let sp = fast_cbrt(s + BIAS) - BIAS_CBRT;
    let x = (lp - mp) * 0.5;
    let y = (lp + mp) * 0.5;
    [x * 14.0 + 0.42, y + 0.01, (sp - y) + 0.55]
}

/// The first `n` pixels of `src` as XYB planes in `out`.
fn to_xyb_planes(src: &[[f32; 3]], out: &mut [Vec<f32>; 3], n: usize) {
    let [px, py, pb] = out;
    for (i, c) in src[..n].iter().enumerate() {
        let [x, y, b] = linear_to_xyb(*c);
        px[i] = x;
        py[i] = y;
        pb[i] = b;
    }
}

/// A 2 × 2 box average in linear light; an odd last column or row is repeated.
fn downsample_2x(src: &[[f32; 3]], sw: usize, sh: usize) -> Vec<[f32; 3]> {
    let (dw, dh) = (sw.div_ceil(2), sh.div_ceil(2));
    let mut dst = Vec::with_capacity(dw * dh);
    for y in 0..dh {
        for x in 0..dw {
            let mut acc = [0.0f32; 3];
            for iy in 0..2 {
                for ix in 0..2 {
                    let sx = (x * 2 + ix).min(sw - 1);
                    let sy = (y * 2 + iy).min(sh - 1);
                    let c = src[sy * sw + sx];
                    for k in 0..3 {
                        acc[k] += c[k];
                    }
                }
            }
            dst.push(acc.map(|v| v * 0.25));
        }
    }
    dst
}

/// The separable Gaussian of `input` (`w × h`) into `out`, through `tmp`: zero outside
/// the image, not renormalised. Each output sums its taps in kernel order from 0, as
/// png2amiga's scalar loops do.
fn gaussian_blur(input: &[f32], out: &mut [f32], tmp: &mut [f32], w: usize, h: usize) {
    let n = w * h;
    let (input, out, tmp) = (&input[..n], &mut out[..n], &mut tmp[..n]);
    // Horizontal.
    for (row, trow) in input.chunks_exact(w).zip(tmp.chunks_exact_mut(w)) {
        trow.fill(0.0);
        for (i, &k) in BLUR_KERNEL.iter().enumerate() {
            // Output x reads row[x + i - BLUR_HALF] when that is inside the row.
            let lo = BLUR_HALF.saturating_sub(i);
            let hi = (w + BLUR_HALF).saturating_sub(i).min(w);
            if lo >= hi {
                continue;
            }
            let src = &row[lo + i - BLUR_HALF..hi + i - BLUR_HALF];
            for (t, &v) in trow[lo..hi].iter_mut().zip(src) {
                *t += k * v;
            }
        }
    }
    // Vertical.
    for (y, orow) in out.chunks_exact_mut(w).enumerate() {
        orow.fill(0.0);
        for (i, &k) in BLUR_KERNEL.iter().enumerate() {
            let Some(sy) = (y + i).checked_sub(BLUR_HALF).filter(|&sy| sy < h) else {
                continue;
            };
            for (o, &v) in orow.iter_mut().zip(&tmp[sy * w..(sy + 1) * w]) {
                *o += k * v;
            }
        }
    }
}

/// `a × b` elementwise into `out`.
fn multiply(a: &[f32], b: &[f32], out: &mut [f32]) {
    for ((o, &x), &y) in out.iter_mut().zip(a).zip(b) {
        *o = x * y;
    }
}

/// `x⁴`, as `(x²)²`.
fn fourth(x: f64) -> f64 {
    let x = x * x;
    x * x
}

/// `std::max(x, 0.0)`: NaN stays NaN, unlike `f64::max`.
fn max0(x: f64) -> f64 {
    if x < 0.0 { 0.0 } else { x }
}

/// A map's 1-norm (mean) and 4-norm.
#[derive(Clone, Copy, Default)]
struct Norms {
    one: f64,
    four: f64,
}

impl Norms {
    fn new(sum1: f64, sum4: f64, n: usize) -> Self {
        let inv_n = 1.0 / n as f64;
        Self {
            one: inv_n * sum1,
            four: (inv_n * sum4).sqrt().sqrt(),
        }
    }
}

/// SSIM error (1 − SSIM without the luminance-contrast product's double gamma
/// correction) of one plane, as norms.
fn ssim_plane(mu1: &[f32], mu2: &[f32], s11: &[f32], s22: &[f32], s12: &[f32]) -> Norms {
    let n = mu1.len();
    let (mut sum1, mut sum4) = (0.0f64, 0.0f64);
    for i in 0..n {
        let (m1, m2) = (mu1[i], mu2[i]);
        let (m11, m22, m12) = (m1 * m1, m2 * m2, m1 * m2);
        let num_m = 1.0 - (m1 - m2) * (m1 - m2);
        let num_s = 2.0 * (s12[i] - m12) + C2;
        let denom_s = (s11[i] - m11) + (s22[i] - m22) + C2;
        let d = max0(1.0 - f64::from(num_m * num_s / denom_s));
        sum1 += d;
        sum4 += fourth(d);
    }
    Norms::new(sum1, sum4, n)
}

/// Ringing (edges in the distorted image where the reference is smooth) and blurring
/// (the reverse) of one plane, as norms.
fn edgediff_plane(img1: &[f32], mu1: &[f32], img2: &[f32], mu2: &[f32]) -> (Norms, Norms) {
    let n = img1.len();
    let (mut r1, mut r4, mut b1, mut b4) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    for i in 0..n {
        let d1 = (1.0 + f64::from(img2[i] - mu2[i]).abs())
            / (1.0 + f64::from(img1[i] - mu1[i]).abs())
            - 1.0;
        let artifact = max0(d1);
        r1 += artifact;
        r4 += fourth(artifact);
        let lost = max0(-d1);
        b1 += lost;
        b4 += fourth(lost);
    }
    (Norms::new(r1, r4, n), Norms::new(b1, b4, n))
}

/// One scale's norms per channel: SSIM, ringing and blurring.
#[derive(Clone, Copy, Default)]
struct ScaleScores {
    ssim: [Norms; 3],
    ringing: [Norms; 3],
    blurring: [Norms; 3],
}

fn one_scale(planes: &Planes, w: usize, h: usize, sc: &mut Scratch) -> ScaleScores {
    let n = w * h;
    let mut out = ScaleScores::default();
    for c in 0..3 {
        let a = &planes.a[c][..n];
        let b = &planes.b[c][..n];
        gaussian_blur(a, &mut sc.mu1, &mut sc.tmp, w, h);
        multiply(a, a, &mut sc.prod);
        gaussian_blur(&sc.prod, &mut sc.s11, &mut sc.tmp, w, h);
        gaussian_blur(b, &mut sc.mu2, &mut sc.tmp, w, h);
        multiply(b, b, &mut sc.prod);
        gaussian_blur(&sc.prod, &mut sc.s22, &mut sc.tmp, w, h);
        multiply(a, b, &mut sc.prod);
        gaussian_blur(&sc.prod, &mut sc.s12, &mut sc.tmp, w, h);
        out.ssim[c] = ssim_plane(
            &sc.mu1[..n],
            &sc.mu2[..n],
            &sc.s11[..n],
            &sc.s22[..n],
            &sc.s12[..n],
        );
        (out.ringing[c], out.blurring[c]) = edgediff_plane(a, &sc.mu1[..n], b, &sc.mu2[..n]);
    }
    out
}

/// The weighted sum of every norm, warped by the calibrated cubic and anchored at 100.
fn final_score(scales: &[ScaleScores]) -> f64 {
    let mut ssim = 0.0f64;
    let mut i = 0;
    for c in 0..3 {
        for s in scales {
            for (ssim_n, ring_n, blur_n) in [
                (s.ssim[c].one, s.ringing[c].one, s.blurring[c].one),
                (s.ssim[c].four, s.ringing[c].four, s.blurring[c].four),
            ] {
                ssim += WEIGHTS[i] * ssim_n.abs();
                ssim += WEIGHTS[i + 1] * ring_n.abs();
                ssim += WEIGHTS[i + 2] * blur_n.abs();
                i += 3;
            }
        }
    }
    ssim *= 0.956_238_261_683_484_4;
    ssim = 2.326_765_642_916_932 * ssim - 0.020_884_521_182_843_837 * ssim * ssim
        + 6.248_496_625_763_138e-5 * ssim * ssim * ssim;
    if ssim > 0.0 {
        100.0 - 10.0 * ssim.powf(0.627_633_646_783_138_7)
    } else {
        100.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_images_score_100() {
        let img: Vec<[f32; 3]> = (0..64 * 40)
            .map(|i| {
                let v = (i % 17) as f32 / 16.0;
                [v, 1.0 - v, v * v]
            })
            .collect();
        assert_eq!(ssimulacra2(&img, &img, 64, 40), 100.0);
    }

    #[test]
    fn tiny_and_empty_images() {
        let img = vec![[0.5f32; 3]; 7 * 7];
        let other = vec![[0.1f32; 3]; 7 * 7];
        // No scale fits: upstream's empty sum gives 100.
        assert_eq!(ssimulacra2(&img, &other, 7, 7), 100.0);
        assert_eq!(ssimulacra2(&[], &[], 0, 0), 0.0);
    }

    #[test]
    fn worse_distortion_scores_lower() {
        let (w, h) = (48, 32);
        let img: Vec<[f32; 3]> = (0..w * h)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                let v = if (x / 4 + y / 4) % 2 == 0 { 0.8 } else { 0.1 };
                [v, v * 0.5, 1.0 - v]
            })
            .collect();
        let shift = |d: f32| -> Vec<[f32; 3]> {
            img.iter()
                .enumerate()
                .map(|(i, c)| {
                    let s = if i % 3 == 0 { d } else { -d };
                    c.map(|v| (v + s).clamp(0.0, 1.0))
                })
                .collect()
        };
        let small = ssimulacra2(&img, &shift(0.02), w, h);
        let large = ssimulacra2(&img, &shift(0.2), w, h);
        assert!(small < 100.0 && large < small, "{small} {large}");
    }

    #[test]
    fn fast_cbrt_is_close() {
        for i in 0..=1500 {
            let x = i as f32 / 1000.0 + BIAS;
            let rel = (fast_cbrt(x) - x.cbrt()).abs() / x.cbrt();
            assert!(rel < 2e-6, "{x}: {rel}");
        }
    }

    #[test]
    fn stored_constants_match_their_formulas() {
        // The stored bits are png2amiga's; the platform's libm must agree to ±1 ulp.
        let close = |a: f32, b: f32| a.to_bits().abs_diff(b.to_bits()) <= 1;
        assert!(close(BIAS_CBRT, BIAS.cbrt()));
        let raw: Vec<f32> = (0..BLUR_KERNEL.len())
            .map(|i| {
                let x = i as f32 - BLUR_HALF as f32;
                (-0.5 * x * x / (1.5f32 * 1.5)).exp()
            })
            .collect();
        let sum: f32 = raw.iter().sum();
        for (k, r) in BLUR_KERNEL.iter().zip(&raw) {
            assert!(close(*k, r / sum), "{k} {}", r / sum);
        }
    }
}
