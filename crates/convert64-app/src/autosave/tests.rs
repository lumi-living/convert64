//! The autosave against the in-memory storage (and natively a folder): one save per
//! committed change, sources stored once, the last session and one previous kept,
//! unreferenced blobs collected, a full storage, restore detection and round trips.

use super::*;
use convert64_project::codec::encode_rgba8;
use convert64_project::{JsonCommand, LayerRef};

fn png(seed: u8) -> Vec<u8> {
    let rgba = (0..64 * 40)
        .flat_map(|i| [(i % 64 * 4) as u8 ^ seed, (i / 64 * 6) as u8, seed, 255])
        .collect();
    encode_rgba8(64, 40, rgba)
}

fn doc(seed: u8) -> Document {
    Document::from_png(&format!("pic{seed}.png"), &png(seed)).unwrap()
}

fn opacity(o: f32) -> JsonCommand {
    JsonCommand::LayerSetOpacity {
        layer: LayerRef::Index(0),
        opacity: o,
    }
}

/// An autosave on `storage`, its contents read.
fn autosave(storage: &MemoryStorage) -> Autosave {
    let mut a = Autosave::new(Box::new(storage.clone()));
    assert!(a.poll().is_empty());
    a
}

/// Frames every 50 ms from `from` to `to` seconds, settled; returns the time reached.
fn run(a: &mut Autosave, d: &Document, from: f64, to: f64) -> f64 {
    let mut t = from;
    while t < to {
        a.frame(d, true, t, 1e12 + t * 1000.0);
        a.poll();
        t += 0.05;
    }
    t
}

fn blob_writes(s: &MemoryStorage) -> usize {
    s.names().iter().filter(|n| n.starts_with("blob-")).count()
}

#[test]
fn content_keys_tell_bytes_apart() {
    let a = content_key(&png(1));
    assert_eq!(a, content_key(&png(1)));
    assert_ne!(a, content_key(&png(2)));
    assert_ne!(content_key(b"abcdefgh1"), content_key(b"abcdefgh2"));
    assert_ne!(content_key(b""), content_key(b"\0"));
}

#[test]
fn a_drag_saves_once_on_release() {
    let s = MemoryStorage::new();
    let mut a = autosave(&s);
    let mut d = doc(1);
    a.opened(&d, None, 1e12);
    // Thirty drag frames, the pointer held: nothing is committed yet.
    let mut t = 0.0;
    for i in 0..30 {
        d.run_in_gesture(7, opacity(1.0 - i as f32 / 60.0)).unwrap();
        a.frame(&d, false, t, 1e12);
        a.poll();
        t += 1.0 / 60.0;
    }
    assert_eq!(s.batches(), 0, "nothing during the drag");
    d.end_gesture();
    // Released: one save, about a second later.
    let due = a.frame(&d, true, t, 1e12).unwrap();
    assert!((due - DEBOUNCE_S).abs() < 1e-9, "{due}");
    run(&mut a, &d, t, t + 0.9);
    assert_eq!(s.batches(), 0, "debounced");
    run(&mut a, &d, t + 0.9, t + 5.0);
    assert_eq!(s.batches(), 1, "one save for the whole drag");
}

#[test]
fn every_discrete_command_counts_and_bursts_coalesce() {
    let s = MemoryStorage::new();
    let mut a = autosave(&s);
    let mut d = doc(1);
    a.opened(&d, None, 1e12);
    // Nothing changed: nothing saved.
    run(&mut a, &d, 0.0, 3.0);
    assert_eq!(s.batches(), 0);
    // A command every 0.5 s for 8 s: the debounce never runs out, the cap does.
    let mut t = 3.0;
    let mut saves_at = Vec::new();
    for i in 0..16 {
        d.run(opacity(0.5 + i as f32 / 100.0)).unwrap();
        let before = s.batches();
        t = run(&mut a, &d, t, t + 0.5);
        if s.batches() > before {
            saves_at.push(t);
        }
    }
    assert!(
        !saves_at.is_empty() && saves_at[0] - 3.0 <= MAX_WAIT_S + 0.55,
        "{saves_at:?}"
    );
    assert!(saves_at.len() <= 2, "16 commands in 8 s: {saves_at:?}");
    run(&mut a, &d, t, t + 2.0);
    let rec: Record =
        serde_json::from_slice(&s.get(&storage::record_name(a.records()[0].id)).unwrap()).unwrap();
    let back = restore(&rec, vec![png(1)]).unwrap();
    assert_eq!(
        back.session().project(),
        d.session().project(),
        "the last change is in"
    );
}

