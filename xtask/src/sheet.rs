//! `cargo xtask dither-sheet` (#116): every method id rendered on the dither-preview
//! test card and on `fixtures/portrait/source.png`, in the layout of the png2c64 sheets
//! in `docs/reference/dither-previews/`, plus a sheet pairing ours with png2c64's for the
//! methods both have.
//!
//! Both inputs get png2c64's preprocessing (`--gamma 1.5`, its default, as the
//! `Gamma::PNG2C64` adjustment) so the two converters solve the same picture; the card
//! is dithered at strength 1.0 and the portrait at 0.7, the settings the png2c64 sheets
//! were made with. Nothing here is tuned for the comparison: every method runs with its
//! recipe defaults.

use crate::font;
use convert64_core::geom::Vec2;
use convert64_core::layer::{
    Adjustments, DitherFamily, DitherKind, Gamma, Layer, Recipe, SourceImage, Transform,
};
use convert64_core::mode::Mode;
use convert64_core::palette::{LinearRgb, Palette, Rgb8, linear_to_srgb8, srgb8_to_linear};
use convert64_core::solve::{SolveOptions, Solver, Steering};
use convert64_core::{Compositor, SuperGrid};
use image::{Rgb, RgbImage};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// png2c64's ids with their Convert64 equivalents: the 30 methods both have.
/// png2c64's `sierra` is Sierra Lite and its `blue-noise` is interleaved gradient noise
/// over a 64 × 64 tile; they are paired with ours of the same name and family.
pub(crate) const SHARED: [(&str, DitherKind); 30] = [
    ("none", DitherKind::None),
    ("bayer4", DitherKind::Bayer4),
    ("bayer8", DitherKind::Bayer8),
    ("fs", DitherKind::FloydSteinberg),
    ("atkinson", DitherKind::Atkinson),
    ("sierra", DitherKind::SierraLite),
    ("checker", DitherKind::Checker),
    ("bayer2x2", DitherKind::Bayer2x2Mc),
    ("h2x4", DitherKind::H2x4),
    ("clustered", DitherKind::Clustered4),
    ("fs-wide", DitherKind::FsWide),
    ("jarvis", DitherKind::JarvisJudiceNinke),
    ("line2", DitherKind::LineH2),
    ("line-checker", DitherKind::LineChecker),
    ("line4", DitherKind::LineH4),
    ("line8", DitherKind::LineH8),
    ("line-fs", DitherKind::LineFs),
    ("halftone8", DitherKind::Halftone8),
    ("diagonal8", DitherKind::Diagonal8),
    ("spiral5", DitherKind::Spiral5),
    ("hex8", DitherKind::Hex8),
    ("hex5", DitherKind::Hex5),
    ("blue-noise", DitherKind::BlueNoise),
    ("ign", DitherKind::Ign),
    ("r2", DitherKind::R2),
    ("white-noise", DitherKind::WhiteNoise),
    ("crosshatch", DitherKind::Crosshatch),
    ("radial", DitherKind::Radial),
    ("value-noise", DitherKind::ValueNoise),
    ("ostromoukhov", DitherKind::Ostromoukhov),
];

/// Sheet geometry, as in the png2c64 sheets: six tiles of 320 × 200 per row, 8 px apart,
/// each under a 20 px label strip.
const COLUMNS: u32 = 6;
const TILE_W: u32 = 320;
const TILE_H: u32 = 200;
const GAP: u32 = 8;
const LABEL: u32 = 20;
const BACKGROUND: [u8; 3] = [30, 30, 30];
const TEXT: [u8; 3] = [225, 225, 225];

/// The grey-ramp crop the zoom sheet magnifies 4×: the mid-tones of the card's ramp.
const ZOOM: (u32, u32, u32, u32) = (80, 0, 80, 50);

