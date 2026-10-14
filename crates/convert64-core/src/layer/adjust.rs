//! Per-layer colour adjustments (UX §6, group 2). Every field defaults to identity.
//!
//! Applied in a fixed order, png2c64's: gamma → sharpen → levels → brightness/contrast →
//! HSL → curves → range matching. Gamma, levels, brightness/contrast and curves work per
//! channel in linear light and are baked into lookup tables; sharpen works on OKLab L
//! over a neighbourhood (#228); HSL works in OKLCh; range matching (png2c64
//! `--match-range`) remaps the whole image's OKLab extent and so runs over all pixels at
//! once. Alpha is never touched. Passes at their identity value are skipped, so identity
//! adjustments return the input bit for bit.

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

/// How range matching maps the source onto the palette.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RangeMethod {
    /// png2amiga's (ADR-013, #261): per (lightness, hue), each pixel's chroma scaled
    /// by the palette's reachable chroma over the source's 95th percentile, clipped at
    /// the palette's; hue and lightness kept (`png2amiga_ref::preprocess`).
    #[default]
    Hull,
    /// png2c64's: each OKLab axis stretched from the source's percentile range to the
    /// palette's bounding box, shrunk by a margin.
    Box,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RangeMatchData {
    #[serde(default)]
    method: Option<RangeMethod>,
    #[serde(default)]
    palette: Option<Vec<String>>,
    #[serde(default)]
    amount: Option<f32>,
    #[serde(default)]
    percentile: Option<f32>,
    #[serde(default)]
    l: Option<[f32; 2]>,
    #[serde(default)]
    a: Option<[f32; 2]>,
    #[serde(default)]
    b: Option<[f32; 2]>,
}

/// Range matching (png2c64's and png2amiga's `--match-range`), applied last.
///
/// - [`RangeMethod::Hull`] (since #261, the reference's): png2amiga's per-(L, hue)
///   chroma stretch onto the palette's reachable hull, with an **amount** (0–1) that
///   blends the source's chroma (0) with the matched (1, png2amiga's). The palette is
///   stored (its sRGB bytes) so a project file is self-contained.
/// - [`RangeMethod::Box`] (png2c64's, the only one before #261): the image's OKLab
///   extent, between the `percentile` and `1 − percentile` quantiles, mapped onto a
///   target extent: L stretched linearly, a and b scaled around zero so greys stay grey.
///   Projects saved before #261 hold this one and keep it.
///
/// On disk a box is `{ percentile, l, a, b }` as before #261; a hull is
/// `{ method = "hull", palette = [16 × "#rrggbb"], amount }`, `amount` written only when
/// not 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RangeMatch {
    method: RangeMethod,
    percentile: f32,
    l: [f32; 2],
    a: [f32; 2],
    b: [f32; 2],
    palette: [[u8; 3]; 16],
    amount: f32,
}

impl TryFrom<RangeMatchData> for RangeMatch {
    type Error = AdjustError;
    fn try_from(d: RangeMatchData) -> Result<Self, AdjustError> {
        let method = d.method.unwrap_or(if d.l.is_some() {
            RangeMethod::Box
        } else {
            RangeMethod::Hull
        });
        match method {
            RangeMethod::Box => {
                let missing = |what: &'static str| AdjustError::OutOfRange {
                    what,
                    value: f32::NAN,
                };
                Self::new(
                    d.percentile.unwrap_or(Self::DEFAULT_PERCENTILE),
                    d.l.ok_or_else(|| missing("range L"))?,
                    d.a.ok_or_else(|| missing("range a"))?,
                    d.b.ok_or_else(|| missing("range b"))?,
                )
            }
            RangeMethod::Hull => {
                let hex = d.palette.unwrap_or_default();
                let bad = AdjustError::OutOfRange {
                    what: "range palette (16 \"#rrggbb\")",
                    value: hex.len() as f32,
                };
                if hex.len() != 16 {
                    return Err(bad);
                }
                let mut palette = [[0u8; 3]; 16];
                for (slot, h) in palette.iter_mut().zip(&hex) {
                    let v = h
                        .strip_prefix('#')
                        .filter(|t| t.len() == 6)
                        .and_then(|t| u32::from_str_radix(t, 16).ok())
                        .ok_or(bad.clone())?;
                    let [_, r, g, b] = v.to_be_bytes();
                    *slot = [r, g, b];
                }
                Self::hull_bytes(palette, d.amount.unwrap_or(1.0))
            }
        }
    }
}

