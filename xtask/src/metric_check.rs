//! `cargo xtask metric-check` (#273): how often each candidate quality metric ranks
//! Jaakko's eye-ranked pairs (`fixtures/eye-rankings.toml`) the way his eye did.
//!
//! **The pairs.** Each pair's two pictures are re-rendered exactly as the sheet he
//! reviewed showed them ([`reference_sheet::review_tile`]: the same calls as
//! `cargo xtask reference-sheet`, the zone plate as its large-sources sheet), and each is
//! checked pixel for pixel against its tile cropped from the committed sheet.
//!
//! **The reference**, one per fixture and the same for both pictures of a pair: the
//! source (`fixtures/<name>/source.png`, or the 4096 px zone plate) area-averaged in
//! linear light to the raster a metric works at. Area averaging weights every source
//! pixel by the share of it an output pixel covers, so the reference is the picture an
//! ideal, alias-free screen of that size would show; no scaler of either tool is
//! favoured, and nothing of either tool's front end (gamma 1.5, range matching) is in
//! it.
//!
//! **The candidates.** Each picture is the 320 × 200 Pepto render the sheet showed.
//!
//! - `s2`: [`ssimulacra2`] at the logical raster (160 × 200 multicolor, one sample per
//!   double-wide pixel; 320 × 200 hi-res) against the reference at that raster. Higher
//!   is better. This is the CLI's and `--json`'s score, unchanged.
//! - `s2-blur`: S2 at the display raster (320 × 200, a multicolor pixel two display
//!   pixels wide) after a Gaussian blur of both pictures in linear light, σ = 1 display
//!   pixel (half a multicolor pixel across, one line down), taps −3..3 normalised to sum
//!   1, separable, edges clamped. Higher is better.
//! - `s2-crt`: S2 at the display raster after a simple PAL/CRT model on both: the
//!   picture to gamma-encoded sRGB (the video signal), to YUV (BT.601: Y = 0.299 R +
//!   0.587 G + 0.114 B, U = 0.492 (B − Y), V = 0.877 (R − Y)); Y blurred across by a
//!   Gaussian σ = 0.5 px (luma bandwidth), U and V by σ = 2 px (PAL chroma bandwidth)
//!   and each line's U, V averaged with the line above (the PAL delay line); back to
//!   RGB, clamped, to linear light; then a vertical Gaussian σ = 0.5 line (beam spread).
//!   No scanline gaps, no mask, no bloom. Higher is better.
//! - `oklab-blur`: the golden harness's blurred OKLab error (#142,
//!   `crates/convert64-core/tests/golden.rs` `blur_mse`): both pictures at 320 × 200 in
//!   linear light blurred by the binomial `[1 4 6 4 1] / 16` twice per axis (edges
//!   clamped), then the mean squared OKLab distance. Lower is better. The harness
//!   scored against the compositor's frame; here it is the common reference above.
//!
//! A metric agrees with a pair when it scores the winner strictly better; a tie counts
//! as a disagreement. No metric becomes a gate here (TAD §6).

use crate::aliasing;
use crate::reference_sheet::{identical, png, review_tile, review_tile_place, source};
use crate::repo_root;
use convert64_core::mode::Mode;
use convert64_core::palette::{LinearRgb, srgb8_to_linear};
use convert64_core::quality::ssimulacra2;
use convert64_core::solve::png2c64_ref::colour::{linear_to_srgb, srgb_to_linear};
use image::{RgbImage, RgbaImage};

type Lin = [f32; 3];

/// One judged pair.
struct Pair {
    sheet: String,
    fixture: String,
    winner: String,
    loser: String,
    low: bool,
}

fn pairs() -> Result<Vec<Pair>, String> {
    let path = repo_root().join("fixtures/eye-rankings.toml");
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let doc: toml::Table = text
        .parse()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let list = doc
        .get("pair")
        .and_then(toml::Value::as_array)
        .ok_or("eye-rankings.toml: no [[pair]]")?;
    list.iter()
        .map(|p| {
            let s = |k: &str| {
                p.get(k)
                    .and_then(toml::Value::as_str)
                    .map(str::to_owned)
                    .ok_or_else(|| format!("eye-rankings.toml: a pair without `{k}`"))
            };
            Ok(Pair {
                sheet: s("sheet")?,
                fixture: s("fixture")?,
                winner: s("winner")?,
                loser: s("loser")?,
                low: p.get("confidence").and_then(toml::Value::as_str) == Some("low"),
            })
        })
        .collect()
}

