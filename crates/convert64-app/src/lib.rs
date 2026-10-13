//! The app's library half: what the user interface drives but can be tested without it
//! (`cargo test -p convert64-app`). The binary (`main.rs`) is the egui front end.

pub mod document;
pub mod icons;
pub mod scheduler;
pub mod shell;
pub mod theme;
pub mod view;
pub mod workspace;
