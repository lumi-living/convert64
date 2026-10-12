//! Why a project could not be read, run or written.

use std::path::PathBuf;

/// Why a project or recipe could not be loaded, run or saved.
#[derive(Debug)]
pub enum ProjectError {
    /// Reading or writing a file failed.
    Io(PathBuf, String),
    /// The JSON is malformed or has the wrong shape.
    Json(String),
    /// A value is not allowed.
    Invalid(String),
}

impl std::fmt::Display for ProjectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(p, e) => write!(f, "{}: {e}", p.display()),
            Self::Json(e) => write!(f, "recipe JSON: {e}"),
            Self::Invalid(e) => f.write_str(e),
        }
    }
}

impl std::error::Error for ProjectError {}

pub(crate) fn invalid(e: impl std::fmt::Display) -> ProjectError {
    ProjectError::Invalid(e.to_string())
}
