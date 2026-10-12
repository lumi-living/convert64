//! Where a project's files come from and go to (#192).
//!
//! A project names files by path: linked sources and masks, a `.vpl` palette, the
//! `mask_load` command's PNG and the project file itself. The session never opens them
//! itself; it asks a [`Store`] for their bytes. The native front-ends use `FsStore`
//! (the `fs` feature); the browser build hands the files it was given to a
//! [`MemoryStore`], and tests do too.

use crate::error::ProjectError;
use crate::paths::absolute;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Reads and writes whole files by path. Paths arrive joined to the project file's
/// directory and normalised ([`absolute`]).
pub trait Store: Send + Sync {
    /// The bytes of the file at `path`.
    fn read(&self, path: &Path) -> Result<Vec<u8>, ProjectError>;
    /// Replaces the file at `path` with `bytes`.
    fn write(&self, path: &Path, bytes: &[u8]) -> Result<(), ProjectError>;
}

/// A shared store: several sessions (or the app and a session) over the same files.
impl<S: Store + ?Sized> Store for std::sync::Arc<S> {
    fn read(&self, path: &Path) -> Result<Vec<u8>, ProjectError> {
        (**self).read(path)
    }

    fn write(&self, path: &Path, bytes: &[u8]) -> Result<(), ProjectError> {
        (**self).write(path, bytes)
    }
}

/// Files held in memory, keyed by their normalised path: the browser's dropped or
/// fetched files, or a test's.
#[derive(Debug, Default)]
pub struct MemoryStore {
    files: Mutex<BTreeMap<PathBuf, Vec<u8>>>,
}

impl MemoryStore {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds or replaces a file.
    pub fn insert(&self, path: impl AsRef<Path>, bytes: impl Into<Vec<u8>>) {
        self.lock().insert(absolute(path.as_ref()), bytes.into());
    }

    /// A file's bytes, if the store has it.
    pub fn get(&self, path: impl AsRef<Path>) -> Option<Vec<u8>> {
        self.lock().get(&absolute(path.as_ref())).cloned()
    }

    /// The paths of every file, sorted.
    pub fn paths(&self) -> Vec<PathBuf> {
        self.lock().keys().cloned().collect()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<PathBuf, Vec<u8>>> {
        self.files
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl Store for MemoryStore {
    fn read(&self, path: &Path) -> Result<Vec<u8>, ProjectError> {
        self.get(path)
            .ok_or_else(|| ProjectError::Io(path.to_owned(), "no such file".to_owned()))
    }

    fn write(&self, path: &Path, bytes: &[u8]) -> Result<(), ProjectError> {
        self.insert(path, bytes);
        Ok(())
    }
}

/// The file system, for the native front-ends.
#[cfg(feature = "fs")]
#[derive(Clone, Copy, Debug, Default)]
pub struct FsStore;

#[cfg(feature = "fs")]
impl Store for FsStore {
    fn read(&self, path: &Path) -> Result<Vec<u8>, ProjectError> {
        std::fs::read(path).map_err(|e| ProjectError::Io(path.to_owned(), e.to_string()))
    }

    fn write(&self, path: &Path, bytes: &[u8]) -> Result<(), ProjectError> {
        std::fs::write(path, bytes).map_err(|e| ProjectError::Io(path.to_owned(), e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_store_normalises_paths() {
        let s = MemoryStore::new();
        s.insert("art/../a.png", [1u8, 2]);
        assert_eq!(s.read(Path::new("./a.png")).unwrap(), vec![1, 2]);
        s.write(Path::new("b/c.png"), &[3]).unwrap();
        assert_eq!(s.get("b/./c.png"), Some(vec![3]));
        assert!(s.read(Path::new("missing.png")).is_err());
        assert_eq!(s.paths().len(), 2);
    }
}
