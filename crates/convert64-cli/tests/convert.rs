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
        // ADR-013 §4: the result's SSIMULACRA2 against the prepared frame.
        let s2 = v["ssimulacra2"].as_f64().expect("ssimulacra2 score");
        assert!(s2.is_finite() && s2 <= 100.0, "{s2}");
        assert!(v["timing_ms"]["ssimulacra2"].is_f64());
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

/// #228: `--sharpen`, `--sharpen-radius` and `--sharpen-threshold` set every layer's
/// sharpen; a saved project carries it, and bad values are input errors.
#[test]
fn sharpen_from_the_command_line() {
    let dir = temp("convert-sharpen");
    let png = source(&dir);
    let out = dir.join("sharp.kla");
    let project = dir.join("sharp.convert64");
    let r = convert(&[
        "convert",
        png.to_str().unwrap(),
        "--sharpen",
        "-0.5",
        "--sharpen-radius",
        "0.8",
        "--sharpen-threshold",
        "0.02",
        "-o",
        out.to_str().unwrap(),
        "--save-project",
        project.to_str().unwrap(),
    ]);
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let check = convert(&["validate", out.to_str().unwrap()]);
    assert_eq!(String::from_utf8_lossy(&check.stdout).trim(), "legal");
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&project).unwrap()).unwrap();
    let s = &v["layers"][0]["adjustments"]["sharpen"];
    assert_eq!(s["amount"], -0.5, "{v}");
    assert!((s["radius"].as_f64().unwrap() - 0.8).abs() < 1e-6, "{v}");
    assert!(
        (s["threshold"].as_f64().unwrap() - 0.02).abs() < 1e-6,
        "{v}"
    );
    for bad in [
        ["--sharpen", "4"],
        ["--sharpen-radius", "wide"],
        ["--sharpen-radius", "0.1"],
        ["--sharpen-threshold", "0.5"],
    ] {
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

#[test]
fn cell_metrics_from_the_command_line() {
    let dir = temp("convert-cell-metric");
    let png = source(&dir);
    let out = dir.join("cm.kla");
    for args in [
        ["--cell-metric", "blur"],
        ["--cell-metric", "ssim"],
        ["--metric", "blur"],
        ["--metric", "cielab"],
        ["--ordered-semantics", "legacy"],
        ["--no-chroma-guard", "--strength=1"],
    ] {
        let r = convert(&[
            "convert",
            png.to_str().unwrap(),
            "--dither",
            "bayer4",
            args[0],
            args[1],
            "-o",
            out.to_str().unwrap(),
        ]);
        assert!(
            r.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&r.stderr)
        );
    }
    let bad = convert(&[
        "convert",
        png.to_str().unwrap(),
        "--cell-metric",
        "psnr",
        "-o",
        out.to_str().unwrap(),
    ]);
    assert_eq!(bad.status.code(), Some(2));
}

#[test]
fn solver_and_divergences_from_the_command_line() {
    let dir = temp("diverge");
    let png = source(&dir);
    let out = dir.join("out.kla");
    let json = |args: &[&str]| -> serde_json::Value {
        let mut all = vec![
            "--json",
            "convert",
            png.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
        ];
        all.extend_from_slice(args);
        let r = convert(&all);
        assert!(
            r.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&r.stderr)
        );
        serde_json::from_slice(&r.stdout).expect("JSON report")
    };
    let default = json(&[]);
    assert_eq!(default["solver"], "reference");
    assert_eq!(default["refine"], false);
    assert_eq!(json(&["--solver", "native"])["solver"], "native");
    // #261: png2c64's port, the old reference, by name.
    let old = json(&["--solver", "png2c64"]);
    assert_eq!(old["solver"], "png2c64");
    assert_eq!(old["legal"], true);
    let d = json(&[
        "--diverge",
        "chroma-guard,max-luma-diff=1",
        "--diverge",
        "pair-bias=0.01",
    ]);
    // Divergences are png2c64 options: the png2c64 port solves (ADR-013).
    assert_eq!(d["solver"], "png2c64");
    assert_eq!(d["legal"], true);
    // A native recipe knob asks for the native solver.
    let k = json(&["--max-luma-diff", "0"]);
    assert_eq!(k["solver"], "native");
    assert!(
        k["solver_fallback"]
            .as_str()
            .unwrap()
            .contains("max_luma_diff")
    );
    for bad in ["sparkle", "max-luma-diff=9", "mix-levels", "chroma-guard=1"] {
        let r = convert(&[
            "convert",
            png.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
            "--diverge",
            bad,
        ]);
        assert_eq!(r.status.code(), Some(2), "--diverge {bad}");
    }
}

