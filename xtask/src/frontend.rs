//! `cargo xtask front-end-sheet` (#190): the resampler A/B. Every fixture at its png2c64
//! baseline settings, prepared two ways and solved by the reference solver:
//!
//! - **A, the compositor:** the supersampled composite (4×, Lanczos-3 / bilinear onto the
//!   super grid, box filter to the target grid), gamma applied after resampling;
//! - **B, png2c64's front end:** its Mitchell bicubic straight to the target grid, then
//!   gamma, then its preprocessing — what a plain project gets by default;
//!
//! next to png2c64's own baseline. Each row shows A's prepared frame and solve, B's, and
//! the baseline, with the share of the baseline's pixels each solve reproduces. A human
//! judges whether A is at least as good as B by eye (#190's exit).

use crate::font;
use crate::repo_root;
use crate::sheet::save;
use convert64_core::geom::Vec2;
use convert64_core::layer::{
    Adjustments, DitherKind, Gamma, Layer, Recipe, SourceImage, Transform,
};
use convert64_core::mode::Mode;
use convert64_core::palette::{Palette, linear_to_srgb8};
use convert64_core::solve::{SolveOptions, Solver, Steering};
use convert64_core::{Compositor, SuperGrid};
use image::{Rgb, RgbImage};
use std::sync::Arc;

const TILE_W: u32 = 320;
const TILE_H: u32 = 200;
const GAP: u32 = 8;
const LABEL: u32 = 20;
const NOTE: u32 = 20;
const BACKGROUND: [u8; 3] = [30, 30, 30];
const TEXT: [u8; 3] = [225, 225, 225];

/// The fixtures with their baselines' mode, method and strength.
pub(crate) const FIXTURES: [(&str, Mode, DitherKind, f32); 6] = [
    ("portrait", Mode::MulticolorBitmap, DitherKind::Checker, 0.7),
    ("terrain", Mode::MulticolorBitmap, DitherKind::Checker, 0.7),
    ("title", Mode::MulticolorBitmap, DitherKind::Checker, 0.7),
    ("gradient", Mode::MulticolorBitmap, DitherKind::Checker, 0.7),
    ("photo", Mode::MulticolorBitmap, DitherKind::Checker, 0.7),
    ("lineart", Mode::HiresBitmap, DitherKind::None, 0.0),
];

/// One fixture prepared and solved with the front end `png2c64` on or off: the prepared
/// frame and the solve, both at 320 × 200.
fn render(
    name: &str,
    mode: Mode,
    kind: DitherKind,
    strength: f32,
    png2c64: bool,
) -> Result<(RgbImage, RgbImage), String> {
    let adjustments = Adjustments {
        gamma: Gamma::PNG2C64,
        ..Adjustments::default()
    };
    render_with(name, mode, kind, strength, &adjustments, png2c64)
}

