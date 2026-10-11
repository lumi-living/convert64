//! Colour-difference metrics for the solver (`docs/TAD.md` §3.4).
//!
//! Every metric maps a linear-light colour to three coordinates and measures squared
//! Euclidean distance between coordinates, optionally weighting the first axis (luma for
//! Pepto Y'UV). Coordinates are scaled so the four metrics have comparable ranges
//! (roughly 0–1 per axis), which lets pixels with different metrics share one cell
//! score.
//!
//! The arithmetic is platform-independent: cube roots and the sRGB transfer use basic
//! IEEE operations only (Newton steps in `f64`), not the platform's `cbrtf`/`powf`, so a
//! solve gives the same bytes on macOS, Linux, Windows and wasm.

use crate::colour::{ColourIndex, ColourSet};
use crate::layer;
use crate::palette::convert::{
    D50_WHITE, LIN_SRGB_TO_LMS_F32, LIN_SRGB_TO_XYZ_D50_F32, LMS_TO_OKLAB_F32, mul_f32,
};
use crate::palette::{Palette, luma};
use serde::{Deserialize, Serialize};

/// A colour-difference metric.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Metric {
    /// OKLab (default): perceptually even, cheap.
    #[default]
    OkLab,
    /// CIELAB relative to D50, scaled by 1/100.
    CieLab,
    /// PAL Y'UV of the gamma-encoded colour, the space C64 palettes are derived in, with
    /// the luma axis weighted (2–3× is typical).
    PeptoYuv {
        /// Weight of luma differences relative to chroma (distance uses its square).
        luma_weight: f32,
    },
    /// Gamma-encoded sRGB.
    Rgb,
}

impl Metric {
    /// Default luma weight for [`Metric::PeptoYuv`].
    pub const DEFAULT_LUMA_WEIGHT: f32 = 2.5;

    /// The metric a recipe selects; `luma_weight` applies to Pepto Y'UV.
    pub const fn from_recipe(m: layer::Metric, luma_weight: f32) -> Self {
        match m {
            layer::Metric::Oklab => Self::OkLab,
            layer::Metric::Cielab => Self::CieLab,
            layer::Metric::PeptoYuv => Self::PeptoYuv { luma_weight },
            layer::Metric::Rgb => Self::Rgb,
        }
    }

    /// Which coordinate space (an index into [`MetricTables`]).
    pub const fn space(self) -> Space {
        match self {
            Self::OkLab => Space::OkLab,
            Self::CieLab => Space::CieLab,
            Self::PeptoYuv { .. } => Space::PeptoYuv,
            Self::Rgb => Space::Rgb,
        }
    }

    /// Per-axis weights of the squared distance.
    pub fn weights(self) -> [f32; 3] {
        match self {
            Self::PeptoYuv { luma_weight } => [luma_weight * luma_weight, 1.0, 1.0],
            _ => [1.0; 3],
        }
    }

    /// The colour's coordinates in this metric's space.
    pub fn coords(self, linear: [f32; 3]) -> [f32; 3] {
        self.space().coords(linear)
    }

    /// Squared weighted Euclidean distance between two coordinate triples.
    #[inline]
    pub fn distance(self, a: [f32; 3], b: [f32; 3]) -> f32 {
        let w = self.weights();
        let d = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
        w[0] * d[0] * d[0] + w[1] * d[1] * d[1] + w[2] * d[2] * d[2]
    }

    /// Weighted dot product, the inner product [`Self::distance`] derives from (used to
    /// project a pixel onto the segment between two colours).
    #[inline]
    pub fn dot(self, a: [f32; 3], b: [f32; 3]) -> f32 {
        let w = self.weights();
        w[0] * a[0] * b[0] + w[1] * a[1] * b[1] + w[2] * a[2] * b[2]
    }
}

/// The four coordinate spaces.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Space {
    /// OKLab.
    OkLab,
    /// CIELAB (D50) / 100.
    CieLab,
    /// PAL Y'UV of gamma-encoded sRGB, unweighted.
    PeptoYuv,
    /// Gamma-encoded sRGB.
    Rgb,
}

impl Space {
    /// All spaces, in table order.
    pub const ALL: [Self; 4] = [Self::OkLab, Self::CieLab, Self::PeptoYuv, Self::Rgb];

    /// Index into [`MetricTables`].
    pub const fn index(self) -> usize {
        self as usize
    }

