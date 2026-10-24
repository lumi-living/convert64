//! `cargo xtask png2amiga-methods-sheet` (#275, ADR-014 §6): png2amiga's 32 methods
//! that Convert64 did not have, each run on png2c64's pipeline
//! (`png2c64_ref::amiga::convert`: png2c64's front end and cell stage, png2amiga's
//! threshold pattern or per-pixel pass), for a human to judge by eye. One sheet per
//! family of the catalogue (`docs/reference/dither-methods.md` §7), one row per input
//! (the six fixtures and the dither-preview test card), the png2c64 baseline first, then
//! the family's methods; under each tile its id and the share of its pixels identical to
//! the baseline. No SSIMULACRA2 (ADR-014 §7).
//!
//! Flags: png2c64's baseline ones (multicolor, strength 0.7, error clamp 0.1, gamma 1.5,
//! Pepto, cell metric `mse`); lineart in hi-res, as its baseline is (whose method is
//! `none`). The baseline tile is png2c64's port at those flags with `checker` (lineart:
//! `none`); the Markdown checks it against the committed `fixtures/<name>/baseline.png`.
//!
//! Writes `docs/reference/dither-previews/png2amiga-methods/sheet-<family>.png` and
//! `sheet-<family>.md`.

use crate::cause_sheet::{draw, palette};
use crate::font;
use crate::reference_sheet::{identical, png};
use crate::repo_root;
use crate::sheet::save;
use convert64_core::layer::DitherKind;
use convert64_core::solve::png2amiga_ref::dither::Method as P2a;
use convert64_core::solve::png2c64_ref::amiga::{self, Port};
use convert64_core::solve::png2c64_ref::{self as p2c, quantize};
use convert64_core::solve::reference_png2amiga::ref_method;
use image::{Rgb, RgbImage};

const TILE_W: u32 = 320;
const TILE_H: u32 = 200;
const GAP: u32 = 8;
const LABEL: u32 = 14;
const LINE: u32 = 10;
/// Text lines under a tile: id, identical share, how it runs.
const NOTES: u32 = 3;
const BACKGROUND: [u8; 3] = [30, 30, 30];
const TEXT: [u8; 3] = [225, 225, 225];
const DIM: [u8; 3] = [160, 160, 160];

/// The baseline's strength and error clamp, every method's too.
const STRENGTH: f32 = 0.7;
const ERROR_CLAMP: f32 = 0.1;

/// The inputs: name, source path (from the repository root), hi-res, and the committed
/// png2c64 baseline to check the baseline tile against.
const INPUTS: [(&str, &str, bool, Option<&str>); 7] = [
    (
        "portrait",
        "fixtures/portrait/source.png",
        false,
        Some("fixtures/portrait/baseline.png"),
    ),
    (
        "terrain",
        "fixtures/terrain/source.png",
        false,
        Some("fixtures/terrain/baseline.png"),
    ),
    (
        "title",
        "fixtures/title/source.png",
        false,
        Some("fixtures/title/baseline.png"),
    ),
    (
        "gradient",
        "fixtures/gradient/source.png",
        false,
        Some("fixtures/gradient/baseline.png"),
    ),
    (
        "photo",
        "fixtures/photo/source.png",
        false,
        Some("fixtures/photo/baseline.png"),
    ),
    (
        "lineart",
        "fixtures/lineart/source.png",
        true,
        Some("fixtures/lineart/baseline.png"),
    ),
    (
        "testcard",
        "docs/reference/dither-previews/testcard.png",
        false,
        None,
    ),
];

/// One family's sheet: its file slug, its name and its methods, with what each is on
/// png2amiga's C64 path.
struct Family {
    slug: &'static str,
    name: &'static str,
    methods: &'static [(DitherKind, &'static str)],
}

