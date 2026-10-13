//! `.convert64` project files (#163): a project saved by the CLI reloads identically and
//! solves to the same picture; version-1 recipe files migrate; sources are linked or
//! embedded per layer.

use convert64_project::file::SourceSpec;
use convert64_project::{Embed, ProjectFile, Session};
use std::path::{Path, PathBuf};

fn temp(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A 64 × 40 picture: green on the left, red on the right, with a vertical ramp.
fn write_source(path: &Path) {
    let img = image::RgbaImage::from_fn(64, 40, |x, y| {
        let v = (y * 6) as u8;
        if x < 32 {
            image::Rgba([v / 4, 200, v / 3, 255])
        } else {
            image::Rgba([210, v / 2, 40, 255])
        }
    });
    img.save(path).unwrap();
}

/// A recipe exercising every kind of state a project file holds: masks with sources
/// and modifiers, a shared-pixel layer and steering.
const RECIPE: &str = r#"{
  "version": 1,
  "target": "multicolor-bitmap",
  "supersample": 2,
  "layers": [
    { "name": "photo", "source": "photo.png",
      "transform": { "scale": { "x": 5.0, "y": 5.0 } },
      "recipe": { "method": { "kind": "bayer4" }, "strength": 0.75 } }
  ],
  "commands": [
    { "command": "mask_clear", "layer": "photo", "value": 0.0 },
    { "command": "mask_key", "layer": "photo",
      "key": { "channel": "chroma", "centre": [0.0, 0.58, 0.0], "range": 0.12, "softness": 0.1 },
      "combine": "max" },
    { "command": "mask_set_modifier", "layer": 0, "modifier": { "feather": 2.0 } },
    { "command": "layer_via_mask", "source_layer": 0, "cut": false,
      "selection": { "polygons": [[{ "x": 0, "y": 0 }, { "x": 32, "y": 0 }, { "x": 32, "y": 40 }, { "x": 0, "y": 40 }]] } },
    { "command": "cell_pin", "cells": [{ "x": 3, "y": 4 }], "colours": [0, 1, 2, 5], "full": true },
    { "command": "cell_lock", "cells": [{ "x": 39, "y": 24 }] },
    { "command": "cell_deny", "cells": [{ "x": 10, "y": 10 }], "colours": [7] }
  ],
  "solve": { "background": { "fixed": 0 }, "hysteresis": 0.97 }
}"#;

fn solved_bytes(s: &mut Session) -> Vec<u8> {
    let sol = s.solve(true);
    convert64_formats::BitmapFormat::for_mode(sol.image.mode())
        .unwrap()
        .write(&sol.image)
        .unwrap()
}

#[test]
fn a_saved_project_reloads_identically_and_solves_the_same() {
    let dir = temp("project-round-trip");
    write_source(&dir.join("photo.png"));
    std::fs::write(dir.join("recipe.json"), RECIPE).unwrap();

    // Version 1 → project (the migration), saved next to the recipe: sources stay linked.
    let mut from_recipe = Session::open(&dir.join("recipe.json")).unwrap();
    let first = dir.join("a.convert64");
    from_recipe.save(&first, Embed::Keep).unwrap();
    let text_a = std::fs::read_to_string(&first).unwrap();
    let file: ProjectFile = serde_json::from_str(&text_a).unwrap();
    assert_eq!(file.schema_version, convert64_project::SCHEMA_VERSION);
    assert_eq!(file.supersample, 2, "a recipe's explicit factor is kept");
    assert_eq!(file.layers.len(), 2);
    assert!(
        matches!(&file.layers[0].source, SourceSpec::Linked(l) if l.path == Path::new("photo.png"))
    );
    assert!(matches!(&file.layers[1].source, SourceSpec::Shared(s) if s.layer == 0));
    assert!(
        file.commands.is_empty(),
        "commands are saved as their result"
    );
    assert_eq!(file.steering.pins.len(), 1);
    assert_eq!(file.steering.locks.len(), 1);
    assert_eq!(file.steering.deny.len(), 1);

    // load → save → load is the identity, byte for byte.
    let mut a = Session::open(&first).unwrap();
    let second = dir.join("b.convert64");
    a.save(&second, Embed::Keep).unwrap();
    assert_eq!(std::fs::read_to_string(&second).unwrap(), text_a);
    let mut b = Session::open(&second).unwrap();

    // The migrated recipe, the project and its reload solve to the same picture.
    let picture = solved_bytes(&mut from_recipe);
    assert_eq!(solved_bytes(&mut a), picture);
    assert_eq!(solved_bytes(&mut b), picture);
}