    /// A linear-light colour's coordinates in this space.
    pub fn coords(self, rgb: [f32; 3]) -> [f32; 3] {
        let rgb = rgb.map(|v| v.max(0.0));
        match self {
            Self::OkLab => {
                let lms = mul_f32(&LIN_SRGB_TO_LMS_F32, rgb).map(cbrt);
                mul_f32(&LMS_TO_OKLAB_F32, lms)
            }
            Self::CieLab => {
                const EPSILON: f32 = 216.0 / 24_389.0;
                const KAPPA: f32 = 24_389.0 / 27.0;
                let xyz = mul_f32(&LIN_SRGB_TO_XYZ_D50_F32, rgb);
                let f = |i: usize| {
                    #[expect(clippy::cast_possible_truncation, reason = "f32 by design")]
                    let v = xyz[i] / D50_WHITE[i] as f32;
                    if v > EPSILON {
                        cbrt(v)
                    } else {
                        (KAPPA * v + 16.0) / 116.0
                    }
                };
                let (fx, fy, fz) = (f(0), f(1), f(2));
                [
                    (116.0 * fy - 16.0) / 100.0,
                    5.0 * (fx - fy),
                    2.0 * (fy - fz),
                ]
            }
            Self::PeptoYuv => {
                let [r, g, b] = rgb.map(encode_srgb);
                [
                    0.299 * r + 0.587 * g + 0.114 * b,
                    -0.147_13 * r - 0.288_86 * g + 0.436 * b,
                    0.615 * r - 0.514_99 * g - 0.100_01 * b,
                ]
            }
            Self::Rgb => rgb.map(encode_srgb),
        }
    }
}

/// Cube root of a non-negative number with basic IEEE operations only: a bit-level
/// estimate refined by three Newton steps in `f64`. Accurate to the last bit of `f32`
/// in practice, and identical on every platform.
#[expect(
    clippy::cast_possible_truncation,
    reason = "the f64 refinement is rounded back to f32 by design"
)]
pub fn cbrt(x: f32) -> f32 {
    if x <= 0.0 {
        return 0.0;
    }
    let mut y = f64::from(f32::from_bits(x.to_bits() / 3 + 0x2a51_4067));
    let xd = f64::from(x);
    for _ in 0..3 {
        y -= (y * y * y - xd) / (3.0 * y * y);
    }
    y as f32
}

/// The sRGB transfer function (linear → gamma-encoded) with basic IEEE operations only:
/// `x^(5/12) = (√√∛x)^5`.
#[expect(
    clippy::cast_possible_truncation,
    reason = "the f64 result is rounded back to f32 by design"
)]
pub fn encode_srgb(x: f32) -> f32 {
    if x <= 0.003_130_8 {
        return 12.92 * x.max(0.0);
    }
    if x >= 1.0 {
        return 1.0;
    }
    let t = f64::from(cbrt(x)).sqrt().sqrt();
    let p = t * t * t * t * t;
    (1.055 * p - 0.055) as f32
}

/// The palette's 16 colours in every space: built once per palette object, so switching
/// preset (Pepto → Colodore) means building new tables and re-solving (TAD §3.4).
#[derive(Clone, Debug, PartialEq)]
pub struct MetricTables {
    palette: Palette,
    coords: [[[f32; 3]; 16]; 4],
}

impl MetricTables {
    /// Tables for `palette`.
    pub fn new(palette: &Palette) -> Self {
        let mut coords = [[[0.0; 3]; 16]; 4];
        for space in Space::ALL {
            for c in ColourIndex::ALL {
                let lin = palette.linear(c);
                coords[space.index()][c.as_usize()] = space.coords([lin.r, lin.g, lin.b]);
            }
        }
        Self {
            palette: palette.clone(),
            coords,
        }
    }

    /// The palette these tables describe (the preview's palette too).
    pub const fn palette(&self) -> &Palette {
        &self.palette
    }

    /// Colour `c`'s coordinates in `space`.
    #[inline]
    pub fn colour(&self, space: Space, c: ColourIndex) -> [f32; 3] {
        self.coords[space.index()][c.as_usize()]
    }

    /// All 16 colours' coordinates in `space`, in index order.
    #[inline]
    pub fn colours(&self, space: Space) -> &[[f32; 3]; 16] {
        &self.coords[space.index()]
    }
}

