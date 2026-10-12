//! Dithering, ported from png2c64's `dither.cpp`: every ordered matrix and analytical
//! threshold, the ordered pickers (`apply_ordered_matrix`, `apply_ordered_analytical`),
//! error diffusion with its incoming and outgoing clamps, `--adaptive` and serpentine
//! scanning, and Ostromoukhov's per-pixel scaling of Floyd–Steinberg.
//!
//! How png2c64's command line uses them (`main.cpp`): an ordered method only biases the
//! cell search through [`ordered_threshold`] (see [`super::quantize`]) and the pixels are
//! the search's own nearest colours; [`apply`] runs for the diffusion methods only. The
//! ordered pickers are ported because they are part of `dither::apply`, but the
//! reference pipeline ([`super::convert`]) does not call them, as png2c64 does not.

use super::colour::{Lab, dist2, linear_to_oklab};
use super::quantize::Screen;
use super::{Image, Rgb};

/// png2c64's dither methods (`dither::Method`), by their command-line names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Method {
    /// `none`.
    None,
    /// `bayer4`.
    Bayer4,
    /// `bayer8`.
    Bayer8,
    /// `checker`.
    Checker,
    /// `bayer2x2`.
    Bayer2x2,
    /// `h2x4`.
    H2x4,
    /// `clustered`.
    Clustered,
    /// `line2`.
    Line2,
    /// `line-checker`.
    LineChecker,
    /// `line4`.
    Line4,
    /// `line8`.
    Line8,
    /// `fs`.
    FloydSteinberg,
    /// `atkinson`.
    Atkinson,
    /// `sierra` (Sierra Lite).
    SierraLite,
    /// `fs-wide`.
    FsWide,
    /// `jarvis`.
    Jarvis,
    /// `line-fs`.
    LineFs,
    /// `halftone8`.
    Halftone8,
    /// `diagonal8`.
    Diagonal8,
    /// `spiral5`.
    Spiral5,
    /// `hex8`.
    Hex8,
    /// `hex5`.
    Hex5,
    /// `blue-noise` (interleaved gradient noise in a 64 × 64 tile).
    BlueNoise,
    /// `ign`.
    Ign,
    /// `r2`.
    R2,
    /// `white-noise`.
    WhiteNoise,
    /// `crosshatch`.
    Crosshatch,
    /// `radial`.
    Radial,
    /// `value-noise`.
    ValueNoise,
    /// `ostromoukhov`.
    Ostromoukhov,
}

impl Method {
    /// Every method, in png2c64's order.
    pub const ALL: [Self; 30] = [
        Self::None,
        Self::Bayer4,
        Self::Bayer8,
        Self::Checker,
        Self::Bayer2x2,
        Self::H2x4,
        Self::Clustered,
        Self::Line2,
        Self::LineChecker,
        Self::Line4,
        Self::Line8,
        Self::FloydSteinberg,
        Self::Atkinson,
        Self::SierraLite,
        Self::FsWide,
        Self::Jarvis,
        Self::LineFs,
        Self::Halftone8,
        Self::Diagonal8,
        Self::Spiral5,
        Self::Hex8,
        Self::Hex5,
        Self::BlueNoise,
        Self::Ign,
        Self::R2,
        Self::WhiteNoise,
        Self::Crosshatch,
        Self::Radial,
        Self::ValueNoise,
        Self::Ostromoukhov,
    ];