const FAMILIES: [Family; 5] = [
    Family {
        slug: "ordered",
        name: "ordered",
        methods: &[
            (
                DitherKind::Bayer3,
                "dispersed-dot 3 x 3 (magic-square permutation)",
            ),
            (
                DitherKind::Bayer5,
                "dispersed-dot 5 x 5 (magic-square permutation)",
            ),
            (
                DitherKind::Bayer6,
                "dispersed-dot 6 x 6 (magic-square permutation)",
            ),
            (
                DitherKind::Bayer7,
                "dispersed-dot 7 x 7 (magic-square permutation)",
            ),
            (DitherKind::V4x2, "4 x 2 literal matrix"),
            (DitherKind::Bayer4x2, "4 x 2 Bayer"),
            (DitherKind::Bayer2x4, "2 x 4 Bayer"),
            (
                DitherKind::LineVChecker,
                "column-biased 2 x 2 (`vline-checker`)",
            ),
            (
                DitherKind::Fractal16,
                "16 x 16 self-nested Bayer (Obra Dinn)",
            ),
            (DitherKind::AsepriteOld, "Aseprite's old 4 x 4"),
            (DitherKind::Libcaca3, "libcaca's 3 x 3"),
            (DitherKind::Libcaca6, "libcaca's 6 x 6"),
            (DitherKind::Pegasus, "Pegasus' 8 x 8"),
        ],
    },
    Family {
        slug: "noise",
        name: "ordered noise",
        methods: &[
            (
                DitherKind::VoidCluster,
                "64 x 64 void-and-cluster blue noise",
            ),
            (
                DitherKind::ClusterNoise,
                "64 x 64 void-and-cluster, clustered",
            ),
            (
                DitherKind::CranleyBayer,
                "Bayer 8 with a hashed offset per 8 x 8 tile",
            ),
            (
                DitherKind::IgnTri,
                "interleaved gradient noise, triangular distribution",
            ),
            (DitherKind::R2Tri, "R2 sequence, triangular distribution"),
        ],
    },
    Family {
        slug: "effects",
        name: "effects",
        methods: &[
            (DitherKind::Quasicrystal, "sum of five cosines"),
            (
                DitherKind::Truchet,
                "quarter-arc Truchet tiles, 8 x 8 (threshold up to 0.915)",
            ),
        ],
    },
    Family {
        slug: "diffusion",
        name: "error diffusion",
        methods: &[
            (
                DitherKind::FloydSteinbergOklab,
                "Floyd-Steinberg with the residual in OKLab (a, b clamped at half the limit)",
            ),
            (
                DitherKind::StructureFs,
                "Floyd-Steinberg in sRGB with a Laplacian bias on L, gated by the 5 x 5 deviation",
            ),
            (
                DitherKind::ContrastFs,
                "Floyd-Steinberg in sRGB with a local-contrast bias on L, gated by the 5 x 5 deviation",
            ),
            (
                DitherKind::Gilbert,
                "no kernel on the C64 path: a plain nearest pick",
            ),
            (
                DitherKind::Dbs,
                "no kernel on the C64 path: a plain nearest pick",
            ),
        ],
    },
    Family {
        slug: "palette-aware",
        name: "palette-aware",
        methods: &[
            (
                DitherKind::Yliluoma,
                "64-entry greedy mixing plan on Bayer 8 (Yliluoma's alg. 2, plain)",
            ),
            (
                DitherKind::OptChecker,
                "nearest colour and its best partner, alternated by checker",
            ),
            (
                DitherKind::OptLine,
                "nearest colour and its best partner, alternated by row",
            ),
            (
                DitherKind::OptLineChecker,
                "4-entry plan on the line-checker matrix",
            ),
            (
                DitherKind::OptVline,
                "nearest colour and its best partner, alternated by column",
            ),
            (
                DitherKind::OptVlineChecker,
                "4-entry plan on the column checker",
            ),
            (
                DitherKind::TriTone,
                "Yliluoma's tri-tone: 4-entry plan on Bayer 2",
            ),
        ],
    },
];

/// The one-line note under a tile: how it runs here.
fn port_note(m: P2a) -> &'static str {
    match Port::of(m) {
        Port::Threshold => "threshold x 0.49 into png2c64's search",
        Port::Pass if m.diffuses() => "png2amiga diffusion in png2c64's cells",
        Port::Pass => "png2amiga picker in png2c64's cells",
        Port::NearestPick => "nearest pick: = png2c64 none (labelled)",
    }
}

/// What the method does on png2c64's pipeline, for the Markdown.
fn port_text(m: P2a) -> &'static str {
    match Port::of(m) {
        Port::Threshold => {
            "png2amiga's threshold as a png2c64 ordered method: `t × 0.7 × 0.7` biases \
             png2c64's cell search (L by 0.15 t, a and b by 0.03 t), each pixel its \
             cell's nearest colour to the biased target"
        }
        Port::Pass if m == P2a::FloydSteinbergOklab => {
            "png2c64's cells for a method without a threshold (unbiased search, then the \
             global Floyd–Steinberg replacement), then png2amiga's pass in them: residual \
             in OKLab, clamped on read (L 0.1, a and b 0.05), × 0.7"
        }
        Port::Pass if m.diffuses() => {
            "png2c64's cells for a method without a threshold (unbiased search, then the \
             global Floyd–Steinberg replacement), then png2amiga's pass in them: its L \
             bias, the residual in gamma-encoded sRGB clamped on read at 0.1, × 0.7"
        }
        Port::Pass => {
            "png2c64's cells for a method without a threshold (unbiased search, then the \
             global Floyd–Steinberg replacement), then png2amiga's picker at strength 0.7 \
             on each pixel's own colour, within the cell's colours"
        }
        Port::NearestPick => {
            "**not a port of a real algorithm**: png2c64's cells as for `none`, then \
             png2amiga's nearest pick, so it is png2c64's `none` (to within png2amiga's \
             colour maths)"
        }
    }
}

