//! Dithering, ported from png2amiga's `dither.cpp` as its C64 path uses it: every
//! method of its catalogue (`kMethodNames`), the ordered thresholds, the palette-aware
//! pickers (the Yliluoma family, Knoll, tri-tone, the "optimal" pairs and plans), the
//! structure-aware biases, and `diffuse_raw_buffer`, the per-pixel scaffolding the C64
//! encoders run every method through.
//!
//! What differs from png2c64's (`png2c64_ref::dither`), besides the many new methods:
//!
//! - **Every method is a second pass** over cell colours chosen beforehand
//!   ([`super::c64`]): an ordered threshold no longer biases the cell search.
//! - **Error diffusion runs in gamma-encoded sRGB**, not OKLab: the pixel's sRGB value
//!   plus its received error (clamped per channel on read only) goes back to OKLab for
//!   the pick, and the residual `(target − chosen) × strength` is spread in sRGB.
//!   `floyd-steinberg-oklab` keeps the residual in OKLab (a/b clamped at half the
//!   limit).
//! - **Ordered offsets** are `threshold × strength × (0.15, 0.03, 0.03)` on the target,
//!   strength once (png2c64 applies it twice).
//! - **No Ostromoukhov, no `adaptive`.** The diffusion kernels of Atkinson, Stucki and
//!   Jarvis are png2amiga's OKLab-tuned weights, not the textbook ones.
//! - **Methods without a C64 implementation** fall through to a plain nearest pick,
//!   as in png2amiga: `gilbert` and `dbs` have no kernel in `diffuse_raw_buffer` and are
//!   not ordered, so on the C64 path they pick each pixel's nearest cell colour.

use super::Rgb;
use super::colour::{
    Lab, fma_dist_sq, linear_to_oklab, linear_to_srgb3, oklab_to_linear, powf, sinf, cosf,
    srgb_to_linear3,
};

/// png2amiga's dither methods, by their command-line names (`kMethodNames`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[expect(missing_docs, reason = "each variant is named by its id, see `name`")]
pub enum Method {
    None,
    Bayer2x2,
    Bayer4x4,
    Bayer8x8,
    Bayer3x3,
    Bayer5x5,
    Bayer6x6,
    Bayer7x7,
    Checker,
    H2x4,
    ClusteredDot,
    Line2,
    LineChecker,
    Line4,
    Line8,
    V4x2,
    Bayer4x2,
    Bayer2x4,
    Vline2,
    VlineChecker,
    Vline4,
    Vline8,
    Halftone8x8,
    Diagonal8x8,
    Spiral5x5,
    Hex8x8,
    Hex5x5,
    BlueNoise,
    VoidCluster,
    ClusterNoise,
    Fractal16,
    FloydSteinberg,
    FloydSteinbergOklab,
    Atkinson,
    SierraLite,
    Stucki,
    Jarvis,
    Dbs,
    Gilbert,
    Riemersma,
    StructureFs,
    ContrastFs,
    Zhoufang,
    Yliluoma,
    Yliluoma2,
    OptChecker,
    Knoll,
    TriTone,
    Yliluoma1,
    OptLine,
    OptLineChecker,
    OptVline,
    OptVlineChecker,
    AsepriteOld,
    Libcaca3,
    Libcaca6,
    Pegasus,
    CranleyBayer,
    Quasicrystal,
    Truchet,
    Ign,
    IgnTri,
    WhiteNoise,
    R2,
    R2Tri,
    Crosshatch,
    Radial,
    ValueNoise,
}

impl Method {
    /// Every method, in png2amiga's `kMethodNames` order.
    pub const ALL: [Self; 68] = [
        Self::None,
        Self::Bayer2x2,
        Self::Bayer4x4,
        Self::Bayer8x8,
        Self::Bayer3x3,
        Self::Bayer5x5,
        Self::Bayer6x6,
        Self::Bayer7x7,
        Self::Checker,
        Self::H2x4,
        Self::ClusteredDot,
        Self::Line2,
        Self::LineChecker,
        Self::Line4,
        Self::Line8,
        Self::V4x2,
        Self::Bayer4x2,
        Self::Bayer2x4,
        Self::Vline2,
        Self::VlineChecker,
        Self::Vline4,
        Self::Vline8,
        Self::Halftone8x8,
        Self::Diagonal8x8,
        Self::Spiral5x5,
        Self::Hex8x8,
        Self::Hex5x5,
        Self::BlueNoise,
        Self::VoidCluster,
        Self::ClusterNoise,
        Self::Fractal16,
        Self::FloydSteinberg,
        Self::FloydSteinbergOklab,
        Self::Atkinson,
        Self::SierraLite,
        Self::Stucki,
        Self::Jarvis,
        Self::Dbs,
        Self::Gilbert,
        Self::Riemersma,
        Self::StructureFs,
        Self::ContrastFs,
        Self::Zhoufang,
        Self::Yliluoma,
        Self::Yliluoma2,
        Self::OptChecker,
        Self::Knoll,
        Self::TriTone,
        Self::Yliluoma1,
        Self::OptLine,
        Self::OptLineChecker,
        Self::OptVline,
        Self::OptVlineChecker,
        Self::AsepriteOld,
        Self::Libcaca3,
        Self::Libcaca6,
        Self::Pegasus,
        Self::CranleyBayer,
        Self::Quasicrystal,
        Self::Truchet,
        Self::Ign,
        Self::IgnTri,
        Self::WhiteNoise,
        Self::R2,
        Self::R2Tri,
        Self::Crosshatch,
        Self::Radial,
        Self::ValueNoise,
    ];