    /// The command-line name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Bayer4 => "bayer4",
            Self::Bayer8 => "bayer8",
            Self::Checker => "checker",
            Self::Bayer2x2 => "bayer2x2",
            Self::H2x4 => "h2x4",
            Self::Clustered => "clustered",
            Self::Line2 => "line2",
            Self::LineChecker => "line-checker",
            Self::Line4 => "line4",
            Self::Line8 => "line8",
            Self::FloydSteinberg => "fs",
            Self::Atkinson => "atkinson",
            Self::SierraLite => "sierra",
            Self::FsWide => "fs-wide",
            Self::Jarvis => "jarvis",
            Self::LineFs => "line-fs",
            Self::Halftone8 => "halftone8",
            Self::Diagonal8 => "diagonal8",
            Self::Spiral5 => "spiral5",
            Self::Hex8 => "hex8",
            Self::Hex5 => "hex5",
            Self::BlueNoise => "blue-noise",
            Self::Ign => "ign",
            Self::R2 => "r2",
            Self::WhiteNoise => "white-noise",
            Self::Crosshatch => "crosshatch",
            Self::Radial => "radial",
            Self::ValueNoise => "value-noise",
            Self::Ostromoukhov => "ostromoukhov",
        }
    }

    /// The method with command-line name `name`.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.name() == name)
    }

    /// `is_ordered`: `none`, the matrices and the analytical thresholds.
    pub const fn is_ordered(self) -> bool {
        !matches!(
            self,
            Self::FloydSteinberg
                | Self::Atkinson
                | Self::SierraLite
                | Self::FsWide
                | Self::Jarvis
                | Self::LineFs
                | Self::Ostromoukhov
        )
    }

    /// The diffusion kernel of an error-diffusion method (`(dx, dy, weight)`); `None`
    /// for the others (Ostromoukhov scales Floyd–Steinberg's per pixel).
    pub fn kernel(self) -> Option<&'static [(i64, i64, f32)]> {
        Some(match self {
            Self::FloydSteinberg | Self::Ostromoukhov => &FLOYD_STEINBERG,
            Self::Atkinson => &ATKINSON,
            Self::SierraLite => &SIERRA_LITE,
            Self::FsWide => &FS_WIDE,
            Self::Jarvis => &JARVIS,
            Self::LineFs => &LINE_FS,
            _ => return None,
        })
    }
}

/// png2c64's `dither::Settings`, with its real defaults (`strength` is 1.0 in the code;
/// its `--help` says 0.7).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    /// The method.
    pub method: Method,
    /// 0 = no dithering, 1 = full.
    pub strength: f32,
    /// Maximum diffused error per OKLab channel.
    pub error_clamp: f32,
    /// Alternate the scan direction per row.
    pub serpentine: bool,
    /// Contrast-adaptive diffusion, 0 = off, 1 = full.
    pub adaptive: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            method: Method::Checker,
            strength: 1.0,
            error_clamp: 0.1,
            serpentine: true,
            adaptive: 0.0,
        }
    }
}

const FLOYD_STEINBERG: [(i64, i64, f32); 4] = [
    (1, 0, 7.0 / 16.0),
    (-1, 1, 3.0 / 16.0),
    (0, 1, 5.0 / 16.0),
    (1, 1, 1.0 / 16.0),
];
const ATKINSON: [(i64, i64, f32); 6] = [
    (1, 0, 1.0 / 8.0),
    (2, 0, 1.0 / 8.0),
    (-1, 1, 1.0 / 8.0),
    (0, 1, 1.0 / 8.0),
    (1, 1, 1.0 / 8.0),
    (0, 2, 1.0 / 8.0),
];
const SIERRA_LITE: [(i64, i64, f32); 3] =
    [(1, 0, 2.0 / 4.0), (-1, 1, 1.0 / 4.0), (0, 1, 1.0 / 4.0)];