#[test]
fn embedded_projects_stand_alone_and_linked_paths_follow_the_file() {
    let dir = temp("project-embed");
    write_source(&dir.join("photo.png"));
    std::fs::write(dir.join("recipe.json"), RECIPE).unwrap();
    let mut s = Session::open(&dir.join("recipe.json")).unwrap();
    let picture = solved_bytes(&mut s);

    // Linked, saved one directory down: the path walks up.
    std::fs::create_dir_all(dir.join("art")).unwrap();
    let linked = dir.join("art/title.convert64");
    s.save(&linked, Embed::Keep).unwrap();
    let file = ProjectFile::read(&linked).unwrap();
    assert!(
        matches!(&file.layers[0].source, SourceSpec::Linked(l) if l.path == Path::new("../photo.png"))
    );
    assert_eq!(solved_bytes(&mut Session::open(&linked).unwrap()), picture);

    // Embedded, then moved away from the source: it still loads and solves the same.
    let embedded = dir.join("embedded.convert64");
    s.save(&embedded, Embed::All).unwrap();
    let elsewhere = temp("project-embed-elsewhere").join("moved.convert64");
    std::fs::copy(&embedded, &elsewhere).unwrap();
    let mut moved = Session::open(&elsewhere).unwrap();
    assert!(matches!(
        &ProjectFile::read(&elsewhere).unwrap().layers[0].source,
        SourceSpec::Embedded(_)
    ));
    assert_eq!(solved_bytes(&mut moved), picture);
    // Keep keeps it embedded; linking an embedded source is an error.
    let again = elsewhere.with_file_name("again.convert64");
    moved.save(&again, Embed::Keep).unwrap();
    assert_eq!(
        std::fs::read_to_string(&again).unwrap(),
        std::fs::read_to_string(&elsewhere).unwrap()
    );
    assert!(moved.save(&again, Embed::None).is_err());
}