impl Serialize for RangeMatch {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        match self.method {
            RangeMethod::Box => {
                let mut st = s.serialize_struct("RangeMatch", 4)?;
                st.serialize_field("percentile", &self.percentile)?;
                st.serialize_field("l", &self.l)?;
                st.serialize_field("a", &self.a)?;
                st.serialize_field("b", &self.b)?;
                st.end()
            }
            RangeMethod::Hull => {
                let n = if self.amount == 1.0 { 2 } else { 3 };
                let mut st = s.serialize_struct("RangeMatch", n)?;
                st.serialize_field("method", &RangeMethod::Hull)?;
                let hex: Vec<String> = self
                    .palette
                    .iter()
                    .map(|[r, g, b]| format!("#{r:02x}{g:02x}{b:02x}"))
                    .collect();
                st.serialize_field("palette", &hex)?;
                if self.amount != 1.0 {
                    st.serialize_field("amount", &self.amount)?;
                }
                st.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for RangeMatch {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let data = RangeMatchData::deserialize(d)?;
        Self::try_from(data).map_err(serde::de::Error::custom)
    }
}

impl RangeMatch {
    /// The OKLab target ranges `(lo, hi)` of L, a and b (a box's).
    pub const fn targets(&self) -> [(f32, f32); 3] {
        [
            (self.l[0], self.l[1]),
            (self.a[0], self.a[1]),
            (self.b[0], self.b[1]),
        ]
    }

    /// png2c64's tail fraction ignored at each end, 1 %.
    pub const DEFAULT_PERCENTILE: f32 = 0.01;

    /// png2c64's inward margin on the palette extent, 5 %.
    pub const DEFAULT_MARGIN: f32 = 0.05;

    /// png2c64's box matching with `percentile` in `0..0.5` onto target ranges
    /// `[lo, hi]` of OKLab L, a and b (finite, `lo ≤ hi`).
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
            method: RangeMethod::Box,
            percentile,
            l,
            a,
            b,
            palette: [[0; 3]; 16],
            amount: 1.0,
        })
    }

    /// png2amiga's hull matching onto `palette` at `amount` (0–1; 1 is png2amiga's).
    pub fn hull(palette: &Palette, amount: f32) -> Result<Self, AdjustError> {
        Self::hull_bytes(palette.rgb_table().map(|c| [c.r, c.g, c.b]), amount)
    }

    fn hull_bytes(palette: [[u8; 3]; 16], amount: f32) -> Result<Self, AdjustError> {
        check("range amount", amount, 0.0, 1.0)?;
        Ok(Self {
            method: RangeMethod::Hull,
            percentile: Self::DEFAULT_PERCENTILE,
            l: [0.0, 1.0],
            a: [0.0, 0.0],
            b: [0.0, 0.0],
            palette,
            amount,
        })
    }

    /// The same matching with another amount (hull only; a box ignores it).
    pub fn with_amount(self, amount: f32) -> Result<Self, AdjustError> {
        match self.method {
            RangeMethod::Hull => Self::hull_bytes(self.palette, amount),
            RangeMethod::Box => Ok(self),
        }
    }

    /// png2c64's box matching onto `palette`: its OKLab extent shrunk by `margin` (a
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

    /// Which matching this is.
    pub const fn method(&self) -> RangeMethod {
        self.method
    }

    /// The hull's amount, 0–1 (1 for a box).
    pub const fn amount(&self) -> f32 {
        self.amount
    }

    /// The hull's palette in linear light (through png2c64's/png2amiga's sRGB table).
    pub fn palette_linear(&self) -> [[f32; 3]; 16] {
        self.palette
            .map(|[r, g, b]| crate::solve::png2amiga_ref::colour::srgb8_to_linear(r, g, b))
    }

    /// Tail fraction ignored at each end (a box's).
    pub const fn percentile(&self) -> f32 {
        self.percentile
    }

    /// Target L range (a box's).
    pub const fn l(&self) -> [f32; 2] {
        self.l
    }

    /// Target a range (a box's).
    pub const fn a(&self) -> [f32; 2] {
        self.a
    }

    /// Target b range (a box's).
    pub const fn b(&self) -> [f32; 2] {
        self.b
    }

    /// Matches linear RGB pixels (all of them measured and changed) in place, as the
    /// reference ports do: a box with png2c64's `match_range_to`, a hull with
    /// png2amiga's `match_palette_range` at the amount.
    pub fn apply_rgb(&self, px: &mut Vec<[f32; 3]>) {
        if px.is_empty() {
            return;
        }
        let n = px.len();
        match self.method {
            RangeMethod::Box => {
                let mut im = crate::solve::png2c64_ref::Image::new(n, 1, std::mem::take(px));
                crate::solve::png2c64_ref::preprocess::match_range_to(
                    &mut im,
                    self.percentile,
                    self.targets(),
                );
                *px = im.px;
            }
            RangeMethod::Hull => {
                let mut im = crate::solve::png2amiga_ref::Image::new(n, 1, std::mem::take(px));
                crate::solve::png2amiga_ref::preprocess::match_palette_range(
                    &mut im,
                    &self.palette_linear(),
                    self.amount,
                );
                *px = im.px;
            }
        }
    }

    /// Remaps `px` (straight-alpha linear RGBA) in place; fully transparent pixels are
    /// neither measured nor changed.
    pub fn apply(&self, px: &mut [[f32; 4]]) {
        if self.method == RangeMethod::Hull {
            let idx: Vec<usize> = (0..px.len()).filter(|&i| px[i][3] > 0.0).collect();
            let mut rgb: Vec<[f32; 3]> = idx
                .iter()
                .map(|&i| [px[i][0], px[i][1], px[i][2]])
                .collect();
            self.apply_rgb(&mut rgb);
            for (&i, c) in idx.iter().zip(rgb) {
                px[i][..3].copy_from_slice(&c);
            }
            return;
        }
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

/// An "Auto levels" suggestion for `source` (#142): range matching onto `palette`
/// (png2amiga's hull at amount 1 since #261) when the source's OKLab L range (1st–99th
/// percentile) covers less than 60 % of the palette's box extent with png2c64's
/// defaults. Only a suggestion: nothing applies it unless asked.
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
    (hi - lo < NARROW_RANGE * (m.l[1] - m.l[0]))
        .then(|| RangeMatch::hull(palette, 1.0).expect("amount 1 is valid"))
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

/// The blur a [`Sharpen`] subtracts (or, for a negative amount, blends toward).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Radius {
    /// png2c64's 3 × 3 box at the target grid, bit for bit; a negative amount blends
    /// toward three passes of it. The default.
    #[default]
    Png2c64,
    /// A Gaussian of this σ in target pixels, `0.3..=3` (fat pixels horizontally in
    /// multicolor, like feather σ, ADR-008), separable.
    Gaussian(f32),
}

