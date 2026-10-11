//! Layer masks (ADR-008): one raster per layer in layer space at the source's
//! resolution, procedural sources combined with it, and parametric modifiers applied at
//! composite time in the fixed order Invert → Levels → SnapToCells → Feather.
//!
//! Effective alpha = PNG alpha × mask; the compositor also multiplies the attribute
//! map's dither strength by the same mask (ADR-002), so dither fades across a feather.

pub(crate) mod eval;
mod source;

pub use eval::HALF_RES_FEATHER;
pub use source::{Combine, Gradient, GradientKind, Key, KeyChannel, MaskSource, SourceEntry};

use crate::geom::Vec2;
use crate::layer::{Levels, SourceImage};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

/// A fresh, process-wide unique generation number. Caches key on it to notice that a
/// raster changed without comparing pixels.
pub(crate) fn next_generation() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// Why a mask value could not be built.
#[derive(Clone, Debug, PartialEq)]
pub enum MaskError {
    /// Width or height is zero.
    Empty,
    /// The value buffer has the wrong length for the size.
    WrongLength {
        /// `width × height`.
        expected: usize,
        /// Values given.
        found: usize,
    },
    /// A value is NaN or outside `0..=1`.
    Value(f32),
    /// The raster's size differs from the layer's source.
    Size {
        /// The layer source's size.
        expected: (u32, u32),
        /// The raster's size.
        found: (u32, u32),
    },
    /// A source or modifier parameter is out of range.
    Param {
        /// Which parameter.
        what: &'static str,
        /// The value given (NaN for a non-finite point).
        value: f32,
    },
    /// A source index does not exist.
    NoSource(usize),
}

impl fmt::Display for MaskError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("a mask raster needs at least one pixel"),
            Self::WrongLength { expected, found } => {
                write!(f, "mask raster needs {expected} values, found {found}")
            }
            Self::Value(v) => write!(f, "mask values must be in 0–1, found {v}"),
            Self::Size { expected, found } => write!(
                f,
                "mask is {}×{} but the layer source is {}×{}",
                found.0, found.1, expected.0, expected.1
            ),
            Self::Param { what, value } => write!(f, "{what} is out of range: {value}"),
            Self::NoSource(i) => write!(f, "the mask has no source {i}"),
        }
    }
}

impl std::error::Error for MaskError {}

/// A greyscale raster of weights `0..=1` in layer space: f32 in memory, u16 on disk
/// (ADR-008).
#[derive(Clone, Debug, PartialEq)]
pub struct MaskRaster {
    width: u32,
    height: u32,
    data: Vec<f32>,
}

impl MaskRaster {
    /// A `width × height` raster with every value `value` (clamped to `0..=1`).
    pub fn filled(width: u32, height: u32, value: f32) -> Result<Self, MaskError> {
        if width == 0 || height == 0 {
            return Err(MaskError::Empty);
        }
        if value.is_nan() {
            return Err(MaskError::Value(value));
        }
        let len = width as usize * height as usize;
        Ok(Self {
            width,
            height,
            data: vec![value.clamp(0.0, 1.0); len],
        })
    }

    /// A raster from row-major values, each in `0..=1`.
    pub fn from_values(width: u32, height: u32, data: Vec<f32>) -> Result<Self, MaskError> {
        if width == 0 || height == 0 {
            return Err(MaskError::Empty);
        }
        let expected = width as usize * height as usize;
        if data.len() != expected {
            return Err(MaskError::WrongLength {
                expected,
                found: data.len(),
            });
        }
        if let Some(&v) = data.iter().find(|v| !(0.0..=1.0).contains(*v)) {
            return Err(MaskError::Value(v));
        }
        Ok(Self {
            width,
            height,
            data,
        })
    }

    /// A raster from row-major 16-bit values (the on-disk form), `0` → 0, `65535` → 1.
    pub fn from_u16(width: u32, height: u32, data: &[u16]) -> Result<Self, MaskError> {
        Self::from_values(
            width,
            height,
            data.iter().map(|&v| f32::from(v) / 65535.0).collect(),
        )
    }