    /// The command-line name (`method_name`).
    pub const fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Bayer2x2 => "bayer2x2",
            Self::Bayer4x4 => "bayer4x4",
            Self::Bayer8x8 => "bayer8x8",
            Self::Bayer3x3 => "bayer3x3",
            Self::Bayer5x5 => "bayer5x5",
            Self::Bayer6x6 => "bayer6x6",
            Self::Bayer7x7 => "bayer7x7",
            Self::Checker => "checker",
            Self::H2x4 => "h2x4",
            Self::ClusteredDot => "clustered-dot",
            Self::Line2 => "line2",
            Self::LineChecker => "line-checker",
            Self::Line4 => "line4",
            Self::Line8 => "line8",
            Self::V4x2 => "v4x2",
            Self::Bayer4x2 => "bayer4x2",
            Self::Bayer2x4 => "bayer2x4",
            Self::Vline2 => "vline2",
            Self::VlineChecker => "vline-checker",
            Self::Vline4 => "vline4",
            Self::Vline8 => "vline8",
            Self::Halftone8x8 => "halftone8x8",
            Self::Diagonal8x8 => "diagonal8x8",
            Self::Spiral5x5 => "spiral5x5",
            Self::Hex8x8 => "hex8x8",
            Self::Hex5x5 => "hex5x5",
            Self::BlueNoise => "blue-noise",
            Self::VoidCluster => "void-cluster",
            Self::ClusterNoise => "cluster-noise",
            Self::Fractal16 => "fractal16",
            Self::FloydSteinberg => "floyd-steinberg",
            Self::FloydSteinbergOklab => "floyd-steinberg-oklab",
            Self::Atkinson => "atkinson",
            Self::SierraLite => "sierra-lite",
            Self::Stucki => "stucki",
            Self::Jarvis => "jarvis",
            Self::Dbs => "dbs",
            Self::Gilbert => "gilbert",
            Self::Riemersma => "riemersma",
            Self::StructureFs => "structure-fs",
            Self::ContrastFs => "contrast-fs",
            Self::Zhoufang => "zhoufang",
            Self::Yliluoma => "yliluoma",
            Self::Yliluoma2 => "yliluoma2",
            Self::OptChecker => "opt-checker",
            Self::Knoll => "knoll",
            Self::TriTone => "tri-tone",
            Self::Yliluoma1 => "yliluoma1",
            Self::OptLine => "opt-line",
            Self::OptLineChecker => "opt-line-checker",
            Self::OptVline => "opt-vline",
            Self::OptVlineChecker => "opt-vline-checker",
            Self::AsepriteOld => "aseprite-old",
            Self::Libcaca3 => "libcaca3",
            Self::Libcaca6 => "libcaca6",
            Self::Pegasus => "pegasus",
            Self::CranleyBayer => "cranley-bayer",
            Self::Quasicrystal => "quasicrystal",
            Self::Truchet => "truchet",
            Self::Ign => "ign",
            Self::IgnTri => "ign-tri",
            Self::WhiteNoise => "white-noise",
            Self::R2 => "r2",
            Self::R2Tri => "r2-tri",
            Self::Crosshatch => "crosshatch",
            Self::Radial => "radial",
            Self::ValueNoise => "value-noise",
        }
    }

    /// The method named `name` (`parse_method_or_null`).
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.name() == name)
    }

    /// `is_ordered`: `none` and every threshold method.
    pub const fn is_ordered(self) -> bool {
        matches!(
            self,
            Self::None
                | Self::Bayer2x2
                | Self::Bayer4x4
                | Self::Bayer8x8
                | Self::Bayer3x3
                | Self::Bayer5x5
                | Self::Bayer6x6
                | Self::Bayer7x7
                | Self::Checker
                | Self::H2x4
                | Self::ClusteredDot
                | Self::Line2
                | Self::LineChecker
                | Self::Line4
                | Self::Line8
                | Self::Vline2
                | Self::VlineChecker
                | Self::Vline4
                | Self::Vline8
                | Self::V4x2
                | Self::Bayer4x2
                | Self::Bayer2x4
                | Self::Halftone8x8
                | Self::Diagonal8x8
                | Self::Spiral5x5
                | Self::Hex8x8
                | Self::Hex5x5
                | Self::BlueNoise
                | Self::VoidCluster
                | Self::ClusterNoise
                | Self::Fractal16
                | Self::AsepriteOld
                | Self::Libcaca3
                | Self::Libcaca6
                | Self::Pegasus
                | Self::CranleyBayer
                | Self::Quasicrystal
                | Self::Truchet
                | Self::Ign
                | Self::IgnTri
                | Self::WhiteNoise
                | Self::R2
                | Self::R2Tri
                | Self::Crosshatch
                | Self::Radial
                | Self::ValueNoise
        )
    }

    /// `is_yliluoma`: the palette-aware pickers.
    pub const fn is_yliluoma(self) -> bool {
        matches!(
            self,
            Self::Yliluoma
                | Self::Yliluoma2
                | Self::OptChecker
                | Self::Knoll
                | Self::TriTone
                | Self::Yliluoma1
                | Self::OptLine
                | Self::OptLineChecker
                | Self::OptVline
                | Self::OptVlineChecker
        )
    }

    /// `error_diffusion_kernel`: `(dx, dy, weight)`; empty for the others.
    pub fn kernel(self) -> &'static [(i64, i64, f32)] {
        match self {
            Self::FloydSteinberg
            | Self::StructureFs
            | Self::ContrastFs
            | Self::Zhoufang
            | Self::Riemersma
            | Self::FloydSteinbergOklab => &FLOYD_STEINBERG,
            Self::Atkinson => &ATKINSON,
            Self::SierraLite => &SIERRA_LITE,
            Self::Stucki => &STUCKI,
            Self::Jarvis => &JARVIS,
            _ => &[],
        }
    }

    /// Whether `diffuse_raw_buffer` diffuses this method's error.
    pub fn diffuses(self) -> bool {
        self != Self::None && !self.is_ordered() && !self.kernel().is_empty()
    }

    /// Whether `diffuse_raw_buffer` offsets the target by the method's threshold.
    pub const fn thresholds(self) -> bool {
        self.is_ordered() && !matches!(self, Self::None)
    }

    /// `needs_structure_bias`.
    pub const fn needs_structure_bias(self) -> bool {
        matches!(self, Self::StructureFs | Self::ContrastFs | Self::Zhoufang)
    }
}

const FLOYD_STEINBERG: [(i64, i64, f32); 4] = [
    (1, 0, 7.0 / 16.0),
    (-1, 1, 3.0 / 16.0),
    (0, 1, 5.0 / 16.0),
    (1, 1, 1.0 / 16.0),
];
/// png2amiga's OKLab-tuned Atkinson (its zero weights kept: they add a zero).
const ATKINSON: [(i64, i64, f32); 6] = [
    (1, 0, 0.25),
    (2, 0, 0.25),
    (-1, 1, 0.25),
    (0, 1, 0.25),
    (1, 1, 0.0),
    (0, 2, 0.0),
];
const SIERRA_LITE: [(i64, i64, f32); 3] =
    [(1, 0, 2.0 / 4.0), (-1, 1, 1.0 / 4.0), (0, 1, 1.0 / 4.0)];
