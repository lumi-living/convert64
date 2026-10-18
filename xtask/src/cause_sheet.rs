//! `cargo xtask cause-sheet` (#273): which of png2amiga's differences from png2c64 cost
//! the quality Jaakko sees by eye. One row per fixture, one tile per swap, and under each
//! tile its exact flags and the share of its pixels identical to the png2c64 baseline
//! (tile 1). No SSIMULACRA2 here: it disagreed with the eye on every #273 sheet.
//!
//! Every tile is rendered by the ports directly (`png2amiga_ref`, `png2c64_ref`), at
//! png2c64's baseline flags (multicolor: checker 0.7, error clamp 0.1, gamma 1.5, Pepto,
//! cell metric `mse`; lineart: hi-res, `none`, gamma 1.5), unless the tile says
//! otherwise:
//!
//! 1. the png2c64 baseline (`fixtures/<name>/baseline.png`);
//! 2. png2amiga's baseline as #273's `sheet-reference-baselines.png` showed it
//!    (`fixtures/png2amiga/<name>/baseline.png`, png2amiga's own render);
//! 3. png2amiga's port at those flags (equals tile 2);
//! 4. tile 3 with Background: Search (png2c64's 16-way search);
//! 5. tile 4 with png2c64's 4-tap Mitchell instead of png2amiga's widened Lanczos-3;
//! 6. png2c64's port with png2amiga's widened Lanczos-3 (the reverse swap);
//! 7. tile 4 with png2c64's colour maths (`PNG2C64_MATHS`);
//! 8. tile 4 with png2c64's cell-colour stage (its search, and for `none` its global
//!    Floyd–Steinberg replacement) instead of png2amiga's, png2amiga's dither pass kept;
//! 9. tile 4 with png2c64's ordered amplitude: png2c64 applies the strength twice
//!    (`0.7 × 0.7 = 0.49`), png2amiga once;
//! 10. tile 8 with that amplitude too;
//! 11. tiles 5, 8 and 9 together: png2amiga's port with png2c64's scaler, cell colours
//!     and amplitude, which must come back to png2c64's baseline;
//! 12. (lineart) tile 4 with the cell colours limited to the greys (the recipe's allowed
//!     colours `0, 1, 11, 12, 15`);
//! 13. (lineart) tile 12 with png2amiga's `--black-point 0.5` (levels on OKLab L).
//!
//! Also written: `sheet-cause.md` (every tile's percentage, the checks, and lineart's
//! evidence) and `sheet-cause-lineart.png` (lineart crops, magnified).

use crate::font;
use crate::reference_sheet::{identical, png, source};
use crate::repo_root;
use crate::sheet::save;
use convert64_core::solve::png2amiga_ref::{self as p2a, c64, colour::PNG2C64_MATHS};
use convert64_core::solve::png2c64_ref::{self as p2c, quantize};
use image::{Rgb, RgbImage, RgbaImage};
use std::sync::atomic::Ordering;

const TILE_W: u32 = 320;
const TILE_H: u32 = 200;
const GAP: u32 = 8;
const LABEL: u32 = 14;
const LINE: u32 = 10;
/// Note lines under a tile.
const NOTES: u32 = 6;
/// Characters per note line at scale 1 (6 px each).
const WRAP: usize = 52;
const BACKGROUND: [u8; 3] = [30, 30, 30];
const TEXT: [u8; 3] = [225, 225, 225];
const DIM: [u8; 3] = [160, 160, 160];

/// The six fixtures and whether each is hi-res (lineart's png2c64 baseline is).
const FIXTURES: [(&str, bool); 6] = [
    ("portrait", false),
    ("terrain", false),
    ("title", false),
    ("gradient", false),
    ("photo", false),
    ("lineart", true),
];

/// Pepto's greys, ascending in lightness: black, dark grey, grey, light grey, white.
const GREYS: [u8; 5] = [0, 11, 12, 15, 1];

/// Which scaler a front end uses.
#[derive(Clone, Copy, PartialEq)]
enum Scaler {
    /// png2amiga's widened Lanczos-3.
    Lanczos,
    /// png2c64's 4-tap Mitchell.
    Mitchell,
}

impl Scaler {
    fn name(self) -> &'static str {
        match self {
            Self::Lanczos => "scaler png2amiga widened lanczos-3",
            Self::Mitchell => "scaler png2c64 4-tap mitchell",
        }
    }
}

