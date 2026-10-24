//! Autosave and restore (#241, usability finding C3): the open project is saved after
//! every committed change, so a reload, a closed tab or a crash loses at most the last
//! second of work, and the next start offers it back.
//!
//! - **When.** A change counts once it is committed: one discrete command, or a drag on
//!   its release (never per frame). Saves are debounced: [`DEBOUNCE_S`] after the last
//!   committed change, at most [`MAX_WAIT_S`] after the first one not yet saved.
//! - **What.** A [`Record`] per session: the project file as JSON with every layer's
//!   source linked as `blobs/<key>`, plus each source's bytes once, as a blob keyed by
//!   its content ([`content_key`]), so autosaving a project with multi-MB sources
//!   writes only the JSON after the first time.
//! - **Where.** A [`Storage`]: OPFS in the browser, IndexedDB where OPFS is missing
//!   (`web`), a folder in the platform data directory natively ([`DirStorage`]). The
//!   last session and one previous are kept; blobs no kept record references are
//!   deleted.
//! - **Restore.** On start, the newest record that differs from the last saved state
//!   (its project's key is not the key at the last Save) is offered in a bar; Restore
//!   opens it as unsaved work, Discard deletes it.
//! - **Failure.** A failed write (a full storage: `QuotaExceededError`) leaves a
//!   one-line warning in the status bar; the next committed change tries again, nothing
//!   retries in a loop.

pub mod storage;
#[cfg(target_arch = "wasm32")]
pub mod web;

use crate::document::Document;
use convert64_core::SourceImage;
use convert64_project::{ProjectError, ProjectFile};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
#[cfg(not(target_arch = "wasm32"))]
pub use storage::DirStorage;
pub use storage::{Batch, Contents, Done, MemoryStorage, Storage, StorageError};

/// Seconds after the last committed change that the autosave runs.
pub const DEBOUNCE_S: f64 = 1.0;

/// At most this many seconds after the first unsaved change, however busy the user is.
pub const MAX_WAIT_S: f64 = 5.0;

/// The record format this build writes and reads.
pub const FORMAT: u32 = 1;

/// Where a recovery file links source `key` (inside the record's own namespace).
fn blob_path(key: &str) -> PathBuf {
    PathBuf::from(format!("blobs/{key}"))
}

/// The content key of `bytes`: their length and two independent 64-bit hashes (FNV-1a
/// over the bytes and a multiply–xorshift over 8-byte words), 128 bits in all. No
/// cryptographic strength is needed: it only tells this user's own sources apart.
pub fn content_key(bytes: &[u8]) -> String {
    let mut fnv: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        fnv = (fnv ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
    }
    let mix = |mut h: u64| {
        h ^= h >> 33;
        h = h.wrapping_mul(0xff51_afd7_ed55_8ccd);
        h ^= h >> 33;
        h = h.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
        h ^ (h >> 33)
    };
    let mut w: u64 = 0x9e37_79b9_7f4a_7c15;
    let (words, rest) = bytes.as_chunks::<8>();
    for c in words {
        w = mix(w ^ u64::from_le_bytes(*c)).wrapping_add(0x9e37_79b9_7f4a_7c15);
    }
    let mut tail = [0u8; 8];
    tail[..rest.len()].copy_from_slice(rest);
    w = mix(w ^ u64::from_le_bytes(tail) ^ (bytes.len() as u64).rotate_left(56));
    format!("{:x}-{fnv:016x}{w:016x}", bytes.len())
}

