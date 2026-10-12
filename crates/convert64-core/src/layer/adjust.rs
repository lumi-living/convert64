//! Per-layer colour adjustments (UX §6, group 2). Every field defaults to identity.
//!
//! Applied in a fixed order: gamma → levels → brightness/contrast → HSL → curves →
//! range matching. Gamma, levels, brightness/contrast and curves work per channel in
//! linear light and are baked into lookup tables; HSL works in OKLCh; range matching
//! (png2c64 `--match-range`) remaps the whole image's OKLab extent and so runs over all
//! pixels at once. Alpha is never touched. Passes at their identity value are skipped,
//! so identity adjustments return the input bit for bit.

use super::SourceImage;
use crate::palette::{LinearRgb, Oklab, Oklch, Palette, linear_to_srgb8, srgb8_to_linear};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::Arc;

/// Why an adjustment value is invalid.
#[derive(Clone, Debug, PartialEq)]
pub enum AdjustError {
    /// A value is NaN, infinite or outside its range.
    OutOfRange {
        /// Which value.
        what: &'static str,
        /// The value given.
        value: f32,
    },
    /// Levels black point is not below the white point.
    LevelsOrder {
        /// Black point.
        black: f32,
        /// White point.
        white: f32,
    },
    /// A curve needs 2–16 points with strictly increasing `x`.
    CurvePoints(String),
}

impl fmt::Display for AdjustError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutOfRange { what, value } => write!(f, "{what} is out of range: {value}"),
            Self::LevelsOrder { black, white } => {
                write!(f, "levels black ({black}) must be below white ({white})")
            }
            Self::CurvePoints(why) => write!(f, "curve: {why}"),
        }
    }
}

impl std::error::Error for AdjustError {}

/// `x^y` evaluated in f64 and rounded to f32: platforms' `pow` differ in the last bit
/// at most, which the rounding absorbs, so adjusted pixels (and the solver's input) are
/// the same on every OS.
#[expect(clippy::cast_possible_truncation, reason = "rounded to f32 by design")]
fn pow(x: f32, y: f32) -> f32 {
    f64::from(x).powf(f64::from(y)) as f32
}

fn check(what: &'static str, value: f32, lo: f32, hi: f32) -> Result<f32, AdjustError> {
    if value.is_finite() && (lo..=hi).contains(&value) {
        Ok(value)
    } else {
        Err(AdjustError::OutOfRange { what, value })
    }
}

/// png2c64's `--gamma`: every channel raised to this power in linear light before any
/// other adjustment, `0.1..=8`. Above 1 darkens mid-tones (png2c64's default is 1.5);
/// 1 is identity. Note the direction is the opposite of [`Levels`]' gamma, which
/// brightens above 1: `Gamma(g)` equals levels gamma `1/g`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "f32", into = "f32")]
pub struct Gamma(f32);

impl Gamma {
    /// 1: no change.
    pub const IDENTITY: Self = Self(1.0);

    /// png2c64's default, 1.5.
    pub const PNG2C64: Self = Self(1.5);

    /// A gamma in `0.1..=8`.
    pub fn new(value: f32) -> Result<Self, AdjustError> {
        check("gamma", value, 0.1, 8.0).map(Self)
    }

    /// The exponent.
    pub const fn get(self) -> f32 {
        self.0
    }

    fn eval(self, v: f32) -> f32 {
        if self.0 == 1.0 {
            v
        } else {
            pow(v.max(0.0), self.0)
        }
    }
}

impl Default for Gamma {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl TryFrom<f32> for Gamma {
    type Error = AdjustError;
    fn try_from(v: f32) -> Result<Self, AdjustError> {
        Self::new(v)
    }
}

impl From<Gamma> for f32 {
    fn from(g: Gamma) -> Self {
        g.0
    }
}

#[derive(Deserialize)]
struct RangeMatchData {
    percentile: f32,
    l: [f32; 2],
    a: [f32; 2],
    b: [f32; 2],
}

/// png2c64's `--match-range` (OKLab range matching): the image's OKLab extent, taken
/// between the `percentile` and `1 − percentile` quantiles of its opaque pixels, is
/// mapped onto a target extent. L is stretched linearly; a and b are scaled around zero
/// so greys stay grey. The target is a palette's extent shrunk by a margin
/// ([`Self::for_palette`]), stored so a project file is self-contained.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "RangeMatchData")]
pub struct RangeMatch {
    percentile: f32,
    l: [f32; 2],
    a: [f32; 2],
    b: [f32; 2],
}

impl TryFrom<RangeMatchData> for RangeMatch {
    type Error = AdjustError;
    fn try_from(d: RangeMatchData) -> Result<Self, AdjustError> {
        Self::new(d.percentile, d.l, d.a, d.b)
    }
}

impl RangeMatch {
    /// png2c64's tail fraction ignored at each end, 1 %.
    pub const DEFAULT_PERCENTILE: f32 = 0.01;

    /// png2c64's inward margin on the palette extent, 5 %.
    pub const DEFAULT_MARGIN: f32 = 0.05;

