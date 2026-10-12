//! Assembler source and C header emitters (`docs/reference/file-formats.md`,
//! "Assembler source we emit").
//!
//! [`emit`] writes a bitmap picture as source for KickAssembler, ACME, 64tass or ca65,
//! or as a C header. The data is the picture file's body in the Koala / Art Studio
//! order (bitmap, Screen RAM, then Colour RAM and background for multicolor), placed at
//! the format's conventional load address, so assembling the source gives the same
//! bytes the binary writers produce ([`BitmapFormat::write`] without its two-byte load
//! address). Only the directives differ between assemblers:
//!
//! | Assembler | Program counter | Byte data | Include binary | Skip PRG header |
//! |---|---|---|---|---|
//! | KickAssembler | `* = $6000 "name"` | `.byte $01,$02` | `.import binary "f.bin"` | `.import c64 "f.kla"` |
//! | ACME | `* = $6000` | `!byte $01,$02` | `!binary "f.bin"` | `!binary "f.kla",,2` |
//! | 64tass | `* = $6000` | `.byte $01,$02` | `.binary "f.bin"` | `.binary "f.kla",2` |
//! | ca65 | `.segment "NAME"` | `.byte $01,$02` | `.incbin "f.bin"` | `.incbin "f.kla",2` |
//!
//! Labels are `lowercase_with_underscores` (`<name>_bitmap`, `<name>_screen`,
//! `<name>_colour`, `<name>_background`), constants `UPPERCASE_WITH_UNDERSCORES`
//! (`<NAME>_BITMAP_SIZE`, …, `<NAME>_BACKGROUND_COLOUR`, `<NAME>_LOAD_ADDRESS`). The data
//! is either written out as byte lines, 16 bytes per line ([`DataStyle::Bytes`]), or
//! included from the raw set's `.bin` files ([`DataStyle::IncludeRaw`]) or from the
//! picture file itself, skipping its load address ([`DataStyle::IncludePicture`]); the
//! included files are returned with the source. A C header always holds the bytes as
//! `static const unsigned char` arrays, since C has no standard binary include.
//!
//! ```
//! use convert64_formats::asm::{Assembler, DataStyle, EmitOptions, emit};
//! # let image = convert64_core::model::BitmapImage::hires_from_bytes(&[0; 8000], &[0x10; 1000]).unwrap();
//! let files = emit(Assembler::Acme, &image, &EmitOptions::new("title")).unwrap();
//! assert_eq!(files[0].0.to_str(), Some("title.a"));
//! let source = String::from_utf8(files[0].1.clone()).unwrap();
//! assert!(source.contains("* = $2000"));
//! assert!(source.contains("title_bitmap\n"));
//! ```

use crate::BitmapFormat;
use crate::raw::Block;
use convert64_core::mode::Mode;
use convert64_core::model::BitmapImage;
use std::fmt::{self, Write as _};
use std::path::PathBuf;

/// A target for [`emit`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Assembler {
    /// KickAssembler (`.asm`).
    KickAssembler,
    /// ACME (`.a`).
    Acme,
    /// 64tass (`.tas`).
    Tass64,
    /// ca65, the cc65 suite's assembler (`.s`).
    Ca65,
    /// A C header (`.h`), for cc65, llvm-mos, oscar64 and friends.
    CHeader,
}

impl Assembler {
    /// Every target, in documentation order.
    pub const ALL: [Self; 5] = [
        Self::KickAssembler,
        Self::Acme,
        Self::Tass64,
        Self::Ca65,
        Self::CHeader,
    ];

    /// The short id for command lines: `kickass`, `acme`, `64tass`, `ca65`, `c`.
    pub const fn id(self) -> &'static str {
        match self {
            Self::KickAssembler => "kickass",
            Self::Acme => "acme",
            Self::Tass64 => "64tass",
            Self::Ca65 => "ca65",
            Self::CHeader => "c",
        }
    }

    /// The target an id names, case-insensitively.
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|a| a.id().eq_ignore_ascii_case(id))
    }

    /// Display name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::KickAssembler => "KickAssembler",
            Self::Acme => "ACME",
            Self::Tass64 => "64tass",
            Self::Ca65 => "ca65",
            Self::CHeader => "C header",
        }
    }

    /// The source file's extension: `asm`, `a`, `tas`, `s`, `h`.
    pub const fn extension(self) -> &'static str {
        match self {
            Self::KickAssembler => "asm",
            Self::Acme => "a",
            Self::Tass64 => "tas",
            Self::Ca65 => "s",
            Self::CHeader => "h",
        }
    }
}