/// One fixture's inputs.
struct Fixture {
    rgb: Vec<[u8; 3]>,
    size: (usize, usize),
    mode: p2a::Mode,
}

impl Fixture {
    fn load(name: &str, hires: bool) -> Result<Self, String> {
        let img: RgbaImage = source(name)?;
        Ok(Self {
            rgb: img.pixels().map(|p| [p.0[0], p.0[1], p.0[2]]).collect(),
            size: (img.width() as usize, img.height() as usize),
            mode: if hires {
                p2a::Mode::Hires
            } else {
                p2a::Mode::Multicolor
            },
        })
    }

    fn hires(&self) -> bool {
        self.mode == p2a::Mode::Hires
    }

    /// The source scaled to the logical raster with `scaler` (none if already that size).
    fn scaled(&self, scaler: Scaler) -> p2c::Image {
        let src = p2c::Image::from_srgb8(self.size.0, self.size.1, &self.rgb);
        let (tw, th) = self.mode.screen();
        if self.size == (tw, th) {
            return src;
        }
        match scaler {
            Scaler::Lanczos => p2a::scale::resample(&src, tw, th),
            Scaler::Mitchell => {
                p2c::scale::bicubic(&src, tw, th, p2c::scale::Kernel::MitchellNetravali)
            }
        }
    }

    /// png2amiga's front end (`prepare`) with `scaler`, at gamma 1.5 and png2amiga's
    /// `--black-point` (0: off).
    fn p2a_front(&self, scaler: Scaler, black_point: f32) -> p2c::Image {
        let mut im = self.scaled(scaler);
        p2a::preprocess::apply(
            &mut im,
            &p2a::preprocess::Settings {
                gamma: 1.5,
                black_point,
                ..p2a::preprocess::Settings::default()
            },
        );
        im
    }

    /// png2c64's front end (`prepare`) with `scaler`, at its default gamma 1.5.
    fn p2c_front(&self, scaler: Scaler) -> p2c::Image {
        let mut im = self.scaled(scaler);
        p2c::preprocess::apply(&mut im, &p2c::preprocess::Settings::default());
        im
    }

    /// png2c64's settings as its baseline command gives them.
    fn p2c_settings(&self) -> p2c::dither::Settings {
        if self.hires() {
            p2c::dither::Settings {
                method: p2c::dither::Method::None,
                ..p2c::dither::Settings::default()
            }
        } else {
            p2c::dither::Settings {
                method: p2c::dither::Method::Checker,
                strength: 0.7,
                error_clamp: 0.1,
                serpentine: true,
                adaptive: 0.0,
            }
        }
    }

    /// png2amiga's encoder on `im` at the baseline flags (multicolor: checker at
    /// `strength`, error clamp 0.1; hi-res: `none`), cell metric `mse`.
    fn p2a_encode(&self, im: &p2c::Image, strength: f32, hooks: &c64::Hooks) -> quantize::Screen {
        let (w, h) = self.mode.screen();
        let px = vec![
            p2a::dither::PxSettings {
                method: if self.hires() {
                    p2a::dither::Method::None
                } else {
                    p2a::dither::Method::Checker
                },
                strength,
                error_clamp: if self.hires() { 0.35 } else { 0.1 },
                origin: (0, 0),
            };
            w * h
        ];
        c64::encode(
            im,
            &palette(),
            self.mode,
            &px,
            c64::Metric::Mse,
            true,
            hooks,
        )
    }

    /// png2c64's cell-colour stage on `im` (its search biased by the ordered threshold
    /// at `strength²`, and for `none` its global Floyd–Steinberg replacement), as
    /// png2amiga hooks: every cell's colours and the background.
    fn p2c_sets(&self, im: &p2c::Image) -> c64::Hooks {
        let (w, h) = self.mode.screen();
        let s = p2c::dither::PxSettings::from(&self.p2c_settings());
        let bias: Vec<Option<f32>> = (0..w * h)
            .map(|i| s.bias_levels(i % w, i / w, None))
            .collect();
        let screen = quantize::quantize_px(
            im,
            &palette(),
            self.mode,
            &bias,
            &quantize::Hooks::default(),
        );
        c64::Hooks {
            background: (!self.hires()).then_some(screen.bg),
            sets: Some(screen.cells.iter().map(|c| c.colors.clone()).collect()),
            ..c64::Hooks::default()
        }
    }
}

