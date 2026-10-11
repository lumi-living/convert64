//! The bitmap solver (`docs/TAD.md` §3.1, E5): chooses legal VIC-II cell colours and
//! dithers the composite into them, driven per pixel by the attribute map (ADR-002).
//!
//! Pure and wasm-safe: no I/O, `rayon` only behind the `parallel` feature.

pub mod metric;

pub use metric::{Metric, MetricTables, Space};