impl fmt::Display for Assembler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// How [`emit`] writes the picture data.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum DataStyle {
    /// Byte lines in the source, 16 bytes per line.
    #[default]
    Bytes,
    /// Include directives for the raw set's block files (`<name>.bitmap.bin`, …), which
    /// are returned with the source.
    IncludeRaw,
    /// One include of the Koala / Art Studio file (`<name>.kla` / `<name>.art`),
    /// skipping its two-byte load address; the file is returned with the source.
    IncludePicture,
}

/// Options for [`emit`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmitOptions {
    /// The base name: file names start with it and labels and constants are made from
    /// it (lowercased / uppercased, every other character than ASCII letters and digits
    /// becoming `_`).
    pub name: String,
    /// How the data is written.
    pub data: DataStyle,
    /// Where the data goes; `None` is the format's conventional load address (`$6000`
    /// Koala, `$2000` Art Studio). Ignored by ca65 (the linker places segments) except
    /// for the `<NAME>_LOAD_ADDRESS` constant and the comment.
    pub load_address: Option<u16>,
}

impl EmitOptions {
    /// Byte lines at the conventional load address.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            data: DataStyle::Bytes,
            load_address: None,
        }
    }

    /// The same options with another data style.
    #[must_use]
    pub fn with_data(mut self, data: DataStyle) -> Self {
        self.data = data;
        self
    }
}

/// Why [`emit`] refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EmitError {
    /// The name is empty, a path, or has no letter or digit.
    Name(String),
    /// The picture is not a bitmap.
    Mode(Mode),
}

impl fmt::Display for EmitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Name(n) => write!(
                f,
                "{n:?} is not a usable name: use a plain file name with a letter or digit"
            ),
            Self::Mode(m) => write!(f, "cannot emit a {m} picture as a bitmap"),
        }
    }
}

impl std::error::Error for EmitError {}

/// `name` as a label: lowercase ASCII letters, digits and `_`; prefixed with `pic_` if
/// it would start with a digit.
fn label_base(name: &str) -> String {
    let mut s: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    if s.starts_with(|c: char| c.is_ascii_digit()) {
        s.insert_str(0, "pic_");
    }
    s
}

/// The label suffix of a block.
const fn block_label(block: Block) -> &'static str {
    match block {
        Block::Bitmap => "bitmap",
        Block::Screen => "screen",
        Block::Colour => "colour",
        Block::Background => "background",
    }
}

/// Everything the per-assembler writers need.
struct Ctx<'a> {
    asm: Assembler,
    image: &'a BitmapImage,
    format: BitmapFormat,
    /// The file stem for file names.
    name: &'a str,
    /// Lowercase label base.
    label: String,
    /// Uppercase constant base.
    konst: String,
    load: u16,
    data: DataStyle,
    blocks: &'static [Block],
}

