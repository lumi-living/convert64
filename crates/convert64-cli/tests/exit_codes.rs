//! `convert64 convert` from a PNG or a project to every M1 format, its exit codes and the
//! `--json` report every command prints (#164).

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Output;

fn temp(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A busy 160 × 100 picture: hue bands with per-pixel noise, so cells clash.
fn source(dir: &Path) -> PathBuf {
    let img = image::RgbaImage::from_fn(160, 100, |x, y| {
        let n = (x * 7919 + y * 104_729) % 97;
        image::Rgba([
            (x * 255 / 160) as u8,
            (y * 255 / 100) as u8,
            ((n * 255) / 97) as u8,
            255,
        ])
    });
    let p = dir.join("in.png");
    img.save(&p).unwrap();
    p
}

fn convert64(dir: &Path, args: &[&str]) -> Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_convert64"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap()
}

fn json(out: &Output) -> Value {
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "{e}: {}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    })
}

fn ok(out: &Output) {
    assert!(
        out.status.success(),
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

const FAST: &[&str] = &["--supersample", "1", "--dither", "bayer4"];

fn args<'a>(head: &[&'a str]) -> Vec<&'a str> {
    head.iter().chain(FAST).copied().collect()
}

#[test]
fn one_solve_writes_every_format_and_reports_it() {
    let dir = temp("convert-formats");
    source(&dir);
    let r = convert64(
        &dir,
        &args(&[
            "convert", "in.png", "-o", "out.kla", "-o", "out.prg", "--json", "--report",
        ]),
    );
    ok(&r);
    let v = json(&r);
    assert_eq!(v["command"], "convert");
    assert_eq!(v["ok"], true);
    assert_eq!(v["exit_code"], 0);
    assert_eq!(v["error"], Value::Null);
    assert_eq!(
        v["files_written"],
        serde_json::json!(["out.kla", "out.prg"])
    );
    assert_eq!(v["mode"], "multicolor-bitmap");
    assert_eq!(v["palette"], "Pepto (PAL)");
    assert_eq!(v["legal"], true);
    assert!(v["clashes"].is_u64());
    assert!(v["summary"]["clashes"].is_u64());
    assert!(v["worst"].as_array().unwrap().len() <= 10);
    for phase in ["load", "solve", "write", "total"] {
        assert!(v["timing_ms"][phase].as_f64().unwrap() >= 0.0, "{phase}");
    }
    // The PRG carries the Koala file verbatim after its loader.
    let kla = std::fs::read(dir.join("out.kla")).unwrap();
    let prg = std::fs::read(dir.join("out.prg")).unwrap();
    assert_eq!(&prg[..2], &[0x01, 0x08]);
    assert_eq!(&prg[convert64_formats::prg::PAYLOAD_OFFSET..], &kla[..]);
    assert_eq!(
        String::from_utf8_lossy(&convert64(&dir, &["validate", "out.kla"]).stdout).trim(),
        "legal"
    );

    // The extension picks the mode for a PNG; --format names an extension-less output.
    ok(&convert64(
        &dir,
        &args(&["convert", "in.png", "-o", "out.art"]),
    ));
    assert_eq!(std::fs::read(dir.join("out.art")).unwrap().len(), 9002);
    ok(&convert64(
        &dir,
        &args(&["convert", "in.png", "-o", "picture", "--format", "art"]),
    ));
    assert_eq!(
        std::fs::read(dir.join("picture")).unwrap(),
        std::fs::read(dir.join("out.art")).unwrap()
    );
    // --mode koala, mc and multicolor are one mode.
    for m in ["koala", "mc", "multicolor"] {
        ok(&convert64(
            &dir,
            &args(&["convert", "in.png", "--mode", m, "-o", "m.kla"]),
        ));
        assert_eq!(std::fs::read(dir.join("m.kla")).unwrap(), kla, "{m}");
    }
}