/// png2amiga's OKLab-tuned Stucki.
const STUCKI: [(i64, i64, f32); 12] = [
    (1, 0, 0.240),
    (2, 0, 0.045),
    (-2, 1, 0.048),
    (-1, 1, 0.095),
    (0, 1, 0.240),
    (1, 1, 0.095),
    (2, 1, 0.048),
    (-2, 2, -0.026),
    (-1, 2, 0.048),
    (0, 2, 0.145),
    (1, 2, 0.048),
    (2, 2, -0.026),
];
/// png2amiga's OKLab-tuned Jarvis–Judice–Ninke.
const JARVIS: [(i64, i64, f32); 12] = [
    (1, 0, 0.196),
    (2, 0, 0.104),
    (-2, 1, 0.113),
    (-1, 1, 0.054),
    (0, 1, 0.146),
    (1, 1, 0.054),
    (2, 1, 0.113),
    (-2, 2, -0.029),
    (-1, 2, 0.062),
    (0, 2, 0.054),
    (1, 2, 0.062),
    (2, 2, -0.029),
];

/// A threshold matrix: `w × h` values, row-major.
struct Matrix {
    w: usize,
    h: usize,
    v: Vec<f32>,
}

impl Matrix {
    /// `(raw + offset) / n − 0.5` for each raw rank, as png2amiga's `make_*` build them.
    fn ranked(w: usize, h: usize, raw: &[i32], offset: f32, n: f32) -> Self {
        assert_eq!(raw.len(), w * h);
        Self {
            w,
            h,
            v: raw.iter().map(|&r| (r as f32 + offset) / n - 0.5).collect(),
        }
    }

    fn literal(w: usize, h: usize, v: &[f32]) -> Self {
        assert_eq!(v.len(), w * h);
        Self { w, h, v: v.to_vec() }
    }

    #[inline]
    fn at(&self, x: usize, y: usize) -> f32 {
        self.v[(y % self.h) * self.w + x % self.w]
    }
}

const BAYER8_RAW: [i32; 64] = [
    0, 32, 8, 40, 2, 34, 10, 42, 48, 16, 56, 24, 50, 18, 58, 26, 12, 44, 4, 36, 14, 46, 6, 38, 60,
    28, 52, 20, 62, 30, 54, 22, 3, 35, 11, 43, 1, 33, 9, 41, 51, 19, 59, 27, 49, 17, 57, 25, 15, 47,
    7, 39, 13, 45, 5, 37, 63, 31, 55, 23, 61, 29, 53, 21,
];

/// `make_fractal16`: Bayer-style recursion to 16 × 16, levels reversed.
fn fractal16() -> Matrix {
    const B2: [[i32; 2]; 2] = [[0, 2], [3, 1]];
    let mut raw = vec![0i32; 256];
    for y in 0..16usize {
        for x in 0..16usize {
            let mut v = 0;
            let (mut yy, mut xx) = (y, x);
            for _ in 0..4 {
                v = v * 4 + B2[yy & 1][xx & 1];
                yy >>= 1;
                xx >>= 1;
            }
            let (mut rev, mut vv) = (0, v);
            for _ in 0..4 {
                rev = rev * 4 + (vv & 3);
                vv >>= 2;
            }
            raw[y * 16 + x] = rev;
        }
    }
    Matrix::ranked(16, 16, &raw, 0.5, 256.0)
}

/// `make_blue_noise64`: interleaved gradient noise tabulated over 64 × 64.
#[expect(clippy::excessive_precision, reason = "png2amiga's literals")]
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

/// A 64 × 64 rank table (`void_cluster_64.inc`, `cluster_noise_64.inc`), normalised.
fn table64(bytes: &[u8]) -> Matrix {
    let raw: Vec<i32> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&b| i32::from(u16::from_le_bytes(b)))
        .collect();
    Matrix::ranked(64, 64, &raw, 0.5, 4096.0)
}