#[test]
fn sources_are_written_once_by_content() {
    let s = MemoryStorage::new();
    let mut a = autosave(&s);
    let mut d = doc(3);
    a.opened(&d, None, 1e12);
    let mut t = 0.0;
    for o in [0.9, 0.8, 0.7] {
        d.run(opacity(o)).unwrap();
        t = run(&mut a, &d, t, t + 2.0);
    }
    assert_eq!(s.batches(), 3);
    // One blob, three record writes.
    assert_eq!(blob_writes(&s), 1);
    assert_eq!(s.writes(), 1 + 3);
    // The record links the source by its key and holds no pixels.
    let r = &a.records()[0];
    assert_eq!(r.sources, vec![content_key(&png(3))]);
    assert!(r.project.contains("blobs/"), "{}", r.project);
    assert!(!r.project.contains("png_base64"));
    // The same picture in another layer is the same blob.
    d.add_layer("again.png", &png(3), None).unwrap();
    run(&mut a, &d, t, t + 2.0);
    assert_eq!(blob_writes(&s), 1);
    assert_eq!(a.records()[0].layers, 2);
}

#[test]
fn the_last_session_and_one_previous_are_kept_and_blobs_collected() {
    let s = MemoryStorage::new();
    let mut a = autosave(&s);
    let mut t = 0.0;
    for seed in 1..=3 {
        let mut d = doc(seed);
        a.opened(&d, None, 1e12 + f64::from(seed) * 1000.0);
        d.run(opacity(0.5)).unwrap();
        t = run(&mut a, &d, t, t + 2.0);
    }
    let names = s.names();
    let records: Vec<_> = names.iter().filter(|n| n.starts_with("session-")).collect();
    assert_eq!(records.len(), 2, "{names:?}");
    assert_eq!(
        blob_writes(&s),
        2,
        "the first session's source went with it"
    );
    assert!(s.get(&storage::blob_name(&content_key(&png(1)))).is_none());
    assert!(s.get(&storage::blob_name(&content_key(&png(3)))).is_some());
    // A blob nothing links (left over from an interrupted batch) goes too.
    s.insert("blob-orphan", vec![1, 2, 3]);
    let mut a = autosave(&s);
    let mut d = doc(3);
    a.opened(&d, None, 2e12);
    d.run(opacity(0.4)).unwrap();
    run(&mut a, &d, 0.0, 2.0);
    assert!(s.get("blob-orphan").is_none());
}

#[test]
fn a_full_storage_warns_once_and_does_not_retry_in_a_loop() {
    let s = MemoryStorage::new();
    s.set_quota(Some(1000));
    let mut a = autosave(&s);
    let mut d = doc(1);
    a.opened(&d, None, 1e12);
    d.run(opacity(0.5)).unwrap();
    let t = run(&mut a, &d, 0.0, 10.0);
    assert_eq!(s.batches(), 1, "one attempt, however long we wait");
    let w = a.warning().unwrap();
    assert!(w.contains("storage is full"), "{w}");
    assert!(a.records().is_empty());
    // The next committed change tries again; with room it succeeds and the warning goes.
    s.set_quota(None);
    d.run(opacity(0.6)).unwrap();
    run(&mut a, &d, t, t + 2.0);
    assert_eq!(s.batches(), 2);
    assert!(a.warning().is_none());
    assert_eq!(a.records().len(), 1);
}

