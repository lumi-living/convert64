//! 16-bit sources (#327): a 16-bit PNG keeps its precision from the file to the
//! compositor and through an embedded save, and an 8-bit PNG takes the path it always
//! took, byte for byte.

use convert64_core::layer::SourceImage;
use convert64_project::codec::{decode_source, encode_source};
use convert64_project::{Embed, MemoryStore, Session};
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;

/// A 256 × 256 RGBA PNG at 16 bits per channel whose red channel takes every one of the
/// 65 536 values once; green and blue run the other way and at another stride, alpha is
/// opaque.
fn gradient16() -> Vec<u8> {
    let values: Vec<u16> = (0..=u16::MAX)
        .flat_map(|v| [v, u16::MAX - v, v.wrapping_mul(7), u16::MAX])
        .collect();
    let img = image::ImageBuffer::<image::Rgba<u16>, _>::from_raw(256, 256, values).unwrap();
    png(image::DynamicImage::ImageRgba16(img))
}

fn png(img: image::DynamicImage) -> Vec<u8> {
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

/// The colour type a PNG decodes to.
fn colour_type(bytes: &[u8]) -> image::ColorType {
    image::load_from_memory(bytes).unwrap().color()
}

/// How many different red values the source holds.
fn distinct_red(src: &SourceImage) -> usize {
    src.pixels()
        .iter()
        .map(|p| p[0].to_bits())
        .collect::<BTreeSet<_>>()
        .len()
}

#[test]
fn a_16_bit_png_keeps_its_precision() {
    let src = decode_source(&gradient16(), "gradient").unwrap();
    assert_eq!(distinct_red(&src), 65_536);
    // No 8-bit bytes for png2c64's front end: the general path takes it (ADR-012).
    assert!(src.srgb8().is_none());
    // Written back as 16-bit, and decoded again to the same pixels.
    let again = encode_source(&src);
    assert_eq!(colour_type(&again), image::ColorType::Rgba16);
    assert_eq!(
        decode_source(&again, "again").unwrap().pixels(),
        src.pixels()
    );
}

#[test]
fn a_16_bit_source_survives_an_embedded_save_and_load() {
    let store = Arc::new(MemoryStore::new());
    store.insert("proj/sky.png", gradient16());
    store.insert(
        "proj/recipe.json",
        r#"{ "version": 1, "target": "multicolor-bitmap",
             "layers": [{ "name": "sky", "source": "sky.png" }] }"#,
    );
    let s = Session::open_in(Box::new(Arc::clone(&store)), Path::new("proj/recipe.json")).unwrap();
    let pixels = s.project().layers[0].source().pixels().to_vec();
    s.save_to_store(Path::new("proj/e.convert64"), Embed::All)
        .unwrap();
    let alone = Arc::new(MemoryStore::new());
    alone.insert("e.convert64", store.get("proj/e.convert64").unwrap());
    let e = Session::open_in(Box::new(alone), Path::new("e.convert64")).unwrap();
    let src = e.project().layers[0].source();
    assert_eq!(distinct_red(src), 65_536);
    assert_eq!(src.pixels(), pixels.as_slice());
}

#[test]
fn an_8_bit_png_decodes_and_encodes_as_before() {
    let rgba: Vec<u8> = (0..64u32 * 40)
        .flat_map(|i| [(i % 256) as u8, (i / 7 % 256) as u8, 200, 255])
        .collect();
    let img = image::RgbaImage::from_raw(64, 40, rgba.clone()).unwrap();
    let src = decode_source(&png(image::DynamicImage::ImageRgba8(img)), "eight").unwrap();
    assert_eq!(src, SourceImage::from_srgba8(64, 40, &rgba).unwrap());
    assert!(src.srgb8().is_some());
    assert_eq!(colour_type(&encode_source(&src)), image::ColorType::Rgba8);
}
