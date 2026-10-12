//! PNG decoding and encoding for layer sources and mask rasters, in memory: the bytes
//! come from and go to a [`Store`](crate::Store).

use crate::error::{ProjectError, invalid};
use convert64_core::layer::SourceImage;
use convert64_core::mask::MaskRaster;
use convert64_core::palette::linear_to_srgb8;

/// Decodes PNG (or other supported image) bytes into a layer source.
pub fn decode_source(bytes: &[u8], what: &str) -> Result<SourceImage, ProjectError> {
    let img = image::load_from_memory(bytes)
        .map_err(|e| invalid(format!("{what}: {e}")))?
        .to_rgba8();
    let (w, h) = img.dimensions();
    SourceImage::from_srgba8(w, h, img.as_raw()).map_err(|e| invalid(format!("{what}: {e}")))
}

/// Decodes greyscale PNG bytes (any bit depth; colour becomes luma) into a mask raster.
pub fn decode_mask(bytes: &[u8], what: &str) -> Result<MaskRaster, ProjectError> {
    let img = image::load_from_memory(bytes)
        .map_err(|e| invalid(format!("{what}: {e}")))?
        .to_luma16();
    let (w, h) = img.dimensions();
    MaskRaster::from_u16(w, h, img.as_raw()).map_err(|e| invalid(format!("{what}: {e}")))
}

fn png_bytes(img: image::DynamicImage) -> Vec<u8> {
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png)
        .unwrap_or_else(|e| unreachable!("PNG encoding into memory: {e}"));
    out.into_inner()
}

/// 16-bit greyscale values, row-major, as a PNG.
pub fn encode_luma16(width: u32, height: u32, values: Vec<u16>) -> Vec<u8> {
    let img = image::ImageBuffer::<image::Luma<u16>, _>::from_raw(width, height, values)
        .expect("width × height values");
    png_bytes(image::DynamicImage::ImageLuma16(img))
}

/// 8-bit RGBA, row-major, as a PNG.
pub fn encode_rgba8(width: u32, height: u32, rgba: Vec<u8>) -> Vec<u8> {
    let img = image::RgbaImage::from_raw(width, height, rgba).expect("width × height pixels");
    png_bytes(image::DynamicImage::ImageRgba8(img))
}

/// A mask raster as a 16-bit greyscale PNG (its on-disk form, ADR-008).
pub fn encode_mask(raster: &MaskRaster) -> Vec<u8> {
    let (w, h) = raster.size();
    encode_luma16(w, h, raster.to_u16())
}

/// A layer source as an 8-bit sRGB RGBA PNG (for sources with no original file).
pub fn encode_source(src: &SourceImage) -> Vec<u8> {
    let (w, h) = src.size();
    let rgba = src
        .pixels()
        .iter()
        .flat_map(|p| {
            [
                linear_to_srgb8(p[0]),
                linear_to_srgb8(p[1]),
                linear_to_srgb8(p[2]),
                (p[3].clamp(0.0, 1.0) * 255.0).round() as u8,
            ]
        })
        .collect();
    encode_rgba8(w, h, rgba)
}