const FS_WIDE: [(i64, i64, f32); 4] = [
    (1, 0, 5.0 / 16.0),
    (-1, 1, 3.0 / 16.0),
    (0, 1, 6.0 / 16.0),
    (1, 1, 2.0 / 16.0),
];
const JARVIS: [(i64, i64, f32); 12] = [
    (1, 0, 7.0 / 48.0),
    (2, 0, 5.0 / 48.0),
    (-2, 1, 3.0 / 48.0),
    (-1, 1, 5.0 / 48.0),
    (0, 1, 7.0 / 48.0),
    (1, 1, 5.0 / 48.0),
    (2, 1, 3.0 / 48.0),
    (-2, 2, 1.0 / 48.0),
    (-1, 2, 3.0 / 48.0),
    (0, 2, 5.0 / 48.0),
    (1, 2, 3.0 / 48.0),
    (2, 2, 1.0 / 48.0),
];
const LINE_FS: [(i64, i64, f32); 4] = [
    (-1, 1, 2.0 / 8.0),
    (0, 1, 3.0 / 8.0),
    (1, 1, 2.0 / 8.0),
    (0, 2, 1.0 / 8.0),
];

/// A threshold matrix: `w × h` values in `-0.5..0.5`, row-major.
struct Matrix {
    w: usize,
    h: usize,
    v: Vec<f32>,
}

/// `(raw + offset) / n − 0.5` for each raw rank, as png2c64's `make_*` builders compute.
fn ranked(w: usize, h: usize, raw: &[i32], offset: f32, n: f32) -> Matrix {
    Matrix {
        w,
        h,
        v: raw.iter().map(|&r| (r as f32 + offset) / n - 0.5).collect(),
    }
}

/// `make_blue_noise64`: interleaved gradient noise tabulated over 64 × 64.
#[expect(clippy::excessive_precision, reason = "png2c64's literal")]
fn blue_noise64() -> Matrix {
    let mut v = Vec::with_capacity(64 * 64);
    for y in 0..64 {
        for x in 0..64 {
            let (fx, fy) = (x as f32, y as f32);
            let mut t = 52.982_918_9 * (0.067_110_56 * fx + 0.005_837_15 * fy);
            t -= t as i32 as f32;
            if t < 0.0 {
                t += 1.0;
            }
            t *= 52.982_918_9;
            t -= t as i32 as f32;
            if t < 0.0 {
                t += 1.0;
            }
            v.push(t - 0.5);
        }
    }
    Matrix { w: 64, h: 64, v }
}

