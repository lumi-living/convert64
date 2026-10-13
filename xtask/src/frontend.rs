//! `cargo xtask front-end-sheet` (#190, #227): the resampler A/B. Every fixture at its
//! png2c64 baseline settings, prepared two ways and solved by the reference solver:
//!
//! - **A, the compositor:** the general path every layer takes. Since ADR-012 (#227) it
//!   is png2c64's Mitchell bicubic straight to the 1× grid, then gamma and png2c64's
//!   preprocessing per layer; before, it was a 4× composite (Lanczos-3 / bilinear, box
//!   filter to the target grid);
//! - **B, png2c64's front end:** its Mitchell bicubic straight to the target grid, then
//!   gamma, then its preprocessing — the compatibility branch a plain project takes;
//!
//! next to png2c64's own baseline. Each row shows A's prepared frame and solve, B's, and
//! the baseline, with the share of the baseline's pixels each solve reproduces. Since
//! ADR-012 A and B are the same picture (its invariant).
//!
//! Then the edit A could not reach before: portrait and lineart **nudged +0.25 px, turned
//! 3° and masked** (an ellipse, feather 1.5 target px) over a flat backdrop, old
//! composite against new. The old tiles are the pre-#227 compositor's (4×,
//! Lanczos-3 / bilinear, Catmull-Rom for the rotated path), rendered once from `main` at
//! `3278845` with this file's `edited` scene and kept in `front-end-old/`; that code no
//! longer exists to re-run.
//!
//! Last, the large sources of #235 (`aliasing`): a 4096 × 2560 zone plate and the title
//! upscaled to 4096 px with grain, each solved **exact** (png2c64's sampling), **auto**
//! (the mip level) and from the source **area-averaged** to the target first, with the
//! grain leakage of each. The command fails if Auto leaks more than 5 %.

use crate::aliasing::{self, LEAKAGE_GATE};
use crate::font;
use crate::repo_root;
use crate::sheet::save;
use convert64_core::geom::Vec2;
use convert64_core::layer::{
    Adjustments, DitherKind, Downscale, Gamma, Layer, Recipe, SourceImage, Transform,
};
use convert64_core::mask::{Mask, MaskRaster, Modifiers};
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

/// The fixtures of the edited rows (#227).
const EDITED: [usize; 2] = [0, 5];

/// The large-source rows (#235): the zone plate and the grainy title.
const ALIAS_ROWS: usize = 2;

