//! The `.convert64` project file and the editing session (ADR-009, #192), shared by the
//! app, the `convert64` CLI and the MCP server.
//!
//! - [`ProjectFile`] is the file's schema (serde), with the version-1 recipe migration
//!   ([`recipe`]), linked, embedded and shared sources, masks on disk and the palette.
//! - [`JsonCommand`] is a registry command as JSON, as project files and front-ends
//!   write it.
//! - [`Session`] loads a project into the core's editor, compositor and solver, runs
//!   commands, solves and saves it back.
//!
//! The crate opens no files by itself: everything a project links goes through a
//! [`Store`], so the browser build loads and saves projects from bytes
//! ([`MemoryStore`]). The `fs` feature adds `FsStore` and path-taking conveniences
//! (`Session::open`, `Session::save`, `ProjectFile::read`) for the native front-ends.
//! The core is built without its `parallel` feature here; native front-ends turn it on.

mod base64;
pub mod codec;
pub mod command;
mod error;
pub mod file;
pub mod paths;
pub mod recipe;
pub mod session;
pub mod store;

pub use command::{JsonCommand, LayerRef};
pub use error::ProjectError;
pub use file::{EXTENSION, Embed, PaletteSpec, ProjectFile, SCHEMA_VERSION, SolveSpec};
pub use recipe::RecipeFile;
pub use session::Session;
#[cfg(feature = "fs")]
pub use store::FsStore;
pub use store::{MemoryStore, Store};