/// The source of a fixture (or the zone plate) as linear pixels.
fn source_linear(fixture: &str) -> Result<(Vec<[f64; 3]>, usize, usize), String> {
    let img: RgbaImage = if fixture == "zoneplate-4096" {
        aliasing::fixtures()?.0
    } else {
        source(fixture)?
    };
    let px = img
        .pixels()
        .map(|p| [0, 1, 2].map(|c| f64::from(srgb8_to_linear(p.0[c]))))
        .collect();
    Ok((px, img.width() as usize, img.height() as usize))
}

/// Area-average weights from `n` source samples to `m`: for each output sample, the
/// covered source samples and the share of each.
fn area_weights(n: usize, m: usize) -> Vec<Vec<(usize, f64)>> {
    let scale = n as f64 / m as f64;
    (0..m)
        .map(|o| {
            let (a, b) = (o as f64 * scale, (o + 1) as f64 * scale);
            let mut w = Vec::new();
            let mut i = a.floor() as usize;
            while (i as f64) < b && i < n {
                let cover = (b.min(i as f64 + 1.0) - a.max(i as f64)).max(0.0);
                if cover > 0.0 {
                    w.push((i, cover / scale));
                }
                i += 1;
            }
            w
        })
        .collect()
}

/// The source area-averaged in linear light to `w × h`.
fn area_average(src: &[[f64; 3]], sw: usize, sh: usize, w: usize, h: usize) -> Vec<Lin> {
    let (wx, wy) = (area_weights(sw, w), area_weights(sh, h));
    let mut across = vec![[0.0f64; 3]; w * sh];
    for y in 0..sh {
        for (x, ws) in wx.iter().enumerate() {
            let mut acc = [0.0; 3];
            for &(i, k) in ws {
                for c in 0..3 {
                    acc[c] += src[y * sw + i][c] * k;
                }
            }
            across[y * w + x] = acc;
        }
    }
    let mut out = vec![[0.0f32; 3]; w * h];
    for (y, ws) in wy.iter().enumerate() {
        for x in 0..w {
            let mut acc = [0.0; 3];
            for &(j, k) in ws {
                for c in 0..3 {
                    acc[c] += across[j * w + x][c] * k;
                }
            }
            out[y * w + x] = acc.map(|v| v as f32);
        }
    }
    out
}

/// A 320 × 200 render as linear pixels, at the display raster or (`step` 2) one sample
/// per multicolor pixel.
fn render(img: &RgbImage, step: u32) -> Vec<Lin> {
    let w = 320 / step;
    (0..w * 200)
        .map(|i| {
            let p = img.get_pixel((i % w) * step, i / w).0;
            [0, 1, 2].map(|c| srgb8_to_linear(p[c]))
        })
        .collect()
}

/// A separable convolution of one plane with `taps` (centred), edges clamped.
fn convolve(v: &[f32], w: usize, h: usize, taps: &[f32], horizontal: bool) -> Vec<f32> {
    let r = (taps.len() / 2) as i64;
    (0..w * h)
        .map(|i| {
            let (x, y) = ((i % w) as i64, (i / w) as i64);
            taps.iter()
                .enumerate()
                .map(|(k, t)| {
                    let d = k as i64 - r;
                    let (nx, ny) = if horizontal {
                        ((x + d).clamp(0, w as i64 - 1), y)
                    } else {
                        (x, (y + d).clamp(0, h as i64 - 1))
                    };
                    t * v[ny as usize * w + nx as usize]
                })
                .sum()
        })
        .collect()
}

/// A normalised Gaussian of `sigma`, taps −⌈3σ⌉..⌈3σ⌉.
fn gaussian(sigma: f32) -> Vec<f32> {
    let r = (3.0 * sigma).ceil() as i32;
    let t: Vec<f32> = (-r..=r)
        .map(|d| (-(d * d) as f32 / (2.0 * sigma * sigma)).exp())
        .collect();
    let s: f32 = t.iter().sum();
    t.iter().map(|v| v / s).collect()
}

/// The planes of an image.
fn planes(img: &[Lin]) -> [Vec<f32>; 3] {
    [0, 1, 2].map(|c| img.iter().map(|p| p[c]).collect())
}

fn merge(p: &[Vec<f32>; 3]) -> Vec<Lin> {
    (0..p[0].len())
        .map(|i| [p[0][i], p[1][i], p[2][i]])
        .collect()
}

/// `s2-blur`'s blur: Gaussian σ = 1 display pixel, both axes, in linear light.
fn blur_gauss(img: &[Lin]) -> Vec<Lin> {
    let g = gaussian(1.0);
    let p = planes(img).map(|v| {
        let a = convolve(&v, 320, 200, &g, true);
        convolve(&a, 320, 200, &g, false)
    });
    merge(&p)
}