/// The matrix of a matrix method, as png2amiga's `constexpr` builders make it.
fn matrix(method: Method) -> Option<Matrix> {
    use Method as M;
    Some(match method {
        M::Bayer2x2 => Matrix::ranked(2, 2, &[0, 2, 3, 1], 0.5, 4.0),
        M::Bayer4x4 => Matrix::ranked(
            4,
            4,
            &[0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5],
            0.5,
            16.0,
        ),
        M::Bayer8x8 => Matrix::ranked(8, 8, &BAYER8_RAW, 0.5, 64.0),
        M::Bayer3x3 => Matrix::ranked(3, 3, &[0, 7, 3, 6, 5, 1, 4, 2, 8], 0.5, 9.0),
        M::Bayer5x5 => Matrix::ranked(
            5,
            5,
            &[
                0, 14, 3, 17, 6, 10, 21, 7, 24, 12, 4, 18, 1, 15, 9, 20, 8, 23, 11, 22, 2, 16, 5,
                19, 13,
            ],
            0.5,
            25.0,
        ),
        M::Bayer6x6 => Matrix::ranked(
            6,
            6,
            &[
                0, 22, 6, 28, 12, 33, 18, 9, 25, 15, 31, 3, 5, 27, 14, 35, 20, 8, 23, 11, 32, 17,
                26, 1, 7, 30, 19, 4, 29, 13, 34, 16, 24, 10, 21, 2,
            ],
            0.5,
            36.0,
        ),
        M::Bayer7x7 => Matrix::ranked(
            7,
            7,
            &[
                0, 30, 6, 36, 12, 42, 18, 24, 11, 38, 17, 44, 23, 4, 7, 33, 20, 27, 14, 47, 31, 40,
                15, 46, 2, 28, 10, 35, 19, 25, 8, 41, 22, 37, 1, 43, 3, 32, 13, 48, 5, 26, 9, 39,
                21, 34, 16, 29, 45,
            ],
            0.5,
            49.0,
        ),
        M::Checker => Matrix::literal(2, 2, &[-0.25, 0.25, 0.25, -0.25]),
        M::H2x4 => Matrix::ranked(2, 4, &[0, 4, 6, 2, 1, 5, 7, 3], 0.5, 8.0),
        M::V4x2 => Matrix::literal(4, 2, &[-0.35, -0.15, 0.15, 0.35, 0.25, 0.05, -0.05, -0.25]),
        M::Bayer4x2 => Matrix::ranked(4, 2, &[0, 4, 1, 5, 6, 2, 7, 3], 0.5, 8.0),
        M::Bayer2x4 => Matrix::ranked(2, 4, &[0, 6, 4, 2, 1, 7, 5, 3], 0.5, 8.0),
        M::ClusteredDot => Matrix::ranked(
            4,
            4,
            &[12, 5, 6, 13, 4, 0, 1, 7, 11, 3, 2, 8, 15, 10, 9, 14],
            0.5,
            16.0,
        ),
        M::Line2 => Matrix::literal(1, 2, &[-0.25, 0.25]),
        M::Vline2 => Matrix::literal(2, 1, &[-0.25, 0.25]),
        M::VlineChecker => Matrix::literal(2, 2, &[-0.35, 0.15, -0.15, 0.35]),
        M::Vline4 => Matrix::literal(4, 1, &[-0.375, -0.125, 0.125, 0.375]),
        M::Vline8 => Matrix::ranked(8, 1, &[0, 4, 2, 6, 1, 5, 3, 7], 0.5, 8.0),
        M::LineChecker => Matrix::literal(2, 2, &[-0.35, -0.15, 0.15, 0.35]),
        M::Line4 => Matrix::literal(1, 4, &[-0.375, -0.125, 0.125, 0.375]),
        M::Line8 => Matrix::ranked(1, 8, &[0, 4, 2, 6, 1, 5, 3, 7], 0.5, 8.0),
        M::Halftone8x8 => Matrix::ranked(
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
        M::Diagonal8x8 => Matrix::ranked(
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
        M::Spiral5x5 => Matrix::ranked(
            5,
            5,
            &[
                20, 21, 22, 23, 24, 19, 6, 7, 8, 9, 18, 5, 0, 1, 10, 17, 4, 3, 2, 11, 16, 15, 14,
                13, 12,
            ],
            0.5,
            25.0,
        ),
        M::Hex8x8 => Matrix::ranked(
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
        M::Hex5x5 => Matrix::ranked(
            5,
            5,
            &[
                4, 3, 0, 1, 2, 0, 1, 2, 4, 3, 2, 4, 3, 0, 1, 3, 0, 1, 2, 4, 1, 2, 4, 3, 0,
            ],
            0.5,
            5.0,
        ),
        M::BlueNoise => blue_noise64(),
        M::VoidCluster => table64(include_bytes!("void_cluster_64.bin")),
        M::ClusterNoise => table64(include_bytes!("cluster_noise_64.bin")),
        M::Fractal16 => fractal16(),
        M::AsepriteOld => Matrix::ranked(
            4,
            4,
            &[0, 8, 2, 10, 14, 6, 12, 4, 3, 11, 1, 9, 15, 7, 13, 5],
            0.5,
            16.0,
        ),
        M::Libcaca3 => Matrix::ranked(3, 3, &[0, 7, 3, 6, 4, 1, 2, 8, 5], 0.5, 9.0),
        M::Libcaca6 => Matrix::ranked(
            6,
            6,
            &[
                0, 28, 4, 24, 14, 32, 20, 8, 30, 12, 22, 2, 6, 26, 16, 34, 10, 18, 31, 13, 23, 3,
                29, 5, 15, 35, 9, 19, 7, 25, 21, 1, 27, 11, 33, 17,
            ],
            0.5,
            36.0,
        ),
        M::Pegasus => Matrix::ranked(
            8,
            8,
            &[
                0, 48, 8, 40, 2, 50, 10, 42, 56, 24, 32, 16, 58, 26, 34, 18, 12, 44, 4, 36, 14, 46,
                6, 38, 28, 60, 20, 52, 30, 62, 22, 54, 3, 51, 11, 43, 1, 49, 9, 41, 59, 27, 35, 19,
                57, 25, 33, 17, 15, 47, 7, 39, 13, 45, 5, 37, 31, 63, 23, 55, 29, 61, 21, 53,
            ],
            0.5,
            64.0,
        ),
        _ => return None,
    })
}

/// Every method's matrix (or `None`), built once, by [`Method::ALL`] index.
fn matrices() -> &'static [Option<Matrix>] {
    static M: std::sync::OnceLock<Vec<Option<Matrix>>> = std::sync::OnceLock::new();
    M.get_or_init(|| Method::ALL.iter().map(|&m| matrix(m)).collect())
}

fn mat(method: Method) -> &'static Matrix {
    matrices()[method as usize]
        .as_ref()
        .expect("a matrix method")
}

/// `std::fmod` on `float` (exact; Rust's `%` is the same operation).
#[inline]
fn fmodf(a: f32, b: f32) -> f32 {
    a % b
}

/// The Cranley–Patterson and Truchet per-tile hash.
#[inline]
fn tile_hash(tx: usize, ty: usize) -> u32 {
    let mut seed = (tx as u32).wrapping_mul(73_856_093) ^ (ty as u32).wrapping_mul(19_349_663);
    seed ^= seed >> 13;
    seed = seed.wrapping_mul(0x5BD1_E995);
    seed ^= seed >> 15;
    seed
}

/// The Wang hash of `white-noise` (and Zhou–Fang's noise).
#[inline]
fn wang(x: usize, y: usize) -> f32 {
    let mut seed = (y as u32).wrapping_mul(65537).wrapping_add(x as u32);
    seed = (seed ^ 61) ^ (seed >> 16);
    seed = seed.wrapping_mul(9);
    seed ^= seed >> 4;
    seed = seed.wrapping_mul(0x27d4_eb2d);
    seed ^= seed >> 15;
    (seed & 0xFFFF) as f32 / 65536.0 - 0.5
}

/// `value_noise`'s hash.
#[inline]
fn value_hash(ix: i32, iy: i32) -> f32 {
    let s = ix
        .wrapping_mul(374_761_393)
        .wrapping_add(iy.wrapping_mul(668_265_263))
        .wrapping_add(1_013_904_223) as u32;
    let s = (s ^ (s >> 13)).wrapping_mul(1_274_126_177);
    (s & 0xFFFF) as f32 / 65536.0
}

/// U(0, 1) → triangle(−1, 1), halved (`ign-tri`, `r2-tri`).
#[inline]
fn triangle(u: f32) -> f32 {
    let t = if u < 0.5 {
        -1.0 + (2.0 * u).sqrt()
    } else {
        1.0 - (2.0 - 2.0 * u).sqrt()
    };
    t * 0.5
}

/// `ordered_threshold`: the raw threshold of a method at `(x, y)`, about `−0.5..0.5`;
/// 0 for the others.
#[expect(
    clippy::excessive_precision,
    clippy::approx_constant,
    reason = "png2amiga's literals, including its truncated 0.7071 and 3.14159265"
)]
pub fn ordered_threshold(method: Method, x: usize, y: usize) -> f32 {
    use Method as M;
    if let Some(m) = &matrices()[method as usize] {
        return m.at(x, y);
    }
    let (fx, fy) = (x as f32, y as f32);
    match method {
        M::CranleyBayer => {
            let seed = tile_hash(x / 8, y / 8);
            let offset = (seed & 0xFFFF) as f32 / 65536.0;
            let v = mat(M::Bayer8x8).at(x, y) + 0.5 + offset;
            fmodf(v, 1.0) - 0.5
        }
        M::Quasicrystal => {
            const N: i32 = 5;
            let scale = 0.40f32;
            let (fx, fy) = (fx * scale, fy * scale);
            let mut sum = 0.0f32;
            for k in 0..N {
                let a = 3.141_592_65_f32 * k as f32 / N as f32;
                sum += cosf(fx * cosf(a) + fy * sinf(a));
            }
            sum / (2.0 * N as f32)
        }
        M::Truchet => {
            let rot = tile_hash(x / 8, y / 8) & 3;
            let lx = (x % 8) as f32 / 8.0 - 0.5;
            let ly = (y % 8) as f32 / 8.0 - 0.5;
            let (rx, ry) = match rot {
                1 => (-ly, lx),
                2 => (-lx, -ly),
                3 => (ly, -lx),
                _ => (lx, ly),
            };
            let r = ((rx + 0.5) * (rx + 0.5) + (ry + 0.5) * (ry + 0.5)).sqrt();
            r - 0.5
        }
        M::Ign => {
            let v = 52.982_918_9 * fmodf(0.067_110_56 * fx + 0.005_837_15 * fy, 1.0);
            fmodf(v, 1.0) - 0.5
        }
        M::IgnTri => {
            let v = 52.982_918_9 * fmodf(0.067_110_56 * fx + 0.005_837_15 * fy, 1.0);
            triangle(fmodf(v, 1.0))
        }
        M::WhiteNoise => wang(x, y),
        M::R2 => fmodf(fx * 0.754_877_666_2 + fy * 0.569_840_291_0 + 0.5, 1.0) - 0.5,
        M::R2Tri => triangle(fmodf(
            fx * 0.754_877_666_2 + fy * 0.569_840_291_0 + 0.5,
            1.0,
        )),
        M::Crosshatch => {
            let tri = |d: f32| 1.0 - (d * 2.0 - 1.0).abs();
            let d0 = tri(fmodf(fy, 8.0) / 8.0);
            let d1 = tri(fmodf(fx, 8.0) / 8.0);
            let d2 = tri(fmodf((fx + fy) * 0.7071, 8.0) / 8.0);
            let d3 = tri(fmodf((fx - fy + 512.0) * 0.7071, 8.0) / 8.0);
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
        M::Radial => {
            // png2amiga's centre (160, 128), for a 320 × 256 Amiga screen.
            let (fx, fy) = (fx - 160.0, fy - 128.0);
            let r = (fx * fx + fy * fy).sqrt();
            let v = fmodf(r * 0.15, 1.0);
            (1.0 - (v * 2.0 - 1.0).abs()) - 0.5
        }
        M::ValueNoise => {
            let scale = 0.125f32;
            let (fx, fy) = (fx * scale, fy * scale);
            let (ix, iy) = (fx.floor() as i32, fy.floor() as i32);
            let mut tx = fx - ix as f32;
            let mut ty = fy - iy as f32;
            tx = tx * tx * (3.0 - 2.0 * tx);
            ty = ty * ty * (3.0 - 2.0 * ty);
            let (v00, v10) = (value_hash(ix, iy), value_hash(ix + 1, iy));
            let (v01, v11) = (value_hash(ix, iy + 1), value_hash(ix + 1, iy + 1));
            let v = v00 * (1.0 - tx) * (1.0 - ty)
                + v10 * tx * (1.0 - ty)
                + v01 * (1.0 - tx) * ty
                + v11 * tx * ty;
            v - 0.5
        }
        _ => 0.0,
    }
}

// ---------------------------------------------------------------------------
// The palette-aware pickers (`pick_yliluoma_family_index` and friends).
// ---------------------------------------------------------------------------

/// A greedy mixing plan of `n` entries: each step adds the colour whose running
/// average (`(sum + c) · 1/(step + 1)`) is nearest the target; `luma4` weights ΔL by 2
/// (Yliluoma 2). Then the plan's indices sorted by lightness (insertion sort, stable).
fn plan(target: Lab, pal: &[Lab], n: usize, luma4: bool) -> (Vec<usize>, Vec<usize>) {
    let mut plan = vec![0usize; n];
    let mut sum = [0.0f32; 3];
    for (step, slot) in plan.iter_mut().enumerate() {
        let (mut best_err, mut best_k) = (f32::MAX, 0usize);
        let n_inv = 1.0 / (step + 1) as f32;
        for (k, c) in pal.iter().enumerate() {
            let avg = [
                (sum[0] + c[0]) * n_inv,
                (sum[1] + c[1]) * n_inv,
                (sum[2] + c[2]) * n_inv,
            ];
            let (dl, da, db) = (avg[0] - target[0], avg[1] - target[1], avg[2] - target[2]);
            let err = if luma4 {
                fma_dist_sq(2.0 * dl, da, db)
            } else {
                fma_dist_sq(dl, da, db)
            };
            if err < best_err {
                best_err = err;
                best_k = k;
            }
        }
        *slot = best_k;
        let c = pal[best_k];
        sum = [sum[0] + c[0], sum[1] + c[1], sum[2] + c[2]];
    }
    let luma: Vec<f32> = plan.iter().map(|&k| pal[k][0]).collect();
    let mut sorted: Vec<usize> = (0..n).collect();
    for i in 1..n {
        let mut j = i;
        while j > 0 && luma[sorted[j - 1]] > luma[sorted[j]] {
            sorted.swap(j, j - 1);
            j -= 1;
        }
    }
    (plan, sorted)
}

/// The plan entry a threshold picks: `b = ⌊(thr + ½) · n⌋`, moved towards the median by
/// `strength` and rounded (`std::round`, half away from zero).
#[expect(
    clippy::cast_possible_truncation,
    reason = "the threshold index is clamped to the plan"
)]
fn plan_pick(plan: &[usize], sorted: &[usize], thr: f32, strength: f32) -> u8 {
    let n = plan.len() as i32;
    let b = (((thr + 0.5) * n as f32) as i32).clamp(0, n - 1);
    let median = n / 2;
    let adjusted =
        ((median as f32 + (b - median) as f32 * strength).round() as i32).clamp(0, n - 1);
    plan[sorted[adjusted as usize]] as u8
}

