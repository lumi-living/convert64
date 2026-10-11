//! Byte-exact readers and writers for Koala, Art Studio, raw bins, `.spd`, CharPad-style bins, PRG stubs and asm/C emitters (`docs/TAD.md §4`).
//!
//! So far: readers for Koala Painter (`.kla`) and Art Studio (`.art`) pictures, and a
//! self-displaying PRG for them ([`prg`]), following `docs/reference/file-formats.md`
//! byte for byte. Readers and writers take and return bytes; the caller does the file
//! I/O. The other writers arrive with E6.

mod bitmap_files;
pub mod prg;

pub use bitmap_files::{BitmapFile, BitmapFormat, ReadError, read_art_studio, read_koala};