fn faithful(m: P2a) -> &'static str {
    match Port::of(m) {
        Port::Threshold => "pattern exact; role and amplitude png2c64's",
        Port::Pass if m.diffuses() => "pass exact; cells png2c64's",
        Port::Pass => "picker exact; cells png2c64's",
        Port::NearestPick => "no: nothing to port on C64",
    }
}

/// One input, prepared: its screens per method id.
struct Row {
    name: &'static str,
    hires: bool,
    baseline: quantize::Screen,
    /// The committed png2c64 baseline, against the baseline tile, %.
    check: Option<f64>,
    /// png2c64's own `none` and `fs` at the same flags (the diffusion family's nearest
    /// existing methods).
    none: RgbImage,
    fs: RgbImage,
    image: p2c::Image,
}

fn mode_of(hires: bool) -> quantize::Mode {
    if hires {
        quantize::Mode::Hires
    } else {
        quantize::Mode::Multicolor
    }
}

fn settings(method: p2c::dither::Method) -> p2c::dither::Settings {
    p2c::dither::Settings {
        method,
        strength: STRENGTH,
        error_clamp: ERROR_CLAMP,
        serpentine: true,
        adaptive: 0.0,
    }
}

fn prepare(
    name: &'static str,
    path: &str,
    hires: bool,
    check: Option<&str>,
) -> Result<Row, String> {
    let root = repo_root();
    let img = image::open(root.join(path))
        .map_err(|e| format!("{path}: {e}"))?
        .to_rgb8();
    let rgb: Vec<[u8; 3]> = img.pixels().map(|p| p.0).collect();
    let mode = mode_of(hires);
    let image = p2c::prepare(
        &rgb,
        (img.width() as usize, img.height() as usize),
        mode,
        &p2c::preprocess::Settings::default(),
        None,
    );
    let pal = palette();
    let base_method = if hires {
        p2c::dither::Method::None
    } else {
        p2c::dither::Method::Checker
    };
    let baseline = p2c::convert(&image, &pal, mode, &settings(base_method));
    let check = match check {
        Some(p) => Some(identical(&draw(&baseline), &png(&root.join(p))?)),
        None => None,
    };
    let none = draw(&p2c::convert(
        &image,
        &pal,
        mode,
        &settings(p2c::dither::Method::None),
    ));
    let fs = draw(&p2c::convert(
        &image,
        &pal,
        mode,
        &settings(p2c::dither::Method::FloydSteinberg),
    ));
    Ok(Row {
        name,
        hires,
        baseline,
        check,
        none,
        fs,
        image,
    })
}

/// A tile: the label above, the picture, the lines under it.
struct Tile {
    label: String,
    img: RgbImage,
    lines: Vec<(String, [u8; 3])>,
}

/// `text` broken into lines of at most `width` characters, at spaces.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = vec![String::new()];
    for word in text.split(' ') {
        let last = lines.last_mut().expect("never empty");
        if !last.is_empty() && last.len() + 1 + word.len() > width {
            lines.push(word.to_owned());
        } else {
            if !last.is_empty() {
                last.push(' ');
            }
            last.push_str(word);
        }
    }
    lines
}

fn draw_sheet(rows: &[Vec<Tile>], header: &[String]) -> RgbImage {
    let columns = rows.iter().map(Vec::len).max().unwrap_or(1) as u32;
    let row_h = LABEL + TILE_H + NOTES * LINE + 2 + GAP;
    let w = columns * TILE_W + (columns + 1) * GAP;
    // The header wrapped to the sheet's width (6 px a character).
    let header: Vec<String> = header
        .iter()
        .flat_map(|h| wrap(h, ((w - 2 * GAP) / 6) as usize))
        .collect();
    let head_h = header.len() as u32 * LINE + GAP;
    let h = GAP + head_h + rows.len() as u32 * row_h;
    let mut img = RgbImage::from_pixel(w, h, Rgb(BACKGROUND));
    for (i, line) in header.iter().enumerate() {
        font::draw(&mut img, GAP, GAP + i as u32 * LINE, line, 1, TEXT);
    }
    for (r, tiles) in rows.iter().enumerate() {
        let y = GAP + head_h + r as u32 * row_h;
        for (c, t) in tiles.iter().enumerate() {
            let x = GAP + c as u32 * (TILE_W + GAP);
            font::draw(&mut img, x + 1, y + 2, &t.label, 1, TEXT);
            image::imageops::replace(&mut img, &t.img, i64::from(x), i64::from(y + LABEL));
            for (k, (line, colour)) in t.lines.iter().enumerate() {
                font::draw(
                    &mut img,
                    x + 1,
                    y + LABEL + TILE_H + 2 + k as u32 * LINE,
                    line,
                    1,
                    *colour,
                );
            }
        }
    }
    img
}