impl Radius {
    /// A Gaussian of `sigma` target pixels, `0.3..=3`.
    pub fn gaussian(sigma: f32) -> Result<Self, AdjustError> {
        check("sharpen radius", sigma, 0.3, 3.0).map(Self::Gaussian)
    }
}

impl fmt::Display for Radius {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Png2c64 => f.write_str("png2c64"),
            Self::Gaussian(s) => write!(f, "{s}"),
        }
    }
}

impl std::str::FromStr for Radius {
    type Err = String;
    /// `png2c64`, or a σ in target pixels.
    fn from_str(s: &str) -> Result<Self, String> {
        if s.trim().eq_ignore_ascii_case("png2c64") {
            return Ok(Self::Png2c64);
        }
        let sigma: f32 = s
            .trim()
            .parse()
            .map_err(|_| format!("sharpen radius must be png2c64 or 0.3–3 px, got {s:?}"))?;
        Self::gaussian(sigma).map_err(|e| e.to_string())
    }
}

impl Serialize for Radius {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Png2c64 => s.serialize_str("png2c64"),
            Self::Gaussian(sigma) => s.serialize_f32(*sigma),
        }
    }
}

impl<'de> Deserialize<'de> for Radius {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Data {
            Sigma(f32),
            Name(String),
        }
        match Data::deserialize(d)? {
            Data::Sigma(sigma) => Self::gaussian(sigma).map_err(serde::de::Error::custom),
            Data::Name(name) => name.parse().map_err(serde::de::Error::custom),
        }
    }
}

#[derive(Deserialize)]
struct SharpenData {
    #[serde(default)]
    amount: f32,
    #[serde(default)]
    radius: Radius,
    #[serde(default)]
    threshold: f32,
}

/// An unsharp mask on OKLab lightness at the target grid (#228), after gamma and before
/// levels (png2c64's position). Never chroma: a and b are kept.
///
/// `amount` (`−1..=3`) is png2c64's `--sharpen`: positive adds `amount × (L − blur)`;
/// negative blends by `−amount` toward the blur (softening). `|amount| < 0.01` is off,
/// as in png2c64. `threshold` (`0..=0.1` in L) leaves a pixel alone where
/// `|L − blur|` is below it, so noise stays unsharpened; it applies to sharpening only,
/// and png2c64 has none (0). The blur is weighted by alpha, so a transparent surround
/// does not pull a halo into a layer's edge.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "SharpenData")]
pub struct Sharpen {
    amount: f32,
    radius: Radius,
    threshold: f32,
}

impl TryFrom<SharpenData> for Sharpen {
    type Error = AdjustError;
    fn try_from(d: SharpenData) -> Result<Self, AdjustError> {
        Self::new(d.amount, d.radius, d.threshold)
    }
}

