//! `cargo xtask lineart-sheet` (#273, ADR-014 §4): the line-art recipe, made on the
//! reference path, next to what it was chosen from.
//!
//! The recipe: hi-res `none` at gamma 1.5, the layer resampled with png2amiga's widened
//! Lanczos-3 (`Resample::Lanczos`), png2c64's black point 0.5, and the cells limited to
//! Pepto's greys (black 0, dark grey 11, grey 12, light grey 15, white 1). It was chosen
//! by eye on the cause sheet's tile 13, which is the same on png2amiga's port; this
//! renders it on our png2c64 path (png2c64's port, its front end) instead.
//!
//! Two rows, each solved through the product pipeline (compositor and solver):
//!
//! - **lineart** (the fixture): its source, png2c64's baseline, png2amiga as shipped (its
//!   committed baseline), the cause sheet's tile 13, and the recipe.
//! - **logo** (synthetic, made here: coloured strokes and filled shapes with dark outlines
//!   on white at 1024 × 640, the kind of outline art and logos *No Glory* needs, which no
//!   fixture covers): its source, the reference at hi-res `none` (png2c64's picture),
//!   the png2amiga solver at the same flags (png2amiga as shipped), the recipe, and the
//!   recipe without the colour limit (Lanczos and the black point only).
//!
//! Each row ends with the recipe's Lanczos alone (no black point, any colour).
//!
//! Written: `docs/reference/dither-previews/convert64-sheets/sheet-lineart.png` and
//! `sheet-lineart.md` (identical-pixel shares and the colours each tile uses).

use crate::cause_sheet::lineart_tile13;
use crate::reference_sheet::{identical, png, source};
use crate::repo_root;
use crate::sheet::{save, sheet_with};
use convert64_core::colour::ColourSet;
use convert64_core::geom::Vec2;
use convert64_core::layer::{
    Adjustments, BlackPoint, DitherKind, FitMode, Gamma, Layer, Method, Metric, PatternPhase,
    Recipe, Resample, SourceImage, Transform,
};
use convert64_core::mode::Mode;
use convert64_core::palette::Palette;
use convert64_core::solve::{SolveOptions, Solver, SolverKind, Steering};
use convert64_core::{ColourIndex, Compositor, SuperGrid};
use image::{Rgb, RgbImage, RgbaImage};
use std::fmt::Write as _;
use std::sync::Arc;

/// Pepto's greys by lightness: black, dark grey, grey, light grey, white.
const GREYS: [u8; 5] = [0, 11, 12, 15, 1];

/// What a render varies.
#[derive(Clone, Copy)]
struct Setup {
    solver: SolverKind,
    resample: Resample,
    black_point: f32,
    greys_only: bool,
}

/// png2c64's picture: the reference at hi-res `none`, gamma 1.5.
const REFERENCE: Setup = Setup {
    solver: SolverKind::Reference,
    resample: Resample::Mitchell,
    black_point: 0.0,
    greys_only: false,
};

/// png2amiga's picture: its solver with its own front end, at the same flags.
const PNG2AMIGA: Setup = Setup {
    solver: SolverKind::Png2amiga,
    ..REFERENCE
};

/// The line-art recipe (ADR-014 §4) on the reference path.
const RECIPE: Setup = Setup {
    solver: SolverKind::Reference,
    resample: Resample::Lanczos,
    black_point: 0.5,
    greys_only: true,
};

/// The recipe without the colour limit.
const RECIPE_ANY_COLOUR: Setup = Setup {
    greys_only: false,
    ..RECIPE
};

/// Only the recipe's Lanczos: no black point, any colour.
const LANCZOS_ONLY: Setup = Setup {
    black_point: 0.0,
    ..RECIPE_ANY_COLOUR
};

