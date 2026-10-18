//! The png2amiga solver end to end (#261, ADR-014 §2): `convert64 convert --solver
//! png2amiga` on every fixture source, with the settings its png2amiga baseline was made
//! with (`fixtures/png2amiga/<name>/README.md`), must be at least 99 % pixel-identical to
//! png2amiga's `baseline.png`; with `--match-range --match-range-method hull`
//! (png2amiga's own matching) to `baseline_match_range.png`, with `--cell-refine` to
//! `baseline_refine.png`, and on the portrait with `--sharpen 0.5` and `--sharpen -0.5`
//! to png2amiga's renders of those. Under that solver a plain picture takes png2amiga's
//! own front end (its widened Lanczos-3, then gamma, sharpen, its preprocessing and range
//! matching), so this holds for the product, not only the port. The renders are
//! committed; nothing here builds png2amiga. CI runs it in the `png2amiga parity` step and
//! prints every percentage.

use std::path::{Path, PathBuf};
use std::process::Command;

const BAR: f64 = 99.0;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn pixels(path: &Path) -> Vec<[u8; 3]> {
    image::open(path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .to_rgb8()
        .pixels()
        .map(|p| p.0)
        .collect()
}

/// One case: its name, the fixture, its arguments and png2amiga's render.
type Case<'a> = (String, &'a str, Vec<&'a str>, String);

#[test]
fn png2amiga_parity_of_convert() {
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("png2amiga-convert");
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    let multicolor = vec![
        "--solver",
        "png2amiga",
        "--mode",
        "mc",
        "--dither",
        "checker",
        "--strength",
        "0.7",
        "--error-clamp",
        "0.1",
        "--gamma",
        "1.5",
    ];
    let hires = vec![
        "--solver",
        "png2amiga",
        "--mode",
        "hires",
        "--dither",
        "none",
        "--gamma",
        "1.5",
    ];
    let mut cases: Vec<Case> = Vec::new();
    for (name, args) in [
        ("portrait", &multicolor),
        ("terrain", &multicolor),
        ("title", &multicolor),
        ("gradient", &multicolor),
        ("photo", &multicolor),
        ("lineart", &hires),
    ] {
        cases.push((
            name.to_owned(),
            name,
            args.clone(),
            "baseline.png".to_owned(),
        ));
        for (flags, render) in [
            (
                &["--match-range", "--match-range-method", "hull"][..],
                "baseline_match_range.png",
            ),
            (&["--cell-refine"][..], "baseline_refine.png"),
        ] {
            let mut a = args.clone();
            a.extend(flags);
            cases.push((format!("{name} {}", flags[0]), name, a, render.to_owned()));
        }
    }
    for amount in ["0.5", "-0.5"] {
        let mut args = multicolor.clone();
        args.extend(["--sharpen", amount]);
        cases.push((
            format!("portrait --sharpen {amount}"),
            "portrait",
            args,
            format!("baseline_sharpen_{amount}.png"),
        ));
    }
    let results: Vec<(String, f64)> = std::thread::scope(|s| {
        let handles: Vec<_> = cases
            .iter()
            .enumerate()
            .map(|(k, (label, name, args, render))| {
                let out = &out;
                s.spawn(move || {
                    let dir = repo().join("fixtures").join(name);
                    let renders = repo().join("fixtures/png2amiga").join(name);
                    let png = out.join(format!("{k}-{name}.png"));
                    let r = Command::new(env!("CARGO_BIN_EXE_convert64"))
                        .arg("convert")
                        .arg(dir.join("source.png"))
                        .args(args)
                        .arg("-o")
                        .arg(&png)
                        .output()
                        .unwrap();
                    assert!(
                        r.status.success(),
                        "{label}: {}",
                        String::from_utf8_lossy(&r.stderr)
                    );
                    let (ours, theirs) = (pixels(&png), pixels(&renders.join(render)));
                    let same = ours.iter().zip(&theirs).filter(|(a, b)| a == b).count();
                    (label.clone(), 100.0 * same as f64 / ours.len() as f64)
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let mut report = String::from(
        "convert64 convert --solver png2amiga vs png2amiga (% of pixels identical):\n",
    );
    for (name, pct) in &results {
        report.push_str(&format!("  {name:<32} {pct:8.3} %\n"));
    }
    println!("{report}");
    assert!(results.iter().all(|(_, p)| *p >= BAR), "{report}");
}
