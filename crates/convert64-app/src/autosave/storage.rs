//! Where autosaved sessions live (#241): a flat set of named entries, each written
//! whole. The autosave logic ([`super::Autosave`]) decides what to write and delete;
//! a [`Storage`] only carries it out. Every operation reports through a callback, so
//! the browser's asynchronous OPFS and IndexedDB (`super::web`) and the synchronous
//! folder and memory stores here look the same to it.
//!
//! Entries are `session-<id>.json` (a [`super::Record`]) and `blob-<key>` (a source's
//! bytes, keyed by their content).

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::Arc;

/// Why a storage operation failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageError {
    /// The storage is full (the browser's `QuotaExceededError`, a full disk).
    pub quota: bool,
    /// What happened, for the status bar's details and the log.
    pub message: String,
}

impl StorageError {
    /// A failure that is not about space.
    pub fn other(message: impl Into<String>) -> Self {
        Self {
            quota: false,
            message: message.into(),
        }
    }

    /// The storage is full.
    pub fn quota(message: impl Into<String>) -> Self {
        Self {
            quota: true,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for StorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// What a storage holds: every session record's bytes and the names of the blobs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Contents {
    /// The bytes of every `session-*.json` entry.
    pub records: Vec<Vec<u8>>,
    /// The keys of every `blob-*` entry.
    pub blobs: Vec<String>,
}

/// Writes, then deletes: the new blobs first, the record that references them last,
/// so an interrupted batch never leaves a record pointing at a missing blob. Deleting
/// an entry that is not there is not an error, and a failed delete does not fail the
/// batch (it leaves an orphan, collected next time).
#[derive(Debug, Clone, Default)]
pub struct Batch {
    /// Entries to write, in order: (name, bytes).
    pub writes: Vec<(String, Arc<Vec<u8>>)>,
    /// Entries to delete after every write succeeded.
    pub deletes: Vec<String>,
}

/// The callback an operation reports through.
pub type Done<T> = Box<dyn FnOnce(Result<T, StorageError>)>;

/// A place for autosaved sessions.
pub trait Storage {
    /// Every record and the blobs' keys.
    fn load(&self, done: Done<Contents>);
    /// The bytes of the named entries, in order.
    fn read(&self, names: Vec<String>, done: Done<Vec<Vec<u8>>>);
    /// Carries out `batch` (see [`Batch`]).
    fn apply(&self, batch: Batch, done: Done<()>);
    /// What it is, for the log ("OPFS", "IndexedDB", a folder's path).
    fn describe(&self) -> String;
}

/// The name of session `id`'s record.
pub fn record_name(id: u64) -> String {
    format!("session-{id}.json")
}

/// The name of the blob with content key `key`.
pub fn blob_name(key: &str) -> String {
    format!("blob-{key}")
}

/// Sorts entry names into records and blobs; anything else is ignored.
pub fn classify(names: impl IntoIterator<Item = String>) -> (Vec<String>, Vec<String>) {
    let mut records = Vec::new();
    let mut blobs = Vec::new();
    for name in names {
        if name.starts_with("session-") && name.ends_with(".json") {
            records.push(name);
        } else if let Some(key) = name.strip_prefix("blob-") {
            blobs.push(key.to_owned());
        }
    }
    records.sort();
    blobs.sort();
    (records, blobs)
}

/// Entries in memory: the tests' storage, and screenshot runs' (which must not touch
/// the user's). Clones share the entries, so a test can look inside after handing one
/// over. An optional quota makes writes fail as a full browser storage does.
#[derive(Debug, Clone, Default)]
pub struct MemoryStorage {
    inner: Rc<RefCell<Memory>>,
}

#[derive(Debug, Default)]
struct Memory {
    entries: BTreeMap<String, Arc<Vec<u8>>>,
    /// Total bytes the entries may hold.
    quota: Option<usize>,
    /// Entries written so far (each write counts once).
    writes: usize,
    /// Batches applied so far.
    batches: usize,
}

impl MemoryStorage {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Writes fail once the entries would hold more than `bytes`.
    pub fn set_quota(&self, bytes: Option<usize>) {
        self.inner.borrow_mut().quota = bytes;
    }

    /// The entry names, sorted.
    pub fn names(&self) -> Vec<String> {
        self.inner.borrow().entries.keys().cloned().collect()
    }

    /// An entry's bytes.
    pub fn get(&self, name: &str) -> Option<Arc<Vec<u8>>> {
        self.inner.borrow().entries.get(name).cloned()
    }

    /// Adds or replaces an entry directly (tests).
    pub fn insert(&self, name: &str, bytes: Vec<u8>) {
        self.inner
            .borrow_mut()
            .entries
            .insert(name.to_owned(), Arc::new(bytes));
    }

    /// Entries written so far.
    pub fn writes(&self) -> usize {
        self.inner.borrow().writes
    }

    /// Batches applied so far.
    pub fn batches(&self) -> usize {
        self.inner.borrow().batches
    }
}

impl Storage for MemoryStorage {
    fn load(&self, done: Done<Contents>) {
        let names = self.names();
        let (records, blobs) = classify(names);
        let m = self.inner.borrow();
        let records = records
            .iter()
            .filter_map(|n| m.entries.get(n).map(|b| b.to_vec()))
            .collect();
        drop(m);
        done(Ok(Contents { records, blobs }));
    }

    fn read(&self, names: Vec<String>, done: Done<Vec<Vec<u8>>>) {
        let m = self.inner.borrow();
        let out: Result<Vec<_>, _> = names
            .iter()
            .map(|n| {
                m.entries
                    .get(n)
                    .map(|b| b.to_vec())
                    .ok_or_else(|| StorageError::other(format!("{n} is missing")))
            })
            .collect();
        drop(m);
        done(out);
    }

    fn apply(&self, batch: Batch, done: Done<()>) {
        let mut m = self.inner.borrow_mut();
        m.batches += 1;
        for (name, bytes) in batch.writes {
            if let Some(quota) = m.quota {
                let used: usize = m
                    .entries
                    .iter()
                    .filter(|(n, _)| **n != name)
                    .map(|(_, b)| b.len())
                    .sum();
                if used + bytes.len() > quota {
                    drop(m);
                    return done(Err(StorageError::quota(format!(
                        "QuotaExceededError: no room for {name}"
                    ))));
                }
            }
            m.writes += 1;
            m.entries.insert(name, bytes);
        }
        for name in batch.deletes {
            m.entries.remove(&name);
        }
        drop(m);
        done(Ok(()));
    }

    fn describe(&self) -> String {
        "memory".to_owned()
    }
}

/// A folder on disk (native): the recovery files in the platform data directory. Each
/// entry is one file, written to a temporary name and renamed into place, so a crash
/// mid-write leaves the old file whole.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone)]
pub struct DirStorage {
    dir: std::path::PathBuf,
}

#[cfg(not(target_arch = "wasm32"))]
impl DirStorage {
    /// The store in `dir` (created on the first write).
    pub fn new(dir: impl Into<std::path::PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// The folder.
    pub fn dir(&self) -> &std::path::Path {
        &self.dir
    }

    fn error(what: &str, e: &std::io::Error) -> StorageError {
        let message = format!("{what}: {e}");
        if e.kind() == std::io::ErrorKind::StorageFull {
            StorageError::quota(message)
        } else {
            StorageError::other(message)
        }
    }

    fn names(&self) -> Result<Vec<String>, StorageError> {
        match std::fs::read_dir(&self.dir) {
            Ok(entries) => Ok(entries
                .filter_map(Result::ok)
                .filter_map(|e| e.file_name().into_string().ok())
                .collect()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(Self::error(&self.dir.display().to_string(), &e)),
        }
    }

    fn write(&self, name: &str, bytes: &[u8]) -> Result<(), StorageError> {
        let path = self.dir.join(name);
        let tmp = self.dir.join(format!("{name}.tmp"));
        std::fs::create_dir_all(&self.dir)
            .and_then(|()| std::fs::write(&tmp, bytes))
            .and_then(|()| std::fs::rename(&tmp, &path))
            .map_err(|e| {
                let _ = std::fs::remove_file(&tmp);
                Self::error(&path.display().to_string(), &e)
            })
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Storage for DirStorage {
    fn load(&self, done: Done<Contents>) {
        let result = self.names().map(|names| {
            let (records, blobs) = classify(names);
            Contents {
                records: records
                    .iter()
                    .filter_map(|n| std::fs::read(self.dir.join(n)).ok())
                    .collect(),
                blobs,
            }
        });
        done(result);
    }

    fn read(&self, names: Vec<String>, done: Done<Vec<Vec<u8>>>) {
        let out = names
            .iter()
            .map(|n| {
                let path = self.dir.join(n);
                std::fs::read(&path).map_err(|e| Self::error(&path.display().to_string(), &e))
            })
            .collect();
        done(out);
    }

    fn apply(&self, batch: Batch, done: Done<()>) {
        for (name, bytes) in &batch.writes {
            if let Err(e) = self.write(name, bytes) {
                return done(Err(e));
            }
        }
        for name in &batch.deletes {
            let _ = std::fs::remove_file(self.dir.join(name));
        }
        done(Ok(()));
    }

    fn describe(&self) -> String {
        self.dir.display().to_string()
    }
}