/// `img` fitted to the hi-res screen and solved through the product pipeline.
fn solve(img: &RgbaImage, s: Setup) -> Result<RgbImage, String> {
    let (w, h) = img.dimensions();
    let src = Arc::new(SourceImage::from_srgba8(w, h, img.as_raw()).map_err(|e| e.to_string())?);
    let mut layer = Layer::new("lineart", src);
    layer.set_transform(
        Transform::new(
            Vec2::ZERO,
            Vec2::new(320.0 / w as f32, 200.0 / h as f32),
            false,
            false,
        )
        .map_err(|e| e.to_string())?,
    );
    let allowed: ColourSet = if s.greys_only {
        GREYS
            .iter()
            .map(|&c| ColourIndex::new(c).expect("a palette index"))
            .collect()
    } else {
        ColourSet::ALL
    };
    layer.set_recipe(
        Recipe::new(
            Method::new(DitherKind::None),
            0.0,
            PatternPhase::default(),
            Metric::default(),
            allowed,
            FitMode::default(),
        )
        .map_err(|e| e.to_string())?,
    );
    layer.set_adjustments(Adjustments {
        gamma: Gamma::PNG2C64,
        black_point: BlackPoint::new(s.black_point).map_err(|e| e.to_string())?,
        ..Adjustments::default()
    });
    layer.set_resample(s.resample);
    let grid = SuperGrid::with_default_factor(Mode::HiresBitmap).map_err(|e| e.to_string())?;
    let mut c = Compositor::new(grid);
    c.set_pipeline(s.solver.pipeline());
    c.update(&[layer], &[]);
    let palette = Palette::pepto_pal();
    let opts = SolveOptions {
        solver: s.solver,
        ..SolveOptions::default()
    };
    let sol = Solver::new(&palette).solve(c.frame(), &Steering::new(), None, &opts);
    if sol.solver != s.solver {
        return Err(format!(
            "{:?} fell back to {:?}: {:?}",
            s.solver, sol.solver, sol.fallback
        ));
    }
    let out = sol.image.to_indexed();
    Ok(RgbImage::from_fn(320, 200, |x, y| {
        let c = palette.rgb(out.pixel(x as u16, y as u16));
        Rgb([c.r, c.g, c.b])
    }))
}

/// A picture scaled to 320 × 200 for its tile (nearest; only to show the source).
fn thumb(img: &RgbaImage) -> RgbImage {
    let (w, h) = img.dimensions();
    RgbImage::from_fn(320, 200, |x, y| {
        let p = img.get_pixel(x * w / 320, y * h / 200).0;
        Rgb([p[0], p[1], p[2]])
    })
}

/// Coverage of a shape at (x, y), 4 × 4 samples per pixel.
fn coverage(x: u32, y: u32, inside: &dyn Fn(f32, f32) -> bool) -> f32 {
    let mut n = 0;
    for sy in 0..4 {
        for sx in 0..4 {
            let (px, py) = (
                x as f32 + (sx as f32 + 0.5) / 4.0,
                y as f32 + (sy as f32 + 0.5) / 4.0,
            );
            if inside(px, py) {
                n += 1;
            }
        }
    }
    n as f32 / 16.0
}

