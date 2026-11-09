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
        write_atomic(path, bytes).map_err(|e| ProjectError::Io(path.to_owned(), e.to_string()))
    }
}

/// Replaces the file at `path` with `bytes`, so that no failure leaves it part-written
/// (#328): the bytes go to a temporary file in the same folder, are flushed to the disk
/// and renamed over the target, which is atomic on one file system (Windows' rename
/// replaces an existing file too). If anything fails the temporary file is removed and
/// the target is as it was.
///
/// A symbolic link is followed, so the file it names is replaced, not the link. An
/// existing file's permissions carry over, and a read-only file is refused as a plain
/// write would refuse it (a rename on Unix would not).
#[cfg(feature = "fs")]
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    write_atomic_with(path, bytes, |file, bytes| file.write_all(bytes))
}

/// [`write_atomic`] with the write itself supplied, so a test can fail it part-way.
#[cfg(feature = "fs")]
fn write_atomic_with(
    path: &Path,
    bytes: &[u8],
    write: impl FnOnce(&mut std::fs::File, &[u8]) -> std::io::Result<()>,
) -> std::io::Result<()> {
    use std::io::{Error, ErrorKind};
    use std::sync::atomic::{AtomicU64, Ordering};
    /// Tells apart the temporary files of saves running at once in one process.
    static NEXT: AtomicU64 = AtomicU64::new(0);

    let target = follow_link(path);
    let existing = match std::fs::metadata(&target) {
        Ok(m) if m.is_dir() => {
            return Err(Error::new(ErrorKind::InvalidInput, "is a folder"));
        }
        Ok(m) if m.permissions().readonly() => {
            return Err(Error::new(
                ErrorKind::PermissionDenied,
                "the file is read-only",
            ));
        }
        Ok(m) => Some(m.permissions()),
        Err(e) if e.kind() == ErrorKind::NotFound => None,
        Err(e) => return Err(e),
    };
    let name = target
        .file_name()
        .ok_or_else(|| Error::new(ErrorKind::InvalidInput, "no file name"))?;
    let dir = match target.parent() {
        Some(d) if !d.as_os_str().is_empty() => d.to_owned(),
        _ => PathBuf::from("."),
    };
    let tmp = dir.join(format!(
        ".{}.{}-{}.part",
        name.to_string_lossy(),
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let written = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)
        .and_then(|mut file| {
            write(&mut file, bytes)?;
            file.sync_all()?;
            if let Some(p) = existing {
                file.set_permissions(p)?;
            }
            Ok(())
        })
        .and_then(|()| std::fs::rename(&tmp, &target));
    if let Err(e) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    // The rename is only durable once the folder's entry is on the disk too. Best
    // effort: the new file is in place either way, and Windows can't open a folder so.
    #[cfg(unix)]
    if let Ok(d) = std::fs::File::open(&dir) {
        let _ = d.sync_all();
    }
    Ok(())
}

/// The file `path` names: a symbolic link's target, else `path` itself.
#[cfg(feature = "fs")]
fn follow_link(path: &Path) -> PathBuf {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => std::fs::canonicalize(path)
            .or_else(|_| {
                std::fs::read_link(path)
                    .map(|to| path.parent().map_or_else(|| to.clone(), |d| d.join(&to)))
            })
            .unwrap_or_else(|_| path.to_owned()),
        _ => path.to_owned(),
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

    /// A fresh, empty directory under the system's temporary one.
    #[cfg(feature = "fs")]
    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("convert64-store-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// The names in `dir`, sorted.
    #[cfg(feature = "fs")]
    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    /// #328: a save that fails part-way (here after 3 bytes) leaves the previous file
    /// byte for byte, and no temporary file beside it.
    #[cfg(feature = "fs")]
    #[test]
    fn a_write_that_fails_part_way_leaves_the_old_file() {
        use std::io::Write;
        let dir = scratch("part-way");
        let path = dir.join("art.convert64");
        let old = b"{\"version\": 2, \"the\": \"old project\"}".to_vec();
        std::fs::write(&path, &old).unwrap();
        let err = write_atomic_with(&path, b"{\"version\": 2, \"new\": 1}", |file, bytes| {
            file.write_all(&bytes[..3])?;
            Err(std::io::Error::new(
                std::io::ErrorKind::StorageFull,
                "disk full",
            ))
        })
        .unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::StorageFull);
        assert_eq!(std::fs::read(&path).unwrap(), old);
        assert_eq!(names(&dir), ["art.convert64"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// #328: a read-only project is not replaced, on any platform (a rename would
    /// replace it on Unix, where only the folder's permissions count).
    #[cfg(feature = "fs")]
    #[test]
    fn a_read_only_file_is_left_as_it_was() {
        let dir = scratch("read-only");
        let path = dir.join("art.convert64");
        std::fs::write(&path, b"old").unwrap();
        let mut permissions = std::fs::metadata(&path).unwrap().permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(&path, permissions.clone()).unwrap();
        assert!(FsStore.write(&path, b"new").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"old");
        assert_eq!(names(&dir), ["art.convert64"]);
        #[expect(clippy::permissions_set_readonly_false, reason = "the test's own file")]
        permissions.set_readonly(false);
        std::fs::set_permissions(&path, permissions).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// #328: a save creates a new file, replaces an existing one (a rename over it,
    /// which Windows allows too) and keeps its permissions, leaving nothing else.
    #[cfg(feature = "fs")]
    #[test]
    fn a_write_creates_or_replaces_the_file_and_nothing_else() {
        let dir = scratch("replace");
        let path = dir.join("art.convert64");
        FsStore.write(&path, b"first").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"first");
        let before = std::fs::metadata(&path).unwrap().permissions();
        FsStore
            .write(&path, b"second, longer than the first")
            .unwrap();
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"second, longer than the first"
        );
        FsStore.write(&path, b"3").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"3");
        assert_eq!(std::fs::metadata(&path).unwrap().permissions(), before);
        assert_eq!(names(&dir), ["art.convert64"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// #328: a save through a symbolic link replaces the file it names and keeps the
    /// link.
    #[cfg(all(feature = "fs", unix))]
    #[test]
    fn a_write_through_a_link_replaces_the_linked_file() {
        let dir = scratch("link");
        let file = dir.join("art.convert64");
        let link = dir.join("link.convert64");
        std::fs::write(&file, b"old").unwrap();
        std::os::unix::fs::symlink("art.convert64", &link).unwrap();
        FsStore.write(&link, b"new").unwrap();
        assert!(
            std::fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(std::fs::read(&file).unwrap(), b"new");
        assert_eq!(names(&dir), ["art.convert64", "link.convert64"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
