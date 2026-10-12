//! Byte-exact readers and writers for Koala, Art Studio, raw bins, `.spd`, CharPad-style bins, PRG stubs and asm/C emitters (`docs/TAD.md §4`).
//!
//! So far, following `docs/reference/file-formats.md` byte for byte:
//!
//! - readers and writers for Koala Painter (`.kla`, `.koa`) and Art Studio (`.art`,
//!   `.aas`) pictures ([`BitmapFormat`]);
//! - a self-displaying PRG for them ([`prg`]);
//! - which bit pair each cell colour takes when they are written ([`bitpairs`]);
//! - the raw bitmap set, headerless `.bin` files plus a JSON manifest ([`raw`]);
//! - assembler source for KickAssembler, ACME, 64tass and ca65, and C headers ([`asm`]);
//! - a palette-exact PNG preview at integer zoom ([`png`]).
//!
//! Readers and writers take and return bytes; the caller does the file I/O.

pub mod asm;
mod bitmap_files;
pub mod bitpairs;
pub mod png;
pub mod prg;
pub mod raw;

pub use bitmap_files::{
    BitmapFile, BitmapFormat, ReadError, WriteError, read_art_studio, read_koala,
};