/// One autosaved session: what the restore bar says about it and the project itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Record {
    /// [`FORMAT`].
    pub format: u32,
    /// The session: one per opened document, kept across a restore.
    pub id: u64,
    /// The document's name, as the title showed it.
    pub name: String,
    /// The project's file (native), for Save after a restore.
    #[serde(default)]
    pub path: Option<PathBuf>,
    /// Layers in the project.
    pub layers: usize,
    /// When it was written, ms since the Unix epoch.
    pub modified_ms: f64,
    /// [`content_key`] of `project`.
    pub project_key: String,
    /// `project_key` as it was at the last Save, if the session was ever saved (or
    /// opened from a file and not changed).
    #[serde(default)]
    pub saved_key: Option<String>,
    /// The blobs `project` links, by key.
    pub sources: Vec<String>,
    /// For a source that was linked from a file (native), that file: a restore links it
    /// again if it still holds the same bytes.
    #[serde(default)]
    pub linked: BTreeMap<String, PathBuf>,
    /// The project file, as JSON, its sources linked as `blobs/<key>`.
    pub project: String,
}

impl Record {
    /// Whether it holds changes the last Save does not (what the restore bar offers).
    pub fn differs_from_saved(&self) -> bool {
        self.saved_key.as_deref() != Some(self.project_key.as_str())
    }

    /// The restore bar's question: "Restore your last session (title.convert64,
    /// 3 layers, 2 min ago)?".
    pub fn offer_text(&self, now_ms: f64) -> String {
        let layers = match self.layers {
            1 => "1 layer".to_owned(),
            n => format!("{n} layers"),
        };
        format!(
            "Restore your last session ({}, {layers}, {})?",
            self.name,
            age(now_ms - self.modified_ms)
        )
    }
}

/// `ms` ago in words: "just now", "2 min ago", "3 h ago", "2 days ago".
pub fn age(ms: f64) -> String {
    let s = (ms / 1000.0).max(0.0);
    if s < 60.0 {
        "just now".to_owned()
    } else if s < 3600.0 {
        format!("{} min ago", (s / 60.0).floor())
    } else if s < 86_400.0 {
        format!("{} h ago", (s / 3600.0).floor())
    } else {
        match (s / 86_400.0).floor() as u64 {
            1 => "1 day ago".to_owned(),
            n => format!("{n} days ago"),
        }
    }
}

/// When to save: a committed change starts the clock; more changes push it back up to
/// [`MAX_WAIT_S`] after the first.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Debounce {
    /// The revision last counted as committed.
    seen: u64,
    /// When the first and the last unsaved committed change were seen (seconds).
    first: Option<f64>,
    last: Option<f64>,
}

impl Debounce {
    /// Starts at `revision`, with nothing to save.
    pub fn new(revision: u64) -> Self {
        Self {
            seen: revision,
            first: None,
            last: None,
        }
    }

    /// Notes the document's `revision` at `now` (seconds). While a drag is in progress
    /// (`settled` false) its changes are not committed yet: they count once, on the
    /// first settled frame.
    pub fn observe(&mut self, revision: u64, settled: bool, now: f64) {
        if settled && revision != self.seen {
            self.seen = revision;
            self.first.get_or_insert(now);
            self.last = Some(now);
        }
    }

    /// Seconds until the save is due (0: now); `None` with nothing to save.
    pub fn due_in(&self, now: f64) -> Option<f64> {
        let (first, last) = (self.first?, self.last?);
        let at = (last + DEBOUNCE_S).min(first + MAX_WAIT_S);
        Some((at - now).max(0.0))
    }

    /// Whether the save is due now; if so, the clock stops until the next change.
    pub fn take(&mut self, now: f64) -> bool {
        if self.due_in(now) == Some(0.0) {
            self.first = None;
            self.last = None;
            return true;
        }
        false
    }

    /// Makes a save due now (a change arrived while the last write was running).
    pub fn force(&mut self, now: f64) {
        self.first = Some(now - MAX_WAIT_S);
        self.last = Some(now - MAX_WAIT_S);
    }
}

/// A source's content key and bytes.
pub type Blob = (String, Arc<Vec<u8>>);

/// The session being edited.
#[derive(Debug)]
struct Current {
    id: u64,
    debounce: Debounce,
    saved_key: Option<String>,
}

/// What finished in the storage.
enum Event {
    Loaded(Result<Contents, StorageError>),
    Applied(Result<(), StorageError>, Applied),
    Read(Result<Vec<Vec<u8>>, StorageError>, Box<Record>),
}