    /// Range matching with `percentile` in `0..0.5` onto target ranges `[lo, hi]` of
    /// OKLab L, a and b (finite, `lo ≤ hi`).
    pub fn new(
        percentile: f32,
        l: [f32; 2],
        a: [f32; 2],
        b: [f32; 2],
    ) -> Result<Self, AdjustError> {
        check("range percentile", percentile, 0.0, 0.499)?;
        for (what, r) in [("range L", l), ("range a", a), ("range b", b)] {
            check(what, r[0], -2.0, 2.0)?;
            check(what, r[1], r[0], 2.0)?;
        }
        Ok(Self {
            percentile,
            l,
            a,
            b,
        })
    }

    /// png2c64's matching onto `palette`: its OKLab extent shrunk by `margin` (a
    /// fraction of each span, `0..0.5`) on each side.
    pub fn for_palette(
        palette: &Palette,
        percentile: f32,
        margin: f32,
    ) -> Result<Self, AdjustError> {
        check("range margin", margin, 0.0, 0.499)?;
        let labs: Vec<Oklab> = crate::colour::ColourIndex::ALL
            .into_iter()
            .map(|c| palette.oklab(c))
            .collect();
        let extent = |f: fn(&Oklab) -> f32| {
            let lo = labs.iter().map(f).fold(f32::INFINITY, f32::min);
            let hi = labs.iter().map(f).fold(f32::NEG_INFINITY, f32::max);
            let m = (hi - lo) * margin;
            [lo + m, hi - m]
        };
        Self::new(
            percentile,
            extent(|c| c.l),
            extent(|c| c.a),
            extent(|c| c.b),
        )
    }

    /// Tail fraction ignored at each end.
    pub const fn percentile(&self) -> f32 {
        self.percentile
    }

    /// Target L range.
    pub const fn l(&self) -> [f32; 2] {
        self.l
    }

    /// Target a range.
    pub const fn a(&self) -> [f32; 2] {
        self.a
    }

    /// Target b range.
    pub const fn b(&self) -> [f32; 2] {
        self.b
    }

    /// Remaps `px` (straight-alpha linear RGBA) in place; fully transparent pixels are
    /// neither measured nor changed.
    pub fn apply(&self, px: &mut [[f32; 4]]) {
        let mut labs: Vec<(usize, Oklab)> = px
            .iter()
            .enumerate()
            .filter(|(_, p)| p[3] > 0.0)
            .map(|(i, p)| {
                let lab = LinearRgb {
                    r: p[0],
                    g: p[1],
                    b: p[2],
                }
                .to_oklab();
                (i, lab)
            })
            .collect();
        if labs.is_empty() {
            return;
        }
        let p = self.percentile;
        let range =
            |f: fn(&Oklab) -> f32| source_range(labs.iter().map(|(_, c)| f(c)).collect(), p);
        let (sl, sa, sb) = (range(|c| c.l), range(|c| c.a), range(|c| c.b));
        crate::par::for_each(&mut labs, |(_, c)| {
            c.l = remap(c.l, sl, self.l);
            c.a = scale_around_zero(c.a, sa, self.a);
            c.b = scale_around_zero(c.b, sb, self.b);
        });
        for (i, lab) in labs {
            let out = LinearRgb::from_oklab(lab);
            px[i][..3].copy_from_slice(&[out.r, out.g, out.b].map(|v| v.clamp(0.0, 1.0)));
        }
    }
}

/// The `p` and `1 − p` quantiles of `v` (png2c64's indices: `⌊n·p⌋` and `⌊n·(1 − p)⌋`).
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "indices into a non-empty pixel list"
)]
fn source_range(mut v: Vec<f32>, p: f32) -> [f32; 2] {
    let n = v.len();
    let at = |q: f32| ((n as f32 * q) as usize).min(n - 1);
    let (lo, hi) = (at(p), at(1.0 - p));
    let lo_v = *v.select_nth_unstable_by(lo, f32::total_cmp).1;
    let hi_v = *v.select_nth_unstable_by(hi, f32::total_cmp).1;
    [lo_v, hi_v]
}

/// png2c64's linear remap of `v` from `src` to `dst`; a flat source maps to the middle.
fn remap(v: f32, src: [f32; 2], dst: [f32; 2]) -> f32 {
    let span = src[1] - src[0];
    if span < 1e-6 {
        return (dst[0] + dst[1]) * 0.5;
    }
    dst[0] + (v - src[0]) / span * (dst[1] - dst[0])
}

/// png2c64's chroma scaling: positive values by `dst.hi / src.hi`, negative ones by
/// `dst.lo / src.lo`, so zero (grey) stays put.
fn scale_around_zero(v: f32, src: [f32; 2], dst: [f32; 2]) -> f32 {
    if v >= 0.0 {
        if src[1] > 1e-6 {
            v * dst[1] / src[1]
        } else {
            v
        }
    } else if src[0] < -1e-6 {
        v * dst[0] / src[0]
    } else {
        v
    }
}

/// A source whose OKLab L span covers less than this share of the palette's (both as
/// [`suggest_range_match`] measures them) has a narrow range.
const NARROW_RANGE: f32 = 0.6;