impl Ctx<'_> {
    fn comment(&self) -> &'static str {
        match self.asm {
            Assembler::KickAssembler => "//",
            Assembler::CHeader => " *",
            _ => ";",
        }
    }

    fn byte_directive(&self) -> &'static str {
        match self.asm {
            Assembler::Acme => "!byte",
            _ => ".byte",
        }
    }

    fn picture_file(&self) -> String {
        format!("{}.{}", self.name, self.format.extensions()[0])
    }

    fn block_file(&self, block: Block) -> String {
        format!("{}.{}", self.name, block.suffix())
    }

    /// `label` as a definition line: `name:` for KickAssembler and ca65, a bare name
    /// for ACME and 64tass.
    fn label_line(&self, label: &str) -> String {
        match self.asm {
            Assembler::KickAssembler | Assembler::Ca65 => format!("{label}:\n"),
            _ => format!("{label}\n"),
        }
    }

    /// A constant definition.
    fn constant(&self, name: &str, value: &str) -> String {
        match self.asm {
            Assembler::KickAssembler => format!(".const {name} = {value}\n"),
            Assembler::CHeader => format!("#define {name} {value}\n"),
            _ => format!("{name} = {value}\n"),
        }
    }

    /// A label defined as an offset from another (for [`DataStyle::IncludePicture`]).
    fn alias(&self, label: &str, base: &str, offset: usize) -> String {
        match self.asm {
            Assembler::KickAssembler => format!(".label {label} = {base} + {offset}\n"),
            _ => format!("{label} = {base} + {offset}\n"),
        }
    }

    fn include(&self, file: &str, skip_header: bool) -> String {
        let f = file;
        match (self.asm, skip_header) {
            (Assembler::KickAssembler, false) => format!("    .import binary \"{f}\"\n"),
            (Assembler::KickAssembler, true) => format!("    .import c64 \"{f}\"\n"),
            (Assembler::Acme, false) => format!("    !binary \"{f}\"\n"),
            (Assembler::Acme, true) => format!("    !binary \"{f}\",,2\n"),
            (Assembler::Tass64, false) => format!("    .binary \"{f}\"\n"),
            (Assembler::Tass64, true) => format!("    .binary \"{f}\",2\n"),
            (_, false) => format!("    .incbin \"{f}\"\n"),
            (_, true) => format!("    .incbin \"{f}\",2\n"),
        }
    }

    fn byte_lines(&self, bytes: &[u8]) -> String {
        let mut s = String::new();
        for chunk in bytes.chunks(16) {
            let items: Vec<String> = chunk.iter().map(|b| format!("${b:02x}")).collect();
            let _ = writeln!(s, "    {} {}", self.byte_directive(), items.join(","));
        }
        s
    }

    fn mode_text(&self) -> &'static str {
        match self.format {
            BitmapFormat::Koala => "multicolor bitmap, Koala layout",
            BitmapFormat::ArtStudio => "hi-res bitmap, Art Studio layout",
        }
    }

    fn total(&self) -> usize {
        self.blocks.iter().map(|b| b.size()).sum()
    }

    fn header(&self) -> String {
        let c = self.comment();
        let mut s = String::new();
        let _ = writeln!(
            s,
            "{c} {}: {}, generated by Convert64.",
            self.name,
            self.mode_text()
        );
        let mut offset = 0;
        for &b in self.blocks {
            let size = b.size();
            let _ = writeln!(
                s,
                "{c}   ${:04x}  {:<10}  {size:>4} {}",
                usize::from(self.load) + offset,
                block_label(b),
                if size == 1 { "byte" } else { "bytes" }
            );
            offset += size;
        }
        let picture = self.picture_file();
        let source = format!("{}.{}", self.name, self.asm.extension());
        let prg = format!("{}.prg", self.name);
        // KickAssembler, ACME (`-f cbm`) and 64tass write a program file with a two-byte
        // load address, which is then the picture file itself.
        let assemble = match self.asm {
            Assembler::KickAssembler => {
                Some(format!("java -jar KickAss.jar {source} -o {picture}"))
            }
            Assembler::Acme => Some(format!("acme -f cbm -o {prg} {source}")),
            Assembler::Tass64 => Some(format!("64tass -o {prg} {source}")),
            Assembler::Ca65 | Assembler::CHeader => None,
        };
        if let Some(cmd) = assemble {
            let _ = writeln!(s, "{c} Assemble: {cmd}");
            if self.load == self.format.default_load_address() {
                let _ = writeln!(s, "{c} {prg} is then {picture} byte for byte.");
            }
        } else if self.asm == Assembler::Ca65 {
            let _ = writeln!(
                s,
                "{c} The data is in segment \"{seg}\"; link it at ${:04x}, e.g. with\n\
                 {c}   MEMORY {{ PIC: start = ${:04x}, size = ${:04x}, file = %O; }}\n\
                 {c}   SEGMENTS {{ {seg}: load = PIC, type = ro; }}",
                self.load,
                self.load,
                self.total(),
                seg = self.konst
            );
        }
        s
    }

    fn constants(&self) -> String {
        let k = &self.konst;
        let mut s = String::new();
        let hex = |v: u16| match self.asm {
            Assembler::CHeader => format!("0x{v:04x}"),
            _ => format!("${v:04x}"),
        };
        s += &self.constant(&format!("{k}_LOAD_ADDRESS"), &hex(self.load));
        for &b in self.blocks {
            let name = format!("{k}_{}_SIZE", block_label(b).to_ascii_uppercase());
            s += &self.constant(&name, &b.size().to_string());
        }
        if let Some(bg) = self.image.background() {
            let v = match self.asm {
                Assembler::CHeader => format!("0x{:02x}", bg.get()),
                _ => format!("${:02x}", bg.get()),
            };
            s += &self.constant(&format!("{k}_BACKGROUND_COLOUR"), &v);
        }
        s
    }

    fn origin(&self) -> String {
        match self.asm {
            Assembler::KickAssembler => format!("* = ${:04x} \"{}\"\n", self.load, self.label),
            Assembler::Ca65 => format!(".segment \"{}\"\n", self.konst),
            _ => format!("* = ${:04x}\n", self.load),
        }
    }

    fn label_of(&self, b: Block) -> String {
        format!("{}_{}", self.label, block_label(b))
    }

    fn assembler_source(&self) -> String {
        let mut s = self.header();
        s.push('\n');
        s += &self.constants();
        s.push('\n');
        s += &self.origin();
        match self.data {
            DataStyle::IncludePicture => {
                let first = self.label_of(Block::Bitmap);
                s += &self.label_line(&first);
                s += &self.include(&self.picture_file(), true);
                let mut offset = 0;
                for &b in self.blocks {
                    if b != Block::Bitmap {
                        s += &self.alias(&self.label_of(b), &first, offset);
                    }
                    offset += b.size();
                }
            }
            DataStyle::IncludeRaw => {
                for &b in self.blocks {
                    s += &self.label_line(&self.label_of(b));
                    s += &self.include(&self.block_file(b), false);
                }
            }
            DataStyle::Bytes => {
                for &b in self.blocks {
                    s += &self.label_line(&self.label_of(b));
                    s += &self.byte_lines(&b.bytes(self.image));
                }
            }
        }
        s
    }

    fn c_header(&self) -> String {
        let mut s = String::from("/*\n");
        s += &self.header();
        s += " * Assembled data order and addresses as above; the arrays hold the same bytes.\n */\n";
        let guard = format!("{}_H", self.konst);
        let _ = writeln!(s, "#ifndef {guard}\n#define {guard}\n");
        s += &self.constants();
        for &b in self.blocks {
            let size = format!(
                "{}_{}_SIZE",
                self.konst,
                block_label(b).to_ascii_uppercase()
            );
            let _ = writeln!(
                s,
                "\nstatic const unsigned char {}[{size}] = {{",
                self.label_of(b)
            );
            for chunk in b.bytes(self.image).chunks(16) {
                let items: Vec<String> = chunk.iter().map(|v| format!("0x{v:02x}")).collect();
                let _ = writeln!(s, "    {},", items.join(", "));
            }
            s += "};\n";
        }
        let _ = writeln!(s, "\n#endif /* {guard} */");
        s
    }
}

