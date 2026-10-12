//! `convert64 convert` end to end (#111): a PNG becomes a legal Koala or Art Studio file.

use std::path::{Path, PathBuf};
use std::process::Command;

fn temp(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn source(dir: &Path) -> PathBuf {
    let img = image::RgbaImage::from_fn(160, 100, |x, y| {
        image::Rgba([(x * 255 / 160) as u8, (y * 255 / 100) as u8, 120, 255])
    });
    let p = dir.join("in.png");
    img.save(&p).unwrap();
    p
}

fn convert(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_convert64"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn converts_to_legal_koala_and_art_studio() {
    let dir = temp("convert");
    let png = source(&dir);
    for (mode, out, dither) in [
        ("mc", "out.kla", "bayer4"),
        ("hires", "out.art", "atkinson"),
    ] {
        let out = dir.join(out);
        let r = convert(&[
            "convert",
            png.to_str().unwrap(),
            "--mode",
            mode,
            "--dither",
            dither,
            "--strength",
            "0.8",
            "-o",
            out.to_str().unwrap(),
            "--json",
            "--report",
        ]);
        assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
        let v: serde_json::Value = serde_json::from_slice(&r.stdout).unwrap();
        assert!(v["summary"]["clashes"].is_u64());
        let check = convert(&["validate", out.to_str().unwrap()]);
        assert_eq!(
            String::from_utf8_lossy(&check.stdout).trim(),
            "legal",
            "{mode}"
        );
    }
}

#[test]
fn a_fixed_background_and_bad_options() {
    let dir = temp("convert-bg");
    let png = source(&dir);
    let out = dir.join("bg.kla");
    let r = convert(&[
        "convert",
        png.to_str().unwrap(),
        "--bg",
        "6",
        "-o",
        out.to_str().unwrap(),
    ]);
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    assert_eq!(std::fs::read(&out).unwrap()[10_002], 6, "background byte");
    let bad = convert(&[
        "convert",
        png.to_str().unwrap(),
        "--dither",
        "wobble",
        "-o",
        out.to_str().unwrap(),
    ]);
    assert_eq!(bad.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&bad.stderr).contains("unknown dither method"));
}

#[test]
fn solver_knobs_from_the_command_line() {
    let dir = temp("convert-knobs");
    let png = source(&dir);
    let out = dir.join("knobs.kla");
    let r = convert(&[
        "convert",
        png.to_str().unwrap(),
        "--dither",
        "floyd-steinberg",
        "--no-cell-clamp",
        "--max-luma-diff",
        "0",
        "--adaptive",
        "0.3",
        "--hysteresis",
        "0.97",
        "-o",
        out.to_str().unwrap(),
        "--json",
        "--report",
    ]);
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let v: serde_json::Value = serde_json::from_slice(&r.stdout).unwrap();
    assert_eq!(v["summary"]["luma_crossings"], 0);
    let check = convert(&["validate", out.to_str().unwrap()]);
    assert_eq!(String::from_utf8_lossy(&check.stdout).trim(), "legal");
    let bad = convert(&[
        "convert",
        png.to_str().unwrap(),
        "--max-luma-diff",
        "9",
        "-o",
        out.to_str().unwrap(),
    ]);
    assert_eq!(bad.status.code(), Some(2));
}

#[test]
fn mix_levels_gamma_and_range_matching_from_the_command_line() {
    let dir = temp("convert-levels");
    let png = source(&dir);
    let out = dir.join("levels.kla");
    let r = convert(&[
        "convert",
        png.to_str().unwrap(),
        "--dither",
        "bayer2",
        "--strength",
        "0.7",
        "--mix-levels",
        "8",
        "--smooth",
        "--gamma",
        "1.5",
        "--match-range",
        "-o",
        out.to_str().unwrap(),
    ]);
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let check = convert(&["validate", out.to_str().unwrap()]);
    assert_eq!(String::from_utf8_lossy(&check.stdout).trim(), "legal");
    for bad in [["--mix-levels", "17"], ["--gamma", "9"]] {
        let r = convert(&[
            "convert",
            png.to_str().unwrap(),
            bad[0],
            bad[1],
            "-o",
            out.to_str().unwrap(),
        ]);
        assert_eq!(r.status.code(), Some(2), "{bad:?}");
    }
}

/// Every ordered, noise and effect id of `docs/reference/dither-methods.md` §1–§2.
const THRESHOLD_IDS: &[&str] = &[
    "none",
    "bayer2",
    "bayer4",
    "bayer8",
    "bayer16",
    "bayer2x2-mc",
    "checker",
    "h2x4",
    "clustered4",
    "clustered8",
    "halftone8",
    "dot",
    "diamond",
    "star",
    "line-h2",
    "line-h4",
    "line-h8",
    "line-v2",
    "line-v4",
    "line-v8",
    "line-checker",
    "diag-down",
    "diag-up",
    "diagonal8",
    "crosshatch",
    "spiral5",
    "hex5",
    "hex8",
    "radial",
    "custom",
    "blue-noise",
    "ign",
    "r2",
    "white-noise",
    "value-noise",
];

/// Every diffusion id of `docs/reference/dither-methods.md` §3.
const DIFFUSION_IDS: &[&str] = &[
    "fs",
    "false-fs",
    "fs-wide",
    "line-fs",
    "jarvis",
    "stucki",
    "burkes",
    "sierra3",
    "sierra2",
    "sierra-lite",
    "atkinson",
    "stevenson-arce",
    "shiau-fan1",
    "shiau-fan2",
    "ostromoukhov",
    "zhou-fang",
    "riemersma",
];

/// Every palette-aware id of `docs/reference/dither-methods.md` §4.
const AWARE_IDS: &[&str] = &[
    "yliluoma1",
    "yliluoma2",
    "yliluoma3",
    "knoll",
    "luma",
    "pair",
];

#[test]
fn convert_accepts_every_method_id_of_the_reference() {
    let dir = temp("convert-ids");
    let png = source(&dir);
    let out = dir.join("id.kla");
    for id in THRESHOLD_IDS.iter().chain(DIFFUSION_IDS).chain(AWARE_IDS) {
        let r = convert(&[
            "convert",
            png.to_str().unwrap(),
            "--dither",
            id,
            "--bg",
            "0",
            "-o",
            out.to_str().unwrap(),
        ]);
        assert!(
            r.status.success(),
            "{id}: {}",
            String::from_utf8_lossy(&r.stderr)
        );
    }
}