/// An "Auto levels" suggestion for `source` (#142): range matching onto `palette` with
/// png2c64's defaults when the source's OKLab L range (1st–99th percentile) covers less
/// than 60 % of the palette's. Only a suggestion: nothing applies it unless asked.
pub fn suggest_range_match(source: &SourceImage, palette: &Palette) -> Option<RangeMatch> {
    let ls: Vec<f32> = source
        .pixels()
        .iter()
        .filter(|p| p[3] > 0.0)
        .map(|p| {
            LinearRgb {
                r: p[0],
                g: p[1],
                b: p[2],
            }
            .to_oklab()
            .l
        })
        .collect();
    if ls.is_empty() {
        return None;
    }
    let m = RangeMatch::for_palette(
        palette,
        RangeMatch::DEFAULT_PERCENTILE,
        RangeMatch::DEFAULT_MARGIN,
    )
    .expect("png2c64's defaults are valid");
    let [lo, hi] = source_range(ls, RangeMatch::DEFAULT_PERCENTILE);
    (hi - lo < NARROW_RANGE * (m.l[1] - m.l[0])).then_some(m)
}

#[derive(Deserialize)]
struct LevelsData {
    black: f32,
    white: f32,
    gamma: f32,
}

/// Input levels in linear light: `out = clamp((v − black) / (white − black))^(1/gamma)`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "LevelsData")]
pub struct Levels {
    black: f32,
    white: f32,
    gamma: f32,
}

impl TryFrom<LevelsData> for Levels {
    type Error = AdjustError;
    fn try_from(d: LevelsData) -> Result<Self, AdjustError> {
        Self::new(d.black, d.white, d.gamma)
    }
}

impl Default for Levels {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Levels {
    /// Black 0, white 1, gamma 1.
    pub const IDENTITY: Self = Self {
        black: 0.0,
        white: 1.0,
        gamma: 1.0,
    };

    /// Levels with `0 ≤ black < white ≤ 1` and gamma in `0.1..=10`.
    pub fn new(black: f32, white: f32, gamma: f32) -> Result<Self, AdjustError> {
        check("levels black", black, 0.0, 1.0)?;
        check("levels white", white, 0.0, 1.0)?;
        check("levels gamma", gamma, 0.1, 10.0)?;
        if black >= white {
            return Err(AdjustError::LevelsOrder { black, white });
        }
        Ok(Self {
            black,
            white,
            gamma,
        })
    }

    /// Black point.
    pub const fn black(&self) -> f32 {
        self.black
    }

    /// White point.
    pub const fn white(&self) -> f32 {
        self.white
    }

    /// Gamma (> 1 brightens mid-tones).
    pub const fn gamma(&self) -> f32 {
        self.gamma
    }

    /// The levels curve at `v`.
    pub fn eval(&self, v: f32) -> f32 {
        let t = ((v - self.black) / (self.white - self.black)).clamp(0.0, 1.0);
        if self.gamma == 1.0 {
            t
        } else {
            pow(t, 1.0 / self.gamma)
        }
    }
}

#[derive(Deserialize)]
struct BrightnessContrastData {
    #[serde(default)]
    brightness: f32,
    #[serde(default)]
    contrast: f32,
}

/// Brightness and contrast in linear light, each `−1..=1` (the UI's −100…100).
///
/// Brightness is exposure: `v × 4^brightness`. Contrast is a power curve pivoting on
/// linear middle grey (0.18): `0.18 × (v / 0.18)^(4^contrast)`. Both keep black at black
/// and are monotone; the result is clamped to `0..=1`.
#[derive(Clone, Copy, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(try_from = "BrightnessContrastData")]
pub struct BrightnessContrast {
    brightness: f32,
    contrast: f32,
}

impl TryFrom<BrightnessContrastData> for BrightnessContrast {
    type Error = AdjustError;
    fn try_from(d: BrightnessContrastData) -> Result<Self, AdjustError> {
        Self::new(d.brightness, d.contrast)
    }
}

/// Linear middle grey, the contrast pivot.
const MID_GREY: f32 = 0.18;

impl BrightnessContrast {
    /// Both values in `−1..=1`.
    pub fn new(brightness: f32, contrast: f32) -> Result<Self, AdjustError> {
        Ok(Self {
            brightness: check("brightness", brightness, -1.0, 1.0)?,
            contrast: check("contrast", contrast, -1.0, 1.0)?,
        })
    }

    /// Brightness, `−1..=1`.
    pub const fn brightness(&self) -> f32 {
        self.brightness
    }

    /// Contrast, `−1..=1`.
    pub const fn contrast(&self) -> f32 {
        self.contrast
    }

