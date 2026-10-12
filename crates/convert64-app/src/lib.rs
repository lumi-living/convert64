//! The app's library half: what the user interface drives but can be tested without it
//! (`cargo test -p convert64-app`). The binary (`main.rs`) is the egui front end.

pub mod scheduler;
pub mod theme;
