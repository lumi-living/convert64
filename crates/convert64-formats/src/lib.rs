//! Byte-exact readers and writers for Koala, Art Studio, raw bins, `.spd`, CharPad-style bins, PRG stubs and asm/C emitters (`docs/TAD.md §4`).
//!
//! So far: readers for Koala Painter (`.kla`) and Art Studio (`.art`) pictures, following
//! `docs/reference/file-formats.md` byte for byte. Readers take bytes; the caller does
//! the file I/O. Writers arrive with E6.

mod bitmap_files;

pub use bitmap_files::{BitmapFile, BitmapFormat, ReadError, read_art_studio, read_koala};