pub(crate) fn run() -> Result<(), String> {
    let root = repo_root();
    let out = root.join("docs/reference/dither-previews/png2amiga-methods");
    let pal = palette();
    let mut rows = Vec::new();
    for (name, path, hires, check) in INPUTS {
        rows.push(prepare(name, path, hires, check)?);
        eprintln!("png2amiga-methods-sheet: {name} prepared");
    }
    for fam in &FAMILIES {
        // Per input, per method: its picture.
        let mut grid: Vec<Vec<RgbImage>> = Vec::new();
        for row in &rows {
            let mode = mode_of(row.hires);
            let pics = fam
                .methods
                .iter()
                .map(|&(kind, _)| {
                    let m = ref_method(kind).expect("a png2amiga method");
                    draw(&amiga::convert(
                        &row.image,
                        &pal,
                        mode,
                        m,
                        STRENGTH,
                        ERROR_CLAMP,
                    ))
                })
                .collect();
            grid.push(pics);
            eprintln!("png2amiga-methods-sheet: {} {} done", fam.slug, row.name);
        }
        let tiles: Vec<Vec<Tile>> = rows
            .iter()
            .zip(&grid)
            .map(|(row, pics)| {
                let base = draw(&row.baseline);
                let mut t = vec![Tile {
                    label: format!(
                        "{} 1. png2c64 baseline: {}",
                        row.name,
                        if row.hires {
                            "hires none"
                        } else {
                            "checker 0.7"
                        }
                    ),
                    lines: vec![
                        (
                            if row.hires {
                                "none (hires)".to_owned()
                            } else {
                                "checker".to_owned()
                            },
                            TEXT,
                        ),
                        ("the baseline".to_owned(), TEXT),
                        (
                            if row.hires {
                                "png2c64 port, hires, gamma 1.5".to_owned()
                            } else {
                                "png2c64 port, mc, clamp 0.1, gamma 1.5".to_owned()
                            },
                            DIM,
                        ),
                    ],
                    img: base.clone(),
                }];
                for (k, (&(kind, _), pic)) in fam.methods.iter().zip(pics).enumerate() {
                    let m = ref_method(kind).expect("a png2amiga method");
                    t.push(Tile {
                        label: format!("{} {}. {}", row.name, k + 2, kind.id()),
                        lines: vec![
                            (kind.id().to_owned(), TEXT),
                            (
                                format!("identical to baseline: {:.3} %", identical(pic, &base)),
                                TEXT,
                            ),
                            (port_note(m).to_owned(), DIM),
                        ],
                        img: pic.clone(),
                    });
                }
                t
            })
            .collect();
        let header = vec![
            format!(
                "png2amiga's {} methods on png2c64's pipeline (#275): png2c64's front end (mitchell, gamma 1.5) and cell stage, png2amiga's threshold or per-pixel pass. flags: png2c64's baseline, multicolor, strength 0.7, error clamp 0.1, pepto, metric mse, lineart hires (its baseline is hires none).",
                fam.name
            ),
            "tile 1 is the png2c64 baseline (checker 0.7, lineart none), rendered by png2c64's port. under each tile: its id, the share of its pixels identical to tile 1, and how it runs here. no ssimulacra2 on this sheet.".to_owned(),
            match fam.slug {
                "diffusion" => "the pass is png2amiga's (residual in gamma-encoded srgb, floyd-steinberg-oklab in oklab, clamped on read at 0.1, x 0.7) in png2c64's cells for a method without a threshold (unbiased search, then the global floyd-steinberg replacement). gilbert and dbs have no kernel on png2amiga's c64 path (a plain nearest pick there), so here they are png2c64's none: shown for completeness, not a port of curve diffusion or direct binary search.".to_owned(),
                "palette-aware" => "the pickers choose among the cell's colours, which here are png2c64's for a method without a threshold (unbiased search, then the global floyd-steinberg replacement), as png2amiga takes its own from a global floyd-steinberg pass.".to_owned(),
                _ => "ordered semantics are png2c64's: the threshold x strength x strength (0.49) biases the cell search and each pixel is its cell's nearest colour to the biased target (png2amiga: x 0.7, after its own cells).".to_owned(),
            },
        ];
        save(
            &draw_sheet(&tiles, &header),
            &out.join(format!("sheet-{}.png", fam.slug)),
        )?;
        let md = markdown(fam, &rows, &grid);
        std::fs::write(out.join(format!("sheet-{}.md", fam.slug)), &md)
            .map_err(|e| e.to_string())?;
        println!("{md}");
    }
    Ok(())
}