/// The matrix of a matrix method, built as png2c64's `constexpr` builders build it.
fn matrix(method: Method) -> Option<Matrix> {
    Some(match method {
        Method::Bayer4 => ranked(
            4,
            4,
            &[0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5],
            0.5,
            16.0,
        ),
        Method::Bayer8 => ranked(
            8,
            8,
            &[
                0, 32, 8, 40, 2, 34, 10, 42, 48, 16, 56, 24, 50, 18, 58, 26, 12, 44, 4, 36, 14, 46,
                6, 38, 60, 28, 52, 20, 62, 30, 54, 22, 3, 35, 11, 43, 1, 33, 9, 41, 51, 19, 59, 27,
                49, 17, 57, 25, 15, 47, 7, 39, 13, 45, 5, 37, 63, 31, 55, 23, 61, 29, 53, 21,
            ],
            0.5,
            64.0,
        ),
        Method::Checker => Matrix {
            w: 2,
            h: 2,
            v: vec![-0.25, 0.25, 0.25, -0.25],
        },
        Method::Bayer2x2 => ranked(2, 2, &[0, 2, 3, 1], 0.5, 4.0),
        Method::H2x4 => ranked(2, 4, &[0, 4, 6, 2, 1, 5, 7, 3], 0.5, 8.0),
        Method::Clustered => ranked(
            4,
            4,
            &[12, 5, 6, 13, 4, 0, 1, 7, 11, 3, 2, 8, 15, 10, 9, 14],
            0.5,
            16.0,
        ),
        Method::Line2 => Matrix {
            w: 1,
            h: 2,
            v: vec![-0.25, 0.25],
        },
        Method::LineChecker => Matrix {
            w: 2,
            h: 2,
            v: vec![-0.35, -0.15, 0.15, 0.35],
        },
        Method::Line4 => Matrix {
            w: 1,
            h: 4,
            v: vec![-0.375, -0.125, 0.125, 0.375],
        },
        Method::Line8 => ranked(1, 8, &[0, 4, 2, 6, 1, 5, 3, 7], 0.5, 8.0),
        Method::Halftone8 => ranked(
            8,
            8,
            &[
                13, 7, 8, 14, 17, 21, 22, 18, 6, 1, 3, 9, 28, 31, 29, 23, 5, 2, 4, 10, 27, 32, 30,
                24, 16, 12, 11, 15, 20, 26, 25, 19, 17, 21, 22, 18, 13, 7, 8, 14, 28, 31, 29, 23,
                6, 1, 3, 9, 27, 32, 30, 24, 5, 2, 4, 10, 20, 26, 25, 19, 16, 12, 11, 15,
            ],
            -0.5,
            32.0,
        ),
        Method::Diagonal8 => ranked(
            8,
            8,
            &[
                24, 10, 12, 26, 35, 47, 49, 37, 8, 0, 2, 14, 45, 59, 61, 51, 22, 6, 4, 16, 43, 57,
                63, 53, 30, 20, 18, 28, 33, 41, 55, 39, 34, 46, 48, 36, 25, 11, 13, 27, 44, 58, 60,
                50, 9, 1, 3, 15, 42, 56, 62, 52, 23, 7, 5, 17, 32, 40, 54, 38, 31, 21, 19, 29,
            ],
            0.5,
            64.0,
        ),
        Method::Spiral5 => ranked(
            5,
            5,
            &[
                20, 21, 22, 23, 24, 19, 6, 7, 8, 9, 18, 5, 0, 1, 10, 17, 4, 3, 2, 11, 16, 15, 14,
                13, 12,
            ],
            0.5,
            25.0,
        ),
        Method::Hex8 => ranked(
            8,
            8,
            &[
                3, 4, 2, 7, 1, 6, 0, 5, 6, 0, 5, 3, 4, 2, 7, 1, 2, 7, 1, 6, 0, 5, 3, 4, 5, 3, 4, 2,
                7, 1, 6, 0, 1, 6, 0, 5, 3, 4, 2, 7, 4, 2, 7, 1, 6, 0, 5, 3, 0, 5, 3, 4, 2, 7, 1, 6,
                7, 1, 6, 0, 5, 3, 4, 2,
            ],
            0.5,
            8.0,
        ),
        Method::Hex5 => ranked(
            5,
            5,
            &[
                4, 3, 0, 1, 2, 0, 1, 2, 4, 3, 2, 4, 3, 0, 1, 3, 0, 1, 2, 4, 1, 2, 4, 3, 0,
            ],
            0.5,
            5.0,
        ),
        Method::BlueNoise => blue_noise64(),
        _ => return None,
    })
}

/// The matrices, built once.
fn matrices() -> &'static [Option<Matrix>; 30] {
    static M: std::sync::OnceLock<[Option<Matrix>; 30]> = std::sync::OnceLock::new();
    M.get_or_init(|| Method::ALL.map(matrix))
}

/// `value_noise`'s integer hash.
fn hash(ix: i32, iy: i32) -> f32 {
    let s = ix
        .wrapping_mul(374_761_393)
        .wrapping_add(iy.wrapping_mul(668_265_263))
        .wrapping_add(1_013_904_223) as u32;
    let s = (s ^ (s >> 13)).wrapping_mul(1_274_126_177);
    (s & 0xFFFF) as f32 / 65536.0
}