/// `pick_yliluoma_index`: a 64-entry plan on Bayer 8.
fn pick_yliluoma(target: Lab, pal: &[Lab], x: usize, y: usize, mode2: bool, strength: f32) -> u8 {
    if pal.is_empty() {
        return 0;
    }
    let (p, s) = plan(target, pal, 64, mode2);
    plan_pick(&p, &s, mat(Method::Bayer8x8).at(x, y), strength)
}

/// `pick_knoll_index`: a 16-entry plan on Bayer 4.
fn pick_knoll(target: Lab, pal: &[Lab], x: usize, y: usize, strength: f32) -> u8 {
    if pal.is_empty() {
        return 0;
    }
    let (p, s) = plan(target, pal, 16, false);
    plan_pick(&p, &s, mat(Method::Bayer4x4).at(x, y), strength)
}

/// `pick_tri_tone_index`: a 4-entry plan on Bayer 2.
fn pick_tri_tone(target: Lab, pal: &[Lab], x: usize, y: usize, strength: f32) -> u8 {
    if pal.is_empty() {
        return 0;
    }
    let (p, s) = plan(target, pal, 4, false);
    plan_pick(&p, &s, mat(Method::Bayer2x2).at(x, y), strength)
}

/// `opt_plan4_pick_by_threshold`: a 4-entry plan indexed by a 2 × 2 line matrix.
fn opt_plan4(target: Lab, pal: &[Lab], thr: f32, strength: f32) -> u8 {
    if pal.is_empty() {
        return 0;
    }
    let (p, s) = plan(target, pal, 4, false);
    plan_pick(&p, &s, thr, strength)
}

