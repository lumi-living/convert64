//! Shared helpers for the formats crate's integration tests: the six fixtures, their
//! png2c64 baselines and our own solves of them.

#![allow(dead_code, reason = "each test binary uses a different subset")]

use convert64_core::geom::Vec2;
use convert64_core::layer::{
    Adjustments, DitherKind, Gamma, Layer, Recipe, SourceImage, Transform,
};
use convert64_core::mode::Mode;
use convert64_core::model::BitmapImage;
use convert64_core::solve::{SolveOptions, Solver, Steering};
use convert64_core::{Compositor, Palette, SuperGrid};
use convert64_formats::BitmapFormat;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

/// The six fixtures and the format of their png2c64 baseline.
pub const FIXTURES: [(&str, BitmapFormat); 6] = [
    ("portrait", BitmapFormat::Koala),
    ("terrain", BitmapFormat::Koala),
    ("title", BitmapFormat::Koala),
    ("gradient", BitmapFormat::Koala),
    ("photo", BitmapFormat::Koala),
    ("lineart", BitmapFormat::ArtStudio),
];

/// Both bitmap modes.
pub const MODES: [Mode; 2] = [Mode::MulticolorBitmap, Mode::HiresBitmap];

/// `fixtures/<name>`.
pub fn fixture_dir(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(name)
}

/// The committed png2c64 baseline file of a fixture.
pub fn baseline_bytes(name: &str, format: BitmapFormat) -> Vec<u8> {
    let file = match format {
        BitmapFormat::Koala => "baseline.kla",
        BitmapFormat::ArtStudio => "baseline.art",
    };
    let path = fixture_dir(name).join(file);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The baseline decoded.
pub fn baseline(name: &str, format: BitmapFormat) -> BitmapImage {
    format
        .read(&baseline_bytes(name, format))
        .unwrap_or_else(|e| panic!("{name}: {e}"))
        .image
}

/// The fixture's `source.png` solved in `mode` by the real solver: fitted to the
/// screen, png2c64's gamma 1.5, Bayer 2×2 at 70 % (multicolor) or no dither (hi-res),
/// Pepto PAL. Close to the golden harness's solve; these tests only need some real
/// solver output, not that exact one. Cached per test binary.
pub fn solved(name: &str, mode: Mode) -> Arc<BitmapImage> {
    type Cache = Mutex<HashMap<(String, Mode), Arc<BitmapImage>>>;
    static CACHE: OnceLock<Cache> = OnceLock::new();
    let key = (name.to_owned(), mode);
    if let Some(s) = CACHE
        .get_or_init(Mutex::default)
        .lock()
        .expect("cache")
        .get(&key)
    {
        return Arc::clone(s);
    }
    let path = fixture_dir(name).join("source.png");
    let img = image::open(&path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .to_rgba8();
    let (w, h) = img.dimensions();
    let src = Arc::new(SourceImage::from_srgba8(w, h, img.as_raw()).expect("decoded PNG"));
    let mut layer = Layer::new(name, src);
    layer.set_transform(
        Transform::new(
            Vec2::ZERO,
            Vec2::new(320.0 / w as f32, 200.0 / h as f32),
            false,
            false,
        )
        .expect("fit to screen"),
    );
    let recipe = match mode {
        Mode::MulticolorBitmap => Recipe::with_method(DitherKind::Bayer2, 0.7),
        _ => Recipe::with_method(DitherKind::None, 0.0),
    };
    layer.set_recipe(recipe.expect("valid recipe"));
    layer.set_adjustments(Adjustments {
        gamma: Gamma::PNG2C64,
        ..Adjustments::default()
    });
    let mut c = Compositor::new(SuperGrid::with_default_factor(mode).expect("bitmap grid"));
    c.update(&[layer], &[]);
    let solution = Solver::new(&Palette::pepto_pal()).solve(
        c.frame(),
        &Steering::new(),
        None,
        &SolveOptions::default(),
    );
    let image = Arc::new(solution.image);
    CACHE
        .get_or_init(Mutex::default)
        .lock()
        .expect("cache")
        .insert(key, Arc::clone(&image));
    image
}

/// Solves every fixture in both modes on parallel threads, filling [`solved`]'s cache.
pub fn solve_all() {
    std::thread::scope(|s| {
        for (name, _) in FIXTURES {
            for mode in MODES {
                s.spawn(move || solved(name, mode));
            }
        }
    });
}
