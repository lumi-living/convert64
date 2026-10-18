//! Projects without a file system (#192): the browser build's path. Sources, masks and
//! the project file itself live in a `MemoryStore`; a saved project reloads identically
//! and solves to the same picture, as on disk (`convert64-cli/tests/project.rs`).

use convert64_project::codec::{encode_luma16, encode_rgba8};
use convert64_project::file::SourceSpec;
use convert64_project::{Embed, MemoryStore, ProjectFile, Session, Store};
use std::path::Path;
use std::sync::Arc;

/// A 64 × 40 picture: green on the left, red on the right, with a vertical ramp.
fn source_png() -> Vec<u8> {
    let rgba = (0..40u32)
        .flat_map(|y| {
            (0..64u32).flat_map(move |x| {
                let v = (y * 6) as u8;
                if x < 32 {
                    [v / 4, 200, v / 3, 255]
                } else {
                    [210, v / 2, 40, 255]
                }
            })
        })
        .collect();
    encode_rgba8(64, 40, rgba)
}

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
    { "command": "mask_load", "layer": "photo", "path": "masks/ramp.png" },
    { "command": "mask_set_modifier", "layer": 0, "modifier": { "feather": 2.0 } },
    { "command": "layer_via_mask", "source_layer": 0, "cut": false,
      "selection": { "polygons": [[{ "x": 0, "y": 0 }, { "x": 32, "y": 0 }, { "x": 32, "y": 40 }, { "x": 0, "y": 40 }]] } },
    { "command": "cell_pin", "cells": [{ "x": 3, "y": 4 }], "colours": [0, 1, 2, 5], "full": true }
  ],
  "solve": { "background": { "fixed": 0 } }
}"#;

fn store() -> Arc<MemoryStore> {
    let s = Arc::new(MemoryStore::new());
    s.insert("proj/photo.png", source_png());
    let ramp = (0..40u32)
        .flat_map(|_| (0..64u32).map(|x| (x * 1000) as u16))
        .collect();
    s.insert("proj/masks/ramp.png", encode_luma16(64, 40, ramp));
    s.insert("proj/recipe.json", RECIPE);
    s
}

fn open(store: &Arc<MemoryStore>, path: &str) -> Session {
    Session::open_in(Box::new(Arc::clone(store)), Path::new(path)).unwrap()
}

#[test]
fn a_project_round_trips_through_memory() {
    let store = store();
    let mut s = open(&store, "proj/recipe.json");
    assert_eq!(s.project().layers.len(), 2);
    let picture = s.solve(true).image.clone();

    s.save_to_store(Path::new("proj/a.convert64"), Embed::Keep)
        .unwrap();
    let text_a = String::from_utf8(store.get("proj/a.convert64").unwrap()).unwrap();
    let file = ProjectFile::parse(&text_a).unwrap();
    assert!(
        matches!(&file.layers[0].source, SourceSpec::Linked(l) if l.path == Path::new("photo.png"))
    );
    assert!(matches!(&file.layers[1].source, SourceSpec::Shared(s) if s.layer == 0));

    // load → save → load is the identity, and the solve is the same.
    let mut a = open(&store, "proj/a.convert64");
    a.save_to_store(Path::new("proj/b.convert64"), Embed::Keep)
        .unwrap();
    assert_eq!(store.get("proj/b.convert64").unwrap(), text_a.as_bytes());
    assert_eq!(a.solve(true).image, picture);

    // Saved one directory down, the linked path walks up.
    a.save_to_store(Path::new("proj/art/c.convert64"), Embed::Keep)
        .unwrap();
    let c = ProjectFile::read_from(store.as_ref(), Path::new("proj/art/c.convert64")).unwrap();
    assert!(
        matches!(&c.layers[0].source, SourceSpec::Linked(l) if l.path == Path::new("../photo.png"))
    );

    // Embedded, the project stands alone in a store that has nothing else.
    a.save_to_store(Path::new("proj/e.convert64"), Embed::All)
        .unwrap();
    let alone = Arc::new(MemoryStore::new());
    alone.insert("e.convert64", store.get("proj/e.convert64").unwrap());
    let mut e = open(&alone, "e.convert64");
    assert_eq!(e.solve(true).image, picture);
    assert!(
        e.save_to_store(Path::new("l.convert64"), Embed::None)
            .is_err()
    );
}

