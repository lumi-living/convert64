//! The bitmap solver (`docs/TAD.md` §3.1, E5): chooses legal VIC-II cell colours and
//! dithers the composite into them, driven per pixel by the attribute map (ADR-002).
//!
//! Pure and wasm-safe: no I/O, `rayon` only behind the `parallel` feature.

pub mod cell;
pub mod dither;
pub mod input;
pub mod metric;
pub mod ordered;
pub mod solver;

pub use cell::{CellChoice, Constraints};
pub use input::{Input, InputOptions};
pub use metric::{Metric, MetricTables, Space};
pub use solver::{Background, Solution, SolveOptions, Solver};