impl Sharpen {
    /// Amount in `−1..=3`, threshold in `0..=0.1`; a Gaussian radius in `0.3..=3`.
    pub fn new(amount: f32, radius: Radius, threshold: f32) -> Result<Self, AdjustError> {
        if let Radius::Gaussian(sigma) = radius {
            Radius::gaussian(sigma)?;
        }
        Ok(Self {
            amount: check("sharpen amount", amount, -1.0, 3.0)?,
            radius,
            threshold: check("sharpen threshold", threshold, 0.0, 0.1)?,
        })
    }

    /// png2c64's `--sharpen amount`: its 3 × 3 kernel, no threshold.
    pub fn png2c64(amount: f32) -> Result<Self, AdjustError> {
        Self::new(amount, Radius::Png2c64, 0.0)
    }

    /// The amount, `−1..=3`.
    pub const fn amount(&self) -> f32 {
        self.amount
    }

    /// The blur.
    pub const fn radius(&self) -> Radius {
        self.radius
    }

    /// The threshold in OKLab L, `0..=0.1`.
    pub const fn threshold(&self) -> f32 {
        self.threshold
    }

    /// Whether it changes nothing (`|amount| < 0.01`, png2c64's dead zone).
    pub fn is_off(&self) -> bool {
        self.amount.abs() < 0.01
    }

    /// Whether it is png2c64's `--sharpen` exactly: its kernel and no threshold.
    pub fn is_png2c64(&self) -> bool {
        self.radius == Radius::Png2c64 && self.threshold == 0.0
    }

    fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// Sharpens `px` (straight-alpha linear RGBA, `w × h`, row-major) in place on a grid
    /// `scale ×` the target grid, so the radius stays in target pixels; alpha is
    /// untouched. See [`Self::apply_rgb`].
    pub fn apply(&self, px: &mut [[f32; 4]], size: (usize, usize), scale: u32) {
        if self.is_off() || px.is_empty() {
            return;
        }
        let alpha: Vec<f32> = px.iter().map(|p| p[3]).collect();
        let mut rgb: Vec<[f32; 3]> = px.iter().map(|p| [p[0], p[1], p[2]]).collect();
        self.apply_rgb(&mut rgb, &alpha, size, scale);
        for (p, c) in px.iter_mut().zip(&rgb) {
            p[..3].copy_from_slice(c);
        }
    }

    /// Sharpens `px` (straight linear RGB, `w × h`, row-major, with its `alpha`) in place
    /// on a grid `scale ×` the target grid. Pixels of alpha 0 are neither used nor
    /// changed. Each pixel the kernel changes goes through the reference's OKLab round
    /// trip and clamp, with png2amiga's OKLab conversions (ADR-013); with every alpha 1
    /// and `scale` 1, png2c64's radius (the 3 × 3 kernel png2amiga keeps) is png2amiga's
    /// `apply_sharpen` bit for bit.
    pub fn apply_rgb(
        &self,
        px: &mut [[f32; 3]],
        alpha: &[f32],
        (w, h): (usize, usize),
        scale: u32,
    ) {
        use crate::solve::png2amiga_ref::colour::{clamped, linear_to_oklab, oklab_to_linear};
        use crate::solve::png2c64_ref::preprocess::unsharp_box;
        if self.is_off() || px.is_empty() {
            return;
        }
        debug_assert_eq!(px.len(), w * h);
        debug_assert_eq!(alpha.len(), w * h);
        let lit: Vec<([f32; 3], f32)> = px.iter().copied().zip(alpha.iter().copied()).collect();
        let mut l: Vec<f32> =
            crate::par::map(
                &lit,
                |&(p, a)| {
                    if a > 0.0 { linear_to_oklab(p)[0] } else { 0.0 }
                },
            );
        let step = scale.max(1) as usize;
        let skipped = match self.radius {
            Radius::Png2c64 => unsharp_box(
                &mut l,
                Some(alpha),
                (w, h),
                step,
                self.amount,
                self.threshold,
            ),
            Radius::Gaussian(sigma) => unsharp_gaussian(
                &mut l,
                alpha,
                (w, h),
                sigma * step as f32,
                self.amount,
                self.threshold,
            ),
        };
        let mut rows: Vec<_> = px
            .chunks_mut(w)
            .zip(l.chunks(w).zip(skipped.chunks(w)))
            .map(|(p, (l, s))| (p, l, s))
            .collect();
        crate::par::for_each(&mut rows, |(p, l, s)| {
            for ((p, &lv), &skip) in p.iter_mut().zip(*l).zip(*s) {
                if skip {
                    continue;
                }
                let mut lab = linear_to_oklab(*p);
                lab[0] = lv.clamp(0.0, 1.0);
                *p = clamped(oklab_to_linear(lab));
            }
        });
    }
}

