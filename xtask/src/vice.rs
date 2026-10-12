//! `cargo xtask vice-check`: show every committed png2c64 baseline in VICE and compare
//! the capture with the baseline.
//!
//! For each fixture the task:
//!
//! 1. builds a self-displaying PRG from the committed `baseline.kla`/`.art` with
//!    `convert64_formats::prg::display_prg`, writes it to
//!    `target/xtask/vice/<fixture>.prg` and checks that the picture file it carries
//!    equals the baseline byte for byte, so VICE shows exactly what is committed;
//! 2. runs `x64sc` on it (PAL, Pepto palette with neutral colour settings, no CRT
//!    filter, warp) until a fixed cycle count, and saves the exit screenshot to
//!    `fixtures/<fixture>/vice.png`;
//! 3. crops the 320×200 display window out of VICE's 384×272 frame, maps every pixel
//!    to its nearest Pepto palette index and compares it with the baseline decoded by
//!    `convert64-formats`: the share of agreeing pixels, and whether every cell shows
//!    only colours the baseline's cell uses;
//! 4. passes at ≥ 99.5 % agreement with no cell outside its colours, and records the
//!    VICE version, command and result in the fixture README.
//!
//! VICE is not on the CI runners, so this is a local developer check, documented in
//! `docs/TAD.md` §6. It needs nothing but VICE and a clone of the repository. The
//! comparison itself runs in CI on the committed `vice.png` captures, with negative
//! controls (the tests at the end of this file).

use convert64_core::model::{CellPos, IndexedBitmap};
use convert64_core::{ColourIndex, ColourSet, Palette};
use convert64_formats::prg::{PAYLOAD_OFFSET, display_prg};
use convert64_formats::{BitmapFile, BitmapFormat};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::repo_root;

pub(crate) const FIXTURES: [(&str, BitmapFormat); 6] = [
    ("portrait", BitmapFormat::Koala),
    ("terrain", BitmapFormat::Koala),
    ("title", BitmapFormat::Koala),
    ("gradient", BitmapFormat::Koala),
    ("photo", BitmapFormat::Koala),
    ("lineart", BitmapFormat::ArtStudio),
];

/// Pixel agreement required to pass. Never lower this to make a fixture pass.
pub(crate) const MIN_AGREEMENT: f64 = 99.5;

/// VICE's PAL frame with normal borders, and where the 320×200 display window sits.
pub(crate) const FRAME: (u32, u32) = (384, 272);
const WINDOW: (u32, u32) = (32, 35);

/// `x64sc` options, apart from the PRG and screenshot path. Neutral colour settings make
/// VICE render the external Pepto palette's RGB values unchanged; the fixed autostart
/// delay and 30 M cycles (about 30 s of emulated time) make the capture deterministic.
const X64SC_ARGS: &[&str] = &[
    "-default",
    "-pal",
    "-VICIIextpal",
    "-VICIIpalette",
    "pepto-pal",
    "-VICIIfilter",
    "0",
    "-VICIIsaturation",
    "1000",
    "-VICIIcontrast",
    "1000",
    "-VICIIbrightness",
    "1000",
    "-VICIIgamma",
    "1000",
    "-VICIItint",
    "1000",
    "-sounddev",
    "dummy",
    "-warp",
    "-autostartprgmode",
    "1",
    // A random autostart delay made the screenshot sometimes catch BASIC still typing.
    "+autostart-delay-random",
    "-limitcycles",
    "30000000",
];

struct Outcome {
    agreement: f64,
    cells_total: usize,
    cells_outside: usize,
    diff_crop: Option<PathBuf>,
}

pub(crate) fn run(x64sc: &Path, only: &[String]) -> Result<(), String> {
    let root = repo_root();
    let mut failed = Vec::new();
    let mut ran = 0;
    for (name, format) in FIXTURES {
        if !only.is_empty() && !only.iter().any(|o| o == name) {
            continue;
        }
        ran += 1;
        let (version, outcome) = check(&root, x64sc, name, format)?;
        let pass = outcome.agreement >= MIN_AGREEMENT && outcome.cells_outside == 0;
        println!(
            "{name:<9} {:>7.3} % agree, {}/{} cells outside the baseline's colours  {}",
            outcome.agreement,
            outcome.cells_outside,
            outcome.cells_total,
            if pass { "PASS" } else { "FAIL" }
        );
        if pass {
            write_readme_line(&root, name, &version, &outcome)?;
        } else {
            if let Some(crop) = &outcome.diff_crop {
                println!("          differing region: {}", crop.display());
            }
            failed.push(name);
        }
    }
    if ran == 0 {
        return Err(format!("no fixture matches {only:?}"));
    }
    if failed.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "below {MIN_AGREEMENT} % or cells outside the baseline's colours: {}",
            failed.join(", ")
        ))
    }
}

