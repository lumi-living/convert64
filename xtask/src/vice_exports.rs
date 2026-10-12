//! `cargo xtask vice-check`, part 2: Convert64's own exports in VICE (#162).
//!
//! Part 1 ([`crate::vice`]) shows the committed png2c64 baselines. This part solves each
//! fixture's `source.png` in both bitmap modes with the real solver (the golden
//! harness's png2c64-like recipe: gamma 1.5, Bayer 2×2 at 70 % for multicolor, no
//! dither for hi-res, Pepto PAL) and checks every bitmap export of the solve:
//!
//! - `write`: the Koala / Art Studio file `BitmapFormat::write` gives;
//! - `stable`, `frequency`, `packed`: the same written with each `BitpairPolicy`;
//! - `raw`: the raw set `raw::write` gives, read back from its manifest with
//!   `raw::read_files` and turned into a picture file whose body must equal the set's
//!   `.bin` files concatenated, so the PRG carries the raw set's bytes.
//!
//! The assembler and C emitters are not run here: their output is checked byte for
//! byte against `write` by `crates/convert64-formats/tests/asm_emit.rs`, so they show
//! what `write` shows.
//!
//! Each export becomes a display PRG (`convert64_formats::prg::display_prg`), runs in
//! `x64sc` with the same options as part 1, and its capture must agree with the solve
//! on ≥ 99.5 % of pixels with every cell showing only the solve's colours. Captures go
//! to `target/xtask/vice/exports/`, not into the repository: they follow the solver,
//! which changes. Passing results are recorded in each fixture's README with a
//! fingerprint of the solve they were made from.

use crate::repo_root;
use crate::vice::{FIXTURES, FRAME, MIN_AGREEMENT, run_x64sc, score};
use convert64_core::geom::Vec2;
use convert64_core::layer::{
    Adjustments, DitherKind, Gamma, Layer, Recipe, SourceImage, Transform,
};
use convert64_core::mode::Mode;
use convert64_core::model::BitmapImage;
use convert64_core::solve::{SolveOptions, Solver, Steering};
use convert64_core::{Compositor, Palette, SuperGrid};
use convert64_formats::bitpairs::BitpairPolicy;
use convert64_formats::prg::{PAYLOAD_OFFSET, display_prg};
use convert64_formats::{BitmapFormat, raw};
use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use std::sync::Arc;

/// The exports checked for every solve, in README order.
const EXPORTS: [&str; 5] = ["write", "stable", "frequency", "packed", "raw"];

/// VICE runs at once.
const PARALLEL: usize = 4;

