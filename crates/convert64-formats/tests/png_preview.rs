//! PNG preview export on the six fixtures (#160): palette-exact, integer zoom 1–8, 2:1
//! aspect for multicolor, byte-stable.

mod common;

use common::{FIXTURES, MODES, baseline, fixture_dir, solve_all, solved};
use convert64_core::Palette;
use convert64_core::model::IndexedBitmap;
use convert64_formats::png::{MAX_ZOOM, preview, size};

fn decode(png: &[u8]) -> image::RgbImage {
    image::load_from_memory_with_format(png, image::ImageFormat::Png)
        .expect("a valid PNG")
        .to_rgb8()
}

/// The golden harness's rendering (`crates/convert64-core/tests/golden.rs`, `render`):
/// 320 × 200, each logical pixel's palette colour, multicolor pixels doubled.
fn harness_render(img: &IndexedBitmap, palette: &Palette) -> image::RgbImage {
    let x_scale = 320 / u32::from(img.width());
    image::RgbImage::from_fn(320, 200, |x, y| {
        let c = palette.rgb(img.pixel(
            u16::try_from(x / x_scale).expect("x"),
            u16::try_from(y).expect("y"),
        ));
        image::Rgb([c.r, c.g, c.b])
    })
}

/// At 1× the preview of every png2c64 baseline is png2c64's own 320 × 200 preview
/// (`baseline.png`, Pepto PAL) pixel for pixel, and the harness's rendering.
#[test]
fn baselines_at_1x_match_png2c64_and_the_harness() {
    let pepto = Palette::pepto_pal();
    for (name, format) in FIXTURES {
        let image = baseline(name, format);
        let got = decode(&preview(&image, &pepto, 1).expect("zoom 1"));
        let png2c64 = image::open(fixture_dir(name).join("baseline.png"))
            .expect("baseline.png")
            .to_rgb8();
        assert_eq!(got.dimensions(), (320, 200), "{name}");
        assert!(got == png2c64, "{name}: differs from baseline.png");
        assert!(
            got == harness_render(&image.to_indexed(), &pepto),
            "{name}: differs from the harness render"
        );
    }
}

/// Our own solves, both modes: the 1× preview is the harness's rendering, and every
/// zoom is the 1× preview with each pixel repeated `zoom × zoom` times, and the same
/// picture always gives the same bytes.
#[test]
fn solves_preview_at_every_zoom() {
    solve_all();
    let pepto = Palette::pepto_pal();
    let colours: Vec<[u8; 3]> = pepto.rgb_table().iter().map(|c| [c.r, c.g, c.b]).collect();
    for (name, _) in FIXTURES {
        for mode in MODES {
            let image = solved(name, mode);
            let one = decode(&preview(&image, &pepto, 1).expect("zoom 1"));
            assert!(
                one == harness_render(&image.to_indexed(), &pepto),
                "{name} {mode}"
            );
            assert!(
                one.pixels().all(|p| colours.contains(&p.0)),
                "{name} {mode}"
            );
            // The largest zoom on one picture per mode: encoding 2560 × 1600 is slow in
            // debug builds.
            let zooms: &[u8] = if name == "portrait" {
                &[2, 3, MAX_ZOOM]
            } else {
                &[2, 3]
            };
            for &zoom in zooms {
                let png = preview(&image, &pepto, zoom).expect("legal zoom");
                assert_eq!(
                    png,
                    preview(&image, &pepto, zoom).expect("again"),
                    "byte-stable"
                );
                let big = decode(&png);
                assert_eq!(big.dimensions(), size(zoom));
                let z = u32::from(zoom);
                assert!(
                    big.enumerate_pixels()
                        .all(|(x, y, p)| p == one.get_pixel(x / z, y / z)),
                    "{name} {mode} zoom {zoom}"
                );
            }
        }
    }
}

/// The preview uses the palette object it is given, not a built-in one.
#[test]
fn the_given_palette_is_used() {
    let colodore = Palette::colodore(50.0, 100.0, 50.0);
    let image = baseline("portrait", convert64_formats::BitmapFormat::Koala);
    let got = decode(&preview(&image, &colodore, 1).expect("zoom 1"));
    assert!(got == harness_render(&image.to_indexed(), &colodore));
    assert!(got != harness_render(&image.to_indexed(), &Palette::pepto_pal()));
}
