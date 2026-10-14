//! The png2c64 parity bar of #181, no longer a gate since ADR-013 (#261; the gate is
//! `png2amiga_parity.rs`): the png2c64 port (`solve::png2c64_ref`, `--solver png2c64`)
//! against png2c64's committed output, pixel for pixel. Run it with `--ignored`.
//!
//! - Every one of png2c64's 30 method ids on the dither-preview test card (strength 1.0)
//!   and on the portrait (png2c64's defaults), against
//!   `docs/reference/dither-previews/png2c64-card/` and `png2c64-portrait/`.
//! - Every fixture with the settings its archived baseline was made with
//!   (`fixtures/archive/png2c64/<name>/`).
//!
//! Each must be at least 99 % identical. The report lists the percentage of every case
//! and, for a case under the bar, the first differing cell with both colour sets. The
//! renders come from png2c64's reference build (`docs/reference/png2c64-reference.md`);
//! regenerate them with `cargo xtask png2c64-previews` and `png2c64-baselines`.

use convert64_core::solve::png2c64_ref::colour::{PEPTO_HEX, palette_linear};
use convert64_core::solve::png2c64_ref::dither::{Method, Settings};
use convert64_core::solve::png2c64_ref::quantize::{Mode, Screen};
use convert64_core::solve::png2c64_ref::{Image, convert, prepare, preprocess};
use std::path::PathBuf;

/// The bar, in percent of pixels.
const BAR: f64 = 99.0;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// An input prepared as png2c64 prepares it (scale, gamma 1.5).
fn prepared(path: &str, mode: Mode) -> Image {
    let img = image::open(repo().join(path))
        .unwrap_or_else(|e| panic!("{path}: {e}"))
        .to_rgb8();
    let rgb: Vec<[u8; 3]> = img.pixels().map(|p| p.0).collect();
    let size = (img.width() as usize, img.height() as usize);
    prepare(&rgb, size, mode, &preprocess::Settings::default(), None)
}

/// A png2c64 render (320 × 200, Pepto colours) as palette indices on `mode`'s grid.
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

/// One comparison: an input, its settings and png2c64's render of it.
struct Case {
    label: String,
    input: &'static str,
    mode: Mode,
    settings: Settings,
    reference: String,
}

/// The first cell where ours and png2c64's differ: its position, our colours by slot
/// and the colours png2c64's render shows in it.
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
        "first difference in cell ({cx}, {cy}): ours {:?} (background {}), png2c64 shows {shown:?}",
        cell.colors, ours.bg
    ))
}

fn cases() -> Vec<Case> {
    let mut out = Vec::new();
    for m in Method::ALL {
        for (name, input, dir) in [
            (
                "card",
                "docs/reference/dither-previews/testcard.png",
                "png2c64-card",
            ),
            (
                "portrait",
                "fixtures/portrait/source.png",
                "png2c64-portrait",
            ),
        ] {
            out.push(Case {
                label: format!("{name} {}", m.name()),
                input,
                mode: Mode::Multicolor,
                settings: Settings {
                    method: m,
                    strength: 1.0,
                    ..Settings::default()
                },
                reference: format!("docs/reference/dither-previews/{dir}/{}.png", m.name()),
            });
        }
    }
    // The fixture baselines' recorded settings: multicolor `checker` at 0.7 with
    // error clamp 0.1, hi-res `none` for the line art; gamma 1.5 and Pepto throughout.
    for (name, mode, method, strength) in [
        ("portrait", Mode::Multicolor, Method::Checker, 0.7),
        ("terrain", Mode::Multicolor, Method::Checker, 0.7),
        ("title", Mode::Multicolor, Method::Checker, 0.7),
        ("gradient", Mode::Multicolor, Method::Checker, 0.7),
        ("photo", Mode::Multicolor, Method::Checker, 0.7),
        ("lineart", Mode::Hires, Method::None, 1.0),
    ] {
        out.push(Case {
            label: format!("fixture {name} {}", method.name()),
            input: Box::leak(format!("fixtures/{name}/source.png").into_boxed_str()),
            mode,
            settings: Settings {
                method,
                strength,
                error_clamp: 0.1,
                ..Settings::default()
            },
            reference: format!("fixtures/archive/png2c64/{name}/baseline.png"),
        });
    }
    out
}

/// Runs one case: the percentage of identical pixels and, under the bar, the first
/// difference.
fn run(case: &Case, image: &Image) -> (f64, Option<String>) {
    let screen = convert(
        image,
        &palette_linear(&PEPTO_HEX),
        case.mode,
        &case.settings,
    );
    let theirs = render_indices(&case.reference, case.mode);
    let ours = screen.indices();
    let same = ours.iter().zip(&theirs).filter(|(a, b)| a == b).count();
    let pct = 100.0 * same as f64 / ours.len() as f64;
    let note = if pct < BAR {
        first_difference(&screen, &theirs)
    } else {
        None
    };
    (pct, note)
}

#[test]
#[ignore = "not a gate since ADR-013 (#261); run with --ignored"]
fn png2c64_parity_every_method_and_fixture() {
    let cases = cases();
    let threads = std::thread::available_parallelism().map_or(2, |n| n.get());
    let next = std::sync::atomic::AtomicUsize::new(0);
    let results = std::sync::Mutex::new(vec![None; cases.len()]);
    let inputs = std::sync::Mutex::new(std::collections::HashMap::new());
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(case) = cases.get(i) else { break };
                    let key = (case.input, case.mode == Mode::Hires);
                    let image = {
                        let cached = inputs.lock().unwrap().get(&key).cloned();
                        cached.unwrap_or_else(|| {
                            let im = std::sync::Arc::new(prepared(case.input, case.mode));
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
    let mut report = String::from("png2c64 parity (% of pixels identical):\n");
    let mut failures = Vec::new();
    for (case, r) in cases.iter().zip(results) {
        let (pct, note) = r.expect("every case ran");
        report.push_str(&format!("  {:<28} {pct:8.3} %\n", case.label));
        if pct < BAR {
            failures.push(format!(
                "{} at {pct:.3} %: {}",
                case.label,
                note.unwrap_or_default()
            ));
        }
    }
    println!("{report}");
    assert!(
        failures.is_empty(),
        "{report}\nunder {BAR} %:\n  {}",
        failures.join("\n  ")
    );
}
