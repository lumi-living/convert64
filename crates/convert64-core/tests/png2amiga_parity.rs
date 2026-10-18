//! The png2amiga port's parity test (#261; kept by ADR-014 as a test of the port, not of
//! quality): the png2amiga solver
//! (`solve::png2amiga_ref`) against png2amiga's own renders, pixel for pixel.
//!
//! - Every method png2amiga's C64 path accepts, on the dither-preview test card in
//!   multicolor and in hi-res (Convert64's settings at strength 1) and on the portrait
//!   (png2amiga's own C64 defaults: gamma 1, error clamp 0.35, the blur metric), against
//!   `docs/reference/dither-previews/png2amiga-card/`, `png2amiga-card-hires/` and
//!   `png2amiga-portrait/`.
//! - Every fixture with its png2amiga baseline's settings (`fixtures/png2amiga/<name>/`), with
//!   `--match-range` and with `--cell-refine` (the Refine step) too, and the portrait
//!   with `--sharpen 0.5` and `--sharpen -0.5`.
//!
//! Each must be at least 99 % identical. The report lists every case's percentage and,
//! for a case below 100 %, the first differing cell with both colour sets. The renders
//! come from png2amiga's reference build (`docs/reference/png2amiga-reference.md`) and
//! are committed: this test reads them and never builds png2amiga, so CI needs no C++
//! toolchain. Regenerate them with `cargo xtask png2amiga renders`.

use convert64_core::solve::png2amiga_ref::c64::Metric;
use convert64_core::solve::png2amiga_ref::colour::{PEPTO_HEX, palette_linear};
use convert64_core::solve::png2amiga_ref::dither::{Method, PxSettings};
use convert64_core::solve::png2amiga_ref::{
    Image, Mode, Screen, Settings, convert, prepare, preprocess, refine,
};
use std::path::PathBuf;

/// The bar, in percent of pixels.
const BAR: f64 = 99.0;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// How an input is prepared: gamma, sharpen, and whether the range is matched.
#[derive(Clone, Copy, PartialEq)]
struct Front {
    gamma: f32,
    sharpen: f32,
    match_range: bool,
}

/// An input prepared as png2amiga prepares it.
fn prepared(path: &str, mode: Mode, f: Front) -> Image {
    let img = image::open(repo().join(path))
        .unwrap_or_else(|e| panic!("{path}: {e}"))
        .to_rgb8();
    let rgb: Vec<[u8; 3]> = img.pixels().map(|p| p.0).collect();
    let size = (img.width() as usize, img.height() as usize);
    let pal = palette_linear(&PEPTO_HEX);
    prepare(
        &rgb,
        size,
        mode,
        &preprocess::Settings {
            gamma: f.gamma,
            sharpen: f.sharpen,
            ..preprocess::Settings::default()
        },
        f.match_range.then_some((&pal, 1.0)),
    )
}

/// A render (320 × 200, Pepto colours) as palette indices on `mode`'s grid.
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

/// One comparison.
struct Case {
    label: String,
    input: &'static str,
    mode: Mode,
    front: Front,
    settings: Settings,
    refine: bool,
    reference: String,
}

/// The first cell where ours and png2amiga's differ: its position, our colours by slot
/// and the colours png2amiga's render shows in it.
fn first_difference(ours: &Screen, theirs: &[u8]) -> Option<String> {
    let (w, _) = ours.mode.screen();
    let (cw, ch) = ours.mode.cell();
    let cols = ours.mode.cells().0;
    let mine = ours.indices();
    let i = mine.iter().zip(theirs).position(|(a, b)| a != b)?;
    let (cx, cy) = ((i % w) / cw, (i / w) / ch);
    let cell = &ours.cells[cy * cols + cx];
    let mut shown: Vec<u8> = (0..cw * ch)
        .map(|p| theirs[(cy * ch + p / cw) * w + cx * cw + p % cw])
        .collect();
    shown.sort_unstable();
    shown.dedup();
    Some(format!(
        "first difference at ({}, {}) in cell ({cx}, {cy}): ours {:?}, png2amiga shows {shown:?}",
        i % w,
        i / w,
        cell.colors
    ))
}

/// Convert64's baseline settings (`xtask/src/png2amiga.rs`).
fn baseline_settings(mode: Mode) -> Settings {
    match mode {
        Mode::Multicolor => Settings {
            method: Method::Checker,
            strength: 0.7,
            error_clamp: 0.1,
            serpentine: true,
            metric: Metric::Mse,
        },
        Mode::Hires => Settings {
            method: Method::None,
            strength: 1.0,
            error_clamp: 0.35,
            serpentine: true,
            metric: Metric::Mse,
        },
    }
}