/// [`unsharp_box`](crate::solve::png2c64_ref::preprocess::unsharp_box) with a Gaussian
/// of `sigma` grid pixels instead of png2c64's box: separable, cut at 3σ, each tap
/// weighted by `weight` (alpha) and the sum normalised by the weights it met, so the
/// plane's edges and a transparent surround pull in nothing. A negative `strength`
/// blends by `−strength` (at most 1) toward the blur. Returns the pixels left alone
/// (weight 0, or under `threshold` when sharpening).
fn unsharp_gaussian(
    l: &mut [f32],
    weight: &[f32],
    (w, h): (usize, usize),
    sigma: f32,
    strength: f32,
    threshold: f32,
) -> Vec<bool> {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "3σ of at most 3 × 8 grid pixels"
    )]
    let r = (3.0 * sigma).ceil() as usize;
    let taps: Vec<f32> = (0..=r)
        .map(|k| {
            let k = k as f32;
            (-k * k / (2.0 * sigma * sigma)).exp()
        })
        .collect();
    // Blur the weighted lightness and the weights alike; their ratio is the blur.
    let num: Vec<f32> = l.iter().zip(weight).map(|(v, a)| v * a).collect();
    let pass = |src: &[f32], out: &mut [f32], horizontal: bool| {
        let (lines, len, line_stride, step) = if horizontal {
            (h, w, w, 1)
        } else {
            (w, h, 1, w)
        };
        for line in 0..lines {
            let base = line * line_stride;
            for i in 0..len {
                let mut acc = src[base + i * step] * taps[0];
                for (k, &t) in taps.iter().enumerate().skip(1) {
                    if i >= k {
                        acc += src[base + (i - k) * step] * t;
                    }
                    if i + k < len {
                        acc += src[base + (i + k) * step] * t;
                    }
                }
                out[base + i * step] = acc;
            }
        }
    };
    let blur = |src: &[f32]| {
        let mut tmp = vec![0.0f32; w * h];
        let mut out = vec![0.0f32; w * h];
        pass(src, &mut tmp, true);
        pass(&tmp, &mut out, false);
        out
    };
    let (bn, bw) = crate::par::join(|| blur(&num), || blur(weight));
    let amount = if strength > 0.0 {
        strength
    } else {
        (-strength).clamp(0.0, 1.0)
    };
    let mut skipped = vec![false; w * h];
    for (i, v) in l.iter_mut().enumerate() {
        if weight[i] <= 0.0 || bw[i] <= 0.0 {
            skipped[i] = true;
            continue;
        }
        let d = *v - bn[i] / bw[i];
        if strength > 0.0 {
            if d.abs() < threshold {
                skipped[i] = true;
                continue;
            }
            *v += d * amount;
        } else {
            *v -= d * amount;
        }
    }
    skipped
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
    /// Unsharp mask on lightness, after gamma (#228); off by default.
    #[serde(default, skip_serializing_if = "Sharpen::is_default")]
    pub sharpen: Sharpen,
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

/// [`Adjustments`] compiled: the separable passes as tables, the identity passes
/// dropped. Range matching is not part of the plan ([`Adjustments::apply_image`] runs
/// it after).
#[derive(Clone, Debug)]
pub struct AdjustPlan {
    /// Gamma alone, when a sharpen runs between it and the rest of `tone`.
    gamma: Option<Lut>,
    sharpen: Option<Sharpen>,
    /// Gamma (unless in `gamma`), levels, then brightness/contrast, the same for every
    /// channel.
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
        let (levels, bc) = (self.levels, self.brightness_contrast);
        let sharpen = (!self.sharpen.is_off()).then_some(self.sharpen);
        // With a sharpen between them, gamma gets its own table; otherwise one table
        // runs gamma, levels and brightness/contrast together, as before #228.
        let (gamma, pre) = match sharpen {
            Some(_) => (Gamma::IDENTITY, self.gamma),
            None => (self.gamma, Gamma::IDENTITY),
        };
        let gamma_lut = (pre != Gamma::IDENTITY).then(|| {
            Lut::build(ToneFn::Tone(
                pre,
                Levels::IDENTITY,
                BrightnessContrast::default(),
            ))
        });
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
        AdjustPlan {
            gamma: gamma_lut,
            sharpen,
            tone,
            hsl,
            curves,
        }
    }

    /// Applies every adjustment to `px` (straight-alpha linear RGBA, `w × h`,
    /// row-major) in place, in order, on a grid `scale ×` the target grid (the sharpen's
    /// radius is in target pixels). Pixels of alpha 0 are left alone.
    pub fn apply_image(&self, px: &mut [[f32; 4]], size: (usize, usize), scale: u32) {
        self.plan().apply_image(px, size, scale);
        if let Some(m) = &self.match_range {
            m.apply(px);
        }
    }
}