/// What the storage holds once a batch is applied.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Applied {
    /// The records kept, the new one among them.
    pub kept: Vec<Record>,
    /// Blob keys written.
    pub written: Vec<String>,
    /// Blob keys deleted.
    pub deleted: Vec<String>,
}

/// What the app has to act on.
#[derive(Debug)]
pub enum Notice {
    /// The offered session, opened: show it, unsaved.
    Restored(Box<Document>, Box<Record>),
    /// It could not be opened: the message names why.
    RestoreFailed(String),
}

/// A source the last snapshot stored: its pixels (kept alive so the pointer stays
/// theirs), key and bytes.
struct Cached {
    image: Arc<SourceImage>,
    key: String,
    bytes: Arc<Vec<u8>>,
    linked: Option<PathBuf>,
}

/// The autosave: the open session's debounce, what the storage holds, the restore
/// offer and the last failure.
#[derive(Default)]
pub struct Autosave {
    storage: Option<Box<dyn Storage>>,
    inbox: Rc<RefCell<Vec<Event>>>,
    loaded: bool,
    records: Vec<Record>,
    blobs: BTreeSet<String>,
    current: Option<Current>,
    in_flight: bool,
    /// A save fell due while a write was running: save again when it finishes.
    again: bool,
    offer: Option<Record>,
    restoring: bool,
    warning: Option<String>,
    cache: Vec<Cached>,
    last_id: u64,
}

impl std::fmt::Debug for Autosave {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Autosave")
            .field("storage", &self.storage.as_ref().map(|s| s.describe()))
            .field("loaded", &self.loaded)
            .field("records", &self.records.len())
            .field("blobs", &self.blobs.len())
            .field("current", &self.current)
            .field("offer", &self.offer.as_ref().map(|r| r.id))
            .field("warning", &self.warning)
            .finish_non_exhaustive()
    }
}

impl Autosave {
    /// The autosave into `storage`; it starts reading what is there at once.
    pub fn new(storage: Box<dyn Storage>) -> Self {
        let this = Self {
            storage: Some(storage),
            ..Self::default()
        };
        let inbox = Rc::clone(&this.inbox);
        if let Some(s) = &this.storage {
            s.load(Box::new(move |r| inbox.borrow_mut().push(Event::Loaded(r))));
        }
        this
    }

    /// Whether it saves anywhere.
    pub fn is_enabled(&self) -> bool {
        self.storage.is_some()
    }

    /// The session the restore bar offers.
    pub fn offer(&self) -> Option<&Record> {
        self.offer.as_ref().filter(|_| !self.restoring)
    }

    /// The last failure, for the status bar.
    pub fn warning(&self) -> Option<&str> {
        self.warning.as_deref()
    }

    /// Hides the warning (until the next failure).
    pub fn dismiss_warning(&mut self) {
        self.warning = None;
    }

    /// Whether a write is running.
    pub fn is_writing(&self) -> bool {
        self.in_flight
    }

    /// The records the storage holds, as far as this session knows.
    pub fn records(&self) -> &[Record] {
        &self.records
    }

    /// Handles what finished in the storage; returns what the app must act on.
    pub fn poll(&mut self) -> Vec<Notice> {
        let events = std::mem::take(&mut *self.inbox.borrow_mut());
        let mut notices = Vec::new();
        for event in events {
            match event {
                Event::Loaded(Ok(contents)) => self.loaded_contents(contents),
                Event::Loaded(Err(e)) => {
                    log(&format!("convert64: autosave is off: {e}"));
                    self.warning = Some(format!("Autosave is off: {e}. Save to keep your work."));
                    self.storage = None;
                }
                Event::Applied(Ok(()), applied) => {
                    self.in_flight = false;
                    self.records = applied.kept;
                    self.blobs.extend(applied.written);
                    for key in &applied.deleted {
                        self.blobs.remove(key);
                    }
                    self.warning = None;
                }
                Event::Applied(Err(e), _) => {
                    self.in_flight = false;
                    // No retry here: the next committed change tries again.
                    self.again = false;
                    log(&format!("convert64: autosave failed: {e}"));
                    self.warning = Some(if e.quota {
                        "Autosave failed: storage is full. Save to keep your work.".to_owned()
                    } else {
                        format!("Autosave failed: {e}. Save to keep your work.")
                    });
                }
                Event::Read(result, record) => {
                    self.restoring = false;
                    let restored = result
                        .map_err(|e| e.to_string())
                        .and_then(|blobs| restore(&record, blobs).map_err(|e| e.to_string()));
                    match restored {
                        Ok(doc) => {
                            self.offer = None;
                            notices.push(Notice::Restored(Box::new(doc), record));
                        }
                        Err(e) => notices.push(Notice::RestoreFailed(e)),
                    }
                }
            }
        }
        notices
    }