/// `ordered_threshold`: the raw threshold of an ordered method at `(x, y)`, in
/// `-0.5..0.5`; 0 for `none` and the diffusion methods.
#[expect(
    clippy::excessive_precision,
    clippy::approx_constant,
    reason = "png2c64's literals, including its truncated 0.7071 (not 1/√2)"
)]
pub fn ordered_threshold(method: Method, x: usize, y: usize) -> f32 {
    if let Some(m) = &matrices()[method as usize] {
        return m.v[(y % m.h) * m.w + x % m.w];
    }
    let (fx, fy) = (x as f32, y as f32);
    match method {
        Method::Ign => {
            let v = 52.982_918_9 * ((0.067_110_56 * fx + 0.005_837_15 * fy) % 1.0);
            v % 1.0 - 0.5
        }
        Method::R2 => {
            let v = (fx * 0.754_877_666_2 + fy * 0.569_840_291_0 + 0.5) % 1.0;
            v - 0.5
        }
        Method::WhiteNoise => {
            let mut seed = (y as u64 * 65537 + x as u64) as u32;
            seed = (seed ^ 61) ^ (seed >> 16);
            seed = seed.wrapping_mul(9);
            seed ^= seed >> 4;
            seed = seed.wrapping_mul(0x27d4_eb2d);
            seed ^= seed >> 15;
            (seed & 0xFFFF) as f32 / 65536.0 - 0.5
        }
        Method::Crosshatch => {
            let tri = |d: f32| 1.0 - (d * 2.0 - 1.0).abs();
            let d0 = tri((fy % 8.0) / 8.0);
            let d1 = tri((fx % 8.0) / 8.0);
            let d2 = tri((((fx + fy) * 0.7071) % 8.0) / 8.0);
            let d3 = tri((((fx - fy + 512.0) * 0.7071) % 8.0) / 8.0);
            let cands = [
                d0,
                d0 * 0.5 + d1 * 0.5,
                d0 * 0.3 + d1 * 0.3 + d2 * 0.4,
                d0 * 0.25 + d1 * 0.25 + d2 * 0.25 + d3 * 0.25,
            ];
            // `std::min({..})`: the first smallest.
            let t = cands
                .into_iter()
                .fold(cands[0], |m, v| if v < m { v } else { m });
            t - 0.5
        }
        Method::Radial => {
            let (fx, fy) = (fx - 160.0, fy - 100.0);
            let r = (fx * fx + fy * fy).sqrt();
            let v = (r * 0.15) % 1.0;
            (1.0 - (v * 2.0 - 1.0).abs()) - 0.5
        }
        Method::ValueNoise => {
            let (fx, fy) = (fx * 0.125, fy * 0.125);
            let (ix, iy) = (fx.floor() as i32, fy.floor() as i32);
            let mut tx = fx - ix as f32;
            let mut ty = fy - iy as f32;
            tx = tx * tx * (3.0 - 2.0 * tx);
            ty = ty * ty * (3.0 - 2.0 * ty);
            let (v00, v10) = (hash(ix, iy), hash(ix + 1, iy));
            let (v01, v11) = (hash(ix, iy + 1), hash(ix + 1, iy + 1));
            let v = v00 * (1.0 - tx) * (1.0 - ty)
                + v10 * tx * (1.0 - ty)
                + v01 * (1.0 - tx) * ty
                + v11 * tx * ty;
            v - 0.5
        }
        _ => 0.0,
    }
}

/// `find_nearest_oklab`: the first nearest slot and its colour.
fn nearest(p: Lab, slots: &[u8], pal: &[Lab; 16]) -> (u8, Lab) {
    let (mut best, mut idx, mut lab) = (f32::MAX, 0u8, [0.0; 3]);
    for (s, &c) in slots.iter().enumerate() {
        let cl = pal[usize::from(c)];
        let d = dist2(p, cl);
        if d < best {
            best = d;
            idx = s as u8;
            lab = cl;
        }
    }
    (idx, lab)
}

/// `find_nearest_pair_oklab`'s result: nearest and second-nearest slot, their squared
/// distances and the nearest colour (the second colour is never read).
struct Pair {
    a: u8,
    b: u8,
    da: f32,
    db: f32,
    lab_a: Lab,
}

