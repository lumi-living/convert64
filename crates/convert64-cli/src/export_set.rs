//! A project's export set as files (#287): what `convert64 export` without `--asm` and
//! the MCP server's `export_run` write, the same files the app's Export panel writes
//! (`convert64_formats::export::set_files` on the full solve).

use convert64_project::Session;
use std::path::{Path, PathBuf};

/// The export set's files for the session's project, from a full solve, named after
/// `stem` where a row has no name of its own.
///
/// # Errors
/// The project has no export set, or a row can't be written (the message names it).
pub fn files(session: &mut Session, stem: &str) -> Result<Vec<(String, Vec<u8>)>, String> {
    let exports = session.project().exports.clone();
    if exports.is_empty() {
        return Err(
            "the project has no export set: add rows in the app's Export tab or with \
             export_set, or name targets with --asm"
                .to_owned(),
        );
    }
    let palette = session.palette().clone();
    let solution = session.solve(true);
    convert64_formats::export::set_files(&exports, &solution.image, &palette, stem)
}

/// Writes [`files`] into `dir` (created if missing) and returns each file's path and
/// size in bytes, in row order.
///
/// # Errors
/// As [`files`], or a file can't be written.
pub fn write(
    session: &mut Session,
    stem: &str,
    dir: &Path,
) -> Result<Vec<(PathBuf, usize)>, String> {
    let files = files(session, stem)?;
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut written = Vec::with_capacity(files.len());
    for (name, bytes) in files {
        let path = dir.join(name);
        std::fs::write(&path, &bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        written.push((path, bytes.len()));
    }
    Ok(written)
}

/// The base name files get when no row names its own: the project file's name
/// (`title.convert64` gives `title`), else `picture`.
pub fn default_stem(project: &Path) -> String {
    project.file_stem().map_or_else(
        || "picture".to_owned(),
        |s| s.to_string_lossy().into_owned(),
    )
}