/// The fixture's `source.png` solved in `mode` (see the module documentation).
pub(crate) fn solve(root: &Path, name: &str, mode: Mode) -> Result<BitmapImage, String> {
    let path = root.join("fixtures").join(name).join("source.png");
    let img = image::open(&path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .to_rgba8();
    let (w, h) = img.dimensions();
    let src = SourceImage::from_srgba8(w, h, img.as_raw())
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let mut layer = Layer::new(name, Arc::new(src));
    layer.set_transform(
        Transform::new(
            Vec2::ZERO,
            Vec2::new(320.0 / w as f32, 200.0 / h as f32),
            false,
            false,
        )
        .map_err(|e| e.to_string())?,
    );
    let recipe = match mode {
        Mode::MulticolorBitmap => Recipe::with_method(DitherKind::Bayer2, 0.7),
        _ => Recipe::with_method(DitherKind::None, 0.0),
    };
    layer.set_recipe(recipe.map_err(|e| e.to_string())?);
    layer.set_adjustments(Adjustments {
        gamma: Gamma::PNG2C64,
        ..Adjustments::default()
    });
    let mut c = Compositor::new(SuperGrid::with_default_factor(mode).map_err(|e| e.to_string())?);
    c.update(&[layer], &[]);
    let solution = Solver::new(&Palette::pepto_pal()).solve(
        c.frame(),
        &Steering::new(),
        None,
        &SolveOptions::default(),
    );
    Ok(solution.image)
}

/// The picture file for one export of `image`.
pub(crate) fn export(name: &str, image: &BitmapImage, export: &str) -> Result<Vec<u8>, String> {
    let format = BitmapFormat::for_mode(image.mode()).ok_or("not a bitmap")?;
    let policy = |id| BitpairPolicy::from_id(id).ok_or(format!("no policy {id}"));
    match export {
        "write" => format.write(image).map_err(|e| e.to_string()),
        "stable" | "frequency" | "packed" => format
            .write_with(image, policy(export)?)
            .map_err(|e| e.to_string()),
        "raw" => {
            let files =
                raw::write(image, name, &Palette::pepto_pal()).map_err(|e| e.to_string())?;
            let set = raw::read_files(&files).map_err(|e| e.to_string())?;
            let file = format.write(&set.image).map_err(|e| e.to_string())?;
            let bins: Vec<u8> = files
                .iter()
                .filter(|(p, _)| p.extension().is_some_and(|e| e == "bin"))
                .flat_map(|(_, b)| b.iter().copied())
                .collect();
            if file[2..] != bins[..] {
                return Err(format!(
                    "{name}: the raw set's .bin files are not the picture body"
                ));
            }
            Ok(file)
        }
        other => Err(format!("unknown export {other}")),
    }
}

/// FNV-1a, 64 bit: a fingerprint of the solve the README line was made from.
pub(crate) fn fingerprint(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

fn mode_id(mode: Mode) -> &'static str {
    match mode {
        Mode::MulticolorBitmap => "mc",
        _ => "hires",
    }
}

/// One VICE run's result.
struct Run {
    fixture: &'static str,
    mode: Mode,
    export: &'static str,
    agreement: f64,
    cells_outside: usize,
    cells_total: usize,
    version: String,
}

impl Run {
    fn pass(&self) -> bool {
        self.agreement >= MIN_AGREEMENT && self.cells_outside == 0
    }
}

pub(crate) fn run(x64sc: &Path, only: &[String]) -> Result<(), String> {
    let root = repo_root();
    let dir = root.join("target/xtask/vice/exports");
    fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    // Solve and build every PRG first, so the VICE runs only run VICE.
    let mut jobs = Vec::new();
    let mut solves = Vec::new();
    for (name, _) in FIXTURES {
        if !only.is_empty() && !only.iter().any(|o| o == name) {
            continue;
        }
        for mode in [Mode::MulticolorBitmap, Mode::HiresBitmap] {
            let image = solve(&root, name, mode)?;
            let format = BitmapFormat::for_mode(mode).ok_or("bitmap mode")?;
            let print = fingerprint(&format.write(&image).map_err(|e| e.to_string())?);
            for ex in EXPORTS {
                let file = export(name, &image, ex)?;
                let prg = display_prg(format, &file).map_err(|e| format!("{name} {ex}: {e}"))?;
                if prg.get(PAYLOAD_OFFSET..) != Some(&file[..]) {
                    return Err(format!("{name} {ex}: the PRG does not carry the export"));
                }
                let stem = format!("{name}-{}-{ex}", mode_id(mode));
                let prg_path = dir.join(format!("{stem}.prg"));
                fs::write(&prg_path, &prg).map_err(|e| format!("{}: {e}", prg_path.display()))?;
                jobs.push((name, mode, ex, prg_path, dir.join(format!("{stem}.png"))));
            }
            solves.push((name, mode, image, print));
        }
    }
    if jobs.is_empty() {
        return Err(format!("no fixture matches {only:?}"));
    }
    let mut runs = Vec::new();
    for chunk in jobs.chunks(PARALLEL) {
        let results: Vec<Result<Run, String>> = std::thread::scope(|s| {
            let handles: Vec<_> = chunk
                .iter()
                .map(|(name, mode, ex, prg, shot)| {
                    let image = &solves
                        .iter()
                        .find(|(n, m, _, _)| n == name && m == mode)
                        .expect("solved")
                        .2;
                    s.spawn(move || -> Result<Run, String> {
                        let version = run_x64sc(x64sc, prg, shot)?;
                        let capture = image::open(shot)
                            .map_err(|e| format!("{}: {e}", shot.display()))?
                            .to_rgb8();
                        if capture.dimensions() != FRAME {
                            return Err(format!(
                                "{}: VICE frame is {:?}, expected {FRAME:?}",
                                shot.display(),
                                capture.dimensions()
                            ));
                        }
                        let sc = score(&capture, &image.to_indexed());
                        Ok(Run {
                            fixture: name,
                            mode: *mode,
                            export: ex,
                            agreement: sc.agreement,
                            cells_outside: sc.cells_outside,
                            cells_total: sc.cells_total,
                            version,
                        })
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|h| {
                    h.join()
                        .unwrap_or_else(|_| Err("VICE thread panicked".into()))
                })
                .collect()
        });
        for r in results {
            let r = r?;
            println!(
                "{:<9} {:<5} {:<9} {:>7.3} % agree, {}/{} cells outside  {}",
                r.fixture,
                mode_id(r.mode),
                r.export,
                r.agreement,
                r.cells_outside,
                r.cells_total,
                if r.pass() { "PASS" } else { "FAIL" }
            );
            runs.push(r);
        }
    }
    let failed: Vec<String> = runs
        .iter()
        .filter(|r| !r.pass())
        .map(|r| format!("{} {} {}", r.fixture, mode_id(r.mode), r.export))
        .collect();
    // README lines only for fixtures whose every export passed.
    for (name, _) in FIXTURES {
        let mine: Vec<&Run> = runs.iter().filter(|r| r.fixture == name).collect();
        if mine.is_empty() || mine.iter().any(|r| !r.pass()) {
            continue;
        }
        let prints: Vec<(Mode, u64)> = solves
            .iter()
            .filter(|(n, ..)| *n == name)
            .map(|(_, m, _, p)| (*m, *p))
            .collect();
        write_readme(&root, name, &mine, &prints)?;
    }
    if failed.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "below {MIN_AGREEMENT} % or cells outside the solve's colours: {}",
            failed.join(", ")
        ))
    }
}

const START: &str = "<!-- vice-check-exports:start -->";
const END: &str = "<!-- vice-check-exports:end -->";

fn mode_text(mode: Mode) -> &'static str {
    match mode {
        Mode::MulticolorBitmap => "multicolor (Koala)",
        _ => "hi-res (Art Studio)",
    }
}

