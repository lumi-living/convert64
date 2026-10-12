//! PNG preview export (`docs/reference/file-formats.md`, "PNG preview").
//!
//! The preview shows the picture as the VIC-II displays it: every pixel is exactly one
//! of the 16 colours of the palette object passed in (the solve's own, CLAUDE.md rule
//! 5), never resampled or filtered. Multicolor pixels are twice as wide as tall (each is
//! `2 × zoom` PNG pixels wide and `zoom` tall), so both modes give a `320·zoom ×
//! 200·zoom` image. The PNG is 8-bit RGB: the `image` crate's encoder cannot write an
//! indexed (palette) PNG, and no other PNG dependency is allowed (TAD §4).

use convert64_core::Palette;
use convert64_core::model::{BitmapImage, IndexedBitmap};
use std::fmt;

/// The largest zoom factor.
pub const MAX_ZOOM: u8 = 8;

/// The display size at zoom 1: 320 × 200 in both modes.
pub const DISPLAY: (u32, u32) = (320, 200);

/// Why a preview could not be made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreviewError {
    /// The zoom is not 1 to [`MAX_ZOOM`].
    Zoom(u8),
    /// The PNG encoder failed (it writes to memory, so this should not happen).
    Encode(String),
}

impl fmt::Display for PreviewError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Zoom(z) => write!(f, "zoom must be 1 to {MAX_ZOOM}, not {z}"),
            Self::Encode(e) => write!(f, "PNG encoding failed: {e}"),
        }
    }
}

impl std::error::Error for PreviewError {}

/// The preview's size for `zoom`: `(320 · zoom, 200 · zoom)`.
pub const fn size(zoom: u8) -> (u32, u32) {
    (DISPLAY.0 * zoom as u32, DISPLAY.1 * zoom as u32)
}

/// The preview as 8-bit RGB pixels, row by row, `size(zoom)` large.
///
/// # Errors
/// [`PreviewError::Zoom`] unless `1 <= zoom <= MAX_ZOOM`.
pub fn rgb(image: &IndexedBitmap, palette: &Palette, zoom: u8) -> Result<Vec<u8>, PreviewError> {
    if !(1..=MAX_ZOOM).contains(&zoom) {
        return Err(PreviewError::Zoom(zoom));
    }
    let (w, h) = size(zoom);
    let z = u32::from(zoom);
    // Display pixels per logical pixel horizontally: 2 for multicolor, 1 for hi-res.
    let x_scale = DISPLAY.0 / u32::from(image.width());
    let mut out = Vec::with_capacity((w * h * 3) as usize);
    let mut row = Vec::with_capacity((w * 3) as usize);
    for y in 0..DISPLAY.1 {
        row.clear();
        for x in 0..DISPLAY.0 {
            let c = palette.rgb(image.pixel(
                u16::try_from(x / x_scale).unwrap_or(0),
                u16::try_from(y).unwrap_or(0),
            ));
            for _ in 0..z {
                row.extend([c.r, c.g, c.b]);
            }
        }
        for _ in 0..z {
            out.extend_from_slice(&row);
        }
    }
    Ok(out)
}

/// Encodes 8-bit RGB pixels of `size(zoom)` as a PNG with the `image` crate's default
/// settings (no metadata chunks, so the same pixels always give the same bytes).
fn encode(pixels: &[u8], (w, h): (u32, u32)) -> Result<Vec<u8>, PreviewError> {
    use image::ImageEncoder as _;
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(pixels, w, h, image::ExtendedColorType::Rgb8)
        .map_err(|e| PreviewError::Encode(e.to_string()))?;
    Ok(png)
}

/// A PNG preview of a pixel-level picture (the solver's output).
///
/// # Errors
/// [`PreviewError::Zoom`] unless `1 <= zoom <= MAX_ZOOM`.
pub fn preview_indexed(
    image: &IndexedBitmap,
    palette: &Palette,
    zoom: u8,
) -> Result<Vec<u8>, PreviewError> {
    encode(&rgb(image, palette, zoom)?, size(zoom))
}