/// Emits `image` for `asm`: the source file first (`<name>.<extension>`), then any
/// file it includes. Paths are bare file names, to be written next to each other.
///
/// # Errors
/// [`EmitError::Name`] for an empty name, one with a path separator, or one without an
/// ASCII letter or digit; [`EmitError::Mode`] for a picture that is not a bitmap.
pub fn emit(
    asm: Assembler,
    image: &BitmapImage,
    opts: &EmitOptions,
) -> Result<Vec<(PathBuf, Vec<u8>)>, EmitError> {
    let name = opts.name.as_str();
    if name.is_empty()
        || name.contains(['/', '\\', '\0'])
        || !name.chars().any(|c| c.is_ascii_alphanumeric())
    {
        return Err(EmitError::Name(name.to_owned()));
    }
    let format = BitmapFormat::for_mode(image.mode()).ok_or(EmitError::Mode(image.mode()))?;
    let label = label_base(name);
    let ctx = Ctx {
        asm,
        image,
        format,
        name,
        konst: label.to_ascii_uppercase(),
        label,
        load: opts.load_address.unwrap_or(format.default_load_address()),
        data: opts.data,
        blocks: Block::for_mode(image.mode()),
    };
    let source = if asm == Assembler::CHeader {
        ctx.c_header()
    } else {
        ctx.assembler_source()
    };
    let mut out = vec![(
        PathBuf::from(format!("{name}.{}", asm.extension())),
        source.into_bytes(),
    )];
    match (asm, opts.data) {
        (Assembler::CHeader, _) | (_, DataStyle::Bytes) => {}
        (_, DataStyle::IncludeRaw) => out.extend(
            ctx.blocks
                .iter()
                .map(|&b| (PathBuf::from(ctx.block_file(b)), b.bytes(image))),
        ),
        (_, DataStyle::IncludePicture) => {
            let bytes = format
                .write(image)
                .unwrap_or_else(|_| unreachable!("the format is the image's mode's"));
            out.push((PathBuf::from(ctx.picture_file()), bytes));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip() {
        for a in Assembler::ALL {
            assert_eq!(Assembler::from_id(a.id()), Some(a));
            assert_eq!(Assembler::from_id(&a.id().to_ascii_uppercase()), Some(a));
        }
        assert_eq!(Assembler::from_id("masm"), None);
    }

    #[test]
    fn labels_are_lowercase_with_underscores() {
        assert_eq!(label_base("My Title-2"), "my_title_2");
        assert_eq!(label_base("64er"), "pic_64er");
    }

    #[test]
    fn bad_names_are_refused() {
        let img = BitmapImage::hires_from_bytes(&[0; 8000], &[0; 1000]).expect("legal");
        for bad in ["", "a/b", "c:\\x", "---"] {
            assert_eq!(
                emit(Assembler::Acme, &img, &EmitOptions::new(bad)),
                Err(EmitError::Name(bad.to_owned()))
            );
        }
    }
}
