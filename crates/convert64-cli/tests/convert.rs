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