/// #198: `--rotate` and `--anchor` turn the layer through `layer_rotate`; the saved
/// project holds the rotation, a turn changes the output, and bad values are refused.
#[test]
fn rotate_and_anchor_from_the_command_line() {
    let dir = temp("rotate");
    let png = source(&dir);
    let plain = dir.join("plain.kla");
    let r = convert(&[
        "convert",
        png.to_str().unwrap(),
        "-o",
        plain.to_str().unwrap(),
    ]);
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let out = dir.join("turned.kla");
    let project = dir.join("turned.convert64");
    let r = convert(&[
        "convert",
        png.to_str().unwrap(),
        "--rotate",
        "-3.5",
        "--anchor",
        "0,1",
        "--save-project",
        project.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
    ]);
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&project).unwrap()).unwrap();
    let t = &v["layers"][0]["transform"];
    assert_eq!(t["rotate"], -3.5, "{t}");
    assert_eq!(
        t["anchor"],
        serde_json::json!({ "x": 0.0, "y": 1.0 }),
        "{t}"
    );
    assert_ne!(std::fs::read(&plain).unwrap(), std::fs::read(&out).unwrap());
    let check = convert(&["validate", out.to_str().unwrap()]);
    assert_eq!(String::from_utf8_lossy(&check.stdout).trim(), "legal");
    for bad in [
        ["--rotate", "nan"],
        ["--anchor", "0.5"],
        ["--anchor", "2,0"],
    ] {
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

/// #235: `--downscale exact` keeps png2c64's sampling on a large source, which Auto
/// (the default) samples from a mip level; the saved project records it; up to a 4×
/// shrink the two give the same file.
#[test]
fn downscale_from_the_command_line() {
    let dir = temp("downscale");
    // Fine stripes, 5 source pixels per hi-res pixel: past Auto's 4× threshold.
    let big = dir.join("big.png");
    image::RgbaImage::from_fn(1600, 1000, |x, y| {
        let v = if (x / 2 + y / 3) % 2 == 0 { 40 } else { 220 };
        image::Rgba([v, (x % 256) as u8, (y % 256) as u8, 255])
    })
    .save(&big)
    .unwrap();
    let small = source(&dir);
    let run = |png: &Path, name: &str, extra: &[&str]| {
        let out = dir.join(name);
        let mut args = vec!["convert", png.to_str().unwrap()];
        args.extend_from_slice(extra);
        args.extend(["-o", out.to_str().unwrap()]);
        let r = convert(&args);
        assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
        std::fs::read(out).unwrap()
    };
    let project = dir.join("exact.convert64");
    let p = project.to_str().unwrap();
    let auto = run(&big, "auto.kla", &[]);
    let exact = run(
        &big,
        "exact.kla",
        &["--downscale", "exact", "--save-project", p],
    );
    assert_ne!(auto, exact);
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&project).unwrap()).unwrap();
    assert_eq!(v["layers"][0]["downscale"], "exact");
    assert_eq!(
        run(&small, "small-auto.kla", &["--downscale", "auto"]),
        run(&small, "small-exact.kla", &["--downscale", "exact"])
    );
    let r = convert(&[
        "convert",
        small.to_str().unwrap(),
        "--downscale",
        "area",
        "-o",
        dir.join("bad.kla").to_str().unwrap(),
    ]);
    assert_eq!(r.status.code(), Some(2));
}

/// #261: `--cell-refine` turns on png2amiga's Refine (`solve.refine`): the report says
/// so, the saved project holds it, and the result is legal.
#[test]
fn cell_refine_from_the_command_line() {
    let dir = temp("cell-refine");
    let png = source(&dir);
    let out = dir.join("refined.kla");
    let project = dir.join("refined.convert64");
    let r = convert(&[
        "--json",
        "convert",
        png.to_str().unwrap(),
        "--cell-refine",
        "--save-project",
        project.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
    ]);
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let v: serde_json::Value = serde_json::from_slice(&r.stdout).unwrap();
    assert_eq!(v["refine"], true, "{v}");
    assert_eq!(v["solver"], "reference", "{v}");
    assert_eq!(v["legal"], true, "{v}");
    let check = convert(&["validate", out.to_str().unwrap()]);
    assert_eq!(String::from_utf8_lossy(&check.stdout).trim(), "legal");
    let p: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&project).unwrap()).unwrap();
    assert_eq!(p["solve"]["refine"], true, "{p}");
    // Off, it is not written.
    let plain = dir.join("plain.convert64");
    let r = convert(&[
        "convert",
        png.to_str().unwrap(),
        "--save-project",
        plain.to_str().unwrap(),
    ]);
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let p: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&plain).unwrap()).unwrap();
    assert!(p["solve"].get("refine").is_none(), "{p}");
}