    fn loaded_contents(&mut self, contents: Contents) {
        self.loaded = true;
        self.blobs = contents.blobs.into_iter().collect();
        self.records = contents
            .records
            .iter()
            .filter_map(|bytes| match serde_json::from_slice::<Record>(bytes) {
                Ok(r) if r.format == FORMAT => Some(r),
                Ok(r) => {
                    log(&format!(
                        "convert64: autosave: skipping session {} (format {})",
                        r.id, r.format
                    ));
                    None
                }
                Err(e) => {
                    log(&format!("convert64: autosave: skipping a record: {e}"));
                    None
                }
            })
            .collect();
        let current = self.current.as_ref().map(|c| c.id);
        self.offer = newest(&self.records)
            .filter(|r| r.differs_from_saved() && Some(r.id) != current)
            .cloned();
    }

    /// A document was opened: a new session, or (`restored`) the offered one going
    /// on. `wall_ms` is the time (ms since the Unix epoch).
    pub fn opened(&mut self, doc: &Document, restored: Option<&Record>, wall_ms: f64) {
        if self.storage.is_none() {
            return;
        }
        let (id, saved_key) = match restored {
            Some(r) => (r.id, r.saved_key.clone()),
            None => {
                let id = (wall_ms.max(0.0) as u64).max(self.last_id + 1);
                // An unchanged file is its own saved state: undoing back to it leaves
                // nothing to restore.
                let saved = if doc.is_dirty() {
                    None
                } else {
                    self.snapshot_key(doc)
                };
                (id, saved)
            }
        };
        self.last_id = self.last_id.max(id);
        self.current = Some(Current {
            id,
            debounce: Debounce::new(doc.revision()),
            saved_key,
        });
    }

    /// Called every frame with the open document: saves when a committed change is
    /// due. `settled` is false while a drag holds the pointer or a key; `now` is egui's
    /// time (seconds). Returns the seconds until the next save is due, for a repaint.
    pub fn frame(&mut self, doc: &Document, settled: bool, now: f64, wall_ms: f64) -> Option<f64> {
        let cur = self.current.as_mut()?;
        cur.debounce.observe(doc.revision(), settled, now);
        if self.again && !self.in_flight {
            self.again = false;
            cur.debounce.force(now);
        }
        let wait = cur.debounce.due_in(now)?;
        if wait > 0.0 {
            return Some(wait);
        }
        if self.in_flight {
            cur.debounce.take(now);
            self.again = true;
            return None;
        }
        if !self.loaded {
            // Not before the storage is read: the plan needs to know what is there.
            return Some(0.25);
        }
        cur.debounce.take(now);
        self.write(doc, wall_ms);
        None
    }

    /// The key [`Self::saved`] wants for `doc` as it is now (call it when a Save
    /// writes the document).
    pub fn snapshot_key(&mut self, doc: &Document) -> Option<String> {
        self.storage.as_ref()?;
        match self.snapshot(doc, 0.0) {
            Ok((record, _)) => Some(record.project_key),
            Err(e) => {
                log(&format!("convert64: autosave: {e}"));
                None
            }
        }
    }