/// Distance from (x, y) to the segment a–b.
fn segment(x: f32, y: f32, a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let t = (((x - a.0) * dx + (y - a.1) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
    ((x - a.0 - t * dx).powi(2) + (y - a.1 - t * dy).powi(2)).sqrt()
}

/// Signed distance to a rounded rectangle (negative inside).
fn rounded_rect(x: f32, y: f32, (x0, y0, x1, y1): (f32, f32, f32, f32), r: f32) -> f32 {
    let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    let (hx, hy) = ((x1 - x0) / 2.0 - r, (y1 - y0) / 2.0 - r);
    let (qx, qy) = ((x - cx).abs() - hx, (y - cy).abs() - hy);
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    outside + qx.max(qy).min(0.0) - r
}

/// Whether (x, y) is inside the triangle a, b, c.
fn triangle(x: f32, y: f32, [a, b, c]: [(f32, f32); 3]) -> bool {
    let side = |p: (f32, f32), q: (f32, f32)| (q.0 - p.0) * (y - p.1) - (q.1 - p.1) * (x - p.0);
    let (d1, d2, d3) = (side(a, b), side(b, c), side(c, a));
    !((d1 < 0.0 || d2 < 0.0 || d3 < 0.0) && (d1 > 0.0 || d2 > 0.0 || d3 > 0.0))
}

/// The synthetic logo: 1024 × 640, on white, antialiased by 4 × 4 coverage, painted
/// bottom to top: a red disc with a black ring, a blue rounded panel with a navy
/// outline, a yellow triangle outlined in black on it, five wavy strokes 2–6 px wide
/// (green, orange, purple, cyan, black), and a black cross 3 px wide.
pub(crate) fn logo() -> RgbaImage {
    type Shape = (Box<dyn Fn(f32, f32) -> bool>, [u8; 3]);
    let tri = [(745.0, 200.0), (630.0, 400.0), (860.0, 400.0)];
    let panel = (560.0, 150.0, 930.0, 450.0);
    let mut shapes: Vec<Shape> = vec![
        (
            Box::new(|x, y| (x - 290.0).hypot(y - 290.0) < 170.0),
            [200, 48, 44],
        ),
        (
            Box::new(|x, y| {
                let d = (x - 290.0).hypot(y - 290.0);
                (170.0..182.0).contains(&d)
            }),
            [16, 16, 16],
        ),
        (
            Box::new(move |x, y| rounded_rect(x, y, panel, 40.0) < 0.0),
            [42, 80, 184],
        ),
        (
            Box::new(move |x, y| {
                let d = rounded_rect(x, y, panel, 40.0);
                (-8.0..0.0).contains(&d)
            }),
            [10, 26, 80],
        ),
        (Box::new(move |x, y| triangle(x, y, tri)), [240, 192, 32]),
        (
            Box::new(move |x, y| {
                let d = (0..3)
                    .map(|k| segment(x, y, tri[k], tri[(k + 1) % 3]))
                    .fold(f32::MAX, f32::min);
                d < 2.0
            }),
            [16, 16, 16],
        ),
    ];
    let colours = [
        [42, 154, 58],
        [224, 112, 32],
        [128, 64, 160],
        [32, 160, 176],
        [16, 16, 16],
    ];
    for (k, colour) in colours.into_iter().enumerate() {
        let width = 2.0 + k as f32;
        let base = 520.0 + 22.0 * k as f32;
        let phase = k as f32 * 0.9;
        shapes.push((
            Box::new(move |x, y| {
                let f = base + 24.0 * (x / 45.0 + phase).sin();
                let slope = 24.0 / 45.0 * (x / 45.0 + phase).cos();
                (60.0..980.0).contains(&x) && (y - f).abs() / slope.hypot(1.0) < width / 2.0
            }),
            colour,
        ));
    }
    shapes.push((
        Box::new(|x, y| {
            segment(x, y, (70.0, 40.0), (190.0, 160.0)) < 1.5
                || segment(x, y, (190.0, 40.0), (70.0, 160.0)) < 1.5
        }),
        [16, 16, 16],
    ));
    RgbaImage::from_fn(1024, 640, |x, y| {
        let mut px = [255.0f32; 3];
        for (inside, colour) in &shapes {
            let a = coverage(x, y, inside.as_ref());
            if a > 0.0 {
                for (p, c) in px.iter_mut().zip(colour) {
                    *p = *p * (1.0 - a) + f32::from(*c) * a;
                }
            }
        }
        let [r, g, b] = px.map(|v| v.round().clamp(0.0, 255.0) as u8);
        image::Rgba([r, g, b, 255])
    })
}

/// How many pixels of `img` show each Pepto colour, by index (non-zero only).
fn colours(img: &RgbImage) -> String {
    let pal = Palette::pepto_pal();
    let mut n = [0usize; 16];
    for p in img.pixels() {
        if let Some(i) = (0..16).find(|&i| {
            let c = pal.rgb(ColourIndex::ALL[i]);
            [c.r, c.g, c.b] == p.0
        }) {
            n[i] += 1;
        }
    }
    let grey = |i: usize| GREYS.contains(&(i as u8));
    let mut parts: Vec<String> = (0..16)
        .filter(|&i| n[i] > 0)
        .map(|i| format!("{i}{}: {}", if grey(i) { "" } else { "*" }, n[i]))
        .collect();
    let off: usize = (0..16).filter(|&i| !grey(i)).map(|i| n[i]).sum();
    parts.push(format!("non-grey {off}"));
    parts.join(", ")
}

/// Renders the sheet and its numbers.
pub(crate) fn run() -> Result<(), String> {
    let root = repo_root();
    let out = root.join("docs/reference/dither-previews/convert64-sheets");
    let src = source("lineart")?;
    let baseline = png(&root.join("fixtures/lineart/baseline.png"))?;
    let shipped = png(&root.join("fixtures/png2amiga/lineart/baseline.png"))?;
    let tile13 = lineart_tile13()?;
    let ours = solve(&src, RECIPE)?;
    let lanczos = solve(&src, LANCZOS_ONLY)?;
    // Checks: the product pipeline reproduces both tools' committed renders.
    let ref_check = identical(&solve(&src, REFERENCE)?, &baseline);
    let p2a_check = identical(&solve(&src, PNG2AMIGA)?, &shipped);

    let logo = logo();
    let logo_ref = solve(&logo, REFERENCE)?;
    let logo_p2a = solve(&logo, PNG2AMIGA)?;
    let logo_ours = solve(&logo, RECIPE)?;
    let logo_any = solve(&logo, RECIPE_ANY_COLOUR)?;
    let logo_lanczos = solve(&logo, LANCZOS_ONLY)?;

    let pct = |a: &RgbImage, b: &RgbImage| format!("{:.3} %", identical(a, b));
    let tiles = vec![
        ("lineart: source".to_owned(), thumb(&src)),
        ("1. png2c64 baseline".to_owned(), baseline.clone()),
        ("2. png2amiga as shipped".to_owned(), shipped.clone()),
        (
            format!("3. cause tile 13 ({} = 1)", pct(&tile13, &baseline)),
            tile13.clone(),
        ),
        (
            format!("4. line-art recipe ({} = 3)", pct(&ours, &tile13)),
            ours.clone(),
        ),
        ("5. lanczos only".to_owned(), lanczos.clone()),
        ("logo (synthetic): source".to_owned(), thumb(&logo)),
        ("1. png2c64 hires none".to_owned(), logo_ref.clone()),
        ("2. png2amiga as shipped".to_owned(), logo_p2a.clone()),
        ("3. line-art recipe".to_owned(), logo_ours.clone()),
        ("4. recipe, any colour".to_owned(), logo_any.clone()),
        ("5. lanczos only".to_owned(), logo_lanczos.clone()),
    ];
    save(&sheet_with(&tiles, 6), &out.join("sheet-lineart.png"))?;
    save(&thumb(&logo), &out.join("lineart-logo-source.png"))?;

    let mut md = String::from(
        "<!-- Written by `cargo xtask lineart-sheet`; do not edit by hand. -->\n\n\
         # Line-art sheet (#273, ADR-014 §4)\n\n\
         [sheet-lineart.png](sheet-lineart.png). The line-art recipe: hi-res `none`, gamma \
         1.5, Lanczos resampling, black point 0.5, cells limited to Pepto's greys (0, 11, \
         12, 15, 1), on the reference path (png2c64's port and front end). Tile 13 of the \
         cause sheet is the same recipe on png2amiga's port. Shares are of the 320 × 200 \
         pixels through Pepto; no SSIMULACRA2.\n\n\
         ## lineart\n\n| Pair | Identical |\n|---|---|\n",
    );
    for (a, an, b, bn) in [
        (&ours, "4. line-art recipe", &tile13, "3. tile 13"),
        (
            &ours,
            "4. line-art recipe",
            &baseline,
            "1. png2c64 baseline",
        ),
        (
            &ours,
            "4. line-art recipe",
            &shipped,
            "2. png2amiga as shipped",
        ),
        (&tile13, "3. tile 13", &baseline, "1. png2c64 baseline"),
        (
            &shipped,
            "2. png2amiga as shipped",
            &baseline,
            "1. png2c64 baseline",
        ),
        (
            &lanczos,
            "5. lanczos only",
            &baseline,
            "1. png2c64 baseline",
        ),
    ] {
        let _ = writeln!(md, "| {an} vs {bn} | {} |", pct(a, b));
    }
    let _ = writeln!(
        md,
        "\nChecks: the product pipeline at hi-res `none`, gamma 1.5 reproduces png2c64's \
         baseline on {ref_check:.3} % of pixels (the reference) and png2amiga's on \
         {p2a_check:.3} % (`--solver png2amiga`).\n\n\
         Colours used (index: pixels; `*` not a grey):\n"
    );
    for (name, img) in [
        ("1. png2c64 baseline", &baseline),
        ("2. png2amiga as shipped", &shipped),
        ("3. tile 13", &tile13),
        ("4. line-art recipe", &ours),
        ("5. lanczos only", &lanczos),
    ] {
        let _ = writeln!(md, "- {name}: {}", colours(img));
    }
    md.push_str(
        "\n## logo\n\nA synthetic logo made by the xtask ([lineart-logo-source.png]\
         (lineart-logo-source.png), 1024 × 640: a red disc with a black ring, a blue \
         panel with a navy outline, a yellow triangle outlined in black, wavy strokes 2–6 \
         px wide in green, orange, purple, cyan and black, a 3 px black cross). No \
         fixture has coloured outline art.\n\n| Pair | Identical |\n|---|---|\n",
    );
    for (a, an, b, bn) in [
        (&logo_ours, "3. line-art recipe", &logo_ref, "1. png2c64"),
        (&logo_any, "4. recipe, any colour", &logo_ref, "1. png2c64"),
        (&logo_p2a, "2. png2amiga", &logo_ref, "1. png2c64"),
        (
            &logo_any,
            "4. recipe, any colour",
            &logo_ours,
            "3. line-art recipe",
        ),
        (&logo_lanczos, "5. lanczos only", &logo_ref, "1. png2c64"),
    ] {
        let _ = writeln!(md, "| {an} vs {bn} | {} |", pct(a, b));
    }
    md.push_str("\nColours used (index: pixels; `*` not a grey):\n\n");
    for (name, img) in [
        ("1. png2c64", &logo_ref),
        ("2. png2amiga", &logo_p2a),
        ("3. line-art recipe", &logo_ours),
        ("4. recipe, any colour", &logo_any),
        ("5. lanczos only", &logo_lanczos),
    ] {
        let _ = writeln!(md, "- {name}: {}", colours(img));
    }
    std::fs::write(out.join("sheet-lineart.md"), &md).map_err(|e| e.to_string())?;
    print!("{md}");
    Ok(())
}