fn family_name(f: DitherFamily) -> &'static str {
    match f {
        DitherFamily::None => "none",
        DitherFamily::Ordered => "ordered",
        DitherFamily::Diffusion => "diffusion",
        DitherFamily::Noise => "noise",
        DitherFamily::Effects => "effects",
        DitherFamily::PaletteAware => "palette-aware",
    }
}

/// A picture to render: its source, mode and strength.
struct Input {
    source: Arc<SourceImage>,
    mode: Mode,
    strength: f32,
}

fn load(path: &Path) -> Result<Arc<SourceImage>, String> {
    let img = image::open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .to_rgba8();
    let (w, h) = img.dimensions();
    SourceImage::from_srgba8(w, h, img.as_raw())
        .map(Arc::new)
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// The picture solved with `kind`, rendered at 320 × 200, and the preprocessed source it
/// was solved from, at the same size.
fn render(input: &Input, kind: DitherKind) -> Result<(RgbImage, Vec<Rgb8>), String> {
    let (w, h) = input.source.size();
    let mut layer = Layer::new("sheet", Arc::clone(&input.source));
    layer.set_transform(
        Transform::new(
            Vec2::ZERO,
            Vec2::new(320.0 / w as f32, 200.0 / h as f32),
            false,
            false,
        )
        .map_err(|e| e.to_string())?,
    );
    layer.set_recipe(Recipe::with_method(kind, input.strength).map_err(|e| e.to_string())?);
    layer.set_adjustments(Adjustments {
        gamma: Gamma::PNG2C64,
        ..Adjustments::default()
    });
    let grid = SuperGrid::with_default_factor(input.mode).map_err(|e| e.to_string())?;
    let mut c = Compositor::new(grid);
    c.update(&[layer], &[]);
    let palette = Palette::pepto_pal();
    let sol =
        Solver::new(&palette).solve(c.frame(), &Steering::new(), None, &SolveOptions::default());
    let px = sol.image.to_indexed();
    let x_scale = 320 / u32::from(px.width());
    let img = RgbImage::from_fn(320, 200, |x, y| {
        let c = palette.rgb(px.pixel((x / x_scale) as u16, y as u16));
        Rgb([c.r, c.g, c.b])
    });
    let (tw, _) = c.frame().grid().target_size();
    let colour = c.frame().colour();
    let source = (0..200u32)
        .flat_map(|y| (0..320u32).map(move |x| (x, y)))
        .map(|(x, y)| {
            let [r, g, b] = colour[(y * tw + x / x_scale) as usize].map(linear_to_srgb8);
            Rgb8::new(r, g, b)
        })
        .collect();
    Ok((img, source))
}

/// One render and its source, or why it failed.
type Rendered = Result<(RgbImage, Vec<Rgb8>), String>;

/// Every method rendered on `input`, in [`DitherKind::ALL`] order, several at a time.
fn render_all(input: &Input) -> Result<Vec<(DitherKind, RgbImage, Vec<Rgb8>)>, String> {
    let kinds = DitherKind::ALL;
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let mut out: Vec<Option<Rendered>> = (0..kinds.len()).map(|_| None).collect();
    std::thread::scope(|s| {
        for (chunk, slots) in kinds
            .chunks(kinds.len().div_ceil(threads))
            .zip(out.chunks_mut(kinds.len().div_ceil(threads)))
        {
            s.spawn(move || {
                for (k, slot) in chunk.iter().zip(slots) {
                    *slot = Some(render(input, *k));
                }
            });
        }
    });
    kinds
        .into_iter()
        .zip(out)
        .map(|(k, r)| r.expect("rendered").map(|(img, src)| (k, img, src)))
        .collect()
}

/// A grid of labelled tiles.
fn sheet(tiles: &[(String, RgbImage)]) -> RgbImage {
    let rows = (tiles.len() as u32).div_ceil(COLUMNS);
    let w = COLUMNS * TILE_W + (COLUMNS + 1) * GAP;
    let h = rows * (TILE_H + LABEL) + (rows + 1) * GAP;
    let mut img = RgbImage::from_pixel(w, h, Rgb(BACKGROUND));
    for (i, (label, tile)) in tiles.iter().enumerate() {
        let (col, row) = (i as u32 % COLUMNS, i as u32 / COLUMNS);
        let x = GAP + col * (TILE_W + GAP);
        let y = GAP + row * (TILE_H + LABEL + GAP);
        // Labels too long for the tile at double size go single size.
        let scale = if label.chars().count() as u32 * 12 + 4 > TILE_W {
            1
        } else {
            2
        };
        font::draw(&mut img, x + 2, y + 3 + 4 * (2 - scale), label, scale, TEXT);
        image::imageops::replace(&mut img, tile, i64::from(x), i64::from(y + LABEL));
    }
    img
}

/// The grey-ramp crop of a card render, magnified 4×.
fn zoom(card: &RgbImage) -> RgbImage {
    let (x, y, w, h) = ZOOM;
    let crop = image::imageops::crop_imm(card, x, y, w, h).to_image();
    image::imageops::resize(&crop, w * 4, h * 4, image::imageops::FilterType::Nearest)
}

fn to_rgb8(img: &RgbImage) -> Vec<Rgb8> {
    img.pixels()
        .map(|p| Rgb8::new(p.0[0], p.0[1], p.0[2]))
        .collect()
}

fn oklab(c: [f64; 3]) -> [f64; 3] {
    #[expect(clippy::cast_possible_truncation, reason = "colour channels")]
    let o = LinearRgb {
        r: c[0] as f32,
        g: c[1] as f32,
        b: c[2] as f32,
    }
    .to_oklab();
    [o.l, o.a, o.b].map(f64::from)
}

fn linear(img: &[Rgb8]) -> Vec<[f64; 3]> {
    img.iter()
        .map(|c| [c.r, c.g, c.b].map(|u| f64::from(srgb8_to_linear(u))))
        .collect()
}

/// The golden harness's blur: binomial `[1 4 6 4 1] / 16`, twice per axis, in linear
/// light, edges clamped.
fn blur(img: Vec<[f64; 3]>) -> Vec<[f64; 3]> {
    const TAPS: [f64; 5] = [1.0, 4.0, 6.0, 4.0, 1.0];
    let mut v = img;
    for horizontal in [true, false, true, false] {
        v = (0..320 * 200)
            .map(|i| {
                let (x, y) = ((i % 320) as i64, (i / 320) as i64);
                let mut acc = [0.0; 3];
                for (k, w) in (-2i64..=2).zip(TAPS) {
                    let (sx, sy) = if horizontal {
                        ((x + k).clamp(0, 319), y)
                    } else {
                        (x, (y + k).clamp(0, 199))
                    };
                    let p = v[(sy * 320 + sx) as usize];
                    for c in 0..3 {
                        acc[c] += w * p[c] / 16.0;
                    }
                }
                acc
            })
            .collect();
    }
    v
}

/// Per-pixel and blurred OKLab MSE of `a` against `source`.
fn mse(a: &[Rgb8], source: &[Rgb8]) -> (f64, f64) {
    let (la, ls) = (linear(a), linear(source));
    let per = |x: &[[f64; 3]], y: &[[f64; 3]]| {
        x.iter()
            .zip(y)
            .map(|(p, q)| {
                let (p, q) = (oklab(*p), oklab(*q));
                (0..3).map(|k| (p[k] - q[k]) * (p[k] - q[k])).sum::<f64>()
            })
            .sum::<f64>()
            / x.len() as f64
    };
    let plain = per(&la, &ls);
    let blurred = per(&blur(la), &blur(ls));
    (plain, blurred)
}

fn save(img: &RgbImage, path: &Path) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    // Best compression with adaptive filtering: these images have a few dozen colours
    // and compress several times smaller than with the defaults.
    let file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let encoder = image::codecs::png::PngEncoder::new_with_quality(
        std::io::BufWriter::new(file),
        image::codecs::png::CompressionType::Best,
        image::codecs::png::FilterType::Adaptive,
    );
    img.write_with_encoder(encoder)
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// Ours next to png2c64's for every shared id, with both scores against the source
/// under each pair.
fn versus(
    card: &[(DitherKind, RgbImage, Vec<Rgb8>)],
    theirs_dir: &Path,
) -> Result<(RgbImage, Vec<String>), String> {
    const PAIRS: u32 = 3;
    const NOTE: u32 = 20;
    let pair_w = 2 * TILE_W + GAP;
    let rows = (SHARED.len() as u32).div_ceil(PAIRS);
    let w = PAIRS * pair_w + (PAIRS + 1) * 2 * GAP;
    let h = rows * (LABEL + TILE_H + NOTE) + (rows + 1) * GAP;
    let mut img = RgbImage::from_pixel(w, h, Rgb(BACKGROUND));
    let mut lines = Vec::new();
    for (i, (theirs_id, kind)) in SHARED.iter().enumerate() {
        let (ours, _, source) = card
            .iter()
            .find(|(k, _, _)| k == kind)
            .map(|(k, img, src)| (img, k, src))
            .expect("every kind is rendered");
        let path = theirs_dir.join(format!("{theirs_id}.png"));
        let theirs = image::open(&path)
            .map_err(|e| format!("{}: {e}", path.display()))?
            .to_rgb8();
        let (o, ob) = mse(&to_rgb8(ours), source);
        let (t, tb) = mse(&to_rgb8(&theirs), source);
        let (col, row) = (i as u32 % PAIRS, i as u32 / PAIRS);
        let x = 2 * GAP + col * (pair_w + 2 * GAP);
        let y = GAP + row * (LABEL + TILE_H + NOTE + GAP);
        let label = if kind.id() == *theirs_id {
            format!("{theirs_id}: convert64 | png2c64")
        } else {
            format!("{} | png2c64 {theirs_id}", kind.id())
        };
        font::draw(&mut img, x + 2, y + 3, &label, 2, TEXT);
        image::imageops::replace(&mut img, ours, i64::from(x), i64::from(y + LABEL));
        image::imageops::replace(
            &mut img,
            &theirs,
            i64::from(x + TILE_W + GAP),
            i64::from(y + LABEL),
        );
        let note = format!("mse {o:.4} | {t:.4}   blur {ob:.4} | {tb:.4}");
        font::draw(&mut img, x + 2, y + LABEL + TILE_H + 4, &note, 2, TEXT);
        lines.push(format!(
            "| `{theirs_id}` | `{}` | {o:.4} | {t:.4} | {ob:.4} | {tb:.4} |",
            kind.id()
        ));
    }
    Ok((img, lines))
}

/// `cargo xtask dither-sheet`. With `input`, renders every method on that picture into
/// `out` (individual PNGs and `sheet.png`); without, regenerates the committed preview
/// set under `docs/reference/dither-previews/convert64-*/` and records `reason`.
pub(crate) fn run(
    root: &Path,
    input: Option<PathBuf>,
    mode: Mode,
    out: Option<PathBuf>,
    reason: Option<&str>,
) -> Result<(), String> {
    let previews = root.join("docs/reference/dither-previews");
    if let Some(path) = input {
        let out = out.ok_or("--input needs --out")?;
        let renders = render_all(&Input {
            source: load(&path)?,
            mode,
            strength: 1.0,
        })?;
        let tiles: Vec<(String, RgbImage)> = renders
            .iter()
            .map(|(k, img, _)| (k.id().to_owned(), img.clone()))
            .collect();
        for (k, img, _) in &renders {
            save(img, &out.join(format!("{}.png", k.id())))?;
        }
        save(&sheet(&tiles), &out.join("sheet.png"))?;
        println!("{} methods → {}", renders.len(), out.display());
        return Ok(());
    }
    let reason = reason.ok_or("regenerating the committed previews needs --reason")?;
    let card = render_all(&Input {
        source: load(&previews.join("testcard.png"))?,
        mode,
        strength: 1.0,
    })?;
    let portrait = render_all(&Input {
        source: load(&root.join("fixtures/portrait/source.png"))?,
        mode,
        strength: 0.7,
    })?;
    let label = |k: DitherKind| format!("{} ({})", k.id(), family_name(k.family()));
    for (dir, set) in [("convert64-card", &card), ("convert64-portrait", &portrait)] {
        for (k, img, _) in set.iter() {
            save(img, &previews.join(dir).join(format!("{}.png", k.id())))?;
        }
    }
    let sheets = previews.join("convert64-sheets");
    let card_tiles: Vec<(String, RgbImage)> = card
        .iter()
        .map(|(k, img, _)| (label(*k), img.clone()))
        .collect();
    save(&sheet(&card_tiles), &sheets.join("sheet-card.png"))?;
    let zoom_tiles: Vec<(String, RgbImage)> = card
        .iter()
        .map(|(k, img, _)| (format!("{} (grey ramp, 4x)", k.id()), zoom(img)))
        .collect();
    save(&sheet(&zoom_tiles), &sheets.join("sheet-zoom.png"))?;
    let portrait_tiles: Vec<(String, RgbImage)> = portrait
        .iter()
        .map(|(k, img, _)| (label(*k), img.clone()))
        .collect();
    save(&sheet(&portrait_tiles), &sheets.join("sheet-portrait.png"))?;
    let (vs, lines) = versus(&card, &previews.join("png2c64-card"))?;
    save(&vs, &sheets.join("sheet-vs-png2c64.png"))?;
    // The scores as a table, and the reason, next to the sheets.
    let mut table = String::from(
        "<!-- Written by `cargo xtask dither-sheet`; do not edit by hand. -->\n\n\
         OKLab MSE of each render on the test card against the card with png2c64's \
         preprocessing (gamma 1.5), per pixel and after the golden harness's blur.\n\n\
         | png2c64 id | Convert64 id | MSE, Convert64 | MSE, png2c64 | blurred, Convert64 | blurred, png2c64 |\n\
         |---|---|---|---|---|---|\n",
    );
    for l in &lines {
        table.push_str(l);
        table.push('\n');
    }
    std::fs::write(sheets.join("scores.md"), table).map_err(|e| e.to_string())?;
    let log = previews.join("CHANGELOG.md");
    let mut text = std::fs::read_to_string(&log).unwrap_or_else(|_| {
        "# Convert64 dither previews: changelog\n\nEvery regeneration of `convert64-*/` with \
         `cargo xtask dither-sheet --reason`.\n\n"
            .to_owned()
    });
    text.push_str(&format!("- {}: {reason}\n", crate::goldens::today()));
    std::fs::write(&log, text).map_err(|e| e.to_string())?;
    println!(
        "{} methods on the card and the portrait → {}",
        card.len(),
        previews.join("convert64-*").display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_shared_id_has_a_png2c64_render_and_a_distinct_kind() {
        let dir = crate::repo_root().join("docs/reference/dither-previews/png2c64-card");
        let mut kinds = std::collections::HashSet::new();
        for (id, kind) in SHARED {
            assert!(dir.join(format!("{id}.png")).exists(), "{id}");
            assert!(kinds.insert(kind), "{kind:?} twice");
        }
    }

    #[test]
    fn every_method_label_can_be_drawn() {
        let mut img = RgbImage::new(400, 20);
        for kind in DitherKind::ALL {
            img.fill(0);
            font::draw(&mut img, 0, 0, kind.id(), 1, [255; 3]);
            let lit = img.pixels().filter(|p| p.0[0] > 0).count();
            // Every character but spaces draws something.
            assert!(lit >= kind.id().len() * 3, "{}", kind.id());
        }
    }
}