    /// The document was saved as `key` ([`Self::snapshot_key`] at the time): the
    /// session's record says so, so it is not offered as unsaved work.
    pub fn saved(&mut self, doc: &Document, key: Option<String>, wall_ms: f64) {
        let Some(cur) = self.current.as_mut() else {
            return;
        };
        cur.saved_key = key;
        let id = cur.id;
        if self.records.iter().any(|r| r.id == id) {
            if self.in_flight {
                self.again = true;
            } else if self.loaded {
                self.write(doc, wall_ms);
            }
        }
    }

    /// Writes the open document's record now (a screenshot seeds the restore bar so).
    pub fn write_now(&mut self, doc: &Document, wall_ms: f64) {
        if self.current.is_some() && self.loaded && !self.in_flight {
            self.write(doc, wall_ms);
        }
    }

    fn write(&mut self, doc: &Document, wall_ms: f64) {
        let (record, blobs) = match self.snapshot(doc, wall_ms) {
            Ok(s) => s,
            Err(e) => {
                self.warning = Some(format!("Autosave failed: {e}. Save to keep your work."));
                return;
            }
        };
        let pinned = self.offer.as_ref().map(|r| r.id);
        let (batch, applied) = plan_write(&self.records, &self.blobs, record, blobs, pinned);
        self.apply(batch, applied);
    }

    fn apply(&mut self, batch: Batch, applied: Applied) {
        let Some(storage) = &self.storage else {
            return;
        };
        self.in_flight = true;
        let inbox = Rc::clone(&self.inbox);
        storage.apply(
            batch,
            Box::new(move |r| inbox.borrow_mut().push(Event::Applied(r, applied))),
        );
    }

    /// Quit without saving (native): the session's record goes, so the next start
    /// does not offer what the user chose to drop.
    pub fn discard_current(&mut self) {
        let Some(id) = self.current.take().map(|c| c.id) else {
            return;
        };
        if self.records.iter().any(|r| r.id == id) {
            let (batch, applied) = plan_delete(&self.records, &self.blobs, id);
            self.apply(batch, applied);
        }
    }

    /// Discard on the restore bar: the offered record is deleted.
    pub fn discard_offer(&mut self) {
        let Some(offer) = self.offer.take() else {
            return;
        };
        let (batch, applied) = plan_delete(&self.records, &self.blobs, offer.id);
        if self.in_flight {
            // Rare (a write started before the bar was answered): forget it here; the
            // next write's plan drops it, since it is no longer pinned.
            self.records.retain(|r| r.id != offer.id);
            return;
        }
        self.apply(batch, applied);
    }

    /// Restore on the restore bar: its sources are read, then [`Notice::Restored`].
    pub fn restore_offer(&mut self) {
        let (Some(storage), Some(offer)) = (&self.storage, &self.offer) else {
            return;
        };
        self.restoring = true;
        let names = offer
            .sources
            .iter()
            .map(|k| storage::blob_name(k))
            .collect();
        let record = Box::new(offer.clone());
        let inbox = Rc::clone(&self.inbox);
        storage.read(
            names,
            Box::new(move |r| inbox.borrow_mut().push(Event::Read(r, record))),
        );
    }

