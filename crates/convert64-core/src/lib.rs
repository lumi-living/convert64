//! Pure Rust, no I/O: document model (`Project`), command registry, compositor, attribute map, solvers per mode and validators; builds for `wasm32-unknown-unknown` unchanged (`docs/TAD.md §4`).
//!
//! This crate must stay free of `std::fs`, threads-by-default and platform
//! code. Parallelism is opt-in through the `parallel` cargo feature; see
//! [`par`] for the single-source iterator helpers.

pub mod colour;
pub mod palette;
pub mod par;

pub use colour::{ColourIndex, InvalidColourIndex};
pub use palette::Palette;