#[test]
fn unsaved_work_is_offered_and_saved_work_is_not() {
    let s = MemoryStorage::new();
    let mut a = autosave(&s);
    let mut d = doc(1);
    a.opened(&d, None, 1e12);
    d.run(opacity(0.5)).unwrap();
    let t = run(&mut a, &d, 0.0, 2.0);
    // A reload: the session is offered.
    let b = autosave(&s);
    let offer = b.offer().expect("unsaved work is offered");
    assert_eq!(offer.layers, 1);
    assert_eq!(offer.name, "pic1.png");
    assert!(offer.differs_from_saved());
    // Saved: the record says so and nothing is offered.
    let key = a.snapshot_key(&d);
    d.mark_saved();
    a.saved(&d, key, 1e12 + 5000.0);
    a.poll();
    assert!(autosave(&s).offer().is_none());
    // Changed and changed back: the same as saved, not offered.
    d.run(opacity(0.7)).unwrap();
    let t = run(&mut a, &d, t, t + 2.0);
    assert!(autosave(&s).offer().is_some());
    assert!(d.undo());
    run(&mut a, &d, t, t + 2.0);
    assert!(
        autosave(&s).offer().is_none(),
        "undone back to the saved state"
    );
}

#[test]
fn an_unchanged_picture_is_not_offered_after_an_undo() {
    let s = MemoryStorage::new();
    let mut a = autosave(&s);
    let mut d = doc(2);
    a.opened(&d, None, 1e12);
    d.run(opacity(0.5)).unwrap();
    let t = run(&mut a, &d, 0.0, 2.0);
    assert!(autosave(&s).offer().is_some());
    d.undo();
    run(&mut a, &d, t, t + 2.0);
    assert!(autosave(&s).offer().is_none());
}

#[test]
fn restore_opens_the_session_as_unsaved_work_and_keeps_its_id() {
    let s = MemoryStorage::new();
    let mut a = autosave(&s);
    let mut d = doc(4);
    d.add_layer("logo.png", &png(5), None).unwrap();
    a.opened(&d, None, 1e12);
    d.run(opacity(0.25)).unwrap();
    run(&mut a, &d, 0.0, 2.0);
    let id = a.records()[0].id;

    let mut b = autosave(&s);
    assert_eq!(b.offer().unwrap().layers, 2);
    b.restore_offer();
    let mut notices = b.poll();
    let Some(Notice::Restored(back, record)) = notices.pop() else {
        panic!("restored");
    };
    assert!(b.offer().is_none());
    assert!(back.is_dirty(), "restored work is unsaved");
    assert_eq!(back.session().project(), d.session().project());
    assert_eq!(back.name, "pic4.png");
    // It goes on in the same record.
    let mut back = *back;
    b.opened(&back, Some(&record), 2e12);
    back.run(opacity(0.3)).unwrap();
    run(&mut b, &back, 0.0, 2.0);
    let ids: Vec<u64> = b.records().iter().map(|r| r.id).collect();
    assert_eq!(ids, vec![id]);
}

#[test]
fn discard_removes_the_offered_session() {
    let s = MemoryStorage::new();
    let mut a = autosave(&s);
    let mut d = doc(1);
    a.opened(&d, None, 1e12);
    d.run(opacity(0.5)).unwrap();
    run(&mut a, &d, 0.0, 2.0);
    let mut b = autosave(&s);
    assert!(b.offer().is_some());
    b.discard_offer();
    b.poll();
    assert!(b.offer().is_none());
    assert!(s.names().is_empty(), "{:?}", s.names());
    assert!(autosave(&s).offer().is_none());
}

#[test]
fn quitting_without_saving_drops_the_session() {
    let s = MemoryStorage::new();
    let mut a = autosave(&s);
    let mut d = doc(1);
    a.opened(&d, None, 1e12);
    d.run(opacity(0.5)).unwrap();
    run(&mut a, &d, 0.0, 2.0);
    a.discard_current();
    a.poll();
    assert!(s.names().is_empty());
}