impl AdjustPlan {
    /// No pass is active.
    pub fn is_identity(&self) -> bool {
        self.gamma.is_none()
            && self.sharpen.is_none()
            && self.tone.is_none()
            && self.hsl.is_none()
            && self.curves.is_none()
    }

    /// Adjusts one straight-alpha linear pixel in place with the per-pixel passes; alpha
    /// is untouched. A sharpen needs the pixel's neighbours: [`Self::apply_image`] runs
    /// it.
    pub fn apply(&self, px: &mut [f32; 4]) {
        self.apply_pre(px);
        self.apply_post(px);
    }

    /// The passes before the sharpen: gamma, when it has its own table.
    fn apply_pre(&self, px: &mut [f32; 4]) {
        if let Some(t) = &self.gamma {
            for v in &mut px[..3] {
                *v = t.eval(*v);
            }
        }
    }

    /// The passes after the sharpen.
    fn apply_post(&self, px: &mut [f32; 4]) {
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

    /// Every pass, the sharpen included, over an image of `w × h` straight-alpha
    /// pixels on a grid `scale ×` the target grid; pixels of alpha 0 are left alone.
    pub fn apply_image(&self, px: &mut [[f32; 4]], size: (usize, usize), scale: u32) {
        let Some(sharpen) = &self.sharpen else {
            if !self.is_identity() {
                crate::par::for_each(px, |p| {
                    if p[3] > 0.0 {
                        self.apply_post(p);
                    }
                });
            }
            return;
        };
        if self.gamma.is_some() {
            crate::par::for_each(px, |p| {
                if p[3] > 0.0 {
                    self.apply_pre(p);
                }
            });
        }
        sharpen.apply(px, size, scale);
        crate::par::for_each(px, |p| {
            if p[3] > 0.0 {
                self.apply_post(p);
            }
        });
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
    /// unchanged (one copy into the shared buffer, made once). Fully transparent pixels
    /// are left alone.
    pub fn get(&mut self, source: &Arc<SourceImage>, adj: &Adjustments) -> Arc<[[f32; 4]]> {
        let hit = matches!(&self.key, Some((s, a)) if Arc::ptr_eq(s, source) && a == adj);
        if !hit || self.pixels.is_none() {
            let mut px = source.pixels().to_vec();
            let (w, h) = source.size();
            // On the source's own pixels, one per target pixel for the sharpen's radius.
            adj.apply_image(&mut px, (w as usize, h as usize), 1);
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
        assert_eq!(narrow.method(), RangeMethod::Hull);
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

    /// A deterministic pseudo-random linear colour per pixel, all opaque.
    fn noise(w: usize, h: usize, seed: u32) -> Vec<[f32; 4]> {
        let mut s = seed;
        let mut next = move || {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            (s >> 8) as f32 / (1u32 << 24) as f32
        };
        (0..w * h).map(|_| [next(), next(), next(), 1.0]).collect()
    }

    fn lab(p: [f32; 4]) -> [f32; 3] {
        crate::solve::png2amiga_ref::colour::linear_to_oklab([p[0], p[1], p[2]])
    }

    fn from_lab(l: [f32; 3], a: f32) -> [f32; 4] {
        let [r, g, b] = crate::solve::png2amiga_ref::colour::oklab_to_linear(l);
        [r, g, b, a]
    }

    /// #228: amount 0 (any radius) is off, and the adjustments return their input bit
    /// for bit, as without a sharpen.
    #[test]
    fn sharpen_amount_zero_is_bit_identical() {
        let (w, h) = (23, 17);
        let src = noise(w, h, 7);
        for radius in [Radius::Png2c64, Radius::gaussian(1.2).unwrap()] {
            for threshold in [0.0, 0.05] {
                let adj = Adjustments {
                    sharpen: Sharpen::new(0.0, radius, threshold).unwrap(),
                    ..Adjustments::default()
                };
                assert!(adj.is_identity());
                let mut px = src.clone();
                adj.apply_image(&mut px, (w, h), 4);
                assert_eq!(px, src);
                // With gamma too: the same as gamma alone, bit for bit.
                let both = Adjustments {
                    gamma: Gamma::PNG2C64,
                    ..adj
                };
                let gamma = Adjustments {
                    gamma: Gamma::PNG2C64,
                    ..Adjustments::default()
                };
                let (mut x, mut y) = (src.clone(), src.clone());
                both.apply_image(&mut x, (w, h), 1);
                gamma.apply_image(&mut y, (w, h), 1);
                assert_eq!(x, y);
            }
        }
        assert!(
            Sharpen::png2c64(0.005).unwrap().is_off(),
            "png2c64's dead zone"
        );
    }

    /// #228: the png2c64 kernel at the target grid is png2amiga's `apply_sharpen` (the
    /// same kernel, png2amiga's OKLab conversions; ADR-013), bit for bit, both ways
    /// (sharpen and soften), after the same gamma.
    #[test]
    fn png2c64_kernel_equals_png2amiga_s_apply_sharpen() {
        use crate::solve::png2amiga_ref::{Image, preprocess};
        let (w, h) = (31, 19);
        let src = noise(w, h, 0x2545_f491);
        for amount in [0.5, 1.7, 3.0, -0.5, -1.0] {
            let adj = Adjustments {
                gamma: Gamma::PNG2C64,
                sharpen: Sharpen::png2c64(amount).unwrap(),
                ..Adjustments::default()
            };
            let mut ours = src.clone();
            adj.apply_image(&mut ours, (w, h), 1);
            let mut im = Image::new(w, h, src.iter().map(|p| [p[0], p[1], p[2]]).collect());
            for p in &mut im.px {
                *p = p.map(|v| Gamma::PNG2C64.eval(v));
            }
            preprocess::apply_sharpen(&mut im, amount);
            for (i, (a, b)) in ours.iter().zip(&im.px).enumerate() {
                assert_eq!(
                    [a[0], a[1], a[2]].map(f32::to_bits),
                    b.map(f32::to_bits),
                    "amount {amount}, pixel {i}"
                );
            }
        }
    }

    /// #228: a Gaussian sharpen of a lightness step overshoots on both sides of the
    /// edge and leaves chroma (OKLab a, b) alone.
    #[test]
    fn gaussian_step_response_changes_lightness_only() {
        let (w, h) = (24, 6);
        let (dark, light) = ([0.45, 0.03, 0.02], [0.65, 0.03, 0.02]);
        let src: Vec<[f32; 4]> = (0..w * h)
            .map(|i| from_lab(if i % w < w / 2 { dark } else { light }, 1.0))
            .collect();
        for scale in [1, 4] {
            let (sw, sh) = (w * scale, h * scale);
            let up: Vec<[f32; 4]> = (0..sw * sh)
                .map(|i| src[(i / sw / scale) * w + (i % sw) / scale])
                .collect();
            let mut px = up.clone();
            Sharpen::new(1.0, Radius::gaussian(0.8).unwrap(), 0.0)
                .unwrap()
                .apply(&mut px, (sw, sh), scale as u32);
            let row = sh / 2 * sw;
            let (before, after) = (lab(px[row + sw / 2 - 1]), lab(px[row + sw / 2]));
            assert!(
                before[0] < dark[0] - 0.02,
                "{scale}: no dark overshoot {before:?}"
            );
            assert!(
                after[0] > light[0] + 0.02,
                "{scale}: no light overshoot {after:?}"
            );
            for (p, q) in px.iter().zip(&up) {
                let (a, b) = (lab(*p), lab(*q));
                assert!(
                    (a[1] - b[1]).abs() < 1e-4 && (a[2] - b[2]).abs() < 1e-4,
                    "{scale}: chroma moved, {a:?} from {b:?}"
                );
                assert_eq!(p[3], q[3]);
            }
            // Far from the edge (3σ = 2.4 target px), nothing changes.
            for i in [row, row + sw - 1] {
                assert!((lab(px[i])[0] - lab(up[i])[0]).abs() < 1e-6, "{scale}: {i}");
            }
        }
    }

    /// #228: under the threshold, a noisy patch stays exactly as it was while a real
    /// edge next to it is still sharpened.
    #[test]
    fn threshold_leaves_noise_untouched() {
        let (w, h) = (40, 20);
        let mut seed = 99u32;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            (seed >> 8) as f32 / (1u32 << 24) as f32
        };
        // Left: grey noise of ±0.004 in L; right: a hard step.
        let src: Vec<[f32; 4]> = (0..w * h)
            .map(|i| {
                let x = i % w;
                let l = if x < 20 {
                    0.5 + 0.008 * (next() - 0.5)
                } else if x < 30 {
                    0.3
                } else {
                    0.8
                };
                from_lab([l, 0.0, 0.0], 1.0)
            })
            .collect();
        for radius in [Radius::Png2c64, Radius::gaussian(1.0).unwrap()] {
            let mut px = src.clone();
            Sharpen::new(2.0, radius, 0.02)
                .unwrap()
                .apply(&mut px, (w, h), 1);
            // png2c64's kernel leaves its border to the OKLab round trip, as png2c64.
            for y in 1..h - 1 {
                for x in 1..17 {
                    assert_eq!(px[y * w + x], src[y * w + x], "{radius}: noise at {x},{y}");
                }
            }
            let mid = h / 2 * w;
            assert!(lab(px[mid + 29])[0] < 0.29, "{radius}: edge not sharpened");
            assert!(lab(px[mid + 30])[0] > 0.81, "{radius}: edge not sharpened");
            // Without the threshold the noise is sharpened too.
            let mut all = src.clone();
            Sharpen::new(2.0, radius, 0.0)
                .unwrap()
                .apply(&mut all, (w, h), 1);
            assert_ne!(all[mid + 5], src[mid + 5], "{radius}");
        }
    }

    /// #228: a layer's edge against a transparent surround gets no halo. The blur is
    /// weighted by alpha, so a flat layer with a soft (partly covered) border is
    /// unchanged and the transparent pixels stay as they were.
    #[test]
    fn transparent_surround_pulls_in_no_halo() {
        let (w, h) = (30, 30);
        let grey = from_lab([0.6, 0.02, -0.01], 1.0);
        let src: Vec<[f32; 4]> = (0..w * h)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                let inside = |v: usize| (10..20).contains(&v);
                let edge = |v: usize| v == 9 || v == 20;
                if inside(x) && inside(y) {
                    grey
                } else if (inside(x) || edge(x)) && (inside(y) || edge(y)) {
                    [grey[0], grey[1], grey[2], 0.4]
                } else {
                    [0.0; 4]
                }
            })
            .collect();
        for scale in [1u32, 2] {
            let s = scale as usize;
            let (sw, sh) = (w * s, h * s);
            let up: Vec<[f32; 4]> = (0..sw * sh)
                .map(|i| src[(i / sw / s) * w + (i % sw) / s])
                .collect();
            for sharpen in [
                Sharpen::png2c64(3.0).unwrap(),
                Sharpen::png2c64(-1.0).unwrap(),
                Sharpen::new(3.0, Radius::gaussian(2.0).unwrap(), 0.0).unwrap(),
            ] {
                let mut px = up.clone();
                sharpen.apply(&mut px, (sw, sh), scale);
                for (i, (p, q)) in px.iter().zip(&up).enumerate() {
                    assert_eq!(p[3], q[3]);
                    if q[3] == 0.0 {
                        assert_eq!(p, q, "{sharpen:?}: transparent pixel {i} changed");
                    } else {
                        let (a, b) = (lab(*p), lab(*q));
                        assert!(
                            (a[0] - b[0]).abs() < 1e-5,
                            "{sharpen:?} ×{scale}: halo at {i}: L {} from {}",
                            a[0],
                            b[0]
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn sharpen_serde_defaults_and_validation() {
        // An old project's adjustments have no sharpen: they load with it off, and an
        // off sharpen is not written.
        let old: Adjustments = toml::from_str("gamma = 1.5\n").unwrap();
        assert_eq!(old.sharpen, Sharpen::default());
        assert!(old.sharpen.is_off() && old.sharpen.is_png2c64());
        let text = toml::to_string(&old).unwrap();
        assert!(!text.contains("sharpen"), "{text}");
        for sharpen in [
            Sharpen::png2c64(-0.5).unwrap(),
            Sharpen::new(1.25, Radius::gaussian(0.8).unwrap(), 0.02).unwrap(),
        ] {
            let adj = Adjustments {
                sharpen,
                ..Adjustments::default()
            };
            let text = toml::to_string(&adj).unwrap();
            assert_eq!(toml::from_str::<Adjustments>(&text).unwrap(), adj, "{text}");
        }
        let parsed: Sharpen = toml::from_str("amount = 0.5\nradius = \"png2c64\"\n").unwrap();
        assert_eq!(parsed, Sharpen::png2c64(0.5).unwrap());
        let parsed: Sharpen = toml::from_str("amount = 1\nradius = 2\n").unwrap();
        assert_eq!(parsed.radius(), Radius::Gaussian(2.0));
        assert_eq!("0.8".parse::<Radius>(), Ok(Radius::Gaussian(0.8)));
        assert_eq!("PNG2C64".parse::<Radius>(), Ok(Radius::Png2c64));
        assert!("0.1".parse::<Radius>().is_err() && "wide".parse::<Radius>().is_err());
        assert!(Sharpen::png2c64(3.5).is_err() && Sharpen::png2c64(-1.5).is_err());
        assert!(Sharpen::new(1.0, Radius::Png2c64, 0.2).is_err());
        assert!(Sharpen::new(1.0, Radius::Gaussian(5.0), 0.0).is_err());
        assert!(toml::from_str::<Sharpen>("amount = 9\n").is_err());
        assert!(toml::from_str::<Sharpen>("radius = 0.1\n").is_err());
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
