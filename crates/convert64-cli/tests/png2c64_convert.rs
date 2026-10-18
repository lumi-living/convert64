//! The parity bar end to end (#190): `convert64 convert` on every fixture source, with the
//! settings its png2c64 baseline was made with (`fixtures/<name>/README.md`), must be at
//! least 99 % pixel-identical to png2c64's `baseline.png`. A plain picture takes png2c64's
//! own front end (its bicubic, then gamma, then its preprocessing) and the reference
//! solver, so this holds for the product, not only the solver. png2c64's `--sharpen`
//! (#228) is held to 99.9 % on the portrait, both ways, against png2c64's renders
//! `baseline_sharpen_0.5.png` and `baseline_sharpen_-0.5.png`. CI runs it in the
//! `png2c64 parity` step and prints every percentage.

use std::path::{Path, PathBuf};
use std::process::Command;

const BAR: f64 = 99.0;

/// The bar of png2c64's `--sharpen` renders (#228).
const SHARPEN_BAR: f64 = 99.9;

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

/// One case: its name, the fixture, its arguments, png2c64's render and the bar.
type Case<'a> = (String, &'a str, Vec<&'a str>, &'a str, f64);

#[test]
fn png2c64_parity_of_convert() {
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("png2c64-convert");
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    let multicolor = vec![
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
    let hires = vec!["--mode", "hires", "--dither", "none", "--gamma", "1.5"];
    let mut cases: Vec<Case> = [
        ("portrait", &multicolor),
        ("terrain", &multicolor),
        ("title", &multicolor),
        ("gradient", &multicolor),
        ("photo", &multicolor),
        ("lineart", &hires),
    ]
    .into_iter()
    .map(|(name, args)| (name.to_owned(), name, args.clone(), "baseline.png", BAR))
    .collect();
    for (amount, render) in [
        ("0.5", "baseline_sharpen_0.5.png"),
        ("-0.5", "baseline_sharpen_-0.5.png"),
    ] {
        let mut args = multicolor.clone();
        args.extend(["--sharpen", amount]);
        cases.push((
            format!("portrait --sharpen {amount}"),
            "portrait",
            args,
            render,
            SHARPEN_BAR,
        ));
    }
    let results: Vec<(String, f64, f64)> = std::thread::scope(|s| {
        let handles: Vec<_> = cases
            .iter()
            .enumerate()
            .map(|(k, (label, name, args, render, bar))| {
                let out = &out;
                s.spawn(move || {
                    let dir = repo().join("fixtures").join(name);
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
                    let (ours, theirs) = (pixels(&png), pixels(&dir.join(render)));
                    let same = ours.iter().zip(&theirs).filter(|(a, b)| a == b).count();
                    (label.clone(), 100.0 * same as f64 / ours.len() as f64, *bar)
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let mut report = String::from("convert64 convert vs png2c64 (% of pixels identical):\n");
    for (name, pct, bar) in &results {
        report.push_str(&format!("  {name:<26} {pct:8.3} %  (bar {bar} %)\n"));
    }
    println!("{report}");
    assert!(results.iter().all(|(_, p, bar)| p >= bar), "{report}");
}
