//! Files in and out (#205): native dialogs through `rfd`, the browser's file picker,
//! and in wasm saves straight to disk (`crate::web`, #241), and files dropped on the window. Results arrive as [`FileEvent`]s
//! the app polls once a frame, so the browser's asynchronous picker and the native
//! dialogs look the same to it.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// The files Open… offers.
pub const OPEN_FILTER: (&str, &[&str]) = ("Images and projects", &["png", "convert64", "json"]);

/// What a picked or dropped file is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    /// Open it (File › Open…).
    Open,
    /// Add it as a layer (New Layer from File…, or dropped on an open project).
    AddLayer,
    /// Load it as the selected layer's mask (the mask options bar's Load PNG…).
    Mask,
    /// Load it as the palette (a VICE `.vpl` file).
    Palette,
}

/// A finished file operation.
#[derive(Debug)]
pub enum FileEvent {
    /// A file the user picked or dropped: its name, bytes and, natively, its path.
    Opened {
        /// The file name.
        name: String,
        /// The contents.
        bytes: Vec<u8>,
        /// Where it is (native only).
        path: Option<PathBuf>,
        /// What it is for.
        purpose: Purpose,
    },
    /// Files written or downloaded, with their sizes in bytes, the verb for the toast
    /// ("Exported", "Saved") and what they were.
    Saved(Vec<(String, usize)>, &'static str, SaveTag),
    /// Something went wrong: the message names cause and remedy.
    Failed(String),
}

/// What a save wrote, so the app can act once it has happened (#241).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveTag {
    /// Exported files.
    Export,
    /// The project, as it was at `revision`; `key` is the autosave's key of it.
    Project {
        /// The document revision saved.
        revision: u64,
        /// [`crate::autosave::Autosave::snapshot_key`] at the time.
        key: Option<String>,
    },
}

/// `bytes` in human units (usability review C3): `512 B`, `9.8 kB`, `10 kB`, `1.8 MB`
/// (SI, as file managers show them).
pub fn human_size(bytes: usize) -> String {
    let b = bytes as f64;
    let (v, unit) = if b < 1000.0 {
        return format!("{bytes} B");
    } else if b < 999_500.0 {
        (b / 1000.0, "kB")
    } else if b < 999_500_000.0 {
        (b / 1e6, "MB")
    } else {
        (b / 1e9, "GB")
    };
    if v < 9.95 {
        format!("{v:.1} {unit}")
    } else {
        format!("{v:.0} {unit}")
    }
}

/// The inbox of finished file operations.
#[derive(Debug, Clone, Default)]
pub struct Files {
    inbox: Arc<Mutex<Vec<FileEvent>>>,
}

impl Files {
    /// Everything that finished since the last call.
    pub fn poll(&self) -> Vec<FileEvent> {
        std::mem::take(&mut *self.inbox.lock().expect("file inbox"))
    }

    fn push(&self, event: FileEvent) {
        self.inbox.lock().expect("file inbox").push(event);
    }

    /// Files dropped on the window this frame, as `Opened` events (read asynchronously in
    /// the browser).
    pub fn take_dropped(&self, ctx: &egui::Context) {
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        for f in dropped {
            #[cfg(not(target_arch = "wasm32"))]
            self.read_path(f.path().to_owned(), Purpose::AddLayer);
            #[cfg(target_arch = "wasm32")]
            {
                let this = self.clone();
                let ctx = ctx.clone();
                eframe::web_sys::js_sys::futures::spawn_local(async move {
                    let name = f.path().to_string_lossy().into_owned();
                    match f.bytes_async().await {
                        Ok(bytes) => this.push(FileEvent::Opened {
                            name,
                            bytes,
                            path: None,
                            purpose: Purpose::AddLayer,
                        }),
                        Err(e) => this.push(FileEvent::Failed(format!("Can't read {name}: {e}."))),
                    }
                    ctx.request_repaint();
                });
            }
        }
    }