/// `pick_yliluoma1_index`: the best `(i ≤ j, ratio r/16)` mix within a lightness cutoff
/// of `0.3 × strength`, placed by Bayer 4.
#[expect(
    clippy::cast_possible_truncation,
    reason = "the threshold index is clamped to the plan"
)]
fn pick_yliluoma1(target: Lab, pal: &[Lab], x: usize, y: usize, strength: f32) -> u8 {
    const N: i32 = 16;
    if pal.len() <= 1 {
        return 0;
    }
    let s = strength.clamp(0.0, 1.0);
    let luma_cutoff = s * 0.30;
    let (mut best_err, mut best_i, mut best_j, mut best_r) = (f32::MAX, 0usize, 0usize, N / 2);
    for i in 0..pal.len() {
        for j in i..pal.len() {
            let dl = (pal[i][0] - pal[j][0]).abs();
            if dl > luma_cutoff {
                continue;
            }
            for r in 0..=N {
                let w_i = r as f32 / N as f32;
                let w_j = 1.0 - w_i;
                let al = pal[i][0] * w_i + pal[j][0] * w_j;
                let aa = pal[i][1] * w_i + pal[j][1] * w_j;
                let ab = pal[i][2] * w_i + pal[j][2] * w_j;
                let err = fma_dist_sq(al - target[0], aa - target[1], ab - target[2]);
                if err < best_err {
                    best_err = err;
                    best_i = i;
                    best_j = j;
                    best_r = r;
                }
            }
        }
    }
    let b = (((mat(Method::Bayer4x4).at(x, y) + 0.5) * N as f32) as i32).clamp(0, N - 1);
    (if b < best_r { best_i } else { best_j }) as u8
}

/// `opt_pair_pick`: the nearest colour A and the partner B whose even mix with A is
/// nearest (with a separation penalty that grows as strength drops); `phase` picks the
/// darker (0) or lighter (1) of the pair.
fn opt_pair(target: Lab, pal: &[Lab], phase: usize, strength: f32) -> u8 {
    if pal.len() <= 1 {
        return 0;
    }
    let s = strength.clamp(0.0, 1.0);
    let (mut a, mut best_a) = (0usize, f32::MAX);
    for (k, c) in pal.iter().enumerate() {
        let d = fma_dist_sq(c[0] - target[0], c[1] - target[1], c[2] - target[2]);
        if d < best_a {
            best_a = d;
            a = k;
        }
    }
    if s == 0.0 {
        return a as u8;
    }
    const MIN_W: f32 = 0.005;
    const MAX_W: f32 = 5.0;
    let sep_w = MIN_W * powf(MAX_W / MIN_W, 1.0 - s);
    let (mut b, mut best_b) = (a, best_a);
    for (k, c) in pal.iter().enumerate() {
        if k == a {
            continue;
        }
        let pa = pal[a];
        let al = (pa[0] + c[0]) * 0.5;
        let aa = (pa[1] + c[1]) * 0.5;
        let ab = (pa[2] + c[2]) * 0.5;
        let avg_err = fma_dist_sq(al - target[0], aa - target[1], ab - target[2]);
        let sep = fma_dist_sq(pa[0] - c[0], pa[1] - c[1], pa[2] - c[2]);
        let err = avg_err + sep_w * sep;
        if err < best_b {
            best_b = err;
            b = k;
        }
    }
    if b == a {
        return a as u8;
    }
    let (mut lo, mut hi) = (a, b);
    if pal[hi][0] < pal[lo][0] {
        std::mem::swap(&mut lo, &mut hi);
    }
    (if phase != 0 { hi } else { lo }) as u8
}

/// `pick_yliluoma_family_index`.
pub fn pick_yliluoma_family(
    method: Method,
    target: Lab,
    pal: &[Lab],
    x: usize,
    y: usize,
    strength: f32,
) -> u8 {
    use Method as M;
    match method {
        M::OptChecker => opt_pair(target, pal, (x + y) & 1, strength),
        M::OptLine => opt_pair(target, pal, y & 1, strength),
        M::OptLineChecker => opt_plan4(target, pal, mat(M::LineChecker).at(x, y), strength),
        M::OptVline => opt_pair(target, pal, x & 1, strength),
        M::OptVlineChecker => opt_plan4(target, pal, mat(M::VlineChecker).at(x, y), strength),
        M::Knoll => pick_knoll(target, pal, x, y, strength),
        M::TriTone => pick_tri_tone(target, pal, x, y, strength),
        M::Yliluoma1 => pick_yliluoma1(target, pal, x, y, strength),
        M::Yliluoma2 => pick_yliluoma(target, pal, x, y, true, strength),
        _ => pick_yliluoma(target, pal, x, y, false, strength),
    }
}

/// `pick_palette_index_with_ostro` with `k_min` 0: the palette-aware picker for the
/// Yliluoma family, else the first nearest colour. Returns the chosen index (the
/// Ostromoukhov ratio it also returns is unused on the C64 path).
pub fn pick(method: Method, target: Lab, pal: &[Lab], x: usize, y: usize, strength: f32) -> u8 {
    if method.is_yliluoma() {
        return pick_yliluoma_family(method, target, pal, x, y, strength);
    }
    let (mut best_d, mut best_k) = (f32::MAX, 0usize);
    for (k, c) in pal.iter().enumerate() {
        let d = fma_dist_sq(target[0] - c[0], target[1] - c[1], target[2] - c[2]);
        if d < best_d {
            best_d = d;
            best_k = k;
        }
    }
    best_k as u8
}

