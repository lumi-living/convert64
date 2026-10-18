//! `convert64 batch` over a directory with a TOML manifest, and `validate` for every
//! format convert64 reads (#165).

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Output;

fn temp(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn picture(path: &Path, seed: u32) {
    image::RgbaImage::from_fn(80, 50, |x, y| {
        image::Rgba([
            (x * 3 + seed * 40) as u8,
            (y * 5) as u8,
            ((x + y) * 2 + seed * 17) as u8,
            255,
        ])
    })
    .save(path)
    .unwrap();
}

fn convert64(dir: &Path, args: &[&str]) -> Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_convert64"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap()
}

fn report(out: &Output) -> Value {
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "{e}: {}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    })
}

const MANIFEST: &str = r#"
out = "build"

[defaults]
dither = "bayer4"
supersample = 1
outputs = ["kla", "prg"]

[files."b.png"]
mode = "hires"
outputs = ["art"]

[files."skip.png"]
skip = true

[files."title.convert64"]
outputs = ["art", "prg"]
max-clashes = 1000
"#;

fn setup(name: &str) -> PathBuf {
    let dir = temp(name);
    let art = dir.join("art");
    std::fs::create_dir_all(&art).unwrap();
    for (i, n) in ["a.png", "b.png", "skip.png", "title.png"]
        .iter()
        .enumerate()
    {
        picture(&art.join(n), i as u32);
    }
    // title.png has a project next to it: the project is converted, not the PNG.
    let r = convert64(
        &art,
        &[
            "convert",
            "title.png",
            "--mode",
            "hires",
            "--save-project",
            "title.convert64",
        ],
    );
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    std::fs::write(art.join("batch.toml"), MANIFEST).unwrap();
    dir
}

#[test]
fn a_directory_converts_with_its_manifest_in_one_report() {
    let dir = setup("batch");
    let r = convert64(&dir, &["batch", "art", "--json", "-j", "3"]);
    let v = report(&r);
    assert_eq!(r.status.code(), Some(0), "{v:#}");
    assert_eq!(v["command"], "batch");
    assert_eq!(v["converted"], 3);
    assert_eq!(v["failed"], 0);
    let skipped: Vec<&str> = v["skipped"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["input"].as_str().unwrap())
        .collect();
    assert_eq!(skipped, ["skip.png", "title.png"]);
    let mut written: Vec<String> = v["files_written"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            Path::new(f.as_str().unwrap())
                .strip_prefix("art/build")
                .unwrap()
                .display()
                .to_string()
        })
        .collect();
    written.sort();
    assert_eq!(
        written,
        ["a.kla", "a.prg", "b.art", "title.art", "title.prg"]
    );
    let files = v["files"].as_array().unwrap();
    assert_eq!(files.len(), 3);
    assert!(
        files
            .iter()
            .all(|f| f["command"] == "convert" && f["ok"] == true)
    );
    assert_eq!(files[2]["input"], "title.convert64");
    assert_eq!(files[2]["mode"], "hires-bitmap");
    assert_eq!(files[2]["max_clashes"], 1000);

    // Every output is legal, whatever its format.
    for f in ["a.kla", "a.prg", "b.art", "title.art", "title.prg"] {
        let r = convert64(&dir.join("art/build"), &["validate", f]);
        assert_eq!(
            String::from_utf8_lossy(&r.stdout).trim(),
            "legal",
            "{f}: {}",
            String::from_utf8_lossy(&r.stderr)
        );
    }

    // One worker gives the same bytes; options on the command line beat the manifest.
    let first = std::fs::read(dir.join("art/build/a.kla")).unwrap();
    let r = convert64(
        &dir,
        &[
            "batch", "art", "-j", "1", "--out", "serial", "--dither", "none",
        ],
    );
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let r = convert64(&dir, &["batch", "art", "-j", "1", "--out", "serial2"]);
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    assert_eq!(std::fs::read(dir.join("serial2/a.kla")).unwrap(), first);
    assert_ne!(std::fs::read(dir.join("serial/a.kla")).unwrap(), first);
}

#[test]
fn batch_errors() {
    let dir = setup("batch-errors");
    // One unreadable file fails (exit 2) without stopping the others.
    std::fs::write(dir.join("art/broken.png"), b"not a png").unwrap();
    let r = convert64(&dir, &["batch", "art", "--json"]);
    let v = report(&r);
    assert_eq!(r.status.code(), Some(2));
    assert_eq!(v["failed"], 1);
    assert_eq!(v["converted"], 3);
    let broken = v["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["input"] == "broken.png")
        .unwrap();
    assert_eq!(broken["exit_code"], 2);
    // A manifest with an unknown key is refused before anything runs.
    std::fs::write(dir.join("bad.toml"), "[defaults]\ndithr = \"bayer4\"\n").unwrap();
    let r = convert64(&dir, &["batch", "art", "--manifest", "bad.toml"]);
    assert_eq!(r.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&r.stderr).contains("dithr"));
    std::fs::write(dir.join("bad.toml"), "[defaults]\noutputs = [\"gif\"]\n").unwrap();
    let r = convert64(&dir, &["batch", "art", "--manifest", "bad.toml"]);
    assert_eq!(r.status.code(), Some(2));
    // Too many clashes: exit 1, the file is still written.
    std::fs::remove_file(dir.join("art/broken.png")).unwrap();
    std::fs::write(
        dir.join("strict.toml"),
        "[defaults]\nsupersample = 1\nmax-clashes = 0\ndither = \"fs\"\n",
    )
    .unwrap();
    let r = convert64(
        &dir,
        &[
            "batch",
            "art",
            "--manifest",
            "strict.toml",
            "--out",
            "strict",
        ],
    );
    assert_eq!(
        r.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&r.stdout)
    );
    assert!(dir.join("strict/a.kla").exists());
}

