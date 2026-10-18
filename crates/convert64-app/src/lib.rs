//! The app's library half: what the user interface drives but can be tested without it
//! (`cargo test -p convert64-app`). The binary (`main.rs`) is the egui front end.

pub mod autosave;
pub mod document;
pub mod drag;
pub mod export;
pub mod files;
pub mod icons;
pub mod scheduler;
pub mod shell;
pub mod theme;
pub mod thumbs;
pub mod view;
#[cfg(target_arch = "wasm32")]
pub mod web;
pub mod workspace;