/// `s2-crt`'s PAL/CRT model (see the module docs).
fn crt(img: &[Lin]) -> Vec<Lin> {
    let enc = |v: f32| linear_to_srgb(v.clamp(0.0, 1.0));
    let [r, g, b] = planes(img).map(|v| v.into_iter().map(enc).collect::<Vec<f32>>());
    let n = r.len();
    let y: Vec<f32> = (0..n)
        .map(|i| 0.299 * r[i] + 0.587 * g[i] + 0.114 * b[i])
        .collect();
    let u: Vec<f32> = (0..n).map(|i| 0.492 * (b[i] - y[i])).collect();
    let v: Vec<f32> = (0..n).map(|i| 0.877 * (r[i] - y[i])).collect();
    let y = convolve(&y, 320, 200, &gaussian(0.5), true);
    let delay = |c: &[f32]| -> Vec<f32> {
        let c = convolve(c, 320, 200, &gaussian(2.0), true);
        (0..n)
            .map(|i| {
                let above = if i >= 320 { c[i - 320] } else { c[i] };
                0.5 * (c[i] + above)
            })
            .collect()
    };
    let (u, v) = (delay(&u), delay(&v));
    let dec = |s: f32| srgb_to_linear(s.clamp(0.0, 1.0));
    let rgb: [Vec<f32>; 3] = [
        (0..n).map(|i| dec(y[i] + v[i] / 0.877)).collect(),
        (0..n)
            .map(|i| {
                let (rr, bb) = (y[i] + v[i] / 0.877, y[i] + u[i] / 0.492);
                dec((y[i] - 0.299 * rr - 0.114 * bb) / 0.587)
            })
            .collect(),
        (0..n).map(|i| dec(y[i] + u[i] / 0.492)).collect(),
    ];
    let beam = gaussian(0.5);
    merge(&rgb.map(|p| convolve(&p, 320, 200, &beam, false)))
}

/// The golden harness's blurred OKLab error (`blur_mse`, #142).
fn oklab_blur(a: &[Lin], b: &[Lin]) -> f64 {
    let t = [1.0f32, 4.0, 6.0, 4.0, 1.0].map(|v| v / 16.0);
    let blur = |img: &[Lin]| {
        merge(&planes(img).map(|mut v| {
            for horizontal in [true, false, true, false] {
                v = convolve(&v, 320, 200, &t, horizontal);
            }
            v
        }))
    };
    let lab = |p: Lin| {
        let c = LinearRgb {
            r: p[0],
            g: p[1],
            b: p[2],
        }
        .to_oklab();
        [c.l, c.a, c.b].map(f64::from)
    };
    let (ba, bb) = (blur(a), blur(b));
    ba.iter()
        .zip(&bb)
        .map(|(&p, &q)| {
            let (x, y) = (lab(p), lab(q));
            (0..3).map(|k| (x[k] - y[k]).powi(2)).sum::<f64>()
        })
        .sum::<f64>()
        / ba.len() as f64
}

/// The candidates: name, whether higher is better.
const METRICS: [(&str, bool); 4] = [
    ("s2", true),
    ("s2-blur", true),
    ("s2-crt", true),
    ("oklab-blur", false),
];

/// The references of one fixture: at the logical raster and at the display raster.
struct Reference {
    logical: Vec<Lin>,
    display: Vec<Lin>,
    mode: Mode,
}

/// Every metric's score of one picture.
fn scores(img: &RgbImage, r: &Reference) -> [f64; 4] {
    let step = if r.mode == Mode::HiresBitmap { 1 } else { 2 };
    let display = render(img, 1);
    [
        ssimulacra2(&r.logical, &render(img, step), 320 / step as usize, 200),
        ssimulacra2(&blur_gauss(&r.display), &blur_gauss(&display), 320, 200),
        ssimulacra2(&crt(&r.display), &crt(&display), 320, 200),
        oklab_blur(&r.display, &display),
    ]
}