fn check(
    root: &Path,
    x64sc: &Path,
    name: &str,
    format: BitmapFormat,
) -> Result<(String, Outcome), String> {
    let fixture = root.join("fixtures").join(name);
    let file_name = match format {
        BitmapFormat::Koala => "baseline.kla",
        BitmapFormat::ArtStudio => "baseline.art",
    };
    let baseline_bytes =
        fs::read(fixture.join(file_name)).map_err(|e| format!("{name}/{file_name}: {e}"))?;
    let baseline: BitmapFile = format
        .read(&baseline_bytes)
        .map_err(|e| format!("{name}/{file_name}: {e}"))?;

    // 1. Build the display PRG; it must carry exactly the committed picture file.
    let prg = display_prg(format, &baseline_bytes).map_err(|e| format!("{name}: {e}"))?;
    if prg.get(PAYLOAD_OFFSET..) != Some(&baseline_bytes[..]) {
        return Err(format!(
            "{name}: the display PRG does not carry the committed {file_name}"
        ));
    }
    let dir = root.join("target/xtask/vice");
    fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let prg_path = dir.join(format!("{name}.prg"));
    fs::write(&prg_path, &prg).map_err(|e| format!("{}: {e}", prg_path.display()))?;

    // 2. Run VICE.
    let shot = fixture.join("vice.png");
    let version = run_x64sc(x64sc, &prg_path, &shot).map_err(|e| format!("{name}: {e}"))?;

    // 3. Compare.
    let capture = image::open(&shot)
        .map_err(|e| format!("{}: {e}", shot.display()))?
        .to_rgb8();
    if capture.dimensions() != FRAME {
        return Err(format!(
            "{name}: VICE frame is {:?}, expected {FRAME:?} (border settings changed?)",
            capture.dimensions()
        ));
    }
    let indexed = baseline.image.to_indexed();
    let outcome = compare(name, &capture, &indexed)?;
    Ok((version, outcome))
}

/// Runs `x64sc` with [`X64SC_ARGS`] on `prg` and saves the exit screenshot to `shot`;
/// returns the VICE version as `VICE 3.10`.
pub(crate) fn run_x64sc(x64sc: &Path, prg: &Path, shot: &Path) -> Result<String, String> {
    // A stale capture must not pass for a new one.
    let _ = fs::remove_file(shot);
    let out = Command::new(x64sc)
        .args(X64SC_ARGS)
        .arg("-autostart")
        .arg(prg)
        .arg("-exitscreenshot")
        .arg(shot)
        .output()
        .map_err(|e| format!("{}: {e}", x64sc.display()))?;
    let log = String::from_utf8_lossy(&out.stdout);
    let version = log
        .lines()
        .find_map(|l| l.trim().strip_prefix("*** VICE Version "))
        .map_or_else(
            || "unknown".to_owned(),
            |v| v.trim_end_matches(" ***").to_owned(),
        );
    if !shot.exists() {
        return Err(format!("VICE wrote no screenshot:\n{log}"));
    }
    Ok(format!("VICE {version}"))
}

/// Nearest Pepto palette index by squared RGB distance.
fn nearest(palette: &Palette, rgb: [u8; 3]) -> ColourIndex {
    let mut best = (u32::MAX, ColourIndex::BLACK);
    for c in ColourIndex::ALL {
        let p = palette.rgb(c);
        let d = [p.r, p.g, p.b]
            .iter()
            .zip(rgb)
            .map(|(&a, b)| {
                let diff = i32::from(a) - i32::from(b);
                diff.unsigned_abs() * diff.unsigned_abs()
            })
            .sum::<u32>();
        if d < best.0 {
            best = (d, c);
        }
    }
    best.1
}

/// How a capture's display window matches a baseline.
pub(crate) struct Score {
    pub(crate) agreement: f64,
    pub(crate) cells_total: usize,
    pub(crate) cells_outside: usize,
    /// Bounding box `(min, max)` of the differing pixels, if any differ.
    differing: Option<((u32, u32), (u32, u32))>,
}

