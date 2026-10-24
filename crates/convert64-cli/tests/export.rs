//! `convert64 export` (#165): a directory of projects exports to every assembler in one
//! command, and `validate` reads the raw sets it writes.

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Output;

fn temp(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn convert64(dir: &Path, args: &[&str]) -> Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_convert64"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap()
}

fn ok(out: &Output) -> Value {
    assert!(
        out.status.success(),
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap_or(Value::Null)
}

/// `art/` with a multicolor and a hi-res project.
fn projects(name: &str) -> PathBuf {
    let dir = temp(name);
    std::fs::create_dir_all(dir.join("art")).unwrap();
    image::RgbaImage::from_fn(80, 50, |x, y| {
        image::Rgba([(x * 3) as u8, (y * 5) as u8, ((x + y) * 2) as u8, 255])
    })
    .save(dir.join("art/pic.png"))
    .unwrap();
    for (project, mode) in [("title", "koala"), ("logo", "hires")] {
        ok(&convert64(
            &dir.join("art"),
            &[
                "convert",
                "pic.png",
                "--mode",
                mode,
                "--supersample",
                "1",
                "--dither",
                "bayer4",
                "--save-project",
                &format!("{project}.convert64"),
            ],
        ));
    }
    dir
}

#[test]
fn a_directory_of_projects_exports_to_every_assembler() {
    let dir = projects("export-all");
    let v = ok(&convert64(
        &dir,
        &["export", "art", "--asm", "all", "--out", "build", "--json"],
    ));
    assert_eq!(v["command"], "export");
    assert_eq!(
        v["assemblers"],
        serde_json::json!(["kickass", "acme", "64tass", "ca65", "c"])
    );
    assert_eq!(v["projects"].as_array().unwrap().len(), 2);
    for (name, colour) in [("title", true), ("logo", false)] {
        for ext in ["asm", "a", "tas", "s", "h"] {
            let src = dir.join(format!("build/{name}.{ext}"));
            let text = std::fs::read_to_string(&src).unwrap();
            assert!(text.contains(name), "{}", src.display());
        }
        assert!(dir.join(format!("build/{name}.bitmap.bin")).exists());
        assert_eq!(
            dir.join(format!("build/{name}.colour.bin")).exists(),
            colour,
            "{name}"
        );
    }
    // The included bitmap is the solved picture: the same bytes as convert's raw set.
    std::fs::create_dir_all(dir.join("raw")).unwrap();
    ok(&convert64(
        &dir,
        &["convert", "art/title.convert64", "-o", "raw/title.json"],
    ));
    for block in ["bitmap", "screen", "colour", "bg"] {
        assert_eq!(
            std::fs::read(dir.join(format!("build/title.{block}.bin"))).unwrap(),
            std::fs::read(dir.join(format!("raw/title.{block}.bin"))).unwrap(),
            "{block}"
        );
    }
    let v = ok(&convert64(&dir, &["validate", "raw/title.json", "--json"]));
    assert_eq!(v["format"], "raw set");
    assert_eq!(v["legal"], true);
    // A raw set with a block missing is not readable.
    std::fs::remove_file(dir.join("raw/title.bg.bin")).unwrap();
    assert_eq!(
        convert64(&dir, &["validate", "raw/title.json"])
            .status
            .code(),
        Some(1)
    );
}

#[test]
fn export_options_and_errors() {
    let dir = projects("export-options");
    // One project, two targets, data in the source, another name.
    ok(&convert64(
        &dir,
        &[
            "export",
            "art/title.convert64",
            "--asm",
            "kickass,acme",
            "--data",
            "bytes",
            "--name",
            "intro",
            "--out",
            "out",
        ],
    ));
    let mut files: Vec<String> = std::fs::read_dir(dir.join("out"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    files.sort();
    assert_eq!(files, ["intro.a", "intro.asm"]);
    for bad in [
        vec!["export", "art", "--asm", "masm", "--out", "x"],
        vec![
            "export", "art", "--asm", "acme", "--out", "x", "--name", "n",
        ],
        vec!["export", "missing.convert64", "--asm", "acme", "--out", "x"],
        vec!["export", "art/title.convert64", "--out", "x"],
    ] {
        assert_eq!(convert64(&dir, &bad).status.code(), Some(2), "{bad:?}");
    }
}

/// #287: without `--asm`, `export` writes each project's export set (what the app's
/// Export panel and `Shift Cmd E` write), named after the project unless a row names
/// itself; a project without one is refused, saying how to add one.
#[test]
fn export_without_asm_writes_the_export_set() {
    let dir = projects("export-set");
    let set = r#"{"command":"export_set","exports":[{"format":"picture"},{"format":"prg"},
        {"format":"kickass","data":"include-raw"},{"format":"png","zoom":4,"name":"title@4x"}]}"#;
    ok(&convert64(
        &dir,
        &[
            "apply",
            "art/title.convert64",
            "--command",
            set,
            "--in-place",
        ],
    ));
    let saved = std::fs::read_to_string(dir.join("art/title.convert64")).unwrap();
    assert!(saved.contains(r#""exports""#), "{saved}");
    let v = ok(&convert64(
        &dir,
        &["export", "art/title.convert64", "--out", "build", "--json"],
    ));
    let report = &v["projects"][0];
    assert_eq!(
        report["exports"],
        serde_json::json!(["picture", "prg", "kickass", "png"])
    );
    let mut files: Vec<String> = std::fs::read_dir(dir.join("build"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    files.sort();
    assert_eq!(
        files,
        [
            "title.asm",
            "title.bg.bin",
            "title.bitmap.bin",
            "title.colour.bin",
            "title.kla",
            "title.prg",
            "title.screen.bin",
            "title@4x.png"
        ]
    );
    assert_eq!(report["files_written"].as_array().unwrap().len(), 8);
    // The Koala file is the solved picture, as convert writes it.
    ok(&convert64(
        &dir,
        &["convert", "art/title.convert64", "-o", "title.kla"],
    ));
    assert_eq!(
        std::fs::read(dir.join("build/title.kla")).unwrap(),
        std::fs::read(dir.join("title.kla")).unwrap()
    );
    let out = convert64(&dir, &["export", "art/logo.convert64", "--out", "x"]);
    assert_eq!(out.status.code(), Some(2));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("no export set"), "{err}");
}