/// `compute_structure_bias`: the per-pixel L offset of `structure-fs` (a clamped
/// Laplacian), `contrast-fs` (local range, signed by the side of its midpoint), both
/// scaled by the 5 × 5 standard deviation, and `zhoufang` (hashed noise of an amplitude
/// peaking at mid-grey). Empty for the other methods.
pub fn structure_bias(lab: &[Lab], w: usize, h: usize, method: Method) -> Vec<f32> {
    if !method.needs_structure_bias() {
        return Vec::new();
    }
    let l = |x: usize, y: usize| lab[y * w + x][0];
    let mut sig = vec![0.0f32; w * h];
    match method {
        Method::StructureFs => {
            for y in 0..h {
                for x in 0..w {
                    let c = l(x, y);
                    let lx0 = if x > 0 { l(x - 1, y) } else { c };
                    let lx1 = if x + 1 < w { l(x + 1, y) } else { c };
                    let ly0 = if y > 0 { l(x, y - 1) } else { c };
                    let ly1 = if y + 1 < h { l(x, y + 1) } else { c };
                    // png2amiga's two ifs; `clamp` gives the same value.
                    sig[y * w + x] = (4.0 * c - lx0 - lx1 - ly0 - ly1).clamp(-0.5, 0.5);
                }
            }
        }
        Method::ContrastFs => {
            for y in 0..h {
                for x in 0..w {
                    let (mut lo, mut hi) = (1.0f32, 0.0f32);
                    for dy in -1i64..=1 {
                        for dx in -1i64..=1 {
                            let (yy, xx) = (y as i64 + dy, x as i64 + dx);
                            if yy < 0 || yy >= h as i64 || xx < 0 || xx >= w as i64 {
                                continue;
                            }
                            let v = l(xx as usize, yy as usize);
                            if v < lo {
                                lo = v;
                            }
                            if v > hi {
                                hi = v;
                            }
                        }
                    }
                    let c = l(x, y);
                    let center = 0.5 * (lo + hi);
                    sig[y * w + x] = (hi - lo) * if c > center { 1.0 } else { -1.0 };
                }
            }
        }
        _ => {
            for y in 0..h {
                for x in 0..w {
                    let dist = (l(x, y) - 0.5).abs();
                    sig[y * w + x] = (1.0 - 2.0 * dist) * 0.3;
                }
            }
        }
    }
    let mut stddev = vec![0.0f32; w * h];
    const R: i64 = 2;
    for y in 0..h {
        for x in 0..w {
            let (mut mean, mut sq, mut count) = (0.0f32, 0.0f32, 0i32);
            for dy in -R..=R {
                for dx in -R..=R {
                    let (yy, xx) = (y as i64 + dy, x as i64 + dx);
                    if yy < 0 || yy >= h as i64 || xx < 0 || xx >= w as i64 {
                        continue;
                    }
                    let v = l(xx as usize, yy as usize);
                    mean += v;
                    sq += v * v;
                    count += 1;
                }
            }
            mean /= count as f32;
            sq /= count as f32;
            let var = sq - mean * mean;
            stddev[y * w + x] = if var > 0.0 { var.sqrt() } else { 0.0 };
        }
    }
    const K_SCALE: f32 = 0.4;
    let mut bias = vec![0.0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            bias[i] = if method == Method::Zhoufang {
                wang(x, y) * sig[i]
            } else {
                (K_SCALE * sig[i] * (stddev[i] / 0.1)).clamp(-0.2, 0.2)
            };
        }
    }
    bias
}

/// One pixel's dither settings. With the same settings for every pixel the diffusion is
/// png2amiga's `diffuse_raw_buffer`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PxSettings {
    /// The method.
    pub method: Method,
    /// 0 = no dithering, 1 = full.
    pub strength: f32,
    /// Maximum received error per channel (sRGB; OKLab L for `floyd-steinberg-oklab`).
    pub error_clamp: f32,
    /// Where the pixel's pattern is anchored: thresholds and the pickers' phase are taken
    /// at `(x − origin.0, y − origin.1)` (`(0, 0)`, the screen, as png2amiga).
    pub origin: (i64, i64),
}

impl PxSettings {
    /// The pattern coordinates of screen pixel `(x, y)`.
    #[inline]
    pub fn phase(&self, x: usize, y: usize) -> (usize, usize) {
        if self.origin == (0, 0) {
            (x, y)
        } else {
            // Every pattern repeats within 3200 × 3200 except the per-tile hashes,
            // which see a shifted grid anyway.
            (
                (x as i64 - self.origin.0).rem_euclid(1 << 20) as usize,
                (y as i64 - self.origin.1).rem_euclid(1 << 20) as usize,
            )
        }
    }
}

