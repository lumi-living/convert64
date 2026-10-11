//! Procedural mask sources (ADR-008): gradients and keys that stay editable and are
//! combined with the painted raster in layer space.

use super::MaskError;
use crate::geom::Vec2;
use crate::palette::{LinearRgb, Oklab};
use serde::{Deserialize, Serialize};

/// How a source combines with the raster (and the sources before it).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Combine {
    /// The larger of the two (union).
    #[default]
    Max,
    /// The smaller of the two (intersection).
    Min,
    /// The product.
    Multiply,
}

impl Combine {
    /// `a` combined with `b`.
    pub fn apply(self, a: f32, b: f32) -> f32 {
        match self {
            Self::Max => a.max(b),
            Self::Min => a.min(b),
            Self::Multiply => a * b,
        }
    }
}

/// Shape of a gradient.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GradientKind {
    /// Along the line `from → to`.
    Linear,
    /// Around `from`, reaching 0 at distance `|to − from|`.
    Radial,
}

fn finite_point(what: &'static str, p: Vec2) -> Result<Vec2, MaskError> {
    if p.is_finite() {
        Ok(p)
    } else {
        Err(MaskError::Param {
            what,
            value: f32::NAN,
        })
    }
}

fn param(what: &'static str, v: f32, lo: f32, hi: f32) -> Result<f32, MaskError> {
    if v.is_finite() && (lo..=hi).contains(&v) {
        Ok(v)
    } else {
        Err(MaskError::Param { what, value: v })
    }
}

#[derive(Deserialize)]
struct GradientData {
    kind: GradientKind,
    from: Vec2,
    to: Vec2,
    #[serde(default = "one")]
    softness: f32,
}

const fn one() -> f32 {
    1.0
}

/// A gradient in layer pixels: 1 at `from`, 0 at `to`.
///
/// With position `t` (0 at `from`, 1 at `to`), the value is
/// `1 − clamp((t − ½) / softness + ½)`: softness 1 is a straight ramp over the whole
/// distance, smaller values steepen it around the midpoint, and 0 is a hard edge there.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "GradientData")]
pub struct Gradient {
    kind: GradientKind,
    from: Vec2,
    to: Vec2,
    softness: f32,
}

impl TryFrom<GradientData> for Gradient {
    type Error = MaskError;
    fn try_from(d: GradientData) -> Result<Self, MaskError> {
        Self::new(d.kind, d.from, d.to, d.softness)
    }
}

impl Gradient {
    /// A gradient; `from` and `to` must be finite and distinct, softness in `0..=1`.
    pub fn new(kind: GradientKind, from: Vec2, to: Vec2, softness: f32) -> Result<Self, MaskError> {
        finite_point("gradient from", from)?;
        finite_point("gradient to", to)?;
        let d = to - from;
        if d.x == 0.0 && d.y == 0.0 {
            return Err(MaskError::Param {
                what: "gradient length",
                value: 0.0,
            });
        }
        Ok(Self {
            kind,
            from,
            to,
            softness: param("gradient softness", softness, 0.0, 1.0)?,
        })
    }

    /// Shape.
    pub const fn kind(&self) -> GradientKind {
        self.kind
    }

    /// Start point (value 1), layer pixels.
    pub const fn from(&self) -> Vec2 {
        self.from
    }

    /// End point (value 0), layer pixels.
    pub const fn to(&self) -> Vec2 {
        self.to
    }

    /// Softness, `0..=1`.
    pub const fn softness(&self) -> f32 {
        self.softness
    }

    /// The value at layer point `p`.
    pub fn value(&self, p: Vec2) -> f32 {
        let d = self.to - self.from;
        let q = p - self.from;
        let len2 = d.x * d.x + d.y * d.y;
        let t = match self.kind {
            GradientKind::Linear => (q.x * d.x + q.y * d.y) / len2,
            GradientKind::Radial => ((q.x * q.x + q.y * q.y) / len2).sqrt(),
        };
        let ramp = if self.softness == 0.0 {
            if t < 0.5 { 0.0 } else { 1.0 }
        } else {
            ((t - 0.5) / self.softness + 0.5).clamp(0.0, 1.0)
        };
        1.0 - ramp
    }
}

/// What a key measures, all in OKLab.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum KeyChannel {
    /// Lightness difference `|ΔL|`.
    Luma,
    /// Chromaticity difference `√(Δa² + Δb²)`, ignoring lightness (a chroma key).
    Chroma,
    /// Full colour difference `√(ΔL² + Δa² + Δb²)`.
    Colour,
}

#[derive(Deserialize)]
struct KeyData {
    channel: KeyChannel,
    centre: [f32; 3],
    range: f32,
    #[serde(default)]
    softness: f32,
}

/// A key on the layer's own source pixels: 1 where a pixel is within `range` of
/// `centre` (a linear sRGB colour, typically sampled under the cursor), falling to 0 over
/// a further `softness`. Distances are OKLab units (L is 0–1).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "KeyData")]
pub struct Key {
    channel: KeyChannel,
    centre: [f32; 3],
    range: f32,
    softness: f32,
    #[serde(skip)]
    centre_lab: [f32; 3],
}

impl TryFrom<KeyData> for Key {
    type Error = MaskError;
    fn try_from(d: KeyData) -> Result<Self, MaskError> {
        Self::new(d.channel, d.centre, d.range, d.softness)
    }
}

