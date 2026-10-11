//! Pure Rust, no I/O: document model (`Project`), command registry, compositor, attribute map, solvers per mode and validators; builds for `wasm32-unknown-unknown` unchanged (`docs/TAD.md §4`).
//!
//! This crate must stay free of `std::fs`, threads-by-default and platform
//! code. Parallelism is opt-in through the `parallel` cargo feature; see
//! [`par`] for the single-source iterator helpers.

pub mod colour;
pub mod mode;
pub mod model;
pub mod palette;
pub mod par;
pub mod rule;
pub mod validate;

pub use colour::{ColourIndex, ColourSet, InvalidColourIndex};
pub use mode::{Mode, ModeSpec};
pub use model::{
    BitmapImage, CellPos, CharColour, CharsetGlobals, CharsetImage, IndexedBitmap, ModelError,
    Sprite, SpriteSet,
};
pub use palette::Palette;
pub use rule::Rule;
pub use validate::{
    C64Image, Location, Placement, Violation, validate, validate_export, validate_placement,
};
