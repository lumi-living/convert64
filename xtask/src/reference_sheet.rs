//! `cargo xtask reference-sheet` (#261, ADR-013): the sheets a human judges the move to
//! png2amiga's reference on, every tile with its SSIMULACRA2 (S2).
//!
//! Every S2 here is png2amiga's own (`quality::ssimulacra2`), taken at the mode's logical
//! raster (160 × 200 multicolor, 320 × 200 hi-res) in linear light against **one common
//! reference per source: png2amiga's prepared image** of it (its widened Lanczos-3 on the
//! original, the baseline's gamma 1.5, its OKLab round trip; no range matching). So the
//! old and new pictures of a row are scored against the same thing, and a tile that
//! moves colours away from the source on purpose (range matching) pays for it.
//!
//! - `sheet-reference-baselines.png`: each fixture's png2c64 baseline (archived) next to
//!   its png2amiga baseline, both the tools' own output at the baselines' settings.
//! - `sheet-match-range.png`: each fixture solved by Convert64 (the product pipeline,
//!   reference solver) with range matching off, png2c64's (box) and png2amiga's (hull,
//!   amount 100 %), with the colour-limited ("clash") cells of each solve.
//! - `sheet-refine.png`: each fixture with the Refine step off and on.
//! - `sheet-large-sources.png`: the 4096 px zone plate and grainy title of #235 under the
//!   old scaler (png2c64's Mitchell on the #257 mip level, emulated here since that code
//!   is gone), png2amiga's exact widened Lanczos-3, and the mip-accelerated Lanczos
//!   (Downscale Auto), each solved by the reference solver at the baselines' recipe; with
//!   the grain leakage and the share of pixels each solve shares with the exact one.
//!
//! The numbers are also printed as Markdown tables.

use crate::aliasing;
use crate::frontend::{FIXTURES, layer_from_rgba};
use crate::repo_root;
use crate::sheet::{save, sheet_with};
use convert64_core::layer::{Downscale, Layer, RangeMatch};
use convert64_core::mode::Mode;
use convert64_core::palette::Palette;
use convert64_core::quality::{render_linear, ssimulacra2};
use convert64_core::solve::png2amiga_ref::{self as p2a, Mode as RefMode};
use convert64_core::solve::{Solution, SolveOptions, Solver, Steering};
use convert64_core::{Compositor, SuperGrid};
use image::{Rgb, RgbImage, RgbaImage};

type Lin = [f32; 3];

fn ref_mode(mode: Mode) -> RefMode {
    if mode == Mode::HiresBitmap {
        RefMode::Hires
    } else {
        RefMode::Multicolor
    }
}

/// png2amiga's prepared image of an 8-bit picture at gamma 1.5: the common reference.
fn prepared(img: &RgbaImage, mode: Mode) -> Vec<Lin> {
    let rgb: Vec<[u8; 3]> = img.pixels().map(|p| [p.0[0], p.0[1], p.0[2]]).collect();
    let size = (img.width() as usize, img.height() as usize);
    p2a::prepare(
        &rgb,
        size,
        ref_mode(mode),
        &p2a::preprocess::Settings {
            gamma: 1.5,
            ..p2a::preprocess::Settings::default()
        },
        None,
    )
    .px
}

fn logical(mode: Mode) -> (usize, usize) {
    ref_mode(mode).screen()
}

/// A 320 × 200 Pepto render as linear colours at the logical raster.
fn render_to_linear(img: &RgbImage, mode: Mode) -> Vec<Lin> {
    let (w, h) = logical(mode);
    let step = 320 / w as u32;
    (0..w * h)
        .map(|i| {
            let p = img.get_pixel((i % w) as u32 * step, (i / w) as u32).0;
            p2a::colour::srgb8_to_linear(p[0], p[1], p[2])
        })
        .collect()
}

fn s2(reference: &[Lin], picture: &[Lin], mode: Mode) -> f64 {
    let (w, h) = logical(mode);
    ssimulacra2(reference, picture, w, h)
}