#[test]
fn validate_reads_display_prgs_and_projects() {
    let dir = temp("validate-formats");
    picture(&dir.join("in.png"), 0);
    let r = convert64(
        &dir,
        &[
            "convert",
            "in.png",
            "--supersample",
            "1",
            "-o",
            "p.prg",
            "-o",
            "p.kla",
            "--save-project",
            "p.convert64",
        ],
    );
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let v = report(&convert64(&dir, &["validate", "p.prg", "--json"]));
    assert_eq!(v["legal"], true);
    assert_eq!(v["format"], "display PRG (Koala)");
    let v = report(&convert64(&dir, &["validate", "p.convert64", "--json"]));
    assert_eq!(v["legal"], true, "{v:#}");
    assert_eq!(v["format"], "convert64 project");
    assert!(v["clashes"].is_u64());
    // A PRG that is not ours, and one whose picture is broken.
    let mut prg = std::fs::read(dir.join("p.prg")).unwrap();
    prg[20] ^= 0xFF;
    std::fs::write(dir.join("other.prg"), &prg).unwrap();
    assert_eq!(
        convert64(&dir, &["validate", "other.prg"]).status.code(),
        Some(2)
    );
    let mut prg = std::fs::read(dir.join("p.prg")).unwrap();
    let last = prg.len() - 1;
    prg[last] = 0x1F; // background colour 31: not a colour
    std::fs::write(dir.join("broken.prg"), &prg).unwrap();
    assert_eq!(
        convert64(&dir, &["validate", "broken.prg"]).status.code(),
        Some(1)
    );
}

/// #261, ADR-014: a manifest takes `solver` (`png2amiga`, and `png2c64` for the
/// reference), `cell-refine`, `match-range-amount`, `match-range-method`, `resample`,
/// `black-point` and `allowed`, as convert's options; a bad amount fails that file.
#[test]
fn manifest_takes_the_png2amiga_options() {
    let dir = temp("batch-261");
    let art = dir.join("art");
    std::fs::create_dir_all(&art).unwrap();
    picture(&art.join("a.png"), 0);
    picture(&art.join("b.png"), 1);
    picture(&art.join("c.png"), 2);
    std::fs::write(
        art.join("batch.toml"),
        r#"
[defaults]
match-range = true
outputs = ["kla"]

[files."a.png"]
solver = "png2amiga"
cell-refine = true
match-range-method = "hull"
match-range-amount = 40

[files."b.png"]
solver = "png2c64"
match-range-method = "box"
resample = "lanczos"
black-point = 0.5
allowed = "0,11,12,15,1"

[files."c.png"]
match-range-method = "hull"
match-range-amount = 400
"#,
    )
    .unwrap();
    let r = convert64(&dir, &["batch", "art", "--json"]);
    let v = report(&r);
    assert_eq!(r.status.code(), Some(2), "{v:#}");
    let file = |name: &str| {
        v["files"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["input"] == name)
            .unwrap_or_else(|| panic!("{name}: {v:#}"))
            .clone()
    };
    let a = file("a.png");
    assert_eq!(
        (a["ok"].clone(), a["refine"].clone()),
        (true.into(), true.into()),
        "{a:#}"
    );
    let b = file("b.png");
    assert_eq!(b["solver"], "reference", "{b:#}");
    assert_eq!(b["ok"], true, "{b:#}");
    assert_eq!(file("a.png")["solver"], "png2amiga", "{a:#}");
    assert_eq!(b["refine"], false, "{b:#}");
    assert_eq!(file("c.png")["exit_code"], 2);
}

/// ADR-014 §4: the line-art recipe file (`examples/recipes/lineart.toml`) works as a
/// manifest: hi-res Art Studio files, solved by the png2amiga solver (#276), using only
/// the greys.
#[test]
fn the_line_art_recipe_file_is_a_manifest() {
    let dir = temp("batch-lineart");
    let art = dir.join("art");
    std::fs::create_dir_all(&art).unwrap();
    picture(&art.join("a.png"), 0);
    let manifest =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/recipes/lineart.toml");
    let r = convert64(
        &dir,
        &[
            "batch",
            "art",
            "--manifest",
            manifest.to_str().unwrap(),
            "--out",
            "build",
            "--json",
        ],
    );
    let v = report(&r);
    assert_eq!(r.status.code(), Some(0), "{v:#}");
    let a = &v["files"][0];
    assert_eq!(a["mode"], "hires-bitmap", "{a:#}");
    assert_eq!(a["solver"], "png2amiga", "{a:#}");
    assert_eq!(a["solver_fallback"], Value::Null, "{a:#}");
    let bytes = std::fs::read(dir.join("build/a.art")).unwrap();
    let file = convert64_formats::BitmapFormat::ArtStudio
        .read(&bytes)
        .unwrap();
    let greys = [0u8, 11, 12, 15, 1];
    let used: Vec<u8> = file
        .image
        .to_indexed()
        .pixels()
        .iter()
        .map(|c| c.get())
        .collect();
    assert!(used.iter().all(|c| greys.contains(c)));
}