    fn eval(&self, v: f32) -> f32 {
        let mut v = v.max(0.0);
        if self.brightness != 0.0 {
            v *= pow(4.0, self.brightness);
        }
        if self.contrast != 0.0 {
            v = MID_GREY * pow(v / MID_GREY, pow(4.0, self.contrast));
        }
        v.clamp(0.0, 1.0)
    }
}

#[derive(Deserialize)]
struct HslData {
    #[serde(default)]
    hue: f32,
    #[serde(default)]
    saturation: f32,
    #[serde(default)]
    lightness: f32,
}

/// Hue, saturation and lightness, applied in OKLCh.
///
/// `hue` rotates the hue angle by that many degrees (any finite value, taken modulo 360).
/// `saturation` (`−1..=1`) scales chroma by `1 + saturation`, so −1 is greyscale.
/// `lightness` (`−1..=1`) moves L towards white (`> 0`) or black (`< 0`) by that fraction
/// and fades chroma by the same fraction, so ±1 is pure white or black.
/// Results outside the sRGB gamut are clamped per channel.
#[derive(Clone, Copy, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(try_from = "HslData")]
pub struct Hsl {
    hue: f32,
    saturation: f32,
    lightness: f32,
}

impl TryFrom<HslData> for Hsl {
    type Error = AdjustError;
    fn try_from(d: HslData) -> Result<Self, AdjustError> {
        Self::new(d.hue, d.saturation, d.lightness)
    }
}

impl Hsl {
    /// Hue in degrees (finite), saturation and lightness in `−1..=1`.
    pub fn new(hue: f32, saturation: f32, lightness: f32) -> Result<Self, AdjustError> {
        if !hue.is_finite() {
            return Err(AdjustError::OutOfRange {
                what: "hue",
                value: hue,
            });
        }
        Ok(Self {
            hue,
            saturation: check("saturation", saturation, -1.0, 1.0)?,
            lightness: check("lightness", lightness, -1.0, 1.0)?,
        })
    }

    /// Hue rotation in degrees.
    pub const fn hue(&self) -> f32 {
        self.hue
    }

    /// Saturation change, `−1..=1`.
    pub const fn saturation(&self) -> f32 {
        self.saturation
    }

    /// Lightness change, `−1..=1`.
    pub const fn lightness(&self) -> f32 {
        self.lightness
    }

    fn is_identity(&self) -> bool {
        self.hue.rem_euclid(360.0) == 0.0 && self.saturation == 0.0 && self.lightness == 0.0
    }

    fn apply(&self, rgb: [f32; 3]) -> [f32; 3] {
        let lab = LinearRgb {
            r: rgb[0],
            g: rgb[1],
            b: rgb[2],
        }
        .to_oklab();
        let mut lch: Oklch = lab.to_oklch();
        lch.h += self.hue.rem_euclid(360.0).to_radians();
        lch.c *= 1.0 + self.saturation;
        let l = self.lightness;
        if l >= 0.0 {
            lch.l += l * (1.0 - lch.l);
            lch.c *= 1.0 - l;
        } else {
            lch.l *= 1.0 + l;
            lch.c *= 1.0 + l;
        }
        let out = LinearRgb::from_oklab(lch.to_oklab());
        [out.r, out.g, out.b].map(|v| v.clamp(0.0, 1.0))
    }
}

/// Most control points a curve may have.
pub const MAX_CURVE_POINTS: usize = 16;

/// A tone curve: a monotone cubic (Fritsch–Carlson) through 2–16 control points
/// `(x, y)` in `0..=1` with strictly increasing `x`, constant beyond the end points.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "Vec<[f32; 2]>", into = "Vec<[f32; 2]>")]
pub struct Curve {
    points: Vec<[f32; 2]>,
    /// Hermite tangents at each point.
    #[serde(skip)]
    tangents: Vec<f32>,
}

impl TryFrom<Vec<[f32; 2]>> for Curve {
    type Error = AdjustError;
    fn try_from(points: Vec<[f32; 2]>) -> Result<Self, AdjustError> {
        Self::new(points)
    }
}

impl From<Curve> for Vec<[f32; 2]> {
    fn from(c: Curve) -> Self {
        c.points
    }
}

impl Default for Curve {
    fn default() -> Self {
        Self::identity()
    }
}

impl Curve {
    /// The straight line `(0, 0)–(1, 1)`.
    pub fn identity() -> Self {
        Self::new(vec![[0.0, 0.0], [1.0, 1.0]]).expect("identity curve is valid")
    }

    /// A curve through `points`.
    pub fn new(points: Vec<[f32; 2]>) -> Result<Self, AdjustError> {
        if !(2..=MAX_CURVE_POINTS).contains(&points.len()) {
            return Err(AdjustError::CurvePoints(format!(
                "needs 2–{MAX_CURVE_POINTS} points, got {}",
                points.len()
            )));
        }
        for p in &points {
            check("curve x", p[0], 0.0, 1.0)?;
            check("curve y", p[1], 0.0, 1.0)?;
        }
        if points.windows(2).any(|w| w[1][0] <= w[0][0]) {
            return Err(AdjustError::CurvePoints(
                "x must be strictly increasing".into(),
            ));
        }
        let tangents = fritsch_carlson(&points);
        Ok(Self { points, tangents })
    }

    /// The control points.
    pub fn points(&self) -> &[[f32; 2]] {
        &self.points
    }

    fn is_identity(&self) -> bool {
        self.points.len() == 2 && self.points[0] == [0.0, 0.0] && self.points[1] == [1.0, 1.0]
    }