    /// The values as 16-bit integers (the on-disk form), rounded to nearest.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "values are in 0..=1, so the product is in 0..=65535"
    )]
    pub fn to_u16(&self) -> Vec<u16> {
        self.data
            .iter()
            .map(|&v| (v * 65535.0).round() as u16)
            .collect()
    }

    /// Width in layer pixels.
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Height in layer pixels.
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// `(width, height)`.
    pub const fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Row-major values.
    pub fn values(&self) -> &[f32] {
        &self.data
    }

    /// The value at `(x, y)`; panics outside the raster.
    pub fn get(&self, x: u32, y: u32) -> f32 {
        assert!(
            x < self.width && y < self.height,
            "({x}, {y}) is outside the mask"
        );
        self.data[y as usize * self.width as usize + x as usize]
    }
}

/// Quantising the mask edge to whole target cells (ADR-008).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SnapToCells {
    /// No snapping.
    #[default]
    Off,
    /// A cell takes the highest mask value in it.
    Expand,
    /// A cell takes the lowest mask value in it.
    Contract,
    /// A cell takes the highest value if its mean is at least ½, else the lowest.
    Nearest,
}

/// Largest feather σ, in target pixels (ADR-008).
pub const MAX_FEATHER: f32 = 32.0;

#[derive(Deserialize)]
struct ModifiersData {
    #[serde(default)]
    invert: bool,
    #[serde(default)]
    levels: Levels,
    #[serde(default)]
    snap: SnapToCells,
    #[serde(default)]
    feather: f32,
}

/// Parametric mask modifiers, never baked into the raster: dragging a slider from 6 to
/// 4 loses nothing.
#[derive(Clone, Copy, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(try_from = "ModifiersData")]
pub struct Modifiers {
    invert: bool,
    levels: Levels,
    snap: SnapToCells,
    feather: f32,
}

impl TryFrom<ModifiersData> for Modifiers {
    type Error = MaskError;
    fn try_from(d: ModifiersData) -> Result<Self, MaskError> {
        Self::new(d.invert, d.levels, d.snap, d.feather)
    }
}

impl Modifiers {
    /// Modifiers; `feather` is a Gaussian σ in target pixels, `0..=32`.
    pub fn new(
        invert: bool,
        levels: Levels,
        snap: SnapToCells,
        feather: f32,
    ) -> Result<Self, MaskError> {
        if !(feather.is_finite() && (0.0..=MAX_FEATHER).contains(&feather)) {
            return Err(MaskError::Param {
                what: "feather",
                value: feather,
            });
        }
        Ok(Self {
            invert,
            levels,
            snap,
            feather,
        })
    }

    /// Only a feather.
    pub fn feathered(feather: f32) -> Result<Self, MaskError> {
        Self::new(false, Levels::IDENTITY, SnapToCells::Off, feather)
    }

    /// Invert the mask first.
    pub const fn invert(&self) -> bool {
        self.invert
    }

    /// Levels after invert.
    pub const fn levels(&self) -> Levels {
        self.levels
    }

    /// Snap after levels.
    pub const fn snap(&self) -> SnapToCells {
        self.snap
    }

    /// Feather σ in target pixels, last.
    pub const fn feather(&self) -> f32 {
        self.feather
    }

    /// Whether every modifier is off.
    pub fn is_identity(&self) -> bool {
        *self == Self::default()
    }
}

/// A layer's mask (ADR-008). Effective alpha = PNG alpha × mask; a layer without a mask
/// behaves as mask = 1.
#[derive(Clone, Debug, PartialEq)]
pub struct Mask {
    raster: MaskRaster,
    sources: Vec<SourceEntry>,
    modifiers: Modifiers,
    /// Changes whenever the raster or the sources do (not the modifiers, which caches
    /// compare by value).
    generation: u64,
}

impl Mask {
    /// A mask with this raster, no sources and no modifiers.
    pub fn new(raster: MaskRaster) -> Self {
        Self {
            raster,
            sources: Vec::new(),
            modifiers: Modifiers::default(),
            generation: next_generation(),
        }
    }