    /// `doc` as a record and the blobs it links, each source hashed (or encoded) only
    /// the first time it is seen.
    fn snapshot(
        &mut self,
        doc: &Document,
        wall_ms: f64,
    ) -> Result<(Record, Vec<Blob>), ProjectError> {
        let mut used: Vec<Cached> = Vec::new();
        let cache = &mut self.cache;
        let file = doc.session().to_recovery_file(&mut |src| {
            let same = |c: &Cached| Arc::ptr_eq(&c.image, src.image);
            let key = if let Some(c) = used.iter().find(|c| same(c)) {
                c.key.clone()
            } else if let Some(i) = cache.iter().position(same) {
                let c = cache.swap_remove(i);
                let key = c.key.clone();
                used.push(c);
                key
            } else {
                let bytes = src.bytes.cloned().unwrap_or_else(|| {
                    Arc::new(convert64_project::codec::encode_source(src.image))
                });
                let key = content_key(&bytes);
                used.push(Cached {
                    image: Arc::clone(src.image),
                    key: key.clone(),
                    bytes,
                    linked: src.linked.map(std::path::Path::to_owned),
                });
                key
            };
            Ok(blob_path(&key))
        })?;
        // Sources no longer in the project are forgotten (an undo brings them back,
        // hashed again).
        *cache = used;
        let project = file.to_json();
        let mut sources: Vec<String> = Vec::new();
        let mut blobs = Vec::new();
        let mut linked = BTreeMap::new();
        for c in cache.iter() {
            if !sources.contains(&c.key) {
                sources.push(c.key.clone());
                blobs.push((c.key.clone(), Arc::clone(&c.bytes)));
                if let Some(p) = &c.linked {
                    linked.insert(c.key.clone(), p.clone());
                }
            }
        }
        let cur = self.current.as_ref();
        Ok((
            Record {
                format: FORMAT,
                id: cur.map_or(0, |c| c.id),
                name: doc.name.clone(),
                path: doc.path().map(std::path::Path::to_owned),
                layers: doc.session().project().layers.len(),
                modified_ms: wall_ms,
                project_key: content_key(project.as_bytes()),
                saved_key: cur.and_then(|c| c.saved_key.clone()),
                sources,
                linked,
                project,
            },
            blobs,
        ))
    }
}

/// The newest record.
fn newest(records: &[Record]) -> Option<&Record> {
    records
        .iter()
        .max_by(|a, b| a.modified_ms.total_cmp(&b.modified_ms))
}

/// The batch that writes `new` (replacing its session's record) with the blobs it
/// links that the storage lacks, keeps it, the newest other record and `pinned` (the
/// one the restore bar offers), and deletes every other record and every blob no kept
/// record links.
pub fn plan_write(
    records: &[Record],
    blobs: &BTreeSet<String>,
    new: Record,
    new_blobs: Vec<Blob>,
    pinned: Option<u64>,
) -> (Batch, Applied) {
    let others: Vec<&Record> = records.iter().filter(|r| r.id != new.id).collect();
    let previous = others
        .iter()
        .max_by(|a, b| a.modified_ms.total_cmp(&b.modified_ms))
        .map(|r| r.id);
    let mut kept: Vec<Record> = others
        .iter()
        .filter(|r| Some(r.id) == previous || Some(r.id) == pinned)
        .map(|r| (*r).clone())
        .collect();
    let mut batch = Batch::default();
    let mut written = Vec::new();
    for (key, bytes) in new_blobs {
        if !blobs.contains(&key) && !written.contains(&key) {
            batch.writes.push((storage::blob_name(&key), bytes));
            written.push(key);
        }
    }
    let json = serde_json::to_vec(&new).expect("a record serialises");
    batch
        .writes
        .push((storage::record_name(new.id), Arc::new(json)));
    for r in records {
        if r.id != new.id && !kept.iter().any(|k| k.id == r.id) {
            batch.deletes.push(storage::record_name(r.id));
        }
    }
    kept.push(new);
    let deleted = unreferenced(&kept, blobs);
    batch
        .deletes
        .extend(deleted.iter().map(|k| storage::blob_name(k)));
    (
        batch,
        Applied {
            kept,
            written,
            deleted,
        },
    )
}

/// The batch that deletes session `id` and the blobs only it linked.
pub fn plan_delete(records: &[Record], blobs: &BTreeSet<String>, id: u64) -> (Batch, Applied) {
    let kept: Vec<Record> = records.iter().filter(|r| r.id != id).cloned().collect();
    let deleted = unreferenced(&kept, blobs);
    let mut batch = Batch::default();
    if records.iter().any(|r| r.id == id) {
        batch.deletes.push(storage::record_name(id));
    }
    batch
        .deletes
        .extend(deleted.iter().map(|k| storage::blob_name(k)));
    (
        batch,
        Applied {
            kept,
            written: Vec::new(),
            deleted,
        },
    )
}

