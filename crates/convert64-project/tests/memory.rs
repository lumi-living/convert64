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