fn palette() -> [p2c::Rgb; 16] {
    p2a::colour::palette_linear(&p2a::colour::PEPTO_HEX)
}

/// A screen drawn at 320 × 200 through Pepto.
fn draw(screen: &quantize::Screen) -> RgbImage {
    let (w, _) = screen.mode.screen();
    let idx = screen.indices();
    let step = 320 / w;
    RgbImage::from_fn(320, 200, |x, y| {
        let i = idx[y as usize * w + x as usize / step];
        let [_, r, g, b] = p2a::colour::PEPTO_HEX[usize::from(i)].to_be_bytes();
        Rgb([r, g, b])
    })
}

/// One tile: its number and name, its flags and picture, and the screen behind it.
struct Tile {
    n: u8,
    name: &'static str,
    flags: Vec<String>,
    img: RgbImage,
    screen: Option<quantize::Screen>,
}

impl Tile {
    /// The multicolor background the tile used, if it is a solve of ours.
    fn background(&self) -> Option<u8> {
        self.screen
            .as_ref()
            .filter(|s| s.mode == p2a::Mode::Multicolor)
            .map(|s| s.bg)
    }
}

/// The method flags of a tile at the baseline settings with ordered strength `strength`.
fn method_flags(hires: bool, strength: &str) -> String {
    if hires {
        "hires, dither none, gamma 1.5, pepto, metric mse".to_owned()
    } else {
        format!("mc, checker {strength}, error clamp 0.1, gamma 1.5, pepto, metric mse")
    }
}

/// What png2c64's port is besides its scaler.
const P2C_REST: &str = "png2c64 maths, cells and pixel pick";
/// What png2amiga's port is besides its scaler.
const P2A_REST: &str = "png2amiga maths, cells and dither pass";

/// What else lineart's row says.
struct Check {
    /// png2c64's port as it is (Mitchell, its front end and encoder) against tile 1, %.
    port_vs_baseline: f64,
    /// The background png2c64's search chose on its own front end.
    port_bg: Option<u8>,
}

/// Tile 13 of lineart's row, as the cause sheet renders it: png2amiga's port with its
/// widened Lanczos-3, gamma 1.5 and `--black-point 0.5`, hi-res `none`, the cells limited
/// to Pepto's greys (`cargo xtask lineart-sheet` compares the line-art recipe with it).
pub(crate) fn lineart_tile13() -> Result<RgbImage, String> {
    let f = Fixture::load("lineart", true)?;
    let greys = GREYS.iter().fold(0u16, |m, &g| m | (1 << g));
    let greys_only = c64::Hooks {
        rules: vec![
            quantize::CellRule {
                candidates: greys,
                required: 0,
            };
            1000
        ],
        ..c64::Hooks::default()
    };
    Ok(draw(&f.p2a_encode(
        &f.p2a_front(Scaler::Lanczos, 0.5),
        1.0,
        &greys_only,
    )))
}