fn write_readme(
    root: &Path,
    name: &str,
    runs: &[&Run],
    prints: &[(Mode, u64)],
) -> Result<(), String> {
    let path = root.join("fixtures").join(name).join("README.md");
    let readme = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let version = &runs[0].version;
    let mut block = format!(
        "{START}\n- VICE check of Convert64's own exports (`cargo xtask vice-check`): {version}, the \
         same `x64sc` options, on display PRGs of `source.png` solved in each mode with the \
         harness's png2c64-like recipe. Exports: `write` (the Koala / Art Studio writer), \
         `stable` / `frequency` / `packed` (bitpair policies) and `raw` (the raw set read back \
         from its manifest). Captures are not committed; the fingerprint is FNV-1a of the \
         `write` file, to tell which solve was checked.\n"
    );
    for &(mode, print) in prints {
        let _ = write!(block, "  - {} solve `{print:016x}`:", mode_text(mode));
        let results: Vec<String> = runs
            .iter()
            .filter(|r| r.mode == mode)
            .map(|r| {
                format!(
                    " `{}` {:.3} % ({}/{} cells)",
                    r.export,
                    r.agreement,
                    r.cells_total - r.cells_outside,
                    r.cells_total
                )
            })
            .collect();
        let _ = writeln!(block, "{}", results.join(","));
    }
    block.push_str(END);
    let updated = match (readme.find(START), readme.find(END)) {
        (Some(s), Some(e)) if e > s => {
            format!("{}{block}{}", &readme[..s], &readme[e + END.len()..])
        }
        _ => {
            // Right after the baseline's VICE line.
            let marker = "<!-- vice-check:end -->";
            match readme.find(marker) {
                Some(i) => {
                    let at = i + marker.len();
                    format!("{}\n\n{block}{}", &readme[..at], &readme[at..])
                }
                None => format!("{}\n\n{block}\n", readme.trim_end()),
            }
        }
    };
    fs::write(&path, updated).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::{EXPORTS, export, fingerprint, solve};
    use crate::repo_root;
    use convert64_core::mode::Mode;
    use convert64_formats::BitmapFormat;

    /// Every export of a real solve is a legal picture file showing the solve.
    #[test]
    fn every_export_shows_the_solve() {
        let root = repo_root();
        for mode in [Mode::MulticolorBitmap, Mode::HiresBitmap] {
            let image = solve(&root, "portrait", mode).expect("solves");
            let format = BitmapFormat::for_mode(mode).expect("bitmap");
            for ex in EXPORTS {
                let file = export("portrait", &image, ex).expect("exports");
                let read = format.read(&file).expect("reads");
                assert_eq!(read.image.to_indexed(), image.to_indexed(), "{mode} {ex}");
            }
        }
        assert_eq!(fingerprint(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fingerprint(b"a"), 0xaf63_dc4c_8601_ec8c);
    }
}