/// The same-luma pair bias for a colour set: `bias` once for every pair of its colours
/// at the same VIC-II luma level (reference §1). Positive values penalise such pairs
/// (they can flicker when alternated on real hardware); negative values reward them
/// (they blend cleanly on PAL, TAD §1.1); 0 (the default) is neutral.
pub fn luma_pair_bias(set: ColourSet, bias: f32) -> f32 {
    if bias == 0.0 {
        return 0.0;
    }
    let colours: Vec<ColourIndex> = set.iter().collect();
    let mut pairs = 0u32;
    for (i, &a) in colours.iter().enumerate() {
        for &b in &colours[i + 1..] {
            pairs += u32::from(luma::is_same_luma_pair(a, b));
        }
    }
    bias * pairs as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const ALL: [Metric; 4] = [
        Metric::OkLab,
        Metric::CieLab,
        Metric::PeptoYuv {
            luma_weight: Metric::DEFAULT_LUMA_WEIGHT,
        },
        Metric::Rgb,
    ];

    #[test]
    fn cbrt_and_srgb_encode_match_the_reference() {
        for i in 0..=1000 {
            let x = i as f32 / 1000.0 * 3.0;
            let want = f64::from(x).cbrt();
            assert!(
                (f64::from(cbrt(x)) - want).abs() <= want * 2e-7 + 1e-12,
                "{x}"
            );
        }
        for k in 0..=255u8 {
            let lin = crate::palette::srgb8_to_linear(k);
            let want = f32::from(k) / 255.0;
            assert!((encode_srgb(lin) - want).abs() < 1e-5, "{k}");
        }
    }

    #[test]
    fn palette_coordinates_agree_with_the_palette_caches() {
        let p = Palette::pepto_pal();
        let t = MetricTables::new(&p);
        for c in ColourIndex::ALL {
            let ok = p.oklab(c);
            let lab = p.lab(c);
            let yuv = p.yuv(c);
            let close = |a: [f32; 3], b: [f32; 3], tol: f32| {
                a.iter().zip(&b).all(|(x, y)| (x - y).abs() <= tol)
            };
            assert!(
                close(t.colour(Space::OkLab, c), [ok.l, ok.a, ok.b], 1e-4),
                "{c}"
            );
            assert!(
                close(
                    t.colour(Space::CieLab, c),
                    [lab.l / 100.0, lab.a / 100.0, lab.b / 100.0],
                    1e-4
                ),
                "{c}"
            );
            assert!(
                close(t.colour(Space::PeptoYuv, c), [yuv.y, yuv.u, yuv.v], 1e-4),
                "{c}"
            );
            let rgb = p.rgb(c);
            assert!(
                close(
                    t.colour(Space::Rgb, c),
                    [rgb.r, rgb.g, rgb.b].map(|v| f32::from(v) / 255.0),
                    1e-4
                ),
                "{c}"
            );
        }
    }

    #[test]
    fn switching_palette_changes_the_tables() {
        let a = MetricTables::new(&Palette::pepto_pal());
        let b = MetricTables::new(&Palette::colodore(50.0, 100.0, 50.0));
        assert_ne!(a, b);
        assert_eq!(a.colour(Space::OkLab, ColourIndex::BLACK), [0.0; 3]);
    }

    #[test]
    fn luma_weight_scales_luma_differences() {
        let w = Metric::PeptoYuv { luma_weight: 3.0 };
        assert!((w.distance([1.0, 0.0, 0.0], [0.0; 3]) - 9.0).abs() < 1e-6);
        assert!((w.distance([0.0, 1.0, 0.0], [0.0; 3]) - 1.0).abs() < 1e-6);
        assert_eq!(
            Metric::from_recipe(layer::Metric::PeptoYuv, 2.0),
            Metric::PeptoYuv { luma_weight: 2.0 }
        );
    }

    #[test]
    fn same_luma_pairs_are_counted() {
        let set: ColourSet = [ColourIndex::BLUE, ColourIndex::BROWN, ColourIndex::WHITE]
            .into_iter()
            .collect();
        assert_eq!(luma_pair_bias(set, 0.5), 0.5);
        let two: ColourSet = [
            ColourIndex::BLUE,
            ColourIndex::BROWN,
            ColourIndex::CYAN,
            ColourIndex::LIGHT_GREY,
        ]
        .into_iter()
        .collect();
        assert_eq!(luma_pair_bias(two, -0.25), -0.5);
        assert_eq!(luma_pair_bias(two, 0.0), 0.0);
    }

    proptest! {
        #[test]
        fn distance_is_symmetric_and_zero_on_identity(
            a in prop::array::uniform3(0.0f32..=1.0),
            b in prop::array::uniform3(0.0f32..=1.0),
            m in 0usize..4,
        ) {
            let metric = ALL[m];
            let (ca, cb) = (metric.coords(a), metric.coords(b));
            prop_assert_eq!(metric.distance(ca, cb), metric.distance(cb, ca));
            prop_assert_eq!(metric.distance(ca, ca), 0.0);
            prop_assert!(metric.distance(ca, cb) >= 0.0);
        }
    }
}
