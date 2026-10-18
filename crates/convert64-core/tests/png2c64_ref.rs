//! The reference solver against png2c64's own output (#181).

use convert64_core::solve::png2c64_ref::colour::{PEPTO_HEX, palette_linear};
use convert64_core::solve::png2c64_ref::quantize::{Mode, Screen, quantize};
use convert64_core::solve::png2c64_ref::scale::{Kernel, bicubic};
use convert64_core::solve::png2c64_ref::{Image, preprocess};
use std::path::PathBuf;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A PNG as png2c64 loads it (8-bit sRGB through its table), scaled to `mode`'s screen
/// with its Mitchell bicubic and preprocessed with its defaults (gamma 1.5).
fn prepared(path: &str, mode: Mode) -> Image {
    let img = image::open(repo().join(path))
        .unwrap_or_else(|e| panic!("{path}: {e}"))
        .to_rgb8();
    let (w, h) = (img.width() as usize, img.height() as usize);
    let rgb: Vec<[u8; 3]> = img.pixels().map(|p| p.0).collect();
    let src = Image::from_srgb8(w, h, &rgb);
    let (tw, th) = mode.screen();
    let mut im = if (w, h) == (tw, th) {
        src
    } else {
        bicubic(&src, tw, th, Kernel::MitchellNetravali)
    };
    preprocess::apply(&mut im, &preprocess::Settings::default());
    im
}

/// A cell as png2c64 encodes it: colours by slot and each pixel's slot.
type KoalaCell = ([u8; 4], Vec<u8>);

/// png2c64's cells from a Koala file: `[bg, screen hi, screen lo, colour RAM]` and each
/// pixel's bit pair.
fn koala_cells(path: &str) -> (u8, Vec<KoalaCell>) {
    let bytes = std::fs::read(repo().join(path)).expect("koala file");
    let d = &bytes[2..];
    let bg = d[10_000] & 15;
    let cells = (0..1000)
        .map(|i| {
            let scr = d[8000 + i];
            let colors = [bg, scr >> 4, scr & 15, d[9000 + i] & 15];
            let px = (0..32)
                .map(|p| (d[i * 8 + p / 4] >> (6 - 2 * (p % 4))) & 3)
                .collect();
            (colors, px)
        })
        .collect();
    (bg, cells)
}

/// png2c64's `checker` threshold at strength `s` (`ordered_threshold × strength`).
fn checker(s: f32) -> impl Fn(usize, usize) -> f32 + Sync {
    move |x, y| {
        if (x + y) % 2 == 0 {
            -0.25 * s
        } else {
            0.25 * s
        }
    }
}

fn agreement(ours: &Screen, theirs: &[KoalaCell]) -> (usize, usize) {
    let sets = ours
        .cells
        .iter()
        .zip(theirs)
        .filter(|(a, (c, _))| a.colors[1..] == c[1..])
        .count();
    let pixels = ours
        .cells
        .iter()
        .zip(theirs)
        .map(|(a, (_, px))| a.px.iter().zip(px).filter(|(x, y)| x == y).count())
        .sum();
    (sets, pixels)
}

#[test]
fn the_portrait_s_background_and_cell_sets_are_png2c64_s() {
    let im = prepared("fixtures/portrait/source.png", Mode::Multicolor);
    let pal = palette_linear(&PEPTO_HEX);
    let thr = checker(0.7);
    let screen = quantize(&im, &pal, Mode::Multicolor, Some(&thr), 0.7);
    let (bg, theirs) = koala_cells("fixtures/portrait/baseline.kla");
    let (sets, pixels) = agreement(&screen, &theirs);
    println!(
        "portrait checker 0.7: background {} (png2c64 {bg}), cell sets {sets}/1000, pixels {:.3} %",
        screen.bg,
        100.0 * pixels as f64 / 32_000.0
    );
    assert_eq!(screen.bg, bg, "background");
    assert!(sets >= 990, "only {sets} of 1000 cell sets are png2c64's");
}

/// A png2c64 render (320 × 200, Pepto colours) as palette indices on `mode`'s grid.
fn render_indices(path: &str, mode: Mode) -> Vec<u8> {
    let img = image::open(repo().join(path))
        .unwrap_or_else(|e| panic!("{path}: {e}"))
        .to_rgb8();
    let step = 320 / mode.screen().0;
    let (w, h) = mode.screen();
    (0..w * h)
        .map(|i| {
            let p = img.get_pixel((i % w * step) as u32, (i / w) as u32).0;
            let hex = u32::from_be_bytes([0, p[0], p[1], p[2]]);
            PEPTO_HEX
                .iter()
                .position(|&c| c == hex)
                .unwrap_or_else(|| panic!("{path}: {hex:06x} is not a Pepto colour"))
                as u8
        })
        .collect()
}

fn percent_equal(a: &[u8], b: &[u8]) -> f64 {
    100.0 * a.iter().zip(b).filter(|(x, y)| x == y).count() as f64 / a.len() as f64
}

#[test]
fn lineart_hires_none_is_png2c64_s_global_fs_choice() {
    let im = prepared("fixtures/lineart/source.png", Mode::Hires);
    let screen = quantize(&im, &palette_linear(&PEPTO_HEX), Mode::Hires, None, 0.0);
    let pct = percent_equal(
        &screen.indices(),
        &render_indices("fixtures/lineart/baseline.png", Mode::Hires),
    );
    println!("lineart hi-res none: {pct:.3} %");
    assert!(pct >= 99.0, "{pct:.3} %");
}

#[test]
fn the_card_s_multicolor_none_is_png2c64_s_global_fs_choice() {
    let im = prepared(
        "docs/reference/dither-previews/testcard.png",
        Mode::Multicolor,
    );
    let screen = quantize(
        &im,
        &palette_linear(&PEPTO_HEX),
        Mode::Multicolor,
        None,
        0.0,
    );
    let pct = percent_equal(
        &screen.indices(),
        &render_indices(
            "docs/reference/dither-previews/png2c64-card/none.png",
            Mode::Multicolor,
        ),
    );
    println!("card multicolor none: {pct:.3} %");
    assert!(pct >= 99.0, "{pct:.3} %");
}

#[test]
fn the_card_s_diffusion_methods_are_png2c64_s() {
    use convert64_core::solve::png2c64_ref::convert;
    use convert64_core::solve::png2c64_ref::dither::{Method, Settings};
    let im = prepared(
        "docs/reference/dither-previews/testcard.png",
        Mode::Multicolor,
    );
    let pal = palette_linear(&PEPTO_HEX);
    for m in Method::ALL.into_iter().filter(|m| !m.is_ordered()) {
        let s = Settings {
            method: m,
            strength: 1.0,
            ..Settings::default()
        };
        let ours = convert(&im, &pal, Mode::Multicolor, &s).indices();
        let theirs = render_indices(
            &format!(
                "docs/reference/dither-previews/png2c64-card/{}.png",
                m.name()
            ),
            Mode::Multicolor,
        );
        let pct = percent_equal(&ours, &theirs);
        println!("card {}: {pct:.3} %", m.name());
        assert!(pct >= 99.0, "{}: {pct:.3} %", m.name());
    }
}