/// `find_nearest_pair_oklab`.
fn nearest_pair(p: Lab, slots: &[u8], pal: &[Lab; 16]) -> Pair {
    let (mut best, mut second) = (f32::MAX, f32::MAX);
    let (mut bi, mut si) = (0u8, 0u8);
    let mut bl = [0.0f32; 3];
    for (s, &c) in slots.iter().enumerate() {
        let cl = pal[usize::from(c)];
        let d = dist2(p, cl);
        if d < best {
            second = best;
            si = bi;
            best = d;
            bi = s as u8;
            bl = cl;
        } else if d < second {
            second = d;
            si = s as u8;
        }
    }
    if slots.len() < 2 {
        si = bi;
        second = best;
    }
    Pair {
        a: bi,
        b: si,
        da: best,
        db: second,
        lab_a: bl,
    }
}

/// The blend fraction of the second colour: `√dA / (√dA + √dB)`, 0 when both are 0.
fn blend(p: &Pair) -> f32 {
    if p.da + p.db > 1e-12 {
        p.da.sqrt() / (p.da.sqrt() + p.db.sqrt())
    } else {
        0.0
    }
}

/// The cell and pixel slot of logical pixel `(x, y)` (`get_pixel_cell`).
fn cell_of(screen: &Screen, x: usize, y: usize) -> (usize, usize) {
    let (cw, ch) = screen.mode.cell();
    (
        (y / ch) * screen.mode.cells().0 + x / cw,
        (y % ch) * cw + x % cw,
    )
}

/// `apply_ordered_matrix` / `apply_ordered_analytical`: per pixel, the nearest and
/// second-nearest cell colours, the second shown where `threshold + 0.5 < t × strength`.
fn apply_ordered(
    image: &Image,
    screen: &mut Screen,
    method: Method,
    pal: &[Lab; 16],
    strength: f32,
) {
    let (w, h) = screen.mode.screen();
    for y in 0..h {
        for x in 0..w {
            let (c, p) = cell_of(screen, x, y);
            let pair = nearest_pair(
                linear_to_oklab(image.at(x, y)),
                &screen.cells[c].colors,
                pal,
            );
            let thr = ordered_threshold(method, x, y) + 0.5;
            screen.cells[c].px[p] = if thr < blend(&pair) * strength {
                pair.b
            } else {
                pair.a
            };
        }
    }
}

/// `compute_contrast_map`: mean absolute L difference to the 4-neighbours over 0.15,
/// capped at 1.
fn contrast_map(lab: &[Lab], w: usize, h: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let l = lab[y * w + x][0];
            let (mut grad, mut count) = (0.0f32, 0.0f32);
            if x > 0 {
                grad += (l - lab[y * w + x - 1][0]).abs();
                count += 1.0;
            }
            if x + 1 < w {
                grad += (l - lab[y * w + x + 1][0]).abs();
                count += 1.0;
            }
            if y > 0 {
                grad += (l - lab[(y - 1) * w + x][0]).abs();
                count += 1.0;
            }
            if y + 1 < h {
                grad += (l - lab[(y + 1) * w + x][0]).abs();
                count += 1.0;
            }
            grad /= count;
            out[y * w + x] = (grad / 0.15).min(1.0);
        }
    }
    out
}