/// `diffuse_raw_buffer`, per pixel. `pick(target, x, y, settings)` chooses each pixel's
/// colour and returns its OKLab; it is called for every pixel in scan order (serpentine
/// on odd rows when any pixel diffuses and `serpentine`). Returns nothing: the picker
/// records its choices.
///
/// Per pixel, as png2amiga: its sRGB value plus its received error (clamped per channel
/// to its `error_clamp`; a Riemersma pixel reads its 16-deep exponential history
/// instead) back to OKLab, plus its structure bias on L, plus `threshold × strength ×
/// (0.15, 0.03, 0.03)` for a threshold method; then the residual `(target − chosen) ×
/// strength` in sRGB to its kernel's neighbours (or into the Riemersma history).
/// `floyd-steinberg-oklab` keeps its own OKLab residual buffer.
pub fn diffuse<F>(image: &super::Image, px: &[PxSettings], serpentine: bool, mut pick: F)
where
    F: FnMut(Lab, usize, usize, &PxSettings) -> Lab,
{
    let (w, h) = (image.w, image.h);
    assert_eq!(px.len(), w * h, "one setting per pixel");
    let any_diff = px.iter().any(|s| s.method.diffuses());
    let lab: Vec<Lab> = image.px.iter().map(|&p| linear_to_oklab(p)).collect();
    // Structure biases, one map per method that needs one.
    let mut biases: Vec<(Method, Vec<f32>)> = Vec::new();
    for s in px {
        if s.method.needs_structure_bias() && !biases.iter().any(|(m, _)| *m == s.method) {
            biases.push((s.method, structure_bias(&lab, w, h, s.method)));
        }
    }
    let needs_riem = px.iter().any(|s| s.method == Method::Riemersma);
    const RIEM_QSIZE: usize = 16;
    let mut riem_weights = [0.0f32; RIEM_QSIZE];
    if needs_riem {
        let ratio = powf(1.0 / 16.0, 1.0 / 15.0);
        let mut w_acc = 1.0f32;
        for i in (0..RIEM_QSIZE).rev() {
            riem_weights[i] = w_acc;
            w_acc *= ratio;
        }
        let total = riem_weights.iter().fold(0.0f32, |t, &v| t + v);
        for v in &mut riem_weights {
            *v /= total;
        }
    }
    let mut riem_queue = [[0.0f32; 3]; RIEM_QSIZE];
    let mut riem_head = 0usize;
    let mut err_s = vec![[0.0f32; 3]; if any_diff { w * h } else { 0 }];
    let any_oklab = px.iter().any(|s| s.method == Method::FloydSteinbergOklab);
    let mut err_lab = vec![[0.0f32; 3]; if any_oklab { w * h } else { 0 }];
    for y in 0..h {
        let reverse = any_diff && serpentine && y & 1 == 1;
        let dir: i64 = if reverse { -1 } else { 1 };
        for step in 0..w {
            let x = if reverse { w - 1 - step } else { step };
            let i = y * w + x;
            let s = &px[i];
            let is_diff = s.method.diffuses();
            let is_oklab = s.method == Method::FloydSteinbergOklab;
            let riem = is_diff && s.method == Method::Riemersma;
            let ec = s.error_clamp;
            let src_lab = lab[i];
            let mut target_s = [0.0f32; 3];
            let target = if is_oklab {
                let e = err_lab[i];
                let ec_ab = ec * 0.5;
                [
                    src_lab[0] + e[0].clamp(-ec, ec),
                    src_lab[1] + e[1].clamp(-ec_ab, ec_ab),
                    src_lab[2] + e[2].clamp(-ec_ab, ec_ab),
                ]
            } else {
                let src_s = linear_to_srgb3(image.px[i]).map(|v| v.clamp(0.0, 1.0));
                target_s = src_s;
                if riem {
                    let mut carry = [0.0f32; 3];
                    for (k, &wk) in riem_weights.iter().enumerate() {
                        let age = (riem_head + RIEM_QSIZE - 1 - k) % RIEM_QSIZE;
                        let q = riem_queue[age];
                        carry = [
                            carry[0] + q[0] * wk,
                            carry[1] + q[1] * wk,
                            carry[2] + q[2] * wk,
                        ];
                    }
                    for c in 0..3 {
                        target_s[c] += carry[c].clamp(-ec, ec);
                    }
                } else if is_diff {
                    let e = err_s[i];
                    for c in 0..3 {
                        target_s[c] += e[c].clamp(-ec, ec);
                    }
                }
                let mut t = linear_to_oklab(srgb_to_linear3(target_s));
                if is_diff
                    && let Some((_, b)) = biases.iter().find(|(m, _)| *m == s.method)
                {
                    t[0] += b[i];
                }
                if s.method.thresholds() {
                    let (px_, py_) = s.phase(x, y);
                    let thr = ordered_threshold(s.method, px_, py_);
                    t[0] += thr * s.strength * 0.15;
                    t[1] += thr * s.strength * 0.03;
                    t[2] += thr * s.strength * 0.03;
                }
                t
            };
            let chosen = pick(target, x, y, s);
            if !is_diff {
                continue;
            }
            if is_oklab {
                let q = [
                    (target[0] - chosen[0]) * s.strength,
                    (target[1] - chosen[1]) * s.strength,
                    (target[2] - chosen[2]) * s.strength,
                ];
                for &(kdx, kdy, kw) in s.method.kernel() {
                    let (nx, ny) = (x as i64 + kdx * dir, y as i64 + kdy);
                    if nx < 0 || ny < 0 || nx as usize >= w || ny as usize >= h {
                        continue;
                    }
                    let e = &mut err_lab[ny as usize * w + nx as usize];
                    *e = [e[0] + q[0] * kw, e[1] + q[1] * kw, e[2] + q[2] * kw];
                }
            } else {
                let chosen_s =
                    linear_to_srgb3(oklab_to_linear(chosen)).map(|v| v.clamp(0.0, 1.0));
                let q = [
                    (target_s[0] - chosen_s[0]) * s.strength,
                    (target_s[1] - chosen_s[1]) * s.strength,
                    (target_s[2] - chosen_s[2]) * s.strength,
                ];
                if riem {
                    riem_queue[riem_head] = q;
                    riem_head = (riem_head + 1) % RIEM_QSIZE;
                } else {
                    for &(kdx, kdy, kw) in s.method.kernel() {
                        let (nx, ny) = (x as i64 + kdx * dir, y as i64 + kdy);
                        if nx < 0 || ny < 0 || nx as usize >= w || ny as usize >= h {
                            continue;
                        }
                        let e = &mut err_s[ny as usize * w + nx as usize];
                        *e = [e[0] + q[0] * kw, e[1] + q[1] * kw, e[2] + q[2] * kw];
                    }
                }
            }
        }
    }
}

/// An `Rgb` in OKLab, for callers that hold linear colours.
pub fn lab_of(c: Rgb) -> Lab {
    linear_to_oklab(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip_and_families_split() {
        for m in Method::ALL {
            assert_eq!(Method::from_name(m.name()), Some(m), "{m:?}");
        }
        assert_eq!(Method::ALL.len(), 68);
        let diff: Vec<_> = Method::ALL.iter().filter(|m| m.diffuses()).collect();
        assert_eq!(diff.len(), 10, "{diff:?}");
        assert!(!Method::Gilbert.diffuses() && !Method::Gilbert.is_ordered());
        assert!(!Method::Dbs.diffuses() && !Method::Dbs.is_ordered());
    }

    #[test]
    fn thresholds_stay_in_range() {
        for m in Method::ALL {
            for y in 0..200 {
                for x in 0..320 {
                    let t = ordered_threshold(m, x, y);
                    // Truchet reaches √2 − ½ (its distance to a far corner), as
                    // png2amiga.
                    assert!((-0.5..=0.915).contains(&t), "{m:?} at {x},{y}: {t}");
                }
            }
        }
    }

    #[test]
    fn matrices_match_png2amiga_s_definitions() {
        assert_eq!(ordered_threshold(Method::Bayer4x4, 0, 0), 0.5 / 16.0 - 0.5);
        assert_eq!(ordered_threshold(Method::Halftone8x8, 1, 1), 0.5 / 32.0 - 0.5);
        assert_eq!(ordered_threshold(Method::VlineChecker, 1, 0), 0.15);
        assert_eq!(ordered_threshold(Method::V4x2, 3, 1), -0.25);
    }
}