/// [`render`] with the layer's own `adjustments` (#228's sharpen sheet).
pub(crate) fn render_with(
    name: &str,
    mode: Mode,
    kind: DitherKind,
    strength: f32,
    adjustments: &Adjustments,
    png2c64: bool,
) -> Result<(RgbImage, RgbImage), String> {
    let path = repo_root().join("fixtures").join(name).join("source.png");
    let img = image::open(&path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .to_rgba8();
    let (w, h) = img.dimensions();
    let src = Arc::new(SourceImage::from_srgba8(w, h, img.as_raw()).map_err(|e| e.to_string())?);
    let mut layer = Layer::new(name, src);
    layer.set_transform(
        Transform::new(
            Vec2::ZERO,
            Vec2::new(320.0 / w as f32, 200.0 / h as f32),
            false,
            false,
        )
        .map_err(|e| e.to_string())?,
    );
    layer.set_recipe(Recipe::with_method(kind, strength).map_err(|e| e.to_string())?);
    layer.set_adjustments(adjustments.clone());
    let grid = SuperGrid::with_default_factor(mode).map_err(|e| e.to_string())?;
    let mut c = Compositor::new(grid);
    c.set_png2c64_front_end(png2c64);
    c.update(&[layer], &[]);
    let frame = c.frame();
    let palette = Palette::pepto_pal();
    let sol = Solver::new(&palette).solve(frame, &Steering::new(), None, &SolveOptions::default());
    let (tw, _) = grid.target_size();
    let step = 320 / tw;
    let colour = frame.colour();
    let prepared = RgbImage::from_fn(320, 200, |x, y| {
        Rgb(colour[(y * tw + x / step) as usize].map(linear_to_srgb8))
    });
    let out = sol.image.to_indexed();
    let solved = RgbImage::from_fn(320, 200, |x, y| {
        let c = palette.rgb(out.pixel((x / step) as u16, y as u16));
        Rgb([c.r, c.g, c.b])
    });
    Ok((prepared, solved))
}

fn identical(a: &RgbImage, b: &RgbImage) -> f64 {
    let same = a.pixels().zip(b.pixels()).filter(|(x, y)| x == y).count();
    100.0 * same as f64 / (a.width() * a.height()) as f64
}

pub(crate) fn run() -> Result<(), String> {
    let rows = FIXTURES.len() as u32;
    let w = 5 * TILE_W + 6 * GAP + 2 * GAP;
    let h = rows * (LABEL + TILE_H + NOTE + GAP) + GAP;
    let mut sheet = RgbImage::from_pixel(w, h, Rgb(BACKGROUND));
    let mut table =
        String::from("| Fixture | A: compositor | B: png2c64's front end |\n|---|---|---|\n");
    for (r, (name, mode, kind, strength)) in FIXTURES.iter().enumerate() {
        let (pa, sa) = render(name, *mode, *kind, *strength, false)?;
        let (pb, sb) = render(name, *mode, *kind, *strength, true)?;
        let base_path = repo_root().join("fixtures").join(name).join("baseline.png");
        let base = image::open(&base_path)
            .map_err(|e| format!("{}: {e}", base_path.display()))?
            .to_rgb8();
        let (ia, ib) = (identical(&sa, &base), identical(&sb, &base));
        let y = GAP + r as u32 * (LABEL + TILE_H + NOTE + GAP);
        font::draw(
            &mut sheet,
            GAP + 2,
            y + 3,
            &format!(
                "{name} ({} {strength}): A compositor | B png2c64 front end | png2c64",
                kind.id()
            ),
            2,
            TEXT,
        );
        let tiles: [(&RgbImage, String); 5] = [
            (&pa, "A prepared".to_owned()),
            (&sa, format!("A solved: {ia:.3}% same")),
            (&pb, "B prepared".to_owned()),
            (&sb, format!("B solved: {ib:.3}% same")),
            (&base, "png2c64".to_owned()),
        ];
        for (c, (tile, note)) in tiles.iter().enumerate() {
            let x = GAP
                + c as u32 * (TILE_W + GAP)
                + if c >= 2 { GAP } else { 0 }
                + if c >= 4 { GAP } else { 0 };
            image::imageops::replace(&mut sheet, *tile, i64::from(x), i64::from(y + LABEL));
            font::draw(&mut sheet, x + 2, y + LABEL + TILE_H + 4, note, 2, TEXT);
        }
        table.push_str(&format!("| {name} | {ia:.3} % | {ib:.3} % |\n"));
        println!("{name:<9} A {ia:8.3} %   B {ib:8.3} %");
    }
    let dir = repo_root().join("docs/reference/dither-previews/convert64-sheets");
    save(&sheet, &dir.join("sheet-front-end-ab.png"))?;
    std::fs::write(
        dir.join("front-end-ab.md"),
        format!(
            "<!-- Written by `cargo xtask front-end-sheet`; do not edit by hand. -->\n\n\
             Share of png2c64's baseline pixels each front end's solve reproduces (#190), \
             at the baselines' settings, reference solver both times.\n\n{table}"
        ),
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}