#[test]
fn hand_written_projects_with_regions_palettes_and_masks() {
    let dir = temp("project-hand-written");
    write_source(&dir.join("photo.png"));
    let mask = image::ImageBuffer::from_fn(64, 40, |x, _| image::Luma([(x * 4) as u8]));
    mask.save(dir.join("mask.png")).unwrap();
    let text = r##"{
      "schema_version": 3,
      "target": "hires-bitmap",
      "supersample": 2,
      "palette": "colodore",
      "layers": [
        { "name": "back", "source": { "path": "photo.png" },
          "transform": { "scale": { "x": 5.0, "y": 5.0 } } },
        { "name": "top", "source": { "layer": 0 }, "visible": false, "opacity": 0.5,
          "blend": "multiply",
          "transform": { "translate": { "x": 8.0, "y": 4.0 }, "scale": { "x": 2.0, "y": 2.0 } },
          "adjustments": { "gamma": 1.5 },
          "mask": { "raster": { "path": "mask.png" }, "modifiers": { "invert": true } } }
      ],
      "regions": [
        { "name": "sky", "mask": { "fill": 0.5 }, "recipe": { "method": { "kind": "none" }, "strength": 1.0 } }
      ],
      "steering": { "allow": [{ "x": 1, "y": 1, "colours": [0, 1] }] },
      "solve": { "refinements": 1, "finalists": 2 }
    }"##;
    std::fs::write(dir.join("hand.convert64"), text).unwrap();
    let mut s = Session::open(&dir.join("hand.convert64")).unwrap();
    assert_eq!(s.palette().name(), "Colodore");
    assert_eq!(s.grid().n(), 2, "a version-3 file keeps its factor");
    assert!(s.notes().is_empty());
    let p = s.project();
    assert!(!p.layers[1].visible());
    assert!(std::sync::Arc::ptr_eq(
        p.layers[0].source(),
        p.layers[1].source()
    ));
    assert_eq!(p.regions.len(), 1);
    let picture = solved_bytes(&mut s);

    let saved = dir.join("saved.convert64");
    s.save(&saved, Embed::Keep).unwrap();
    let file = ProjectFile::read(&saved).unwrap();
    assert_eq!(file.solve.refinements, 1);
    assert_eq!(file.regions.len(), 1);
    let mut reloaded = Session::open(&saved).unwrap();
    assert_eq!(solved_bytes(&mut reloaded), picture);
    let resaved = dir.join("resaved.convert64");
    reloaded.save(&resaved, Embed::Keep).unwrap();
    assert_eq!(
        std::fs::read_to_string(&resaved).unwrap(),
        std::fs::read_to_string(&saved).unwrap()
    );

    for (bad, why) in [
        (r#""source": { "layer": 0 }"#, "not below it"),
        (r#""source": { "png_base64": "!!" }"#, "base64"),
        (r#""source": "missing.png""#, "missing.png"),
    ] {
        let t = format!(
            r#"{{ "schema_version": 2, "target": "hires-bitmap", "layers": [{{ "name": "a", {bad} }}] }}"#
        );
        std::fs::write(dir.join("bad.convert64"), t).unwrap();
        let e = Session::open(&dir.join("bad.convert64")).err().unwrap();
        assert!(e.to_string().contains(why), "{e}");
    }
}

#[test]
fn the_cli_saves_projects() {
    let dir = temp("project-cli");
    write_source(&dir.join("photo.png"));
    std::fs::write(dir.join("recipe.json"), RECIPE).unwrap();
    let run = |args: &[&str]| {
        std::process::Command::new(env!("CARGO_BIN_EXE_convert64"))
            .current_dir(&dir)
            .args(args)
            .output()
            .unwrap()
    };
    let r = run(&["recipe", "apply", "recipe.json", "--save", "p.convert64"]);
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let r = run(&["recipe", "apply", "p.convert64", "--save", "q.convert64"]);
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    assert_eq!(
        std::fs::read_to_string(dir.join("p.convert64")).unwrap(),
        std::fs::read_to_string(dir.join("q.convert64")).unwrap()
    );
    let r = run(&[
        "recipe",
        "apply",
        "p.convert64",
        "--save",
        "e.convert64",
        "--embed",
    ]);
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let embedded = std::fs::read_to_string(dir.join("e.convert64")).unwrap();
    assert!(embedded.contains("png_base64"));
    let r = run(&[
        "recipe",
        "apply",
        "e.convert64",
        "--save",
        "l.convert64",
        "--link",
    ]);
    assert_eq!(
        r.status.code(),
        Some(2),
        "an embedded source has no file to link"
    );
    let r = run(&[
        "recipe",
        "apply",
        "p.convert64",
        "--save",
        "no/such/dir/x.convert64",
    ]);
    assert_eq!(r.status.code(), Some(3), "write errors exit 3");
}

/// #236: a version-2 project with the old default `"supersample": 4` is migrated to 1×
/// on load and the CLI says so once: a `note:` line on stderr, or `notes` in the
/// `--json` report. The saved project is version 3, and it has no note.
#[test]
fn the_cli_reports_a_version_2_project_s_migration_once() {
    const NOTE: &str = "Updated project from v2: now uses 1× resampling";
    let dir = temp("project-migration");
    write_source(&dir.join("photo.png"));
    let v2 = r#"{
      "schema_version": 2,
      "target": "hires-bitmap",
      "supersample": 4,
      "layers": [
        { "name": "photo", "source": { "path": "photo.png" },
          "transform": { "scale": { "x": 5.0, "y": 5.0 } } }
      ]
    }"#;
    std::fs::write(dir.join("old.convert64"), v2).unwrap();
    let run = |args: &[&str]| {
        std::process::Command::new(env!("CARGO_BIN_EXE_convert64"))
            .current_dir(&dir)
            .args(args)
            .output()
            .unwrap()
    };
    let json = |r: &std::process::Output| -> serde_json::Value {
        assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
        serde_json::from_slice(&r.stdout).unwrap()
    };

    let r = run(&[
        "recipe",
        "apply",
        "old.convert64",
        "--save",
        "new.convert64",
    ]);
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let stderr = String::from_utf8_lossy(&r.stderr);
    assert_eq!(stderr.matches(NOTE).count(), 1, "{stderr}");
    assert!(stderr.contains(&format!("note: {NOTE}")), "{stderr}");
    let saved = ProjectFile::read(&dir.join("new.convert64")).unwrap();
    assert_eq!(saved.schema_version, 3);
    assert_eq!(saved.supersample, 1);

    let v = json(&run(&["--json", "recipe", "apply", "old.convert64"]));
    assert_eq!(v["notes"], serde_json::json!([NOTE]));
    for args in [
        &["--json", "apply", "old.convert64"][..],
        &["--json", "convert", "old.convert64", "-o", "old.art"],
        &["--json", "validate", "old.convert64"],
    ] {
        let v = json(&run(args));
        assert_eq!(v["notes"], serde_json::json!([NOTE]), "{args:?}");
    }

    // The migrated, saved project is current: no note, in either form.
    let r = run(&["recipe", "apply", "new.convert64"]);
    assert!(r.status.success());
    assert!(!String::from_utf8_lossy(&r.stderr).contains("note:"));
    let v = json(&run(&["--json", "recipe", "apply", "new.convert64"]));
    assert_eq!(v["notes"], serde_json::json!([]));
}
