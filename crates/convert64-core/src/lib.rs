//! Pure Rust, no I/O: document model (`Project`), command registry, compositor, attribute map, solvers per mode and validators; builds for `wasm32-unknown-unknown` unchanged (`docs/TAD.md §4`).
//!
//! This crate must stay free of `std::fs`, threads-by-default and platform
//! code. Parallelism is opt-in through the `parallel` cargo feature; see
//! [`par`] for the single-source iterator helpers.

pub mod colour;
pub mod command;
pub mod composite;
pub mod document;
pub mod geom;
pub mod layer;
pub mod mask;
pub mod mode;
pub mod model;
pub mod palette;
pub mod par;
pub mod preview;
pub mod quality;
pub mod region;
pub mod rule;
mod scratch;
pub mod solve;
pub mod validate;

pub use colour::{ColourIndex, ColourSet, InvalidColourIndex};
pub use command::{Command, Editor, Project};
pub use composite::{AttributeMap, Canvas, Compositor, TargetFrame};
pub use document::SolveSettings;
pub use geom::{CellSet, PxRect, SuperGrid, Vec2};
pub use layer::{Blend, Downscale, Layer, Recipe, Resample, SourceImage, Transform};
pub use mask::{Mask, MaskRaster};
pub use mode::{Mode, ModeSpec};
pub use model::{
    BitmapImage, CellPos, CharColour, CharsetGlobals, CharsetImage, IndexedBitmap, ModelError,
    Sprite, SpriteSet,
};
pub use palette::Palette;
pub use region::Region;
pub use rule::Rule;
pub use validate::{
    C64Image, Location, Placement, Violation, validate, validate_export, validate_placement,
};
