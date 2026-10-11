//! `cargo xtask vice-check`: show every committed png2c64 baseline in VICE and compare
//! the capture with the baseline.
//!
//! For each fixture the task:
//!
//! 1. takes png2c64's display PRG from `target/xtask/png2c64/<fixture>/baseline.prg`
//!    (written by `cargo xtask png2c64-baselines`) and checks that the picture data it
//!    carries equals the committed `baseline.kla`/`.art` byte for byte, so VICE shows
//!    exactly what is committed;
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
//! `docs/TAD.md` §6.

use convert64_core::model::{CellPos, IndexedBitmap};
use convert64_core::{ColourIndex, ColourSet, Palette};
use convert64_formats::{BitmapFile, BitmapFormat};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::repo_root;

const FIXTURES: [(&str, BitmapFormat); 6] = [
    ("portrait", BitmapFormat::Koala),
    ("terrain", BitmapFormat::Koala),
    ("title", BitmapFormat::Koala),
    ("gradient", BitmapFormat::Koala),
    ("photo", BitmapFormat::Koala),
    ("lineart", BitmapFormat::ArtStudio),
];

/// Pixel agreement required to pass. Never lower this to make a fixture pass.
const MIN_AGREEMENT: f64 = 99.5;

/// Offset of the picture data in png2c64's display PRG (see `baselines.rs`).
const PRG_DATA_OFFSET: usize = 2 + 0x2000 - 0x0801;

/// VICE's PAL frame with normal borders, and where the 320×200 display window sits.
const FRAME: (u32, u32) = (384, 272);
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

    // 1. The PRG must carry exactly the committed picture.
    let prg_path = root
        .join("target/xtask/png2c64")
        .join(name)
        .join("baseline.prg");
    let prg = fs::read(&prg_path).map_err(|e| {
        format!(
            "{}: {e}; run `cargo xtask png2c64-baselines --png2c64 <bin>` first",
            prg_path.display()
        )
    })?;
    let picture = &baseline_bytes[2..format.sizes()[0]];
    if prg.get(PRG_DATA_OFFSET..) != Some(picture) {
        return Err(format!(
            "{name}: the display PRG does not carry the committed {file_name}; regenerate it"
        ));
    }

    // 2. Run VICE.
    let shot = fixture.join("vice.png");
    let out = Command::new(x64sc)
        .args(X64SC_ARGS)
        .arg("-autostart")
        .arg(&prg_path)
        .arg("-exitscreenshot")
        .arg(&shot)
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
        return Err(format!("{name}: VICE wrote no screenshot:\n{log}"));
    }

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
    Ok((format!("VICE {version}"), outcome))
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

fn compare(
    name: &str,
    capture: &image::RgbImage,
    baseline: &IndexedBitmap,
) -> Result<Outcome, String> {
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
    let diff_crop = if agree < 64_000 {
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
        agreement: f64::from(agree) / 640.0,
        cells_total: seen.len(),
        cells_outside,
        diff_crop,
    })
}

const START: &str = "<!-- vice-check:start -->";
const END: &str = "<!-- vice-check:end -->";

fn write_readme_line(root: &Path, name: &str, version: &str, o: &Outcome) -> Result<(), String> {
    let path = root.join("fixtures").join(name).join("README.md");
    let readme = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let line = format!(
        "{START}\n- VICE check (`cargo xtask vice-check`): {version}, `x64sc {} -autostart baseline.prg \
         -exitscreenshot vice.png` on png2c64's display PRG for this baseline. [vice.png](vice.png) \
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
    use super::{MIN_AGREEMENT, nearest};
    use convert64_core::{ColourIndex, Palette};

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