fn cases() -> Vec<Case> {
    let mut out = Vec::new();
    const CARD: &str = "docs/reference/dither-previews/testcard.png";
    const PORTRAIT: &str = "fixtures/portrait/source.png";
    let ours = Front {
        gamma: 1.5,
        sharpen: 0.0,
        match_range: false,
    };
    let theirs = Front { gamma: 1.0, ..ours };
    for m in Method::ALL {
        for (set, input, mode, front, error_clamp, metric) in [
            ("card", CARD, Mode::Multicolor, ours, 0.1, Metric::Mse),
            ("card-hires", CARD, Mode::Hires, ours, 0.1, Metric::Mse),
            (
                "portrait",
                PORTRAIT,
                Mode::Multicolor,
                theirs,
                0.35,
                Metric::Blur,
            ),
        ] {
            out.push(Case {
                label: format!("{set} {}", m.name()),
                input,
                mode,
                front,
                settings: Settings {
                    method: m,
                    strength: 1.0,
                    error_clamp,
                    serpentine: true,
                    metric,
                },
                refine: false,
                reference: format!(
                    "docs/reference/dither-previews/png2amiga-{set}/{}.png",
                    m.name()
                ),
            });
        }
    }
    for (name, mode) in [
        ("portrait", Mode::Multicolor),
        ("terrain", Mode::Multicolor),
        ("title", Mode::Multicolor),
        ("gradient", Mode::Multicolor),
        ("photo", Mode::Multicolor),
        ("lineart", Mode::Hires),
    ] {
        let input: &'static str = Box::leak(format!("fixtures/{name}/source.png").into_boxed_str());
        let mut variants = vec![
            ("", ours, "baseline.png"),
            (
                " --match-range",
                Front {
                    match_range: true,
                    ..ours
                },
                "baseline_match_range.png",
            ),
            (" --cell-refine", ours, "baseline_refine.png"),
        ];
        if name == "portrait" {
            variants.push((
                " --sharpen 0.5",
                Front {
                    sharpen: 0.5,
                    ..ours
                },
                "baseline_sharpen_0.5.png",
            ));
            variants.push((
                " --sharpen -0.5",
                Front {
                    sharpen: -0.5,
                    ..ours
                },
                "baseline_sharpen_-0.5.png",
            ));
        }
        for (suffix, front, render) in variants {
            out.push(Case {
                label: format!("fixture {name}{suffix}"),
                input,
                mode,
                front,
                settings: baseline_settings(mode),
                refine: suffix == " --cell-refine",
                reference: format!("fixtures/png2amiga/{name}/{render}"),
            });
        }
    }
    out
}

/// Runs one case: the percentage of identical pixels and, below 100 %, the first
/// difference.
fn run(case: &Case, image: &Image) -> (f64, Option<String>) {
    let pal = palette_linear(&PEPTO_HEX);
    let mut screen = convert(image, &pal, case.mode, &case.settings);
    if case.refine {
        let s = &case.settings;
        let px = PxSettings {
            method: s.method,
            strength: s.strength,
            error_clamp: s.error_clamp,
            origin: (0, 0),
        };
        refine::refine(image, &mut screen, &pal, &px, s.serpentine);
    }
    let theirs = render_indices(&case.reference, case.mode);
    let ours = screen.indices();
    let same = ours.iter().zip(&theirs).filter(|(a, b)| a == b).count();
    let pct = 100.0 * same as f64 / ours.len() as f64;
    let note = (same < ours.len())
        .then(|| first_difference(&screen, &theirs))
        .flatten();
    (pct, note)
}

#[test]
fn png2amiga_parity_every_method_and_fixture() {
    let cases = cases();
    let threads = std::thread::available_parallelism().map_or(2, |n| n.get());
    let next = std::sync::atomic::AtomicUsize::new(0);
    let results = std::sync::Mutex::new(vec![None; cases.len()]);
    type Key = (&'static str, bool, u32, u32, bool);
    let inputs: std::sync::Mutex<std::collections::HashMap<Key, std::sync::Arc<Image>>> =
        std::sync::Mutex::new(std::collections::HashMap::new());
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(case) = cases.get(i) else { break };
                    let f = case.front;
                    let key = (
                        case.input,
                        case.mode == Mode::Hires,
                        f.gamma.to_bits(),
                        f.sharpen.to_bits(),
                        f.match_range,
                    );
                    let image = {
                        let cached = inputs.lock().unwrap().get(&key).cloned();
                        cached.unwrap_or_else(|| {
                            let im = std::sync::Arc::new(prepared(case.input, case.mode, f));
                            inputs.lock().unwrap().insert(key, im.clone());
                            im
                        })
                    };
                    let r = run(case, &image);
                    results.lock().unwrap()[i] = Some(r);
                }
            });
        }
    });
    let results = results.into_inner().unwrap();
    let mut report = String::from("png2amiga parity (% of pixels identical):\n");
    let mut failures = Vec::new();
    let mut below_100 = Vec::new();
    for (case, r) in cases.iter().zip(results) {
        let (pct, note) = r.expect("every case ran");
        report.push_str(&format!("  {:<36} {pct:8.3} %\n", case.label));
        if let Some(n) = &note {
            below_100.push(format!("{} at {pct:.3} %: {n}", case.label));
        }
        if pct < BAR {
            failures.push(format!(
                "{} at {pct:.3} %: {}",
                case.label,
                note.unwrap_or_default()
            ));
        }
    }
    report.push_str(&format!(
        "{} cases, {} at 100 %\n",
        cases.len(),
        cases.len() - below_100.len()
    ));
    for b in &below_100 {
        report.push_str(&format!("  below 100 %: {b}\n"));
    }
    println!("{report}");
    assert!(
        failures.is_empty(),
        "{report}\nunder {BAR} %:\n  {}",
        failures.join("\n  ")
    );
}