    /// Reads `path` (native) into an `Opened` event, as if picked in Open….
    #[cfg(not(target_arch = "wasm32"))]
    pub fn open_path(&self, path: PathBuf) {
        self.read_path(path, Purpose::Open);
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn read_path(&self, path: PathBuf, purpose: Purpose) {
        let name = path
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        match std::fs::read(&path) {
            Ok(bytes) => self.push(FileEvent::Opened {
                name,
                bytes,
                path: Some(path),
                purpose,
            }),
            Err(e) => self.push(FileEvent::Failed(format!(
                "Can't open {}: {e}.",
                path.display()
            ))),
        }
    }

    /// Open… (`Cmd O`): the native dialog or the browser's file picker, the result
    /// tagged with what it is for.
    pub fn pick_open(&self, ctx: &egui::Context, purpose: Purpose) {
        let filter = match purpose {
            Purpose::Palette => ("VICE palette", &["vpl"][..]),
            Purpose::Mask => ("Greyscale PNG", &["png"][..]),
            _ => OPEN_FILTER,
        };
        let dialog = rfd::AsyncFileDialog::new().add_filter(filter.0, filter.1);
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = ctx;
            let picked = rfd::FileDialog::new()
                .add_filter(filter.0, filter.1)
                .pick_file();
            drop(dialog);
            if let Some(path) = picked {
                self.read_path(path, purpose);
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            let this = self.clone();
            let ctx = ctx.clone();
            eframe::web_sys::js_sys::futures::spawn_local(async move {
                if let Some(file) = dialog.pick_file().await {
                    let bytes = file.read().await;
                    this.push(FileEvent::Opened {
                        name: file.file_name(),
                        bytes,
                        path: None,
                        purpose,
                    });
                    ctx.request_repaint();
                }
            });
        }
    }

    /// Where to save a file named `suggested` (native Save As dialog), or `None` if the
    /// user cancelled.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn pick_save_path(&self, suggested: &str, filter: (&str, &[&str])) -> Option<PathBuf> {
        rfd::FileDialog::new()
            .set_file_name(suggested)
            .add_filter(filter.0, filter.1)
            .save_file()
    }

    /// Writes `files` (natively) next to each other in a folder the user picks; one
    /// file gets a Save dialog instead. In the browser: the Save dialog or a download
    /// for each (`crate::web::save_files`). `FileEvent::Saved` comes once they are
    /// written.
    pub fn save_files(
        &self,
        ctx: &egui::Context,
        files: Vec<(String, Vec<u8>)>,
        verb: &'static str,
        tag: SaveTag,
    ) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = ctx;
            let targets: Option<Vec<(PathBuf, Vec<u8>)>> = if let [(name, _)] = files.as_slice() {
                let ext = std::path::Path::new(name)
                    .extension()
                    .map_or_else(String::new, |e| e.to_string_lossy().into_owned());
                self.pick_save_path(name, ("Export", &[ext.as_str()]))
                    .map(|path| files.into_iter().map(|(_, b)| (path.clone(), b)).collect())
            } else {
                rfd::FileDialog::new()
                    .pick_folder()
                    .map(|dir| files.into_iter().map(|(n, b)| (dir.join(n), b)).collect())
            };
            let Some(targets) = targets else { return };
            let mut saved = Vec::new();
            for (path, bytes) in targets {
                if let Some(dir) = path.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                if let Err(e) = std::fs::write(&path, &bytes) {
                    self.push(FileEvent::Failed(format!(
                        "Can't write {}: {e}.",
                        path.display()
                    )));
                    return;
                }
                let name = path
                    .file_name()
                    .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                saved.push((name, bytes.len()));
            }
            self.push(FileEvent::Saved(saved, verb, tag));
        }
        // Straight to disk, no overlay (#241): the Save dialog where the browser has
        // one, else a download.
        #[cfg(target_arch = "wasm32")]
        {
            let this = self.clone();
            crate::web::save_to_inbox(files, verb, tag, move |e| this.push(e), ctx.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::human_size;

    #[test]
    fn sizes_read_in_human_units() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(999), "999 B");
        assert_eq!(human_size(1000), "1.0 kB");
        assert_eq!(human_size(9_800), "9.8 kB");
        assert_eq!(human_size(10_003), "10 kB");
        assert_eq!(human_size(1_817_097), "1.8 MB");
        assert_eq!(human_size(999_999), "1.0 MB");
        assert_eq!(human_size(25_400_000), "25 MB");
    }
}