/// The blobs in `blobs` no record in `kept` links.
fn unreferenced(kept: &[Record], blobs: &BTreeSet<String>) -> Vec<String> {
    blobs
        .iter()
        .filter(|b| !kept.iter().any(|r| r.sources.contains(b)))
        .cloned()
        .collect()
}

/// Opens `record`, its sources' bytes in `blobs` (in `record.sources` order), as
/// unsaved work. Natively a source that was linked from a file that still holds the
/// same bytes is linked again, so Save writes the project as it was; any other source
/// is embedded. In the browser every source comes from the session's memory store.
pub fn restore(record: &Record, blobs: Vec<Vec<u8>>) -> Result<Document, ProjectError> {
    use convert64_project::file::{Linked, SourceSpec};
    if blobs.len() != record.sources.len() {
        return Err(ProjectError::Invalid(
            "the autosaved session is missing sources".to_owned(),
        ));
    }
    let by_key: BTreeMap<&str, Vec<u8>> = record
        .sources
        .iter()
        .map(String::as_str)
        .zip(blobs)
        .collect();
    let mut file = ProjectFile::parse(&record.project)?;
    let key_of = |spec: &SourceSpec| match spec {
        SourceSpec::Linked(Linked { path }) | SourceSpec::Path(path) => path
            .strip_prefix("blobs")
            .ok()
            .and_then(|k| k.to_str())
            .map(str::to_owned),
        _ => None,
    };
    #[cfg(not(target_arch = "wasm32"))]
    {
        for layer in &mut file.layers {
            let Some(key) = key_of(&layer.source) else {
                continue;
            };
            let bytes = by_key.get(key.as_str()).ok_or_else(|| {
                ProjectError::Invalid(format!("the autosaved source {key} is missing"))
            })?;
            let original = record
                .linked
                .get(&key)
                .filter(|p| std::fs::read(p).is_ok_and(|b| content_key(&b) == key));
            layer.source = match original {
                Some(path) => SourceSpec::Linked(Linked { path: path.clone() }),
                None => SourceSpec::embedded(bytes),
            };
        }
        let session = convert64_project::Session::from_project(file, std::path::Path::new(""))?;
        Ok(Document::restored(
            &record.name,
            session,
            record.path.clone(),
        ))
    }
    #[cfg(target_arch = "wasm32")]
    {
        let store = Arc::new(convert64_project::MemoryStore::new());
        for layer in &mut file.layers {
            if let Some(key) = key_of(&layer.source) {
                let bytes = by_key.get(key.as_str()).ok_or_else(|| {
                    ProjectError::Invalid(format!("the autosaved source {key} is missing"))
                })?;
                store.insert(blob_path(&key), bytes.clone());
            }
        }
        let session =
            convert64_project::Session::load(file, std::path::Path::new(""), Box::new(store))?;
        Ok(Document::restored(&record.name, session, None))
    }
}

/// The default storage of this platform: a folder in the data directory natively
/// (`CONVERT64_AUTOSAVE_DIR` overrides it), OPFS or IndexedDB in the browser.
pub fn platform_storage() -> Option<Box<dyn Storage>> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let dir = std::env::var_os("CONVERT64_AUTOSAVE_DIR")
            .map(PathBuf::from)
            .or_else(|| eframe::storage_dir("Convert64").map(|d| d.join("autosave")))?;
        Some(Box::new(DirStorage::new(dir)))
    }
    #[cfg(target_arch = "wasm32")]
    {
        web::storage()
    }
}

/// The wall clock, ms since the Unix epoch (`Date.now()` in the browser).
pub fn wall_ms() -> f64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0.0, |d| d.as_secs_f64() * 1000.0)
    }
    #[cfg(target_arch = "wasm32")]
    {
        eframe::web_sys::js_sys::Date::now()
    }
}

fn log(line: &str) {
    crate::scheduler::clock::log(line);
}

#[cfg(test)]
mod tests;