#[test]
fn projects_use_their_own_settings_and_flags_override_them() {
    let dir = temp("convert-project");
    source(&dir);
    std::fs::create_dir_all(dir.join("art")).unwrap();
    // A PNG converted with settings and saved as a project ...
    ok(&convert64(
        &dir,
        &args(&[
            "convert",
            "in.png",
            "--mode",
            "hires",
            "--strength",
            "0.6",
            "--palette",
            "colodore",
            "-o",
            "direct.art",
            "--save-project",
            "art/title.convert64",
        ]),
    ));
    let project: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("art/title.convert64")).unwrap())
            .unwrap();
    assert_eq!(project["target"], "hires-bitmap");
    assert_eq!(project["palette"], "colodore");
    assert_eq!(project["layers"][0]["source"]["path"], "../in.png");
    assert_eq!(project["layers"][0]["recipe"]["method"]["kind"], "bayer4");
    // ... converts to the same picture with no flags at all.
    ok(&convert64(
        &dir,
        &["convert", "art/title.convert64", "-o", "from-project.art"],
    ));
    assert_eq!(
        std::fs::read(dir.join("from-project.art")).unwrap(),
        std::fs::read(dir.join("direct.art")).unwrap()
    );
    // The project is hi-res: a .kla output disagrees unless --mode overrides it.
    let r = convert64(&dir, &["convert", "art/title.convert64", "-o", "x.kla"]);
    assert_eq!(r.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&r.stderr).contains("--mode"));
    let r = convert64(
        &dir,
        &[
            "convert",
            "art/title.convert64",
            "--mode",
            "koala",
            "--dither",
            "none",
            "-o",
            "x.kla",
            "--json",
        ],
    );
    ok(&r);
    assert_eq!(json(&r)["mode"], "multicolor-bitmap");
    assert_eq!(json(&r)["palette"], "Colodore");
}

#[test]
fn exit_codes() {
    let dir = temp("convert-exit-codes");
    source(&dir);
    // 1: more clashes than allowed. The file is still written.
    let r = convert64(
        &dir,
        &args(&[
            "convert",
            "in.png",
            "-o",
            "c.kla",
            "--max-clashes",
            "0",
            "--json",
        ]),
    );
    let v = json(&r);
    assert!(
        v["clashes"].as_u64().unwrap() > 0,
        "the noisy source clashes"
    );
    assert_eq!(r.status.code(), Some(1));
    assert_eq!(v["exit_code"], 1);
    assert_eq!(v["ok"], false);
    assert!(dir.join("c.kla").exists());
    let r = convert64(
        &dir,
        &args(&["convert", "in.png", "-o", "c.kla", "--max-clashes", "1000"]),
    );
    assert_eq!(r.status.code(), Some(0));

    // 2: input and option errors.
    for bad in [
        vec!["convert", "missing.png", "-o", "x.kla"],
        vec!["convert", "in.png", "-o", "x.unknown"],
        vec!["convert", "in.png", "-o", "x.kla", "--palette", "vice"],
        vec!["convert", "in.png", "-o", "x.kla", "--strength", "2"],
        vec!["convert", "in.png", "-o", "x.kla", "-o", "y.art"],
        vec![
            "convert", "in.png", "-o", "x.kla", "--format", "art", "-o", "y.kla",
        ],
        vec!["convert", "in.png"],
        vec!["convert", "in.png", "-o", "x.kla", "--no-such-flag"],
        vec!["validate", "missing.kla"],
        vec!["recipe", "apply", "missing.json"],
    ] {
        let r = convert64(&dir, &bad);
        assert_eq!(r.status.code(), Some(2), "{bad:?}");
    }

    // 3: write errors.
    let r = convert64(
        &dir,
        &args(&["convert", "in.png", "-o", "no/such/dir/x.kla", "--json"]),
    );
    assert_eq!(r.status.code(), Some(3));
    let v = json(&r);
    assert_eq!(v["exit_code"], 3);
    assert!(v["error"].as_str().unwrap().contains("no/such/dir/x.kla"));
}

#[test]
fn every_command_has_a_json_report() {
    let dir = temp("json-everywhere");
    source(&dir);
    ok(&convert64(
        &dir,
        &args(&["convert", "in.png", "-o", "a.kla"]),
    ));
    for (cmd, line, code) in [
        ("validate", vec!["validate", "a.kla", "--json"], 0),
        ("validate", vec!["--json", "validate", "missing.kla"], 2),
        ("recipe commands", vec!["recipe", "commands", "--json"], 0),
        (
            "convert",
            vec!["convert", "missing.png", "-o", "x.kla", "--json"],
            2,
        ),
        ("usage", vec!["convert", "--json", "--no-such-flag"], 2),
    ] {
        let r = convert64(&dir, &line);
        assert_eq!(r.status.code(), Some(code), "{line:?}");
        let v = json(&r);
        assert_eq!(v["command"], cmd, "{line:?}");
        assert_eq!(v["exit_code"], code);
        assert_eq!(v["ok"], code == 0);
        assert_eq!(v["error"].is_null(), code == 0, "{line:?}");
        assert!(v["timing_ms"]["total"].is_f64());
        assert!(v["files_written"].is_array());
    }
}
