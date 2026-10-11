//! Layer masks (ADR-008): one raster per layer in layer space at the source's
//! resolution.

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

/// A layer's mask (ADR-008). Effective alpha = PNG alpha × mask; a layer without a mask
/// behaves as mask = 1.
#[derive(Clone, Debug, PartialEq)]
pub struct Mask {
    raster: MaskRaster,
    /// Changes whenever the raster does.
    generation: u64,
}

impl Mask {
    /// A mask with this raster.
    pub fn new(raster: MaskRaster) -> Self {
        Self {
            raster,
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

    /// The raster's generation: equal generations mean an unchanged raster.
    pub const fn generation(&self) -> u64 {
        self.generation
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