    /// The curve's value at `x`.
    pub fn eval(&self, x: f32) -> f32 {
        let p = &self.points;
        let (first, last) = (p[0], p[p.len() - 1]);
        if x <= first[0] {
            return first[1];
        }
        if x >= last[0] {
            return last[1];
        }
        let k = p.partition_point(|q| q[0] <= x) - 1;
        let (x0, y0, x1, y1) = (p[k][0], p[k][1], p[k + 1][0], p[k + 1][1]);
        let h = x1 - x0;
        let t = (x - x0) / h;
        let (t2, t3) = (t * t, t * t * t);
        let h00 = 2.0 * t3 - 3.0 * t2 + 1.0;
        let h10 = t3 - 2.0 * t2 + t;
        let h01 = -2.0 * t3 + 3.0 * t2;
        let h11 = t3 - t2;
        (h00 * y0 + h10 * h * self.tangents[k] + h01 * y1 + h11 * h * self.tangents[k + 1])
            .clamp(0.0, 1.0)
    }
}

/// Fritsch–Carlson tangents: monotone wherever the data is, and never overshooting a
/// local extremum.
fn fritsch_carlson(p: &[[f32; 2]]) -> Vec<f32> {
    let n = p.len();
    let d: Vec<f32> = p
        .windows(2)
        .map(|w| (w[1][1] - w[0][1]) / (w[1][0] - w[0][0]))
        .collect();
    let mut m = vec![0.0; n];
    m[0] = d[0];
    m[n - 1] = d[n - 2];
    for k in 1..n - 1 {
        m[k] = if d[k - 1] * d[k] <= 0.0 {
            0.0
        } else {
            (d[k - 1] + d[k]) / 2.0
        };
    }
    for k in 0..n - 1 {
        if d[k] == 0.0 {
            m[k] = 0.0;
            m[k + 1] = 0.0;
            continue;
        }
        let a = m[k] / d[k];
        let b = m[k + 1] / d[k];
        let s = a * a + b * b;
        if s > 9.0 {
            let tau = 3.0 / s.sqrt();
            m[k] = tau * a * d[k];
            m[k + 1] = tau * b * d[k];
        }
    }
    m
}

/// Tone curves: a master curve applied first, then one per channel.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Curves {
    /// Applied to R, G and B.
    #[serde(default)]
    pub master: Curve,
    /// Red.
    #[serde(default)]
    pub red: Curve,
    /// Green.
    #[serde(default)]
    pub green: Curve,
    /// Blue.
    #[serde(default)]
    pub blue: Curve,
}

impl Curves {
    fn is_identity(&self) -> bool {
        self.master.is_identity()
            && self.red.is_identity()
            && self.green.is_identity()
            && self.blue.is_identity()
    }
}

/// A layer's colour adjustments.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Adjustments {
    /// png2c64-style gamma, applied first.
    #[serde(default)]
    pub gamma: Gamma,
    /// Input levels.
    #[serde(default)]
    pub levels: Levels,
    /// Brightness and contrast.
    #[serde(default)]
    pub brightness_contrast: BrightnessContrast,
    /// Hue, saturation, lightness (OKLCh).
    #[serde(default)]
    pub hsl: Hsl,
    /// Tone curves.
    #[serde(default)]
    pub curves: Curves,
    /// OKLab range matching, applied last.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub match_range: Option<RangeMatch>,
}

/// A separable per-channel tone function.
#[derive(Clone, Debug)]
enum ToneFn {
    /// Gamma, levels, then brightness/contrast.
    Tone(Gamma, Levels, BrightnessContrast),
    /// Master curve then a channel curve.
    Curve(Curve, Curve),
}

impl ToneFn {
    fn eval(&self, v: f32) -> f32 {
        match self {
            Self::Tone(g, l, bc) => bc.eval(l.eval(g.eval(v))),
            Self::Curve(master, ch) => ch.eval(master.eval(v.clamp(0.0, 1.0))),
        }
    }
}

/// A per-channel lookup table that is exact: one entry per 8-bit sRGB value, which is
/// every value a source decoded from a PNG can hold. Other inputs (sources built from
/// linear floats) are evaluated directly. An interpolating table cannot follow the
/// infinite slope of a levels gamma at its black point.
#[derive(Clone, Debug)]
struct Lut {
    f: ToneFn,
    table: [f32; 256],
}

impl Lut {
    fn build(f: ToneFn) -> Self {
        let mut table = [0.0; 256];
        for (k, v) in (0..=255u8).zip(table.iter_mut()) {
            *v = f.eval(srgb8_to_linear(k));
        }
        Self { f, table }
    }

    fn eval(&self, v: f32) -> f32 {
        let k = linear_to_srgb8(v);
        if srgb8_to_linear(k) == v {
            self.table[usize::from(k)]
        } else {
            self.f.eval(v)
        }
    }
}

/// [`Adjustments`] compiled for per-pixel use: the separable passes as tables, the
/// identity passes dropped.
#[derive(Clone, Debug)]
pub struct AdjustPlan {
    /// Gamma, levels, then brightness/contrast, the same for every channel.
    tone: Option<Lut>,
    hsl: Option<Hsl>,
    /// Master then channel curve, per channel.
    curves: Option<[Lut; 3]>,
}

impl Adjustments {
    /// Whether every adjustment is at its identity value.
    pub fn is_identity(&self) -> bool {
        self.plan().is_identity() && self.match_range.is_none()
    }