    /// A fully opaque (all 1) mask of the given size, what **Add mask** creates.
    pub fn opaque(width: u32, height: u32) -> Result<Self, MaskError> {
        MaskRaster::filled(width, height, 1.0).map(Self::new)
    }

    /// The painted raster.
    pub const fn raster(&self) -> &MaskRaster {
        &self.raster
    }

    /// Size of the raster.
    pub const fn size(&self) -> (u32, u32) {
        self.raster.size()
    }

    /// The procedural sources, applied in order after the raster.
    pub fn sources(&self) -> &[SourceEntry] {
        &self.sources
    }

    /// The modifiers.
    pub const fn modifiers(&self) -> Modifiers {
        self.modifiers
    }

    /// Replaces the modifiers.
    pub fn set_modifiers(&mut self, m: Modifiers) {
        self.modifiers = m;
    }

    /// Appends a source.
    pub fn push_source(&mut self, s: SourceEntry) {
        self.sources.push(s);
        self.generation = next_generation();
    }

    /// Removes source `i`.
    pub fn remove_source(&mut self, i: usize) -> Result<SourceEntry, MaskError> {
        if i >= self.sources.len() {
            return Err(MaskError::NoSource(i));
        }
        self.generation = next_generation();
        Ok(self.sources.remove(i))
    }

    /// Replaces the sources.
    pub fn set_sources(&mut self, sources: Vec<SourceEntry>) {
        self.sources = sources;
        self.generation = next_generation();
    }

    /// Replaces the raster with one of the same size.
    pub fn set_raster(&mut self, raster: MaskRaster) -> Result<(), MaskError> {
        if raster.size() != self.raster.size() {
            return Err(MaskError::Size {
                expected: self.raster.size(),
                found: raster.size(),
            });
        }
        self.raster = raster;
        self.generation = next_generation();
        Ok(())
    }

    /// The raster's and sources' generation: equal generations mean unchanged layer-space
    /// content.
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// The raster combined with every source, in layer space. `source` is the layer's
    /// own (unadjusted) pixels, which keys sample; it must be the mask's size.
    pub fn layer_values<'a>(&'a self, source: &SourceImage) -> Cow<'a, [f32]> {
        if self.sources.is_empty() {
            return Cow::Borrowed(self.raster.values());
        }
        assert_eq!(source.size(), self.size(), "mask and source sizes differ");
        let w = self.raster.width as usize;
        let mut out = self.raster.data.clone();
        let mut rows: Vec<(usize, &mut [f32])> = out.chunks_mut(w).enumerate().collect();
        crate::par::for_each(&mut rows, |(y, row)| {
            for (x, v) in row.iter_mut().enumerate() {
                let p = Vec2::new(x as f32 + 0.5, *y as f32 + 0.5);
                for e in &self.sources {
                    let s = match &e.source {
                        MaskSource::Gradient(g) => g.value(p),
                        MaskSource::Key(k) => k.value(source.pixels()[*y * w + x]),
                    };
                    *v = e.combine.apply(*v, s);
                }
            }
        });
        Cow::Owned(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constructors_validate() {
        assert_eq!(MaskRaster::filled(0, 4, 1.0), Err(MaskError::Empty));
        assert_eq!(MaskRaster::filled(4, 0, 1.0), Err(MaskError::Empty));
        assert!(matches!(
            MaskRaster::filled(1, 1, f32::NAN),
            Err(MaskError::Value(_))
        ));
        assert_eq!(
            MaskRaster::from_values(2, 2, vec![0.0; 3]),
            Err(MaskError::WrongLength {
                expected: 4,
                found: 3
            })
        );
        assert_eq!(
            MaskRaster::from_values(1, 1, vec![1.5]),
            Err(MaskError::Value(1.5))
        );
        assert!(matches!(
            MaskRaster::from_values(1, 1, vec![f32::NAN]),
            Err(MaskError::Value(_))
        ));
    }

    #[test]
    fn u16_round_trip() {
        let data: Vec<u16> = (0..=65535).step_by(257).map(|v| v as u16).collect();
        let r = MaskRaster::from_u16(data.len() as u32, 1, &data).unwrap();
        assert_eq!(r.to_u16(), data);
    }
}