/// A solution drawn at 320 × 200.
fn draw(sol: &Solution, palette: &Palette) -> RgbImage {
    let out = sol.image.to_indexed();
    let step = 320 / u32::from(out.width());
    RgbImage::from_fn(320, 200, |x, y| {
        let c = palette.rgb(out.pixel((x / step) as u16, y as u16));
        Rgb([c.r, c.g, c.b])
    })
}

/// `layers` composited on `mode`'s default grid and solved with `opts` on the png2amiga
/// solver with its own front end: the pipeline these #261 review sheets showed, which
/// was the reference before ADR-014.
fn solve(layers: &[Layer], mode: Mode, opts: &SolveOptions) -> Result<Solution, String> {
    let grid = SuperGrid::with_default_factor(mode).map_err(|e| e.to_string())?;
    let mut c = Compositor::new(grid);
    c.set_pipeline(convert64_core::composite::Pipeline::Png2amiga);
    c.update(layers, &[]);
    let palette = Palette::pepto_pal();
    let opts = SolveOptions {
        solver: convert64_core::solve::SolverKind::Png2amiga,
        ..opts.clone()
    };
    Ok(Solver::new(&palette).solve(c.frame(), &Steering::new(), None, &opts))
}

pub(crate) fn source(name: &str) -> Result<RgbaImage, String> {
    let path = repo_root().join("fixtures").join(name).join("source.png");
    Ok(image::open(&path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .to_rgba8())
}

pub(crate) fn png(path: &std::path::Path) -> Result<RgbImage, String> {
    Ok(image::open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .to_rgb8())
}

pub(crate) fn identical(a: &RgbImage, b: &RgbImage) -> f64 {
    let same = a.pixels().zip(b.pixels()).filter(|(x, y)| x == y).count();
    100.0 * same as f64 / f64::from(a.width() * a.height())
}

/// png2c64's Mitchell bicubic on the #257 mip level, as the compositor sampled a
/// 4096 px source fitted to the screen before #261: the source box-reduced 8 × 8 in
/// linear light (three 2 × 2 levels; 4096 → 512 across), then png2c64's scaler and
/// preprocessing at gamma 1.5.
fn old_mitchell_mip(img: &RgbaImage, mode: Mode) -> Vec<Lin> {
    use convert64_core::solve::png2c64_ref as p2c;
    let (sw, sh) = (img.width() as usize, img.height() as usize);
    let (lw, lh) = (sw / 8, sh / 8);
    let lin: Vec<Lin> = img
        .pixels()
        .map(|p| p2c::colour::srgb8_to_linear(p.0[0], p.0[1], p.0[2]))
        .collect();
    let mut level = vec![[0.0f32; 3]; lw * lh];
    for y in 0..lh {
        for x in 0..lw {
            let mut acc = [0.0f32; 3];
            for dy in 0..8 {
                for dx in 0..8 {
                    let p = lin[(y * 8 + dy) * sw + x * 8 + dx];
                    for c in 0..3 {
                        acc[c] += p[c];
                    }
                }
            }
            level[y * lw + x] = acc.map(|v| v / 64.0);
        }
    }
    let (tw, th) = logical(mode);
    let mut im = p2c::scale::bicubic(
        &p2c::Image::new(lw, lh, level),
        tw,
        th,
        p2c::scale::Kernel::MitchellNetravali,
    );
    p2c::preprocess::apply(&mut im, &p2c::preprocess::Settings::default());
    im.px
}

/// The reference solver on prepared colours at the baselines' multicolor recipe.
fn solve_prepared(px: Vec<Lin>, mode: Mode) -> (RgbImage, Vec<Lin>) {
    let rm = ref_mode(mode);
    let (w, h) = rm.screen();
    let pal = p2a::colour::palette_linear(&p2a::colour::PEPTO_HEX);
    let settings = p2a::Settings {
        method: p2a::dither::Method::Checker,
        strength: 0.7,
        error_clamp: 0.1,
        serpentine: true,
        metric: p2a::c64::Metric::Mse,
    };
    let screen = p2a::convert(&p2a::Image::new(w, h, px), &pal, rm, &settings);
    let idx = screen.indices();
    let lin: Vec<Lin> = idx.iter().map(|&i| pal[usize::from(i)]).collect();
    let step = 320 / w;
    let img = RgbImage::from_fn(320, 200, |x, y| {
        let i = idx[y as usize * w + x as usize / step];
        let [_, r, g, b] = p2a::colour::PEPTO_HEX[usize::from(i)].to_be_bytes();
        Rgb([r, g, b])
    });
    (img, lin)
}

/// One tile of the #273 review sheets, rendered as [`run`] renders it (the same calls
/// with the same arguments), for `cargo xtask metric-check`: `sheet` is `baselines`,
/// `match-range`, `refine` or `large-sources`; `variant` the tile's name in
/// `fixtures/eye-rankings.toml`. Returns the 320 × 200 picture and, for the
/// large sources, the source it was made from.
pub(crate) fn review_tile(
    sheet: &str,
    fixture: &str,
    variant: &str,
) -> Result<(RgbImage, Mode), String> {
    let root = repo_root();
    let unknown = || format!("unknown review tile {sheet}/{fixture}/{variant}");
    if sheet == "large-sources" {
        if fixture != "zoneplate-4096" {
            return Err(unknown());
        }
        let mode = Mode::MulticolorBitmap;
        let (zone, _, _) = aliasing::fixtures()?;
        let img = match variant {
            "old-mitchell-mip" => solve_prepared(old_mitchell_mip(&zone, mode), mode).0,
            "png2amiga-exact" => solve_prepared(prepared(&zone, mode), mode).0,
            "mip-accelerated" => {
                let layer = layer_from_rgba(
                    "large",
                    &zone,
                    convert64_core::layer::DitherKind::Checker,
                    0.7,
                    Downscale::Auto,
                )?;
                draw(
                    &solve(&[layer], mode, &SolveOptions::default())?,
                    &Palette::pepto_pal(),
                )
            }
            _ => return Err(unknown()),
        };
        return Ok((img, mode));
    }
    let (_, mode, kind, strength) = FIXTURES
        .into_iter()
        .find(|f| f.0 == fixture)
        .ok_or_else(unknown)?;
    let archived = || png(&root.join(format!("fixtures/{fixture}/baseline.png")));
    let palette = Palette::pepto_pal();
    let base = || -> Result<Layer, String> {
        layer_from_rgba(fixture, &source(fixture)?, kind, strength, Downscale::Auto)
    };
    let img = match (sheet, variant) {
        ("baselines", "png2c64") | ("match-range", "png2c64-baseline") => archived()?,
        ("baselines", "png2amiga") => {
            png(&root.join(format!("fixtures/png2amiga/{fixture}/baseline.png")))?
        }
        ("match-range", "off" | "box" | "hull") => {
            let m = match variant {
                "off" => None,
                "box" => Some(
                    RangeMatch::for_palette(
                        &palette,
                        RangeMatch::DEFAULT_PERCENTILE,
                        RangeMatch::DEFAULT_MARGIN,
                    )
                    .map_err(|e| format!("{e:?}"))?,
                ),
                _ => Some(RangeMatch::hull(&palette, 1.0).map_err(|e| format!("{e:?}"))?),
            };
            let mut layer = base()?;
            let mut adj = layer.adjustments().clone();
            adj.match_range = m;
            layer.set_adjustments(adj);
            draw(&solve(&[layer], mode, &SolveOptions::default())?, &palette)
        }
        ("refine", "off" | "on") => {
            let opts = SolveOptions {
                refine: variant == "on",
                ..SolveOptions::default()
            };
            draw(&solve(&[base()?], mode, &opts)?, &palette)
        }
        _ => return Err(unknown()),
    };
    Ok((img, mode))
}

/// Where [`review_tile`]'s tile sits in the committed sheet [`run`] wrote: the file and
/// the tile's top-left corner (the `sheet_with` layout: 8 px gaps, 20 px labels).
pub(crate) fn review_tile_place(
    sheet: &str,
    fixture: &str,
    variant: &str,
) -> Option<(&'static str, u32, u32)> {
    let row = FIXTURES.iter().position(|f| f.0 == fixture);
    let (file, index, columns) = match (sheet, variant) {
        ("baselines", "png2c64") | ("match-range", "png2c64-baseline") => {
            ("sheet-reference-baselines.png", 2 * row?, 4)
        }
        ("baselines", "png2amiga") => ("sheet-reference-baselines.png", 2 * row? + 1, 4),
        ("match-range", v) => (
            "sheet-match-range.png",
            3 * row? + ["off", "box", "hull"].iter().position(|x| *x == v)?,
            3,
        ),
        ("refine", v) => (
            "sheet-refine.png",
            2 * row? + ["off", "on"].iter().position(|x| *x == v)?,
            4,
        ),
        ("large-sources", v) if fixture == "zoneplate-4096" => (
            "sheet-large-sources.png",
            ["old-mitchell-mip", "png2amiga-exact", "mip-accelerated"]
                .iter()
                .position(|x| *x == v)?,
            3,
        ),
        _ => return None,
    };
    let (col, r) = (index as u32 % columns, index as u32 / columns);
    Some((file, 8 + col * (320 + 8), 8 + r * (200 + 20 + 8) + 20))
}

pub(crate) fn run() -> Result<(), String> {
    let root = repo_root();
    let out = root.join("docs/reference/dither-previews/convert64-sheets");
    let palette = Palette::pepto_pal();
    let mut md = String::new();

    // 1. The baselines, png2c64's against png2amiga's.
    let mut tiles = Vec::new();
    md.push_str("| Fixture | png2c64 baseline S2 | png2amiga baseline S2 |\n|---|---|---|\n");
    for (name, mode, _, _) in FIXTURES {
        let reference = prepared(&source(name)?, mode);
        let old = png(&root.join(format!("fixtures/{name}/baseline.png")))?;
        let new = png(&root.join(format!("fixtures/png2amiga/{name}/baseline.png")))?;
        let (so, sn) = (
            s2(&reference, &render_to_linear(&old, mode), mode),
            s2(&reference, &render_to_linear(&new, mode), mode),
        );
        md.push_str(&format!("| {name} | {so:.2} | {sn:.2} |\n"));
        tiles.push((format!("{name} png2c64 S2 {so:.1}"), old));
        tiles.push((format!("{name} png2amiga S2 {sn:.1}"), new));
    }
    save(
        &sheet_with(&tiles, 4),
        &out.join("sheet-reference-baselines.png"),
    )?;

    // 2. Range matching off / png2c64's box / png2amiga's hull, and 3. Refine off / on.
    let mut range_tiles = Vec::new();
    let mut refine_tiles = Vec::new();
    md.push_str(
        "\n| Fixture | off S2 (clash cells) | png2c64 box S2 (clash cells) | png2amiga hull S2 (clash cells) |\n|---|---|---|---|\n",
    );
    let mut refine_md =
        String::from("\n| Fixture | Refine off S2 | Refine on S2 | gain |\n|---|---|---|---|\n");
    for (name, mode, kind, strength) in FIXTURES {
        let img = source(name)?;
        let reference = prepared(&img, mode);
        let base = layer_from_rgba(name, &img, kind, strength, Downscale::Auto)?;
        let mut row = Vec::new();
        let mut cells = Vec::new();
        for (label, m) in [
            ("off", None),
            (
                "png2c64 box",
                Some(
                    RangeMatch::for_palette(
                        &palette,
                        RangeMatch::DEFAULT_PERCENTILE,
                        RangeMatch::DEFAULT_MARGIN,
                    )
                    .map_err(|e| format!("{e:?}"))?,
                ),
            ),
            (
                "png2amiga hull",
                Some(RangeMatch::hull(&palette, 1.0).map_err(|e| format!("{e:?}"))?),
            ),
        ] {
            let mut layer = base.clone();
            let mut adj = layer.adjustments().clone();
            adj.match_range = m;
            layer.set_adjustments(adj);
            let sol = solve(&[layer], mode, &SolveOptions::default())?;
            let score = s2(&reference, &render_linear(&sol.image, &palette), mode);
            let clashes = sol.report.cells.iter().filter(|c| c.is_clash()).count();
            cells.push(format!("{score:.2} ({clashes})"));
            row.push((
                format!("{name} {label} S2 {score:.1} clash {clashes}"),
                draw(&sol, &palette),
            ));
        }
        md.push_str(&format!("| {name} | {} |\n", cells.join(" | ")));
        range_tiles.extend(row);
        let mut scores = Vec::new();
        for refine in [false, true] {
            let opts = SolveOptions {
                refine,
                ..SolveOptions::default()
            };
            let sol = solve(std::slice::from_ref(&base), mode, &opts)?;
            let score = s2(&reference, &render_linear(&sol.image, &palette), mode);
            scores.push(score);
            refine_tiles.push((
                format!(
                    "{name} refine {} S2 {score:.1}",
                    if refine { "on" } else { "off" }
                ),
                draw(&sol, &palette),
            ));
        }
        refine_md.push_str(&format!(
            "| {name} | {:.2} | {:.2} | {:+.2} |\n",
            scores[0],
            scores[1],
            scores[1] - scores[0]
        ));
    }
    md.push_str(&refine_md);
    save(
        &sheet_with(&range_tiles, 3),
        &out.join("sheet-match-range.png"),
    )?;
    save(&sheet_with(&refine_tiles, 4), &out.join("sheet-refine.png"))?;

    // 4. The large sources.
    let (zone, title, grain) = aliasing::fixtures()?;
    let mode = Mode::MulticolorBitmap;
    let solve_auto = |img: &RgbaImage| -> Result<RgbImage, String> {
        let layer = layer_from_rgba(
            "large",
            img,
            convert64_core::layer::DitherKind::Checker,
            0.7,
            Downscale::Auto,
        )?;
        Ok(draw(
            &solve(&[layer], mode, &SolveOptions::default())?,
            &palette,
        ))
    };
    let three = |img: &RgbaImage| -> Result<[RgbImage; 3], String> {
        Ok([
            solve_prepared(old_mitchell_mip(img, mode), mode).0,
            solve_prepared(prepared(img, mode), mode).0,
            solve_auto(img)?,
        ])
    };
    let mut large = Vec::new();
    md.push_str(
        "\n| Source | old Mitchell + mip S2 | png2amiga exact S2 | mip-accelerated S2 | mip vs exact identical |\n|---|---|---|---|---|\n",
    );
    let names = ["old Mitchell+mip", "png2amiga exact", "mip-accelerated"];
    let clean = three(&title)?;
    for (label, img, reference_img) in [
        ("zone plate", &zone, &zone),
        ("title grain", &grain, &title),
    ] {
        let reference = prepared(reference_img, mode);
        let solves = three(img)?;
        let scores: Vec<f64> = solves
            .iter()
            .map(|s| s2(&reference, &render_to_linear(s, mode), mode))
            .collect();
        md.push_str(&format!(
            "| {label} 4096 px | {:.2} | {:.2} | {:.2} | {:.3} % |\n",
            scores[0],
            scores[1],
            scores[2],
            identical(&solves[2], &solves[1])
        ));
        for ((n, s), score) in names.iter().zip(solves).zip(&scores) {
            large.push((format!("{label} {n} S2 {score:.1}"), s));
        }
    }
    md.push_str("\n| Grain leakage (pixels the grain changes) | old Mitchell + mip | png2amiga exact | mip-accelerated |\n|---|---|---|---|\n");
    let grainy = three(&grain)?;
    let leak: Vec<String> = (0..3)
        .map(|i| format!("{:.1} %", aliasing::changed(&clean[i], &grainy[i])))
        .collect();
    md.push_str(&format!("| title 4096 px | {} |\n", leak.join(" | ")));
    save(&sheet_with(&large, 3), &out.join("sheet-large-sources.png"))?;

    println!("{md}");
    Ok(())
}