    /// Compiles the adjustments into lookup tables.
    pub fn plan(&self) -> AdjustPlan {
        let (gamma, levels) = (self.gamma, self.levels);
        let bc = self.brightness_contrast;
        let tone = (gamma != Gamma::IDENTITY
            || levels != Levels::IDENTITY
            || bc != BrightnessContrast::default())
        .then(|| Lut::build(ToneFn::Tone(gamma, levels, bc)));
        let hsl = (!self.hsl.is_identity()).then_some(self.hsl);
        let c = &self.curves;
        let curves = (!c.is_identity()).then(|| {
            [&c.red, &c.green, &c.blue]
                .map(|ch| Lut::build(ToneFn::Curve(c.master.clone(), ch.clone())))
        });
        AdjustPlan { tone, hsl, curves }
    }
}

impl AdjustPlan {
    /// No pass is active.
    pub fn is_identity(&self) -> bool {
        self.tone.is_none() && self.hsl.is_none() && self.curves.is_none()
    }

    /// Adjusts one straight-alpha linear pixel in place; alpha is untouched.
    pub fn apply(&self, px: &mut [f32; 4]) {
        if let Some(t) = &self.tone {
            for v in &mut px[..3] {
                *v = t.eval(*v);
            }
        }
        if let Some(h) = &self.hsl {
            let [r, g, b] = h.apply([px[0], px[1], px[2]]);
            px[..3].copy_from_slice(&[r, g, b]);
        }
        if let Some(c) = &self.curves {
            for (v, lut) in px[..3].iter_mut().zip(c) {
                *v = lut.eval(*v);
            }
        }
    }
}

/// A layer's adjusted pixels, recomputed only when the source or the adjustments
/// change.
#[derive(Clone, Debug, Default)]
pub struct AdjustCache {
    key: Option<(Arc<SourceImage>, Adjustments)>,
    pixels: Option<Arc<[[f32; 4]]>>,
    recomputes: u64,
}

impl AdjustCache {
    /// The adjusted pixels of `source`. Identity adjustments return the source pixels
    /// unchanged (one copy into the shared buffer, made once).
    pub fn get(&mut self, source: &Arc<SourceImage>, adj: &Adjustments) -> Arc<[[f32; 4]]> {
        let hit = matches!(&self.key, Some((s, a)) if Arc::ptr_eq(s, source) && a == adj);
        if !hit || self.pixels.is_none() {
            let plan = adj.plan();
            let mut px = source.pixels().to_vec();
            if !plan.is_identity() {
                crate::par::for_each(&mut px, |p| plan.apply(p));
            }
            if let Some(m) = &adj.match_range {
                m.apply(&mut px);
            }
            self.pixels = Some(px.into());
            self.key = Some((Arc::clone(source), adj.clone()));
            self.recomputes += 1;
        }
        Arc::clone(self.pixels.as_ref().expect("filled above"))
    }