fn markdown(fam: &Family, rows: &[Row], grid: &[Vec<RgbImage>]) -> String {
    let mut md = format!(
        "<!-- Written by `cargo xtask png2amiga-methods-sheet`; do not edit by hand. -->\n\n\
         # png2amiga's {} methods on png2c64's pipeline (#275)\n\n\
         [sheet-{}.png](sheet-{}.png): each method on png2c64's front end (4-tap Mitchell, \
         gamma 1.5) and cell stage, with png2amiga's threshold pattern or per-pixel pass \
         (`png2c64_ref::amiga::convert`), at png2c64's baseline flags: multicolor, strength \
         0.7, error clamp 0.1, Pepto, cell metric `mse`; lineart in hi-res, as its baseline \
         is. Share of each tile's pixels identical to the png2c64 baseline (checker 0.7; \
         lineart hi-res `none`), at 320 × 200 through Pepto. No SSIMULACRA2.\n\n",
        fam.name, fam.slug, fam.slug
    );
    md.push_str("| Input |");
    for (kind, _) in fam.methods {
        md.push_str(&format!(" `{}` |", kind.id()));
    }
    md.push_str("\n|---|");
    md.push_str(&"---|".repeat(fam.methods.len()));
    md.push('\n');
    for (row, pics) in rows.iter().zip(grid) {
        let base = draw(&row.baseline);
        md.push_str(&format!("| {} |", row.name));
        for pic in pics {
            md.push_str(&format!(" {:.3} % |", identical(pic, &base)));
        }
        md.push('\n');
    }
    if fam.slug == "diffusion" {
        md.push_str(
            "\nThe same tiles against png2c64's nearest existing methods at the same flags \
             (`fs`: its Floyd–Steinberg, residual in OKLab clamped on read and write; \
             `none`):\n\n| Input |",
        );
        for (kind, _) in fam.methods {
            md.push_str(&format!(" `{}` vs `fs` | vs `none` |", kind.id()));
        }
        md.push_str("\n|---|");
        md.push_str(&"---|---|".repeat(fam.methods.len()));
        md.push('\n');
        for (row, pics) in rows.iter().zip(grid) {
            md.push_str(&format!("| {} |", row.name));
            for pic in pics {
                md.push_str(&format!(
                    " {:.3} % | {:.3} % |",
                    identical(pic, &row.fs),
                    identical(pic, &row.none)
                ));
            }
            md.push('\n');
        }
    }
    md.push_str(
        "\n| Id | On png2amiga's C64 path | Here, on png2c64's pipeline | Faithful | \
         png2amiga's own render |\n|---|---|---|---|---|\n",
    );
    for &(kind, what) in fam.methods {
        let m = ref_method(kind).expect("a png2amiga method");
        let theirs = if Port::of(m) == Port::Threshold {
            format!(
                "{what}; offsets the target by `t × 0.7` (L 0.15, a and b 0.03) after the \
                 cell colours are chosen (global Floyd–Steinberg top 3, background black)"
            )
        } else if m.diffuses() || m.is_yliluoma() {
            format!(
                "{what}; within the cell colours png2amiga chose (global Floyd–Steinberg \
                 top 3, background black; hi-res the best-mixing pair)"
            )
        } else {
            what.to_owned()
        };
        md.push_str(&format!(
            "| `{}` | {} | {} | {} | [card](../png2amiga-card/{}.png) |\n",
            kind.id(),
            theirs,
            port_text(m),
            faithful(m),
            m.name()
        ));
    }
    md.push_str(
        "\nCheck: the baseline tile (png2c64's port) against the committed png2c64 \
         baseline (`fixtures/<name>/baseline.png`; the test card has none at strength 0.7):\n\n\
         | Input | Identical |\n|---|---|\n",
    );
    for row in rows {
        md.push_str(&format!(
            "| {} | {} |\n",
            row.name,
            row.check
                .map_or_else(|| "n/a".to_owned(), |v| format!("{v:.3} %"))
        ));
    }
    md
}