/// One fixture, fitted to the screen, at its baseline's recipe and png2c64's gamma.
fn fixture_layer(name: &str, kind: DitherKind, strength: f32) -> Result<Layer, String> {
    let path = repo_root().join("fixtures").join(name).join("source.png");
    let img = image::open(&path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .to_rgba8();
    layer_from_rgba(name, &img, kind, strength, Downscale::Auto)
}

/// A picture as a layer fitted to the screen, at a recipe and png2c64's gamma, sampled
/// as `downscale` says when shrunk a long way (#235).
pub(crate) fn layer_from_rgba(
    name: &str,
    img: &image::RgbaImage,
    kind: DitherKind,
    strength: f32,
    downscale: Downscale,
) -> Result<Layer, String> {
    let (w, h) = img.dimensions();
    let src = Arc::new(SourceImage::from_srgba8(w, h, img.as_raw()).map_err(|e| e.to_string())?);
    let mut layer = Layer::new(name, src);
    layer.set_downscale(downscale);
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
    layer.set_adjustments(Adjustments {
        gamma: Gamma::PNG2C64,
        ..Adjustments::default()
    });
    Ok(layer)
}

/// The edited scene (#227): a flat backdrop, and over it the fixture nudged +0.25 px,
/// turned 3° about its centre and masked by an ellipse (hard in the raster, feather 1.5
/// target px), both with the fixture's recipe.
fn edited(name: &str, kind: DitherKind, strength: f32) -> Result<Vec<Layer>, String> {
    let e = |e: &dyn std::fmt::Display| e.to_string();
    let recipe = Recipe::with_method(kind, strength).map_err(|x| e(&x))?;
    let backdrop =
        SourceImage::from_linear(1, 1, vec![[0.05, 0.07, 0.12, 1.0]]).map_err(|x| e(&x))?;
    let mut bottom = Layer::new("backdrop", Arc::new(backdrop));
    bottom.set_transform(
        Transform::new(Vec2::ZERO, Vec2::new(320.0, 200.0), false, false).map_err(|x| e(&x))?,
    );
    bottom.set_recipe(recipe);
    let mut top = fixture_layer(name, kind, strength)?;
    let t = top.transform();
    top.set_transform(
        Transform::new(
            t.translate() + Vec2::new(0.25, 0.0),
            t.scale(),
            false,
            false,
        )
        .and_then(|t| t.with_rotation(3.0))
        .map_err(|x| e(&x))?,
    );
    let (w, h) = top.source().size();
    let ellipse = (0..w * h)
        .map(|i| {
            let (x, y) = ((i % w) as f32 + 0.5, (i / w) as f32 + 0.5);
            let (u, v) = (x / w as f32 - 0.5, y / h as f32 - 0.5);
            if (u / 0.42).powi(2) + (v / 0.44).powi(2) <= 1.0 {
                1.0
            } else {
                0.0
            }
        })
        .collect();
    let mut mask = Mask::new(MaskRaster::from_values(w, h, ellipse).map_err(|x| e(&x))?);
    mask.set_modifiers(Modifiers::feathered(1.5).map_err(|x| e(&x))?);
    top.set_mask(Some(mask)).map_err(|x| e(&x))?;
    Ok(vec![bottom, top])
}

/// One fixture prepared and solved with the front end `png2c64` on or off: the prepared
/// frame and the solve, both at 320 × 200.
fn render(
    name: &str,
    mode: Mode,
    kind: DitherKind,
    strength: f32,
    png2c64: bool,
) -> Result<(RgbImage, RgbImage), String> {
    let layer = fixture_layer(name, kind, strength)?;
    render_layers(&[layer], mode, png2c64)
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
    let mut layer = fixture_layer(name, kind, strength)?;
    layer.set_adjustments(adjustments.clone());
    render_layers(&[layer], mode, png2c64)
}

/// `layers` composited on `mode`'s default grid and solved: the prepared frame and the
/// solve, both at 320 × 200.
pub(crate) fn render_layers(
    layers: &[Layer],
    mode: Mode,
    png2c64: bool,
) -> Result<(RgbImage, RgbImage), String> {
    let grid = SuperGrid::with_default_factor(mode).map_err(|e| e.to_string())?;
    let mut c = Compositor::new(grid);
    c.set_png2c64_front_end(png2c64);
    c.update(layers, &[]);
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
    let rows = (FIXTURES.len() + EDITED.len() + ALIAS_ROWS) as u32;
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
    table.push_str("\n| Edited (+0.25 px, 3°, masked) | old vs new solve, % same |\n|---|---|\n");
    for (k, &f) in EDITED.iter().enumerate() {
        let (name, mode, kind, strength) = FIXTURES[f];
        let (pn, sn) = render_layers(&edited(name, kind, strength)?, mode, true)?;
        let old = |what: &str| -> Result<RgbImage, String> {
            let p = dir.join("front-end-old").join(format!("{name}-{what}.png"));
            Ok(image::open(&p)
                .map_err(|e| format!("{}: {e}", p.display()))?
                .to_rgb8())
        };
        let (po, so) = (old("prepared")?, old("solved")?);
        let same = identical(&so, &sn);
        let y = GAP + (FIXTURES.len() + k) as u32 * (LABEL + TILE_H + NOTE + GAP);
        font::draw(
            &mut sheet,
            GAP + 2,
            y + 3,
            &format!(
                "{name} nudged 0.25 px, turned 3 deg, masked ({} {strength}): old 4x composite | new Mitchell 1x",
                kind.id()
            ),
            2,
            TEXT,
        );
        let tiles: [(&RgbImage, String); 4] = [
            (&po, "old prepared".to_owned()),
            (&so, "old solved".to_owned()),
            (&pn, "new prepared".to_owned()),
            (&sn, format!("new solved: {same:.1}% as old")),
        ];
        for (c, (tile, note)) in tiles.iter().enumerate() {
            let x = GAP + c as u32 * (TILE_W + GAP) + if c >= 2 { GAP } else { 0 };
            image::imageops::replace(&mut sheet, *tile, i64::from(x), i64::from(y + LABEL));
            font::draw(&mut sheet, x + 2, y + LABEL + TILE_H + 4, note, 2, TEXT);
        }
        table.push_str(&format!("| {name} | {same:.3} % |\n"));
        println!("{name:<9} edited: new solve {same:8.3} % as old");
    }
    // #235: the large sources, exact | auto | area-averaged reference.
    println!("large sources (#235): generating the 4096 px fixtures in target/aliasing/");
    let (zone, title, grain) = aliasing::fixtures()?;
    let zone3 = aliasing::three_ways("zoneplate-4096", &zone)?;
    let clean3 = aliasing::three_ways("title-4096", &title)?;
    let grain3 = aliasing::three_ways("title-grain-4096", &grain)?;
    let leak = aliasing::leakage((&title, &clean3), (&grain, &grain3))?;
    let auto_vs_ref = |three: &[(RgbImage, RgbImage); 3]| identical(&three[1].1, &three[2].1);
    let row0 = FIXTURES.len() + EDITED.len();
    for (k, (title, three, notes)) in [
        (
            "zoneplate-4096 (checker 0.7): exact | auto | area-averaged; beyond the centre the right answer is flat grey",
            &zone3,
            [String::new(), String::new(), String::new()],
        ),
        (
            "title-grain-4096 (grain 14/255, checker 0.7): exact | auto | area-averaged, % of pixels the grain changes",
            &grain3,
            [leak[0], leak[1], leak[2]].map(|l| format!(": {l:.1}%")),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let y = GAP + (row0 + k) as u32 * (LABEL + TILE_H + NOTE + GAP);
        font::draw(&mut sheet, GAP + 2, y + 3, title, 2, TEXT);
        let tiles: [(&RgbImage, String); 5] = [
            (&three[0].1, format!("exact{}", notes[0])),
            (&three[1].1, format!("auto{}", notes[1])),
            (&three[2].1, format!("area-averaged{}", notes[2])),
            (&three[0].0, "exact prepared".to_owned()),
            (&three[1].0, "auto prepared".to_owned()),
        ];
        for (c, (tile, note)) in tiles.iter().enumerate() {
            let x = GAP + c as u32 * (TILE_W + GAP) + if c >= 3 { GAP } else { 0 };
            image::imageops::replace(&mut sheet, *tile, i64::from(x), i64::from(y + LABEL));
            font::draw(&mut sheet, x + 2, y + LABEL + TILE_H + 4, note, 2, TEXT);
        }
    }
    let [exact, auto, reference, srgb] = leak;
    println!(
        "title-grain-4096: grain leakage exact {exact:.1} %, auto {auto:.1} %, \
         reference {reference:.1} % (sRGB-space box {srgb:.1} %)"
    );
    table.push_str(&format!(
        "\n#235, large sources (4096 px, shrunk 12.8×; multicolor, checker 0.7, error clamp \
         0.1, gamma 1.5, pepto). Grain leakage: the share of output pixels that change \
         between `title-grain-4096` and the same upscale without grain (gate: Auto ≤ \
         {LEAKAGE_GATE} %). The reference is the source box-averaged to 320 × 200 in \
         linear light, then the same front end and solver; the evidence (#238) averaged \
         the 8-bit sRGB values instead, which is given beside it. Auto vs reference: the \
         share of the reference's solve that Auto's reproduces.\n\n\
         | Source | exact (png2c64) | auto (mip level) | area-averaged reference | box on sRGB values |\n\
         |---|---|---|---|---|\n\
         | title-grain-4096, grain leakage | {exact:.1} % | {auto:.1} % | {reference:.1} % | {srgb:.1} % |\n\
         | zoneplate-4096, auto vs reference | | {:.1} % same | | |\n\
         | title-grain-4096, auto vs reference | | {:.1} % same | | |\n",
        auto_vs_ref(&zone3),
        auto_vs_ref(&grain3),
    ));
    save(&sheet, &dir.join("sheet-front-end-ab.png"))?;
    // The large-source rows on their own, for review (#235).
    let y0 = GAP + row0 as u32 * (LABEL + TILE_H + NOTE + GAP);
    let rows = image::imageops::crop_imm(&sheet, 0, y0, w, h - y0).to_image();
    save(&rows, &dir.join("sheet-front-end-large.png"))?;
    std::fs::write(
        dir.join("front-end-ab.md"),
        format!(
            "<!-- Written by `cargo xtask front-end-sheet`; do not edit by hand. -->\n\n\
             Share of png2c64's baseline pixels each front end's solve reproduces (#190), \
             at the baselines' settings, reference solver both times. A is the general \
             path, B the compatibility branch; since ADR-012 (#227) they are the same \
             picture. Below, the edited scene (#227): the share of the old composite's \
             solve the new one keeps.\n\n{table}"
        ),
    )
    .map_err(|e| e.to_string())?;
    if auto > LEAKAGE_GATE {
        return Err(format!(
            "#235's gate: grain leakage under Auto is {auto:.1} %, above {LEAKAGE_GATE} %"
        ));
    }
    Ok(())
}