    /// How many times the pixels were (re)computed; for tests and diagnostics.
    pub const fn recomputes(&self) -> u64 {
        self.recomputes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn run(adj: &Adjustments, px: [f32; 4]) -> [f32; 4] {
        let mut p = px;
        adj.plan().apply(&mut p);
        p
    }

    #[test]
    fn identity_is_a_bit_exact_no_op() {
        let adj = Adjustments::default();
        assert!(adj.is_identity());
        for px in [[0.0, 0.5, 1.0, 1.0], [0.123_456, 2.5, 0.0, 0.25]] {
            assert_eq!(run(&adj, px), px);
        }
        // A hue rotation by a whole turn is also identity.
        let turn = Adjustments {
            hsl: Hsl::new(360.0, 0.0, 0.0).unwrap(),
            ..Adjustments::default()
        };
        assert!(turn.is_identity());
    }

    #[test]
    fn constructors_validate() {
        assert!(Levels::new(0.5, 0.5, 1.0).is_err());
        assert!(Levels::new(0.6, 0.5, 1.0).is_err());
        assert!(Levels::new(0.0, 1.0, 0.0).is_err());
        assert!(Levels::new(-0.1, 1.0, 1.0).is_err());
        assert!(Levels::new(0.0, 1.0, f32::NAN).is_err());
        assert!(BrightnessContrast::new(1.5, 0.0).is_err());
        assert!(BrightnessContrast::new(0.0, -1.5).is_err());
        assert!(Hsl::new(f32::INFINITY, 0.0, 0.0).is_err());
        assert!(Hsl::new(0.0, 2.0, 0.0).is_err());
        assert!(Hsl::new(0.0, 0.0, -2.0).is_err());
        assert!(Curve::new(vec![[0.0, 0.0]]).is_err());
        assert!(Curve::new(vec![[0.0, 0.0]; 17]).is_err());
        assert!(Curve::new(vec![[0.5, 0.0], [0.5, 1.0]]).is_err());
        assert!(Curve::new(vec![[0.0, 0.0], [1.0, 1.5]]).is_err());
    }

    #[test]
    fn levels_hit_their_end_points_and_gamma() {
        let adj = Adjustments {
            levels: Levels::new(0.1, 0.6, 2.0).unwrap(),
            ..Adjustments::default()
        };
        let out = run(&adj, [0.1, 0.6, 0.35, 0.7]);
        assert!(out[0].abs() < 1e-4, "{out:?}");
        assert!((out[1] - 1.0).abs() < 1e-4, "{out:?}");
        // Halfway with gamma 2: 0.5^(1/2).
        assert!((out[2] - 0.5f32.sqrt()).abs() < 1e-3, "{out:?}");
        assert_eq!(out[3], 0.7);
    }

    #[test]
    fn table_path_equals_direct_evaluation() {
        let levels = Levels::new(0.2, 0.9, 0.7).unwrap();
        let bc = BrightnessContrast::new(-0.3, 0.4).unwrap();
        let g = Gamma::new(1.5).unwrap();
        let lut = Lut::build(ToneFn::Tone(g, levels, bc));
        for k in 0..=255u8 {
            let v = srgb8_to_linear(k);
            assert_eq!(lut.eval(v), bc.eval(levels.eval(g.eval(v))), "{k}");
        }
        // Off-table input falls back to direct evaluation.
        assert_eq!(lut.eval(0.123_457), bc.eval(levels.eval(g.eval(0.123_457))));
    }

    #[test]
    fn brightness_and_contrast_pivot_and_keep_black() {
        let bc = BrightnessContrast::new(0.0, 0.5).unwrap();
        assert!((bc.eval(MID_GREY) - MID_GREY).abs() < 1e-6);
        assert_eq!(bc.eval(0.0), 0.0);
        assert!(bc.eval(0.1) < 0.1 && bc.eval(0.3) > 0.3);
        let b = BrightnessContrast::new(0.5, 0.0).unwrap();
        assert!((b.eval(0.1) - 0.2).abs() < 1e-6);
    }

    #[test]
    fn curves_hit_their_control_points() {
        let pts = vec![[0.0, 0.1], [0.25, 0.4], [0.6, 0.65], [1.0, 0.9]];
        let c = Curve::new(pts.clone()).unwrap();
        for [x, y] in &pts {
            assert!((c.eval(*x) - y).abs() < 1e-6);
        }
        let adj = Adjustments {
            curves: Curves {
                green: c,
                ..Curves::default()
            },
            ..Adjustments::default()
        };
        for [x, y] in pts {
            let out = run(&adj, [x, x, x, 1.0]);
            assert!((out[0] - x).abs() < 2e-3, "{out:?}");
            assert!((out[1] - y).abs() < 2e-3, "{x}: {out:?}");
        }
    }

    #[test]
    fn master_then_channel() {
        let adj = Adjustments {
            curves: Curves {
                master: Curve::new(vec![[0.0, 0.0], [1.0, 0.5]]).unwrap(),
                red: Curve::new(vec![[0.0, 0.0], [0.5, 1.0]]).unwrap(),
                ..Curves::default()
            },
            ..Adjustments::default()
        };
        let out = run(&adj, [1.0, 1.0, 1.0, 1.0]);
        assert!(
            (out[0] - 1.0).abs() < 1e-3 && (out[1] - 0.5).abs() < 1e-3,
            "{out:?}"
        );
    }

    #[test]
    fn hsl_desaturates_and_rotates() {
        let grey = Adjustments {
            hsl: Hsl::new(0.0, -1.0, 0.0).unwrap(),
            ..Adjustments::default()
        };
        let out = run(&grey, [0.8, 0.2, 0.1, 1.0]);
        assert!(
            (out[0] - out[1]).abs() < 1e-4 && (out[1] - out[2]).abs() < 1e-4,
            "{out:?}"
        );
        let rot = Adjustments {
            hsl: Hsl::new(359.999, 0.0, 0.0).unwrap(),
            ..Adjustments::default()
        };
        let px = [0.4, 0.2, 0.1, 1.0];
        let out = run(&rot, px);
        for i in 0..3 {
            assert!((out[i] - px[i]).abs() < 1e-3, "{out:?}");
        }
        let lighter = Adjustments {
            hsl: Hsl::new(0.0, 0.0, 1.0).unwrap(),
            ..Adjustments::default()
        };
        let out = run(&lighter, [0.1, 0.3, 0.2, 1.0]);
        assert!(
            out.iter().take(3).all(|v| (v - 1.0).abs() < 1e-3),
            "{out:?}"
        );
    }

    #[test]
    fn gamma_is_png2c64s_power_and_runs_first() {
        assert!(Gamma::new(0.05).is_err() && Gamma::new(9.0).is_err());
        let adj = Adjustments {
            gamma: Gamma::PNG2C64,
            levels: Levels::new(0.0, 0.5, 1.0).unwrap(),
            ..Adjustments::default()
        };
        assert!(!adj.is_identity());
        let out = run(&adj, [0.25, 0.5, 1.0, 1.0]);
        // 0.25^1.5 = 0.125, then levels 0–0.5 doubles it.
        assert!((out[0] - 0.25).abs() < 1e-5, "{out:?}");
        assert!((out[1] - 0.5f32.powf(1.5) * 2.0).abs() < 1e-5, "{out:?}");
        let text = toml::to_string(&Adjustments {
            gamma: Gamma::PNG2C64,
            ..Adjustments::default()
        })
        .unwrap();
        assert!(
            text.contains("gamma = 1.5") && !text.contains("match_range"),
            "{text}"
        );
        let back: Adjustments = toml::from_str(&text).unwrap();
        assert_eq!(back.gamma, Gamma::PNG2C64);
    }

    #[test]
    fn range_matching_stretches_l_and_keeps_greys_grey() {
        let p = Palette::pepto_pal();
        let m = RangeMatch::for_palette(&p, 0.0, RangeMatch::DEFAULT_MARGIN).unwrap();
        assert!(m.l()[0] < 0.1 && m.l()[1] > 0.9, "{m:?}");
        // A dim grey ramp from 0.1 to 0.2 linear, plus one transparent pixel.
        let mut px: Vec<[f32; 4]> = (0..=10)
            .map(|i| {
                let v = 0.1 + 0.01 * i as f32;
                [v, v, v, 1.0]
            })
            .collect();
        px.push([0.9, 0.0, 0.0, 0.0]);
        m.apply(&mut px);
        let l = |q: [f32; 4]| {
            LinearRgb {
                r: q[0],
                g: q[1],
                b: q[2],
            }
            .to_oklab()
            .l
        };
        assert!((l(px[0]) - m.l()[0]).abs() < 1e-3, "{:?}", px[0]);
        assert!((l(px[10]) - m.l()[1]).abs() < 1e-3, "{:?}", px[10]);
        for q in &px[..11] {
            assert!(
                (q[0] - q[1]).abs() < 1e-3 && (q[1] - q[2]).abs() < 1e-3,
                "{q:?}"
            );
        }
        assert_eq!(px[11], [0.9, 0.0, 0.0, 0.0]);
        assert!(RangeMatch::new(0.6, [0.0, 1.0], [0.0, 0.0], [0.0, 0.0]).is_err());
        assert!(RangeMatch::new(0.01, [0.5, 0.4], [0.0, 0.0], [0.0, 0.0]).is_err());
    }

    #[test]
    fn auto_levels_suggests_only_for_narrow_sources() {
        let p = Palette::pepto_pal();
        let ramp = |lo: u8, hi: u8| {
            let bytes: Vec<u8> = (0..=255u16)
                .flat_map(|i| {
                    let v = (u16::from(lo) + (u16::from(hi - lo) * i) / 255) as u8;
                    [v, v, v, 255]
                })
                .collect();
            SourceImage::from_srgba8(256, 1, &bytes).unwrap()
        };
        assert!(suggest_range_match(&ramp(0, 255), &p).is_none());
        let narrow = suggest_range_match(&ramp(70, 140), &p).expect("narrow source");
        assert_eq!(narrow.percentile(), RangeMatch::DEFAULT_PERCENTILE);
    }

    #[test]
    fn cache_recomputes_only_on_change() {
        let s =
            Arc::new(SourceImage::from_srgba8(2, 1, &[10, 20, 30, 255, 200, 100, 0, 128]).unwrap());
        let mut cache = AdjustCache::default();
        let mut adj = Adjustments::default();
        let a = cache.get(&s, &adj);
        let b = cache.get(&s, &adj);
        assert!(Arc::ptr_eq(&a, &b));
        assert_eq!(cache.recomputes(), 1);
        assert_eq!(&a[..], s.pixels());
        adj.brightness_contrast = BrightnessContrast::new(0.2, 0.0).unwrap();
        let c = cache.get(&s, &adj);
        assert_eq!(cache.recomputes(), 2);
        assert!(c[0][0] > a[0][0]);
        assert_eq!(c[1][3], a[1][3]);
        cache.get(&s, &adj);
        assert_eq!(cache.recomputes(), 2);
        let other = Arc::new((*s).clone());
        cache.get(&other, &adj);
        assert_eq!(cache.recomputes(), 3);
    }

    #[test]
    fn serde_round_trip_through_constructors() {
        let adj = Adjustments {
            levels: Levels::new(0.05, 0.95, 1.2).unwrap(),
            curves: Curves {
                master: Curve::new(vec![[0.0, 0.0], [0.5, 0.6], [1.0, 1.0]]).unwrap(),
                ..Curves::default()
            },
            ..Adjustments::default()
        };
        let text = toml::to_string(&adj).unwrap();
        let back: Adjustments = toml::from_str(&text).unwrap();
        assert_eq!(back, adj);
        assert!(
            toml::from_str::<Adjustments>("[levels]\nblack = 0.9\nwhite = 0.1\ngamma = 1.0\n")
                .is_err()
        );
    }

    proptest! {
        #[test]
        fn monotone_data_gives_a_monotone_curve(
            mut xs in prop::collection::btree_set(1u16..999, 0..14),
            ys in prop::collection::vec(0.0f32..=1.0, 16),
        ) {
            xs.insert(0);
            xs.insert(1000);
            let xs: Vec<f32> = xs.into_iter().map(|x| f32::from(x) / 1000.0).collect();
            let mut ys: Vec<f32> = ys.into_iter().take(xs.len()).collect();
            ys.sort_by(f32::total_cmp);
            prop_assume!(ys.len() == xs.len());
            let c = Curve::new(xs.iter().zip(&ys).map(|(&x, &y)| [x, y]).collect()).unwrap();
            let mut prev = c.eval(0.0);
            for i in 1..=2000 {
                let v = c.eval(i as f32 / 2000.0);
                prop_assert!(v >= prev - 1e-6, "not monotone at {i}: {v} < {prev}");
                prev = v;
            }
        }
    }
}