fn compare(
    name: &str,
    capture: &image::RgbImage,
    baseline: &IndexedBitmap,
) -> Result<Outcome, String> {
    let s = score(capture, baseline);
    let diff_crop = if let Some((min, max)) = s.differing {
        let dir = repo_root().join("target/xtask/vice");
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let path = dir.join(format!("{name}-diff.png"));
        let crop = image::imageops::crop_imm(
            capture,
            WINDOW.0 + min.0,
            WINDOW.1 + min.1,
            max.0 - min.0 + 1,
            max.1 - min.1 + 1,
        )
        .to_image();
        crop.save(&path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        Some(path)
    } else {
        None
    };
    Ok(Outcome {
        agreement: s.agreement,
        cells_total: s.cells_total,
        cells_outside: s.cells_outside,
        diff_crop,
    })
}

/// Scores the 320×200 display window of a 384×272 VICE frame against a baseline.
pub(crate) fn score(capture: &image::RgbImage, baseline: &IndexedBitmap) -> Score {
    let palette = Palette::pepto_pal();
    let x_scale = 320 / u32::from(baseline.width());
    let spec = baseline.spec();
    let (cell_w, cell_h) = (
        u32::from(spec.cell_width) * x_scale,
        u32::from(spec.cell_height),
    );
    let cells_x = 320 / cell_w;
    let mut seen = vec![ColourSet::EMPTY; usize::try_from(cells_x * 25).unwrap_or(0)];
    let mut agree = 0_u32;
    let (mut min, mut max) = ((u32::MAX, u32::MAX), (0, 0));
    for y in 0..200 {
        for x in 0..320 {
            let got = nearest(&palette, capture.get_pixel(WINDOW.0 + x, WINDOW.1 + y).0);
            let want = baseline.pixel(
                u16::try_from(x / x_scale).unwrap_or(0),
                u16::try_from(y).unwrap_or(0),
            );
            if got == want {
                agree += 1;
            } else {
                min = (min.0.min(x), min.1.min(y));
                max = (max.0.max(x), max.1.max(y));
            }
            seen[usize::try_from((y / cell_h) * cells_x + x / cell_w).unwrap_or(0)].insert(got);
        }
    }
    let cells_outside = seen
        .iter()
        .enumerate()
        .filter(|&(i, used)| {
            let cell = CellPos::new(
                u16::try_from(i % usize::try_from(cells_x).unwrap_or(1)).unwrap_or(0),
                u16::try_from(i / usize::try_from(cells_x).unwrap_or(1)).unwrap_or(0),
            );
            !used.difference(baseline.cell_colours(cell)).is_empty()
        })
        .count();
    Score {
        agreement: f64::from(agree) / 640.0,
        cells_total: seen.len(),
        cells_outside,
        differing: (agree < 64_000).then_some((min, max)),
    }
}

const START: &str = "<!-- vice-check:start -->";
const END: &str = "<!-- vice-check:end -->";

fn write_readme_line(root: &Path, name: &str, version: &str, o: &Outcome) -> Result<(), String> {
    let path = root.join("fixtures").join(name).join("README.md");
    let readme = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let line = format!(
        "{START}\n- VICE check (`cargo xtask vice-check`): {version}, `x64sc {} -autostart baseline.prg \
         -exitscreenshot vice.png` on the display PRG `convert64-formats` builds from this baseline. [vice.png](vice.png) \
         agrees with the baseline on {:.3} % of pixels; {} of {} cells show only the baseline's colours.\n{END}",
        X64SC_ARGS.join(" "),
        o.agreement,
        o.cells_total - o.cells_outside,
        o.cells_total
    );
    let updated = match (readme.find(START), readme.find(END)) {
        (Some(s), Some(e)) if e > s => {
            format!("{}{line}{}", &readme[..s], &readme[e + END.len()..])
        }
        _ => {
            // Right after the png2c64 baseline section, outside it: that section is
            // regenerated as a whole by `png2c64-baselines`.
            let marker = "<!-- png2c64-baseline:end -->";
            match readme.find(marker) {
                Some(i) => {
                    let at = i + marker.len();
                    format!("{}\n\n{line}{}", &readme[..at], &readme[at..])
                }
                None => format!("{}\n\n{line}\n", readme.trim_end()),
            }
        }
    };
    fs::write(&path, updated).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::{FIXTURES, FRAME, MIN_AGREEMENT, Score, nearest, score};
    use convert64_core::model::IndexedBitmap;
    use convert64_core::{ColourIndex, Palette};
    use convert64_formats::BitmapFormat;
    use convert64_formats::prg::{PAYLOAD_OFFSET, display_prg};
    use std::path::PathBuf;

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../fixtures")
            .join(name)
    }

    fn baseline_bytes(name: &str, format: BitmapFormat) -> Vec<u8> {
        let file = match format {
            BitmapFormat::Koala => "baseline.kla",
            BitmapFormat::ArtStudio => "baseline.art",
        };
        let path = fixture(name).join(file);
        std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    fn baseline(name: &str, format: BitmapFormat) -> IndexedBitmap {
        format
            .read(&baseline_bytes(name, format))
            .unwrap_or_else(|e| panic!("{name}: {e}"))
            .image
            .to_indexed()
    }

    /// The committed VICE capture of a fixture.
    fn capture(name: &str) -> image::RgbImage {
        let path = fixture(name).join("vice.png");
        let img = image::open(&path)
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
            .to_rgb8();
        assert_eq!(img.dimensions(), FRAME, "{}", path.display());
        img
    }

    /// The capture moved `dx` pixels right and `dy` down; vacated pixels repeat the
    /// edge.
    fn shifted(img: &image::RgbImage, dx: u32, dy: u32) -> image::RgbImage {
        image::RgbImage::from_fn(img.width(), img.height(), |x, y| {
            *img.get_pixel(x.saturating_sub(dx), y.saturating_sub(dy))
        })
    }

    fn passes(s: &Score) -> bool {
        s.agreement >= MIN_AGREEMENT && s.cells_outside == 0
    }

    #[test]
    fn every_capture_matches_its_own_baseline() {
        for (name, format) in FIXTURES {
            let s = score(&capture(name), &baseline(name, format));
            println!(
                "{name}: {:.3} % agree, {} cells outside",
                s.agreement, s.cells_outside
            );
            assert!(s.agreement >= MIN_AGREEMENT, "{name}: {:.3} %", s.agreement);
            assert_eq!(s.cells_outside, 0, "{name}");
        }
    }

    /// Negative control: a capture compared with another fixture's baseline fails.
    #[test]
    fn a_capture_fails_against_another_fixtures_baseline() {
        for (i, (name, _)) in FIXTURES.iter().enumerate() {
            let (other, format) = FIXTURES[(i + 1) % FIXTURES.len()];
            let s = score(&capture(name), &baseline(other, format));
            println!(
                "{name} vs {other}: {:.3} % agree, {} cells outside",
                s.agreement, s.cells_outside
            );
            assert!(
                s.agreement < MIN_AGREEMENT,
                "{name} vs {other}: {:.3} %",
                s.agreement
            );
            assert!(!passes(&s));
        }
    }

    /// Negative control: a capture one pixel off in either direction fails.
    #[test]
    fn a_capture_shifted_by_one_pixel_fails() {
        for (name, format) in FIXTURES {
            let (img, want) = (capture(name), baseline(name, format));
            for (dir, dx, dy) in [("right", 1, 0), ("down", 0, 1)] {
                let s = score(&shifted(&img, dx, dy), &want);
                println!(
                    "{name} shifted {dir}: {:.3} % agree, {} cells outside",
                    s.agreement, s.cells_outside
                );
                assert!(
                    s.agreement < MIN_AGREEMENT,
                    "{name} shifted {dir}: {:.3} %",
                    s.agreement
                );
                assert!(!passes(&s));
            }
        }
    }

    /// The PRG vice-check runs carries the committed baseline at the documented offset.
    #[test]
    fn display_prgs_carry_the_committed_baselines() {
        for (name, format) in FIXTURES {
            let bytes = baseline_bytes(name, format);
            let prg = display_prg(format, &bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(prg[PAYLOAD_OFFSET..], bytes[..], "{name}");
        }
    }

    #[test]
    fn nearest_maps_exact_and_shifted_colours() {
        let p = Palette::pepto_pal();
        for c in ColourIndex::ALL {
            let rgb = p.rgb(c);
            assert_eq!(nearest(&p, [rgb.r, rgb.g, rgb.b]), c);
        }
        // VICE's default colour settings render Pepto brown as 584900.
        assert_eq!(nearest(&p, [0x58, 0x49, 0x00]), ColourIndex::BROWN);
    }

    #[test]
    fn the_bar_is_99_5_percent() {
        assert!((MIN_AGREEMENT - 99.5).abs() < f64::EPSILON);
    }
}