/// #273: `--bg-rule search` makes the reference search the multicolor background as
/// png2c64 does (`solve.background_rule`, written only when not `black`); without it the
/// background is png2amiga's colour 0.
#[test]
fn bg_rule_from_the_command_line() {
    let dir = temp("bg-rule");
    let png = source(&dir);
    let project = dir.join("search.convert64");
    let run = |extra: &[&str], project: &std::path::Path| {
        let mut args = vec!["--json", "convert", png.to_str().unwrap()];
        args.extend_from_slice(extra);
        let out = project.with_extension("kla");
        args.extend_from_slice(&["--save-project", project.to_str().unwrap()]);
        args.extend_from_slice(&["-o", out.to_str().unwrap()]);
        let r = convert(&args);
        assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
        let v: serde_json::Value = serde_json::from_slice(&r.stdout).unwrap();
        let p: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(project).unwrap()).unwrap();
        (v, p)
    };
    let (v, p) = run(&["--bg-rule", "search"], &project);
    assert_eq!(v["background_rule"], "search", "{v}");
    assert_eq!(v["legal"], true, "{v}");
    assert_eq!(p["solve"]["background_rule"], "search", "{p}");
    let (v, p) = run(&[], &dir.join("black.convert64"));
    assert_eq!(v["background_rule"], "black", "{v}");
    assert_eq!(v["background"], 0, "{v}");
    assert!(p["solve"].get("background_rule").is_none(), "{p}");
    let bad = convert(&["convert", png.to_str().unwrap(), "--bg-rule", "grey"]);
    assert_eq!(bad.status.code(), Some(2));
}

/// #261: `--match-range` is png2amiga's hull onto the project's palette, at
/// `--match-range-amount` (written as `amount` only when not 100 %);
/// `--match-range-method box` is png2c64's. The amount and method need --match-range.
#[test]
fn match_range_amount_and_method_from_the_command_line() {
    let dir = temp("match-range");
    let png = source(&dir);
    let run = |name: &str, extra: &[&str]| -> (Vec<u8>, serde_json::Value) {
        let out = dir.join(format!("{name}.kla"));
        let project = dir.join(format!("{name}.convert64"));
        let mut args = vec![
            "convert",
            png.to_str().unwrap(),
            "--match-range",
            "-o",
            out.to_str().unwrap(),
            "--save-project",
            project.to_str().unwrap(),
        ];
        args.extend_from_slice(extra);
        let r = convert(&args);
        assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
        let check = convert(&["validate", out.to_str().unwrap()]);
        assert_eq!(String::from_utf8_lossy(&check.stdout).trim(), "legal");
        let p: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&project).unwrap()).unwrap();
        let m = p["layers"][0]["adjustments"]["match_range"].clone();
        (std::fs::read(out).unwrap(), m)
    };
    let (full, m) = run("full", &[]);
    assert_eq!(m["method"], "hull", "{m}");
    assert_eq!(m["palette"].as_array().map(Vec::len), Some(16), "{m}");
    assert!(m.get("amount").is_none(), "amount 1 is not written: {m}");
    let (half, m) = run("half", &["--match-range-amount", "50"]);
    assert_eq!(m["method"], "hull", "{m}");
    assert_eq!(m["amount"], 0.5, "{m}");
    assert_ne!(full, half, "50 % matches less than 100 %");
    let (_, m) = run("box", &["--match-range-method", "box"]);
    assert!(
        m.get("method").is_none(),
        "a box is written as before #261: {m}"
    );
    assert!(
        m["l"].is_array() && m["a"].is_array() && m["b"].is_array(),
        "{m}"
    );
    let out = dir.join("bad.kla");
    for bad in [
        &["--match-range-amount", "50"][..],
        &["--match-range-method", "hull"],
        &["--match-range", "--match-range-amount", "150"],
        &["--match-range", "--match-range-amount=-1"],
        &[
            "--match-range",
            "--match-range-method",
            "box",
            "--match-range-amount",
            "50",
        ],
        &["--match-range", "--match-range-method", "cube"],
    ] {
        let mut args = vec![
            "convert",
            png.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
        ];
        args.extend_from_slice(bad);
        let r = convert(&args);
        assert_eq!(r.status.code(), Some(2), "{bad:?}");
    }
}

/// png2amiga's methods (#261), with png2amiga's spellings of some of them.
const PNG2AMIGA_IDS: &[&str] = &[
    "bayer3",
    "bayer5",
    "bayer6",
    "bayer7",
    "v4x2",
    "bayer4x2",
    "bayer2x4",
    "line-v-checker",
    "fractal16",
    "aseprite-old",
    "libcaca3",
    "libcaca6",
    "pegasus",
    "void-cluster",
    "cluster-noise",
    "cranley-bayer",
    "ign-tri",
    "r2-tri",
    "quasicrystal",
    "truchet",
    "floyd-steinberg-oklab",
    "gilbert",
    "dbs",
    "structure-fs",
    "contrast-fs",
    "yliluoma",
    "opt-checker",
    "opt-line",
    "opt-line-checker",
    "opt-vline",
    "opt-vline-checker",
    "tri-tone",
    "bayer3x3",
    "bayer7x7",
    "vline-checker",
    "zhoufang",
];

#[test]
fn convert_accepts_png2amiga_method_ids() {
    let dir = temp("convert-png2amiga-ids");
    let png = source(&dir);
    let out = dir.join("id.kla");
    for id in PNG2AMIGA_IDS {
        let r = convert(&[
            "convert",
            png.to_str().unwrap(),
            "--dither",
            id,
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