/// #236: a version-2 project saved with the old default `"supersample": 4` loads on the
/// 1× grid of ADR-012, says so once, and saves as version 3, which reloads without a
/// note and solves to the same picture. A version-3 file keeps the factor it states.
#[test]
fn a_version_2_project_migrates_to_1x_and_saves_as_version_3() {
    let v2 = r#"{
      "schema_version": 2,
      "target": "multicolor-bitmap",
      "supersample": 4,
      "layers": [
        { "name": "photo", "source": { "path": "photo.png" },
          "transform": { "translate": { "x": 0.25, "y": 0.0 }, "scale": { "x": 5.0, "y": 5.0 } },
          "mask": { "raster": { "fill": 0.75 } } }
      ],
      "solve": { "background": { "fixed": 0 } }
    }"#;
    let store = store();
    store.insert("proj/old.convert64", v2);
    let mut old = open(&store, "proj/old.convert64");
    assert_eq!(old.grid().n(), 1, "composites at 1×");
    assert_eq!(
        old.notes(),
        ["Updated project from v2: now uses 1× resampling"]
    );
    let composite = old.composite_rgba();
    let picture = old.solve(true).image.clone();

    old.save_to_store(Path::new("proj/new.convert64"), Embed::Keep)
        .unwrap();
    let text = String::from_utf8(store.get("proj/new.convert64").unwrap()).unwrap();
    assert!(text.contains(r#""schema_version": 3,"#), "{text}");
    assert!(text.contains(r#""supersample": 1,"#), "{text}");
    let mut new = open(&store, "proj/new.convert64");
    assert!(new.notes().is_empty(), "reported once");
    assert_eq!(new.grid().n(), 1);
    assert_eq!(new.composite_rgba(), composite);
    assert_eq!(new.solve(true).image, picture);
    new.save_to_store(Path::new("proj/again.convert64"), Embed::Keep)
        .unwrap();
    assert_eq!(store.get("proj/again.convert64").unwrap(), text.as_bytes());

    // The same document as version 3 keeps its 4×, the softer composite.
    store.insert(
        "proj/kept.convert64",
        v2.replace(r#""schema_version": 2"#, r#""schema_version": 3"#),
    );
    let mut kept = open(&store, "proj/kept.convert64");
    assert!(kept.notes().is_empty());
    assert_eq!(kept.grid().n(), 4);
    assert_ne!(kept.composite_rgba(), composite);
}

#[test]
fn missing_files_name_themselves() {
    let store = store();
    let bare = Arc::new(MemoryStore::new());
    bare.insert("recipe.json", RECIPE);
    let e = Session::open_in(Box::new(bare), Path::new("recipe.json")).unwrap_err();
    assert!(e.to_string().contains("photo.png"), "{e}");
    assert!(
        Session::open_in(Box::new(store), Path::new("nope.convert64"))
            .unwrap_err()
            .to_string()
            .contains("nope.convert64")
    );
    let s: &dyn Store = &MemoryStore::new();
    assert!(s.read(Path::new("x")).is_err());
}

/// Document-setting commands (#193) reach the compositor and the solver: a new mode is
/// a new grid, a new palette a new solver, and undo brings both back. Palettes save as
/// they were given.
#[test]
fn settings_commands_rebuild_what_they_change_and_save_as_given() {
    use convert64_core::solve::Background;
    use convert64_core::{ColourIndex, Mode};
    use convert64_project::PaletteSpec;

    let store = store();
    let mut s = open(&store, "proj/recipe.json");
    let run = |s: &mut Session, json: &str| {
        s.run(serde_json::from_str(json).unwrap()).unwrap();
    };
    let multicolor = s.solve(true).image.clone();
    run(&mut s, r#"{"command":"mode_set","mode":"hires-bitmap"}"#);
    assert_eq!(s.target(), Mode::HiresBitmap);
    assert_eq!(s.solve(false).image.mode(), Mode::HiresBitmap);
    s.editor_mut().undo().unwrap();
    assert_eq!(s.solve(false).image, multicolor);

    run(&mut s, r#"{"command":"palette_set","palette":"colodore"}"#);
    assert_eq!(s.palette().name(), "Colodore");
    let colours: Vec<String> = (0..16u8)
        .map(|i| format!("#{:02x}{:02x}{:02x}", i * 16, i, 255 - i))
        .collect();
    let custom = serde_json::json!({ "command": "palette_set", "palette": { "custom": colours } });
    run(&mut s, &custom.to_string());
    run(
        &mut s,
        r#"{"command":"background_pin","background":{"fixed":6}}"#,
    );
    run(
        &mut s,
        r#"{"command":"solve_set","refinements":1,"hysteresis":0.97}"#,
    );
    assert_eq!(s.solve(false).background, Some(ColourIndex::BLUE));

    s.save_to_store(Path::new("proj/settings.convert64"), Embed::Keep)
        .unwrap();
    let file =
        ProjectFile::read_from(store.as_ref(), Path::new("proj/settings.convert64")).unwrap();
    assert_eq!(file.palette, PaletteSpec::Custom(colours), "as given");
    assert_eq!(file.solve.background, Background::Fixed(ColourIndex::BLUE));
    assert_eq!(file.solve.refinements, 1);
    assert_eq!(file.solve.hysteresis, Some(0.97));

    // Undoing the custom palette and what followed saves the preset's name again.
    for _ in 0..3 {
        s.editor_mut().undo().unwrap();
    }
    s.save_to_store(Path::new("proj/undone.convert64"), Embed::Keep)
        .unwrap();
    let undone =
        ProjectFile::read_from(store.as_ref(), Path::new("proj/undone.convert64")).unwrap();
    assert_eq!(undone.palette, PaletteSpec::Colodore);
    assert_eq!(
        undone.solve.background,
        Background::Fixed(ColourIndex::BLACK)
    );
    assert_eq!(undone.solve.refinements, 2);
    assert_ne!(
        s.solve(false).background,
        Some(ColourIndex::BLUE),
        "the solve follows the undo"
    );
}

/// Regions made with the region commands (#216) are saved and reload as they were: the
/// first save rounds a painted mask to 1/65535, and from then on load → save is the
/// identity, solve included. Undo takes the regions away again.
#[test]
fn regions_made_by_commands_round_trip() {
    let store = store();
    let mut s = open(&store, "proj/recipe.json");
    let base = s.project().clone();
    let commands = [
        r#"{"command":"region_add","name":"sky","recipe":{"method":{"kind":"none"},"strength":1.0}}"#,
        r#"{"command":"region_add","name":"face","index":0}"#,
        r#"{"command":"region_stroke","region":"sky","points":[{"x":10,"y":10},{"x":200,"y":90}],"radius":14,"hardness":0.3,"flow":0.6}"#,
        r#"{"command":"region_fill","region":0,"polygon":[{"x":150,"y":100},{"x":300,"y":100},{"x":220,"y":190}]}"#,
        r#"{"command":"region_gradient","region":"face","combine":"multiply","gradient":{"kind":"radial","from":{"x":220,"y":140},"to":{"x":290,"y":140},"softness":0.5}}"#,
        r#"{"command":"region_set_recipe","region":"face","recipe":{"strength":0.4,"deny":[7]}}"#,
        r#"{"command":"region_reorder","region":"face","to":1}"#,
        r#"{"command":"region_rename","region":1,"name":"portrait"}"#,
    ];
    for c in commands {
        s.run(serde_json::from_str(c).unwrap())
            .unwrap_or_else(|e| panic!("{c}: {e}"));
    }
    let names: Vec<&str> = s.project().regions.iter().map(|r| r.name()).collect();
    assert_eq!(names, ["sky", "portrait"]);
    assert!(s.project().regions[1].recipe().strength() == 0.4);

    s.save_to_store(Path::new("proj/r.convert64"), Embed::Keep)
        .unwrap();
    let text = String::from_utf8(store.get("proj/r.convert64").unwrap()).unwrap();
    let mut loaded = open(&store, "proj/r.convert64");
    assert_eq!(loaded.project().regions.len(), 2);
    for (a, b) in s.project().regions.iter().zip(&loaded.project().regions) {
        assert_eq!(a.name(), b.name());
        assert_eq!(a.recipe(), b.recipe());
        for (x, y) in a.mask().values().iter().zip(b.mask().values()) {
            assert!((x - y).abs() <= 0.5 / 65535.0, "{x} vs {y}");
        }
    }
    let picture = loaded.solve(true).image.clone();
    loaded
        .save_to_store(Path::new("proj/r2.convert64"), Embed::Keep)
        .unwrap();
    assert_eq!(store.get("proj/r2.convert64").unwrap(), text.as_bytes());
    let mut again = open(&store, "proj/r2.convert64");
    assert_eq!(again.solve(true).image, picture);

    for _ in commands {
        s.editor_mut().undo().unwrap();
    }
    assert_eq!(s.project().regions, base.regions);
}

/// The app's autosave (#241): every layer's own source is handed out once, linked by the
/// path the caller picks; shared pixels stay shared; a `.vpl` palette becomes its
/// colours. Reloaded from the handed-out bytes alone, it is the same project.
#[test]
fn a_recovery_file_links_each_source_once_and_reloads() {
    let store = store();
    let vpl: String = (0..16)
        .map(|i| format!("{:x} {:x} {:x}\n", i * 16, 255 - i * 16, i * 7))
        .collect();
    store.insert("proj/c64.vpl", vpl);
    let mut session = open(&store, "proj/recipe.json");
    session
        .run_at(
            convert64_project::JsonCommand::PaletteSet {
                palette: convert64_project::PaletteSpec::Vpl("c64.vpl".into()),
            },
            Path::new("proj"),
        )
        .unwrap();
    let mut handed = Vec::new();
    let file = session
        .to_recovery_file(&mut |src| {
            let bytes = src
                .bytes
                .map(|b| b.to_vec())
                .unwrap_or_else(|| convert64_project::codec::encode_source(src.image));
            let path = format!("blobs/{}", handed.len());
            handed.push((path.clone(), bytes, src.linked.map(Path::to_owned)));
            Ok(path.into())
        })
        .unwrap();
    // The photo is linked from its file; layer_via_mask's copy shares its pixels.
    assert_eq!(handed.len(), 1, "one own source");
    assert!(
        handed[0]
            .2
            .as_deref()
            .is_some_and(|p| p.ends_with("photo.png"))
    );
    assert!(matches!(file.layers[1].source, SourceSpec::Shared(_)));
    assert!(matches!(
        file.palette,
        convert64_project::PaletteSpec::Custom(ref c) if c.len() == 16
    ));

    let fresh = Arc::new(MemoryStore::new());
    for (path, bytes, _) in handed {
        fresh.insert(path, bytes);
    }
    let back = Session::load(file, Path::new(""), Box::new(Arc::clone(&fresh))).unwrap();
    // Masks round to 1/65535 on their first save, so compare saved forms.
    let saved = |s: &Session| s.to_project_file(Path::new(""), Embed::All).unwrap();
    let (a, b) = (saved(&back), saved(&session));
    assert_eq!(a.layers, b.layers);
    assert_eq!(a.steering, b.steering);
    assert_eq!(a.regions, b.regions);
    assert_eq!(
        back.project().palette.rgb_table(),
        session.project().palette.rgb_table()
    );
}