#[test]
fn the_offered_session_survives_other_sessions_until_answered() {
    let s = MemoryStorage::new();
    let mut a = autosave(&s);
    let mut d = doc(1);
    a.opened(&d, None, 1e12);
    d.run(opacity(0.5)).unwrap();
    run(&mut a, &d, 0.0, 2.0);
    let offered = a.records()[0].id;
    // Two more documents edited while the bar is up.
    let mut b = autosave(&s);
    let mut t = 0.0;
    for seed in 2..=3 {
        let mut d = doc(seed);
        b.opened(&d, None, 1e12 + f64::from(seed) * 1000.0);
        d.run(opacity(0.5)).unwrap();
        t = run(&mut b, &d, t, t + 2.0);
    }
    assert_eq!(b.offer().map(|r| r.id), Some(offered));
    assert!(b.records().iter().any(|r| r.id == offered));
}

#[test]
fn the_restore_bar_says_what_it_offers() {
    let r = Record {
        format: FORMAT,
        id: 1,
        name: "title.convert64".into(),
        path: None,
        layers: 3,
        modified_ms: 0.0,
        project_key: "a".into(),
        saved_key: None,
        sources: Vec::new(),
        linked: BTreeMap::new(),
        project: String::new(),
    };
    assert_eq!(
        r.offer_text(125_000.0),
        "Restore your last session (title.convert64, 3 layers, 2 min ago)?"
    );
    assert_eq!(age(10_000.0), "just now");
    assert_eq!(age(3.5 * 3_600_000.0), "3 h ago");
    assert_eq!(age(86_400_000.0), "1 day ago");
}

#[test]
fn layers_sharing_pixels_round_trip() {
    let s = MemoryStorage::new();
    let mut a = autosave(&s);
    let mut d = doc(6);
    d.run(JsonCommand::LayerDuplicate {
        layer: LayerRef::Index(0),
    })
    .unwrap();
    a.opened(&d, None, 1e12);
    d.run(opacity(0.5)).unwrap();
    run(&mut a, &d, 0.0, 2.0);
    let r = a.records()[0].clone();
    assert_eq!(r.layers, 2);
    let blobs = r
        .sources
        .iter()
        .map(|k| s.get(&storage::blob_name(k)).unwrap().to_vec())
        .collect();
    let back = restore(&r, blobs).unwrap();
    assert_eq!(back.session().project(), d.session().project());
}

/// Natively the recovery files are a folder; a source linked from a file is linked
/// again on restore if the file still holds it, embedded otherwise.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_native_recovery_folder_round_trips() {
    let root = std::env::temp_dir().join(format!("convert64-autosave-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let picture = root.join("art.png");
    std::fs::write(&picture, png(7)).unwrap();
    let storage = DirStorage::new(root.join("autosave"));

    let mut a = Autosave::new(Box::new(storage.clone()));
    a.poll();
    let mut d = Document::open("art.png", &png(7), Some(&picture)).unwrap();
    a.opened(&d, None, 1e12);
    d.run(opacity(0.5)).unwrap();
    run(&mut a, &d, 0.0, 2.0);
    let files: Vec<_> = std::fs::read_dir(storage.dir())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(files.len(), 2, "{files:?}");
    assert!(!files.iter().any(|f| f.ends_with(".tmp")));

    let restore_now = || {
        let mut b = Autosave::new(Box::new(storage.clone()));
        b.poll();
        b.restore_offer();
        match b.poll().pop() {
            Some(Notice::Restored(doc, _)) => *doc,
            other => panic!("{other:?}"),
        }
    };
    let back = restore_now();
    assert!(back.is_dirty());
    assert_eq!(back.session().project(), d.session().project());
    let json = back
        .project_json(&root, convert64_project::Embed::Keep)
        .unwrap();
    assert!(json.contains("\"art.png\""), "linked again: {json}");

    // The picture changed on disk since: the autosaved pixels are embedded instead.
    std::fs::write(&picture, png(8)).unwrap();
    let back = restore_now();
    assert_eq!(back.session().project(), d.session().project());
    let json = back
        .project_json(&root, convert64_project::Embed::Keep)
        .unwrap();
    assert!(json.contains("png_base64"), "{json}");
    let _ = std::fs::remove_dir_all(&root);
}