pub(crate) fn run() -> Result<(), String> {
    let root = repo_root();
    let sheets = root.join("docs/reference/dither-previews/convert64-sheets");
    let pairs = pairs()?;
    let mut refs: Vec<(String, Reference)> = Vec::new();
    let mut tiles: Vec<((String, String, String), [f64; 4])> = Vec::new();
    let mut checked = Vec::new();
    for p in &pairs {
        for v in [&p.winner, &p.loser] {
            let key = (p.sheet.clone(), p.fixture.clone(), v.clone());
            if tiles.iter().any(|(k, _)| *k == key) {
                continue;
            }
            let (img, mode) = review_tile(&p.sheet, &p.fixture, v)?;
            // The tile as the reviewed sheet showed it.
            let (file, x, y) = review_tile_place(&p.sheet, &p.fixture, v)
                .ok_or_else(|| format!("no place for {}/{}/{v}", p.sheet, p.fixture))?;
            let sheet = png(&sheets.join(file))?;
            let shown = image::imageops::crop_imm(&sheet, x, y, 320, 200).to_image();
            checked.push((
                format!("{}/{}/{v}", p.sheet, p.fixture),
                identical(&img, &shown),
            ));
            if !refs.iter().any(|(f, _)| *f == p.fixture) {
                let (src, sw, sh) = source_linear(&p.fixture)?;
                let lw = if mode == Mode::HiresBitmap { 320 } else { 160 };
                refs.push((
                    p.fixture.clone(),
                    Reference {
                        logical: area_average(&src, sw, sh, lw, 200),
                        display: area_average(&src, sw, sh, 320, 200),
                        mode,
                    },
                ));
            }
            let r = &refs.iter().find(|(f, _)| *f == p.fixture).expect("added").1;
            tiles.push((key, scores(&img, r)));
            eprintln!("metric-check: {}/{}/{v}", p.sheet, p.fixture);
        }
    }
    let score = |sheet: &str, fixture: &str, v: &str| {
        tiles
            .iter()
            .find(|(k, _)| k.0 == sheet && k.1 == fixture && k.2 == v)
            .expect("scored")
            .1
    };

    let mut md = String::from(
        "<!-- Written by `cargo xtask metric-check`; do not edit by hand. -->\n\n\
         # Metric check (#273)\n\n\
         Each candidate metric against Jaakko's eye-ranked pairs (`fixtures/eye-rankings.toml`), \
         both pictures of a pair re-rendered as the reviewed sheet showed them and scored \
         against the same reference (the source area-averaged in linear light). A metric \
         agrees when it scores the eye's winner strictly better. No metric is a gate.\n\n",
    );
    let all = pairs.len();
    let high = pairs.iter().filter(|p| !p.low).count();
    md.push_str(&format!(
        "| Metric | agrees, all {all} pairs | agrees, {high} pairs without `low` | pairs it gets wrong |\n|---|---|---|---|\n"
    ));
    for (m, (name, higher)) in METRICS.iter().enumerate() {
        let mut wrong = Vec::new();
        let (mut ok_all, mut ok_high) = (0, 0);
        for p in &pairs {
            let (w, l) = (
                score(&p.sheet, &p.fixture, &p.winner)[m],
                score(&p.sheet, &p.fixture, &p.loser)[m],
            );
            let agrees = if *higher { w > l } else { w < l };
            if agrees {
                ok_all += 1;
                if !p.low {
                    ok_high += 1;
                }
            } else {
                wrong.push(format!(
                    "{} {} {} > {}{}",
                    p.sheet,
                    p.fixture,
                    p.winner,
                    p.loser,
                    if p.low { " (low)" } else { "" }
                ));
            }
        }
        md.push_str(&format!(
            "| `{name}` | {ok_all} / {all} ({:.0} %) | {ok_high} / {high} ({:.0} %) | {} |\n",
            100.0 * ok_all as f64 / all as f64,
            100.0 * ok_high as f64 / high as f64,
            if wrong.is_empty() {
                "none".to_owned()
            } else {
                wrong.join("; ")
            }
        ));
    }
    md.push_str(
        "\nEvery pair's scores (winner / loser; S2 higher is better, `oklab-blur` lower):\n\n\
         | Sheet | Fixture | Eye: winner > loser | `s2` | `s2-blur` | `s2-crt` | `oklab-blur` |\n\
         |---|---|---|---|---|---|---|\n",
    );
    for p in &pairs {
        let (w, l) = (
            score(&p.sheet, &p.fixture, &p.winner),
            score(&p.sheet, &p.fixture, &p.loser),
        );
        md.push_str(&format!(
            "| {} | {} | {} > {}{} | {:.2} / {:.2} | {:.2} / {:.2} | {:.2} / {:.2} | {:.5} / {:.5} |\n",
            p.sheet,
            p.fixture,
            p.winner,
            p.loser,
            if p.low { " (low)" } else { "" },
            w[0],
            l[0],
            w[1],
            l[1],
            w[2],
            l[2],
            w[3],
            l[3]
        ));
    }
    let exact = checked.iter().filter(|(_, s)| *s == 100.0).count();
    md.push_str(&format!(
        "\nRe-renders identical to the reviewed sheets' tiles: {exact} of {}.\n",
        checked.len()
    ));
    for (k, s) in &checked {
        if *s != 100.0 {
            md.push_str(&format!("- {k}: {s:.3} %\n"));
        }
    }
    std::fs::write(sheets.join("metric-check.md"), &md).map_err(|e| e.to_string())?;
    println!("{md}");
    Ok(())
}