fn lab(rgb: [f32; 3]) -> [f32; 3] {
    let Oklab { l, a, b } = LinearRgb {
        r: rgb[0],
        g: rgb[1],
        b: rgb[2],
    }
    .to_oklab();
    [l, a, b]
}

impl Key {
    /// A key; `centre` channels in `0..=1`, `range` and `softness` in `0..=2`.
    pub fn new(
        channel: KeyChannel,
        centre: [f32; 3],
        range: f32,
        softness: f32,
    ) -> Result<Self, MaskError> {
        for v in centre {
            param("key centre", v, 0.0, 1.0)?;
        }
        Ok(Self {
            channel,
            centre,
            range: param("key range", range, 0.0, 2.0)?,
            softness: param("key softness", softness, 0.0, 2.0)?,
            centre_lab: lab(centre),
        })
    }

    /// What is measured.
    pub const fn channel(&self) -> KeyChannel {
        self.channel
    }

    /// The keyed colour, linear sRGB.
    pub const fn centre(&self) -> [f32; 3] {
        self.centre
    }

    /// Full-strength distance.
    pub const fn range(&self) -> f32 {
        self.range
    }

    /// Fall-off distance beyond the range.
    pub const fn softness(&self) -> f32 {
        self.softness
    }

    /// The value for a straight-alpha linear source pixel.
    pub fn value(&self, px: [f32; 4]) -> f32 {
        let [l, a, b] = lab([px[0], px[1], px[2]]);
        let [cl, ca, cb] = self.centre_lab;
        let d = match self.channel {
            KeyChannel::Luma => (l - cl).abs(),
            KeyChannel::Chroma => (a - ca).hypot(b - cb),
            KeyChannel::Colour => ((l - cl).powi(2) + (a - ca).powi(2) + (b - cb).powi(2)).sqrt(),
        };
        if d <= self.range {
            1.0
        } else if self.softness > 0.0 {
            (1.0 - (d - self.range) / self.softness).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }
}

/// A procedural contribution to a mask.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "type")]
pub enum MaskSource {
    /// A linear or radial gradient.
    Gradient(Gradient),
    /// A luma, chroma or colour key.
    Key(Key),
}

/// A source and how it combines with what is below it.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SourceEntry {
    /// The source.
    pub source: MaskSource,
    /// The combine mode.
    #[serde(default)]
    pub combine: Combine,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gradients() {
        let g = Gradient::new(GradientKind::Linear, Vec2::ZERO, Vec2::new(10.0, 0.0), 1.0).unwrap();
        assert_eq!(g.value(Vec2::new(0.0, 5.0)), 1.0);
        assert_eq!(g.value(Vec2::new(10.0, -3.0)), 0.0);
        assert!((g.value(Vec2::new(2.5, 0.0)) - 0.75).abs() < 1e-6);
        let hard =
            Gradient::new(GradientKind::Linear, Vec2::ZERO, Vec2::new(10.0, 0.0), 0.0).unwrap();
        assert_eq!(hard.value(Vec2::new(4.9, 0.0)), 1.0);
        assert_eq!(hard.value(Vec2::new(5.0, 0.0)), 0.0);
        let r = Gradient::new(
            GradientKind::Radial,
            Vec2::new(5.0, 5.0),
            Vec2::new(5.0, 9.0),
            1.0,
        )
        .unwrap();
        assert_eq!(r.value(Vec2::new(5.0, 5.0)), 1.0);
        assert!((r.value(Vec2::new(7.0, 5.0)) - 0.5).abs() < 1e-6);
        assert_eq!(r.value(Vec2::new(0.0, 0.0)), 0.0);
        assert!(Gradient::new(GradientKind::Radial, Vec2::ONE, Vec2::ONE, 1.0).is_err());
        assert!(Gradient::new(GradientKind::Linear, Vec2::ZERO, Vec2::ONE, 1.5).is_err());
    }

    #[test]
    fn keys() {
        let green = [0.0, 1.0, 0.0];
        let k = Key::new(KeyChannel::Colour, green, 0.05, 0.1).unwrap();
        assert_eq!(k.value([0.0, 1.0, 0.0, 1.0]), 1.0);
        assert_eq!(k.value([1.0, 0.0, 1.0, 1.0]), 0.0);
        // Luma ignores hue: a grey of the same lightness matches.
        let l = Key::new(KeyChannel::Luma, [0.5, 0.5, 0.5], 0.01, 0.0).unwrap();
        assert_eq!(l.value([0.5, 0.5, 0.5, 1.0]), 1.0);
        assert_eq!(l.value([0.1, 0.1, 0.1, 1.0]), 0.0);
        // Chroma ignores lightness: a darker green still matches loosely.
        let c = Key::new(KeyChannel::Chroma, green, 0.15, 0.0).unwrap();
        assert_eq!(c.value([0.0, 0.6, 0.0, 1.0]), 1.0);
        assert!(Key::new(KeyChannel::Luma, [1.5, 0.0, 0.0], 0.1, 0.0).is_err());
        assert!(Key::new(KeyChannel::Luma, green, -0.1, 0.0).is_err());
    }

    #[test]
    fn combine() {
        assert_eq!(Combine::Max.apply(0.2, 0.7), 0.7);
        assert_eq!(Combine::Min.apply(0.2, 0.7), 0.2);
        assert!((Combine::Multiply.apply(0.5, 0.5) - 0.25).abs() < 1e-7);
    }
}