/// Every tile of one fixture's row.
fn row(name: &str, hires: bool) -> Result<(Vec<Tile>, Check), String> {
    let root = repo_root();
    let f = Fixture::load(name, hires)?;
    let (s, s2) = if hires { (1.0, 1.0) } else { (0.7, 0.49) };
    let bg = |search: bool| match (hires, search) {
        (true, false) => "no background (hires)",
        (true, true) => "no background (hires): background = tile 3",
        (false, false) => "background black (0)",
        (false, true) => "background search",
    };
    let search = c64::Hooks {
        search_background: !hires,
        ..c64::Hooks::default()
    };
    let solved = |n: u8, name: &'static str, flags: Vec<String>, screen: quantize::Screen| Tile {
        n,
        name,
        flags,
        img: draw(&screen),
        screen: Some(screen),
    };
    let mut tiles = Vec::new();

    // 1 and 2: the tools' own baselines.
    tiles.push(Tile {
        n: 1,
        name: "png2c64 baseline",
        flags: vec![
            "png2c64's own render (fixtures/<name>)".into(),
            method_flags(hires, "0.7"),
            format!(
                "{}, {}",
                if hires {
                    "no background (hires)"
                } else {
                    "background search"
                },
                Scaler::Mitchell.name()
            ),
            P2C_REST.into(),
        ],
        img: png(&root.join(format!("fixtures/{name}/baseline.png")))?,
        screen: None,
    });
    tiles.push(Tile {
        n: 2,
        name: "png2amiga as shipped (#273)",
        flags: vec![
            "png2amiga's own render, the baselines sheet's tile".into(),
            method_flags(hires, "0.7"),
            format!("{}, {}", bg(false), Scaler::Lanczos.name()),
            P2A_REST.into(),
        ],
        img: png(&root.join(format!("fixtures/png2amiga/{name}/baseline.png")))?,
        screen: None,
    });

    let lanczos = f.p2a_front(Scaler::Lanczos, 0.0);
    let mitchell = f.p2a_front(Scaler::Mitchell, 0.0);
    // 3: the port at equal settings.
    tiles.push(solved(
        3,
        "png2amiga port, equal flags",
        vec![
            method_flags(hires, "0.7"),
            format!("{}, {}", bg(false), Scaler::Lanczos.name()),
            P2A_REST.into(),
        ],
        f.p2a_encode(&lanczos, s, &c64::Hooks::default()),
    ));
    // 4: + Background: Search.
    tiles.push(solved(
        4,
        "3 + background search",
        vec![
            method_flags(hires, "0.7"),
            format!("{}, {}", bg(true), Scaler::Lanczos.name()),
            P2A_REST.into(),
        ],
        f.p2a_encode(&lanczos, s, &search),
    ));
    // 5: 4 with Mitchell.
    tiles.push(solved(
        5,
        "4 + png2c64 mitchell",
        vec![
            method_flags(hires, "0.7"),
            format!("{}, {}", bg(true), Scaler::Mitchell.name()),
            P2A_REST.into(),
        ],
        f.p2a_encode(&mitchell, s, &search),
    ));
    // 6: png2c64's port with the Lanczos.
    tiles.push(solved(
        6,
        "png2c64 port + png2amiga lanczos",
        vec![
            method_flags(hires, "0.7"),
            format!(
                "{}, {}",
                if hires {
                    "no background (hires)"
                } else {
                    "png2c64's background search"
                },
                Scaler::Lanczos.name()
            ),
            P2C_REST.into(),
        ],
        p2c::convert(
            &f.p2c_front(Scaler::Lanczos),
            &palette(),
            f.mode,
            &f.p2c_settings(),
        ),
    ));
    // 7: 4 with png2c64's maths.
    PNG2C64_MATHS.store(true, Ordering::Relaxed);
    let t7 = f.p2a_encode(&f.p2a_front(Scaler::Lanczos, 0.0), s, &search);
    PNG2C64_MATHS.store(false, Ordering::Relaxed);
    tiles.push(solved(
        7,
        "4 + png2c64 colour maths",
        vec![
            method_flags(hires, "0.7"),
            format!("{}, {}", bg(true), Scaler::Lanczos.name()),
            "png2c64 maths everywhere (correctly rounded cbrt, no fma)".into(),
            "png2amiga cells and dither pass".into(),
        ],
        t7,
    ));
    // 8: 4 with png2c64's cell colours.
    let p2c_cells = if hires {
        "png2c64 cells: 120-pair search, then global fs top-2"
    } else {
        "png2c64 cells: its background search and 455-triple search biased by checker x 0.49"
    };
    let hooks8 = f.p2c_sets(&lanczos);
    tiles.push(solved(
        8,
        "4 + png2c64 cell colours",
        vec![
            method_flags(hires, "0.7"),
            Scaler::Lanczos.name().into(),
            p2c_cells.into(),
            "png2amiga maths and dither pass in those colours".into(),
        ],
        f.p2a_encode(&lanczos, s, &hooks8),
    ));
    // 9 and 10: png2c64's ordered amplitude.
    let amp = if hires {
        "none has no amplitude: = tile 4"
    } else {
        "dither pass amplitude 0.49 = 0.7 x 0.7, as png2c64"
    };
    tiles.push(solved(
        9,
        "4 + png2c64 ordered amplitude",
        vec![
            method_flags(hires, if hires { "" } else { "0.49" }),
            format!("{}, {}", bg(true), Scaler::Lanczos.name()),
            amp.into(),
            P2A_REST.into(),
        ],
        f.p2a_encode(&lanczos, s2, &search),
    ));
    tiles.push(solved(
        10,
        "8 + png2c64 ordered amplitude",
        vec![
            method_flags(hires, if hires { "" } else { "0.49" }),
            Scaler::Lanczos.name().into(),
            p2c_cells.into(),
            if hires {
                "none has no amplitude: = tile 8".into()
            } else {
                format!("{amp}; png2amiga dither pass")
            },
        ],
        f.p2a_encode(&lanczos, s2, &hooks8),
    ));
    // 11: all three swaps.
    tiles.push(solved(
        11,
        "5 + 8 + 9: all three swaps",
        vec![
            method_flags(hires, if hires { "" } else { "0.49" }),
            Scaler::Mitchell.name().into(),
            p2c_cells.into(),
            "png2amiga maths, front end and dither pass".into(),
        ],
        f.p2a_encode(&mitchell, s2, &f.p2c_sets(&mitchell)),
    ));
    if hires {
        // 12 and 13: lineart's extra tiles.
        let greys = GREYS.iter().fold(0u16, |m, &g| m | (1 << g));
        let greys_only = c64::Hooks {
            rules: vec![
                quantize::CellRule {
                    candidates: greys,
                    required: 0,
                };
                1000
            ],
            ..c64::Hooks::default()
        };
        tiles.push(solved(
            12,
            "4 + greys only (allowed colours)",
            vec![
                method_flags(true, ""),
                Scaler::Lanczos.name().into(),
                "cell colours limited to 0, 1, 11, 12, 15".into(),
                "(a recipe's allowed colours; not a png2c64 swap)".into(),
            ],
            f.p2a_encode(&lanczos, s, &greys_only),
        ));
        tiles.push(solved(
            13,
            "12 + black point 0.5",
            vec![
                method_flags(true, ""),
                Scaler::Lanczos.name().into(),
                "png2amiga --black-point 0.5 (levels on oklab l)".into(),
                "cell colours limited to 0, 1, 11, 12, 15".into(),
            ],
            f.p2a_encode(&f.p2a_front(Scaler::Lanczos, 0.5), s, &greys_only),
        ));
    }
    // Not a tile: png2c64's port as it is, which must reproduce tile 1.
    let port = p2c::convert(
        &f.p2c_front(Scaler::Mitchell),
        &palette(),
        f.mode,
        &f.p2c_settings(),
    );
    let check = Check {
        port_vs_baseline: identical(&draw(&port), &tiles[0].img),
        port_bg: (!hires).then_some(port.bg),
    };
    Ok((tiles, check))
}