/// A PNG preview of an encoded bitmap picture: `320·zoom × 200·zoom`, every pixel a
/// colour of `palette`, multicolor pixels `2·zoom` wide.
///
/// # Errors
/// [`PreviewError::Zoom`] unless `1 <= zoom <= MAX_ZOOM`.
pub fn preview(image: &BitmapImage, palette: &Palette, zoom: u8) -> Result<Vec<u8>, PreviewError> {
    preview_indexed(&image.to_indexed(), palette, zoom)
}

#[cfg(test)]
mod tests {
    use super::*;
    use convert64_core::ColourIndex;
    use convert64_core::mode::Mode;

    fn decode(png: &[u8]) -> image::RgbImage {
        image::load_from_memory_with_format(png, image::ImageFormat::Png)
            .expect("valid PNG")
            .to_rgb8()
    }

    /// A multicolor picture whose pixel (x, y) has colour (x + y) % 4 from a fixed set.
    fn stripes(mode: Mode) -> IndexedBitmap {
        let (w, h) = if mode == Mode::MulticolorBitmap {
            (160, 200)
        } else {
            (320, 200)
        };
        let colours = if mode == Mode::MulticolorBitmap {
            [
                ColourIndex::BLACK,
                ColourIndex::WHITE,
                ColourIndex::RED,
                ColourIndex::CYAN,
            ]
        } else {
            [
                ColourIndex::BLACK,
                ColourIndex::WHITE,
                ColourIndex::BLACK,
                ColourIndex::WHITE,
            ]
        };
        let pixels = (0..h)
            .flat_map(|y| (0..w).map(move |x| colours[(x + y) % 4]))
            .collect();
        let bg = (mode == Mode::MulticolorBitmap).then_some(ColourIndex::BLACK);
        IndexedBitmap::new(mode, pixels, bg).expect("legal")
    }

    #[test]
    fn zoom_is_1_to_8() {
        let img = stripes(Mode::HiresBitmap);
        let p = Palette::pepto_pal();
        for bad in [0, 9, 255] {
            assert_eq!(preview_indexed(&img, &p, bad), Err(PreviewError::Zoom(bad)));
        }
        assert_eq!(
            PreviewError::Zoom(9).to_string(),
            "zoom must be 1 to 8, not 9"
        );
        for zoom in 1..=MAX_ZOOM {
            let png = decode(&preview_indexed(&img, &p, zoom).expect("legal zoom"));
            assert_eq!(png.dimensions(), size(zoom));
        }
    }

    /// Every preview pixel is the palette colour of the logical pixel it shows, at the
    /// right aspect.
    #[test]
    fn pixels_are_palette_exact_at_the_right_aspect() {
        let p = Palette::colodore(50.0, 100.0, 50.0);
        for mode in [Mode::MulticolorBitmap, Mode::HiresBitmap] {
            let img = stripes(mode);
            let px_w = if mode == Mode::MulticolorBitmap { 2 } else { 1 };
            for zoom in [1, 3] {
                let png = decode(&preview_indexed(&img, &p, zoom).expect("legal"));
                let z = u32::from(zoom);
                for (x, y, got) in png.enumerate_pixels() {
                    let lx = u16::try_from(x / (px_w * z)).expect("x");
                    let ly = u16::try_from(y / z).expect("y");
                    let want = p.rgb(img.pixel(lx, ly));
                    assert_eq!(
                        got.0,
                        [want.r, want.g, want.b],
                        "{mode} zoom {zoom} at {x},{y}"
                    );
                }
            }
        }
    }

    #[test]
    fn output_is_deterministic() {
        let img = stripes(Mode::MulticolorBitmap);
        let p = Palette::pepto_pal();
        assert_eq!(
            preview_indexed(&img, &p, 2).expect("legal"),
            preview_indexed(&img, &p, 2).expect("legal")
        );
    }
}