/// `apply_error_diffusion` and `apply_ostromoukhov`: error diffusion in OKLab within
/// each cell's colours. The incoming error is clamped to `error_clamp` per channel
/// (and scaled down by local contrast with `adaptive`), the outgoing error scaled by
/// `strength`, and each neighbour's accumulated error clamped again on write.
fn diffuse(image: &Image, screen: &mut Screen, pal: &[Lab; 16], s: &Settings) {
    let Some(kernel) = s.method.kernel() else {
        return;
    };
    let ostro = s.method == Method::Ostromoukhov;
    let (w, h) = screen.mode.screen();
    let ec = s.error_clamp;
    let clamp = |e: Lab| e.map(|v| v.clamp(-ec, ec));
    let lab: Vec<Lab> = image.px.iter().map(|&p| linear_to_oklab(p)).collect();
    let contrast = if !ostro && s.adaptive > 0.0 {
        contrast_map(&lab, w, h)
    } else {
        Vec::new()
    };
    let mut err = vec![[0.0f32; 3]; w * h];
    for y in 0..h {
        let reverse = s.serpentine && y % 2 == 1;
        for step in 0..w {
            let x = if reverse { w - 1 - step } else { step };
            let idx = y * w + x;
            let (c, p) = cell_of(screen, x, y);
            let mut ce = clamp(err[idx]);
            if !ostro && s.adaptive > 0.0 {
                let scale = 1.0 - s.adaptive * contrast[idx];
                ce = ce.map(|v| v * scale);
            }
            let adj = [
                lab[idx][0] + ce[0],
                lab[idx][1] + ce[1],
                lab[idx][2] + ce[2],
            ];
            let colors = &screen.cells[c].colors;
            let (slot, chosen, scale) = if ostro {
                let pair = nearest_pair(adj, colors, pal);
                (pair.a, pair.lab_a, Some(0.6 + 0.8 * blend(&pair)))
            } else {
                let (slot, chosen) = nearest(adj, colors, pal);
                (slot, chosen, None)
            };
            screen.cells[c].px[p] = slot;
            let q = [
                (adj[0] - chosen[0]) * s.strength,
                (adj[1] - chosen[1]) * s.strength,
                (adj[2] - chosen[2]) * s.strength,
            ];
            for &(kdx, kdy, weight) in kernel {
                let dx = if reverse { -kdx } else { kdx };
                let (nx, ny) = (x as i64 + dx, y as i64 + kdy);
                if nx < 0 || nx as usize >= w || ny < 0 || ny as usize >= h {
                    continue;
                }
                let n = ny as usize * w + nx as usize;
                let wt = scale.map_or(weight, |o| weight * o);
                let e = err[n];
                err[n] = clamp([e[0] + q[0] * wt, e[1] + q[1] * wt, e[2] + q[2] * wt]);
            }
        }
    }
}

/// `dither::apply`: re-picks every pixel's slot within its cell's colours with
/// `settings.method`. `none` leaves the screen as it is.
pub fn apply(image: &Image, screen: &mut Screen, palette: &[Rgb; 16], settings: &Settings) {
    let pal = palette.map(linear_to_oklab);
    match settings.method {
        Method::None => {}
        m if m.is_ordered() => apply_ordered(image, screen, m, &pal, settings.strength),
        _ => diffuse(image, screen, &pal, settings),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip_and_split_ordered_from_diffusion() {
        for m in Method::ALL {
            assert_eq!(Method::from_name(m.name()), Some(m));
            assert_eq!(m.is_ordered(), m.kernel().is_none(), "{m:?}");
        }
        assert_eq!(Method::ALL.iter().filter(|m| !m.is_ordered()).count(), 7);
    }

    #[test]
    fn thresholds_stay_in_range() {
        for m in Method::ALL {
            for y in 0..200 {
                for x in 0..320 {
                    let t = ordered_threshold(m, x, y);
                    assert!((-0.5..=0.5).contains(&t), "{m:?} at {x},{y}: {t}");
                }
            }
        }
    }

    #[test]
    fn matrices_match_png2c64_s_definitions() {
        // Spot values from dither.cpp: Bayer 4 rank 0 at (0,0), halftone's offset −0.5,
        // the checker's ±0.25.
        assert_eq!(ordered_threshold(Method::Bayer4, 0, 0), 0.5 / 16.0 - 0.5);
        assert_eq!(ordered_threshold(Method::Bayer4, 3, 3), 5.5 / 16.0 - 0.5);
        assert_eq!(ordered_threshold(Method::Halftone8, 1, 1), 0.5 / 32.0 - 0.5);
        assert_eq!(ordered_threshold(Method::Checker, 1, 0), 0.25);
        assert_eq!(ordered_threshold(Method::Line8, 5, 3), 6.5 / 8.0 - 0.5);
    }
}