/// `text` broken into lines of at most [`WRAP`] characters, at spaces.
fn wrap(text: &str) -> Vec<String> {
    let mut lines = vec![String::new()];
    for word in text.split(' ') {
        let last = lines.last_mut().expect("never empty");
        if !last.is_empty() && last.len() + 1 + word.len() > WRAP {
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

/// The sheet: one row per fixture, a header with the notes.
fn draw_sheet(rows: &[(String, Vec<Tile>)], header: &[&str]) -> RgbImage {
    let columns = rows.iter().map(|(_, t)| t.len()).max().unwrap_or(1) as u32;
    let row_h = LABEL + TILE_H + (NOTES + 1) * LINE + GAP;
    let head_h = header.len() as u32 * LINE + GAP;
    let w = columns * TILE_W + (columns + 1) * GAP;
    let h = GAP + head_h + rows.len() as u32 * row_h;
    let mut img = RgbImage::from_pixel(w, h, Rgb(BACKGROUND));
    for (i, line) in header.iter().enumerate() {
        font::draw(&mut img, GAP, GAP + i as u32 * LINE, line, 1, TEXT);
    }
    for (r, (name, tiles)) in rows.iter().enumerate() {
        let y = GAP + head_h + r as u32 * row_h;
        for (c, t) in tiles.iter().enumerate() {
            let x = GAP + c as u32 * (TILE_W + GAP);
            font::draw(
                &mut img,
                x + 1,
                y + 2,
                &format!("{name} {}. {}", t.n, t.name),
                1,
                TEXT,
            );
            image::imageops::replace(&mut img, &t.img, i64::from(x), i64::from(y + LABEL));
            let mut notes: Vec<String> = t.flags.iter().flat_map(|f| wrap(f)).collect();
            notes.truncate(NOTES as usize);
            let same = identical(&t.img, &tiles[0].img);
            notes.push(if t.n == 1 {
                "the reference".to_owned()
            } else {
                match t.background() {
                    Some(b) => format!("identical to tile 1: {same:.3} %  (background {b})"),
                    None => format!("identical to tile 1: {same:.3} %"),
                }
            });
            let last = notes.len() - 1;
            for (k, n) in notes.iter().enumerate() {
                let colour = if k == last { TEXT } else { DIM };
                let ny = if k == last {
                    y + LABEL + TILE_H + 2 + NOTES * LINE
                } else {
                    y + LABEL + TILE_H + 2 + k as u32 * LINE
                };
                font::draw(&mut img, x + 1, ny, n, 1, colour);
            }
        }
    }
    img
}

/// Lineart's evidence: magnified crops of the prepared images and the tiles that matter,
/// and the Markdown with each tile's colours and the cells png2amiga gives a non-grey
/// colour.
fn lineart_evidence(tiles: &[Tile], out: &std::path::Path) -> Result<String, String> {
    let f = Fixture::load("lineart", true)?;
    let prepared = |scaler| -> RgbImage {
        let im = f.p2a_front(scaler, 0.0);
        RgbImage::from_fn(320, 200, |x, y| {
            let p = im.px[y as usize * 320 + x as usize];
            Rgb(p.map(|v| (p2a::colour::linear_to_srgb(v) * 255.0).round() as u8))
        })
    };
    let prep_l = prepared(Scaler::Lanczos);
    let prep_m = prepared(Scaler::Mitchell);
    let pick = |n: u8| tiles.iter().find(|t| t.n == n).map(|t| &t.img);
    let panels: Vec<(String, &RgbImage)> = vec![
        ("prepared: png2amiga lanczos, gamma 1.5".into(), &prep_l),
        ("prepared: png2c64 mitchell, gamma 1.5".into(), &prep_m),
        ("1. png2c64 baseline".into(), pick(1).expect("tile 1")),
        ("3. png2amiga (= tile 2)".into(), pick(3).expect("tile 3")),
        ("5. png2amiga + mitchell".into(), pick(5).expect("tile 5")),
        (
            "8. png2amiga + png2c64 cells".into(),
            pick(8).expect("tile 8"),
        ),
        ("12. + greys only".into(), pick(12).expect("tile 12")),
        (
            "13. + greys only + black point 0.5".into(),
            pick(13).expect("tile 13"),
        ),
    ];
    let (cx, cy, cw, ch, z) = (24u32, 16u32, 136u32, 96u32, 3u32);
    let cols = 4u32;
    let mut img = RgbImage::from_pixel(
        cols * (cw * z + GAP) + GAP,
        2 * (ch * z + LABEL + GAP) + GAP,
        Rgb(BACKGROUND),
    );
    for (i, (label, src)) in panels.iter().enumerate() {
        let crop = image::imageops::crop_imm(*src, cx, cy, cw, ch).to_image();
        let big =
            image::imageops::resize(&crop, cw * z, ch * z, image::imageops::FilterType::Nearest);
        let (x, y) = (
            GAP + (i as u32 % cols) * (cw * z + GAP),
            GAP + (i as u32 / cols) * (ch * z + LABEL + GAP),
        );
        font::draw(&mut img, x, y + 2, label, 1, TEXT);
        image::imageops::replace(&mut img, &big, i64::from(x), i64::from(y + LABEL));
    }
    save(&img, &out.join("sheet-cause-lineart.png"))?;

    let mut md = format!(
        "\n## Lineart\n\n[sheet-cause-lineart.png](sheet-cause-lineart.png): pixels \
         ({cx}, {cy})–({}, {}) of 320 × 200, magnified {z}×.\n\n\
         Prepared image (OKLab L of the 320 × 200 picture the encoder sees), pixels per \
         tenth of L:\n\n| Front end | 0.0 | 0.1 | 0.2 | 0.3 | 0.4 | 0.5 | 0.6 | 0.7 | 0.8 | \
         0.9 | 1.0 |\n|---|---|---|---|---|---|---|---|---|---|---|---|\n",
        cx + cw - 1,
        cy + ch - 1
    );
    for (n, scaler) in [
        ("png2amiga Lanczos", Scaler::Lanczos),
        ("png2c64 Mitchell", Scaler::Mitchell),
    ] {
        let mut hist = [0usize; 11];
        for &p in &f.p2a_front(scaler, 0.0).px {
            let l = p2a::colour::linear_to_oklab(p)[0];
            hist[((l * 10.0).round().max(0.0) as usize).min(10)] += 1;
        }
        let cells: Vec<String> = hist.iter().map(ToString::to_string).collect();
        md.push_str(&format!("| {n} | {} |\n", cells.join(" | ")));
    }
    let pal_lab = palette().map(p2a::colour::linear_to_oklab);
    md.push_str(
        "\nPixels per colour in each tile (Pepto's greys by lightness: 0 black L 0.00, \
         11 L 0.39, 12 L 0.53, 15 L 0.67, 1 white L 1.00; the others that appear: ",
    );
    let others: Vec<String> = [3u8, 7, 9, 13]
        .iter()
        .map(|&c| {
            let l = pal_lab[usize::from(c)];
            format!(
                "{c} L {:.2} chroma {:.3}",
                l[0],
                (l[1] * l[1] + l[2] * l[2]).sqrt()
            )
        })
        .collect();
    md.push_str(&others.join(", "));
    md.push_str("):\n\n| Tile | 0 | 11 | 12 | 15 | 1 | other |\n|---|---|---|---|---|---|---|\n");
    let count = |img: &RgbImage| {
        let mut hist = [0usize; 16];
        for p in img.pixels() {
            let i = p2a::colour::PEPTO_HEX
                .iter()
                .position(|h| {
                    let [_, r, g, b] = h.to_be_bytes();
                    p.0 == [r, g, b]
                })
                .expect("a Pepto colour");
            hist[i] += 1;
        }
        hist
    };
    for t in tiles {
        let h = count(&t.img);
        let other: Vec<String> = (0..16)
            .filter(|c| !GREYS.contains(&(*c as u8)) && h[*c] > 0)
            .map(|c| format!("{c}: {}", h[c]))
            .collect();
        md.push_str(&format!(
            "| {}. {} | {} | {} | {} | {} | {} | {} |\n",
            t.n,
            t.name,
            h[0],
            h[11],
            h[12],
            h[15],
            h[1],
            if other.is_empty() {
                "none".to_owned()
            } else {
                other.join(", ")
            }
        ));
    }
    // The cells png2amiga gives a colour that is not a grey.
    let t3 = tiles[2].screen.as_ref().expect("tile 3 is a solve");
    let t5 = tiles[4].screen.as_ref().expect("tile 5 is a solve");
    let lab_l = f
        .p2a_front(Scaler::Lanczos, 0.0)
        .px
        .iter()
        .map(|&p| p2a::colour::linear_to_oklab(p)[0])
        .collect::<Vec<f32>>();
    let lab_m = f
        .p2a_front(Scaler::Mitchell, 0.0)
        .px
        .iter()
        .map(|&p| p2a::colour::linear_to_oklab(p)[0])
        .collect::<Vec<f32>>();
    let stats = |l: &[f32], cx: usize, cy: usize| {
        let mut v: Vec<f32> = (0..64)
            .map(|k| l[(cy * 8 + k / 8) * 320 + cx * 8 + k % 8])
            .collect();
        v.sort_by(f32::total_cmp);
        let line: Vec<f32> = v.iter().copied().filter(|&x| x < 0.97).collect();
        let mean = if line.is_empty() {
            1.0
        } else {
            line.iter().sum::<f32>() / line.len() as f32
        };
        format!("{:.2} / {:.2} ({})", v[0], mean, line.len())
    };
    md.push_str(
        "\nEvery cell png2amiga (tile 3) gives a colour that is not a grey, with its pair, \
         png2c64's and tile 5's, and the prepared cell's darkest L / mean L of its line \
         pixels (L < 0.97; their count) under each scaler:\n\n\
         | Cell | tile 3 pair | tile 1 (png2c64) pair | tile 5 (Mitchell) pair | Lanczos L min / line mean (n) | Mitchell L min / line mean (n) |\n\
         |---|---|---|---|---|---|\n",
    );
    let p2c_port = p2c::convert(
        &f.p2c_front(Scaler::Mitchell),
        &palette(),
        f.mode,
        &f.p2c_settings(),
    );
    for (ci, c) in t3.cells.iter().enumerate() {
        if c.colors.iter().all(|v| GREYS.contains(v)) {
            continue;
        }
        let (x, y) = (ci % 40, ci / 40);
        md.push_str(&format!(
            "| ({x}, {y}) | {:?} | {:?} | {:?} | {} | {} |\n",
            c.colors,
            p2c_port.cells[ci].colors,
            t5.cells[ci].colors,
            stats(&lab_l, x, y),
            stats(&lab_m, x, y)
        ));
    }
    Ok(md)
}

pub(crate) fn run() -> Result<(), String> {
    let root = repo_root();
    let out = root.join("docs/reference/dither-previews/convert64-sheets");
    let mut rows = Vec::new();
    let mut checks = Vec::new();
    for (name, hires) in FIXTURES {
        let (tiles, check) = row(name, hires)?;
        rows.push((name.to_owned(), tiles));
        checks.push(check);
        eprintln!("cause-sheet: {name} done");
    }
    let header = [
        "cause sheet (#273): which of png2amiga's differences from png2c64 cost the quality. every tile at png2c64's baseline flags unless it says otherwise (multicolor checker 0.7, error clamp 0.1, gamma 1.5, pepto, metric mse). the last line under a tile is the share of its pixels identical to tile 1, the png2c64 baseline. no ssimulacra2 on this sheet.",
        "lineart is hi-res, like its png2c64 baseline (dither none, gamma 1.5): hi-res has no background, so its tiles 4-11 run without one and tile 4 equals tile 3; none has no ordered amplitude, so tiles 9 and 10 equal tiles 4 and 8. tiles 12 and 13 are lineart's own (not png2c64 swaps).",
        "tile 7 swaps png2amiga's colour maths for png2c64's everywhere in png2amiga's port (front end, cells, dither pass). tile 8 takes png2c64's whole cell-colour stage, its background included. tile 11 is png2amiga's port with all three swaps that matter: it comes back to png2c64.",
    ];
    save(&draw_sheet(&rows, &header), &out.join("sheet-cause.png"))?;

    let mut md = String::from(
        "<!-- Written by `cargo xtask cause-sheet`; do not edit by hand. -->\n\n\
         # Cause sheet (#273)\n\n\
         Share of each tile's pixels identical to the png2c64 baseline (tile 1), at \
         320 × 200 through Pepto ([sheet-cause.png](sheet-cause.png)). Every tile at \
         png2c64's baseline flags unless named: multicolor checker 0.7, error clamp 0.1, \
         gamma 1.5, Pepto, cell metric `mse`; lineart hi-res `none`, gamma 1.5. In \
         brackets, the multicolor background the tile used. No SSIMULACRA2.\n\n",
    );
    let names: Vec<String> = rows[5]
        .1
        .iter()
        .map(|t| format!("{}. {}", t.n, t.name))
        .collect();
    md.push_str("| Fixture | ");
    md.push_str(&names[1..].join(" | "));
    md.push_str(" |\n|---|");
    md.push_str(&"---|".repeat(names.len() - 1));
    md.push('\n');
    for (name, tiles) in &rows {
        md.push_str(&format!("| {name} |"));
        for k in 1..names.len() {
            match tiles.get(k) {
                Some(t) => {
                    let same = identical(&t.img, &tiles[0].img);
                    match t.background() {
                        Some(b) => md.push_str(&format!(" {same:.3} % ({b}) |")),
                        None => md.push_str(&format!(" {same:.3} % |")),
                    }
                }
                None => md.push_str(" n/a |"),
            }
        }
        md.push('\n');
    }
    md.push_str(
        "\nChecks: tile 3 against tile 2 (png2amiga's port against png2amiga's own \
         render), png2c64's port as it is (Mitchell, its front end and encoder) against \
         tile 1, and the background png2c64's search chose:\n\n\
         | Fixture | tile 3 vs tile 2 | png2c64 port vs tile 1 | png2c64's background |\n\
         |---|---|---|---|\n",
    );
    for ((name, tiles), c) in rows.iter().zip(&checks) {
        md.push_str(&format!(
            "| {name} | {:.3} % | {:.3} % | {} |\n",
            identical(&tiles[2].img, &tiles[1].img),
            c.port_vs_baseline,
            c.port_bg
                .map_or_else(|| "none (hi-res)".to_owned(), |b| b.to_string())
        ));
    }
    md.push_str(&lineart_evidence(&rows[5].1, &out)?);
    std::fs::write(out.join("sheet-cause.md"), &md).map_err(|e| e.to_string())?;
    println!("{md}");
    Ok(())
}
