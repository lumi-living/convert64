//! Self-displaying PRG for Koala and Art Studio pictures (`docs/reference/file-formats.md`,
//! "PRG display stubs").
//!
//! The PRG loads at `$0801`. A one-line BASIC program (`10 SYS2061`) starts a small
//! hand-assembled 6502 loader at `$080D`, and the picture file follows at `$0900`
//! verbatim, its own two-byte load address included. The loader copies the picture to
//! VIC bank 1 (bitmap `$6000`, Screen RAM `$5C00`) and, for Koala, Colour RAM to
//! `$D800`, sets the VIC-II registers and loops forever. The picture therefore always
//! starts at file offset [`PAYLOAD_OFFSET`], so "the PRG carries this file" is a byte
//! comparison.
//!
//! The payload occupies `$0900`–`$3012` at most (a 10 003-byte Koala file), so the copy
//! destinations at `$5C00`–`$7FFF` and `$D800` never overlap it and a plain forward copy
//! is safe.

use crate::bitmap_files::{BitmapFormat, ReadError};

/// Load address of the PRG: the start of BASIC.
pub const LOAD_ADDRESS: u16 = 0x0801;

/// Address the BASIC line `SYS`es to: the first byte after the BASIC program.
pub const SYS_ADDRESS: u16 = 0x080D;

/// Address the picture file is loaded to, two-byte load address included.
pub const PAYLOAD_ADDRESS: u16 = 0x0900;

/// File offset of the picture file inside the PRG: the PRG's own two-byte header plus
/// `PAYLOAD_ADDRESS - LOAD_ADDRESS`. `prg[PAYLOAD_OFFSET..]` is the picture file.
pub const PAYLOAD_OFFSET: usize = 2 + (PAYLOAD_ADDRESS - LOAD_ADDRESS) as usize;

/// Where the loader copies the bitmap: the upper 8 KB of VIC bank 1.
pub const BITMAP_ADDRESS: u16 = 0x6000;

/// Where the loader copies Screen RAM: the last 1 KB slot below the bitmap in bank 1.
pub const SCREEN_ADDRESS: u16 = 0x5C00;

/// Colour RAM, fixed in hardware.
pub const COLOUR_RAM_ADDRESS: u16 = 0xD800;

/// `$D018`: Screen RAM at bank offset `$1C00` (bits 4–7 = 7), bitmap at `$2000` (bit 3).
pub const D018: u8 = 0x78;

/// `$D011`: bitmap mode, display on, 25 rows, vertical scroll 3 (the default).
pub const D011: u8 = 0x3B;

/// `$D016` for the format: multicolor on for Koala, off for Art Studio; 40 columns,
/// horizontal scroll 0.
pub const fn d016(format: BitmapFormat) -> u8 {
    match format {
        BitmapFormat::Koala => 0x18,
        BitmapFormat::ArtStudio => 0x08,
    }
}

/// Zero-page pointers the copy routine uses: source at `$FB/$FC`, destination at
/// `$FD/$FE` (free for user programs).
const SRC: u8 = 0xFB;
const DST: u8 = 0xFD;

/// Offsets of the picture blocks inside the picture file (after its load address).
const FILE_BITMAP: u16 = 2;
const FILE_SCREEN: u16 = 8002;
const FILE_COLOUR: u16 = 9002;
const FILE_BACKGROUND: u16 = 10_002;

const fn lo(addr: u16) -> u8 {
    addr.to_le_bytes()[0]
}

const fn hi(addr: u16) -> u8 {
    addr.to_le_bytes()[1]
}

/// Builds a self-displaying PRG for a Koala or Art Studio file.
///
/// `file` is the complete picture file, load address included; it is embedded
/// unchanged at [`PAYLOAD_OFFSET`]. The file's own load address is not used: the loader
/// always copies the picture to [`BITMAP_ADDRESS`], [`SCREEN_ADDRESS`] and (Koala)
/// [`COLOUR_RAM_ADDRESS`].
///
/// # Errors
/// Whatever [`BitmapFormat::read`] reports for `file`: a size that does not match the
/// format, or bytes that do not form a legal picture.
pub fn display_prg(format: BitmapFormat, file: &[u8]) -> Result<Vec<u8>, ReadError> {
    format.read(file)?;
    let mut prg = LOAD_ADDRESS.to_le_bytes().to_vec();
    prg.extend(basic_line());
    prg.extend(loader(format));
    prg.resize(PAYLOAD_OFFSET, 0);
    prg.extend_from_slice(file);
    Ok(prg)
}

/// `10 SYS2061`, as tokenised BASIC at `$0801`.
fn basic_line() -> Vec<u8> {
    let digits = SYS_ADDRESS.to_string();
    // Link (2) + line number (2) + SYS token (1) + digits + end of line (1).
    let next_line = LOAD_ADDRESS + 6 + digits.len() as u16;
    let mut b = Vec::new();
    b.extend(next_line.to_le_bytes()); // pointer to the next line
    b.extend(10_u16.to_le_bytes()); // line number 10
    b.push(0x9E); // SYS token
    b.extend(digits.bytes()); // "2061" in PETSCII (= ASCII for digits)
    b.push(0x00); // end of line
    b.extend([0x00, 0x00]); // end of program: null link
    debug_assert_eq!(LOAD_ADDRESS as usize + b.len(), SYS_ADDRESS as usize);
    b
}

/// The 6502 loader, assembled to run at [`SYS_ADDRESS`].
fn loader(format: BitmapFormat) -> Vec<u8> {
    let main = main_code(format, 0);
    let copy_routine = SYS_ADDRESS + main.len() as u16;
    let mut code = main_code(format, copy_routine);
    code.extend(copy_code());
    debug_assert!(SYS_ADDRESS as usize + code.len() <= PAYLOAD_ADDRESS as usize);
    code
}

/// Sets up `SRC`/`DST` and calls the copy routine for `pages` 256-byte pages.
fn copy_call(code: &mut Vec<u8>, from: u16, to: u16, pages: u8, copy_routine: u16) {
    code.extend([0xA9, lo(from)]); // LDA #<from
    code.extend([0x85, SRC]); // STA $FB
    code.extend([0xA9, hi(from)]); // LDA #>from
    code.extend([0x85, SRC + 1]); // STA $FC
    code.extend([0xA9, lo(to)]); // LDA #<to
    code.extend([0x85, DST]); // STA $FD
    code.extend([0xA9, hi(to)]); // LDA #>to
    code.extend([0x85, DST + 1]); // STA $FE
    code.extend([0xA2, pages]); // LDX #pages
    code.extend([0x20, lo(copy_routine), hi(copy_routine)]); // JSR copy
}

/// The main program. Its length does not depend on `copy_routine`.
fn main_code(format: BitmapFormat, copy_routine: u16) -> Vec<u8> {
    let mut code = Vec::new();
    code.push(0x78); // SEI: no KERNAL interrupt touches the screen from here on

    // VIC bank 1 ($4000-$7FFF): CIA 2 port A bits 0-1 = %10, other bits kept.
    code.extend([0xAD, 0x00, 0xDD]); // LDA $DD00
    code.extend([0x29, 0xFC]); // AND #$FC
    code.extend([0x09, 0x02]); // ORA #$02
    code.extend([0x8D, 0x00, 0xDD]); // STA $DD00

    // Whole pages are copied; the bytes past each block land in memory the VIC-II does
    // not show ($7F40-$7FFF, $5FE8-$5FFF, $DBE8-$DBFF).
    let payload = PAYLOAD_ADDRESS;
    // Bitmap: 8000 bytes, 32 pages.
    copy_call(
        &mut code,
        payload + FILE_BITMAP,
        BITMAP_ADDRESS,
        32,
        copy_routine,
    );
    // Screen RAM: 1000 bytes, 4 pages.
    copy_call(
        &mut code,
        payload + FILE_SCREEN,
        SCREEN_ADDRESS,
        4,
        copy_routine,
    );
    if format == BitmapFormat::Koala {
        // Colour RAM: 1000 bytes, 4 pages.
        copy_call(
            &mut code,
            payload + FILE_COLOUR,
            COLOUR_RAM_ADDRESS,
            4,
            copy_routine,
        );
    }

    code.extend([0xA9, D018]); // LDA #$78
    code.extend([0x8D, 0x18, 0xD0]); // STA $D018
    code.extend([0xA9, D011]); // LDA #$3B
    code.extend([0x8D, 0x11, 0xD0]); // STA $D011
    code.extend([0xA9, d016(format)]); // LDA #$18 (Koala) / #$08 (Art Studio)
    code.extend([0x8D, 0x16, 0xD0]); // STA $D016

    // Neither format stores a border colour: the border is black. Koala's background
    // byte (bit pair 00) goes to $D021; hi-res bitmaps do not show $D021, so Art
    // Studio sets it black too.
    if format == BitmapFormat::Koala {
        let bg = payload + FILE_BACKGROUND;
        code.extend([0xAD, lo(bg), hi(bg)]); // LDA background
        code.extend([0x8D, 0x21, 0xD0]); // STA $D021
        code.extend([0xA9, 0x00]); // LDA #$00
    } else {
        code.extend([0xA9, 0x00]); // LDA #$00
        code.extend([0x8D, 0x21, 0xD0]); // STA $D021
    }
    code.extend([0x8D, 0x20, 0xD0]); // STA $D020

    let here = SYS_ADDRESS + code.len() as u16;
    code.extend([0x4C, lo(here), hi(here)]); // JMP * (loop forever)
    code
}

/// Copies X pages from `(SRC)` to `(DST)`. Position-independent: only relative
/// branches.
fn copy_code() -> Vec<u8> {
    let mut code = Vec::new();
    code.extend([0xA0, 0x00]); // LDY #$00
    code.extend([0xB1, SRC]); // loop: LDA ($FB),Y
    code.extend([0x91, DST]); // STA ($FD),Y
    code.push(0xC8); // INY
    code.extend([0xD0, 0xF9]); // BNE loop (-7: back over BNE, INY, STA, LDA)
    code.extend([0xE6, SRC + 1]); // INC $FC: next source page
    code.extend([0xE6, DST + 1]); // INC $FE: next destination page
    code.push(0xCA); // DEX
    code.extend([0xD0, 0xF2]); // BNE loop (-14)
    code.push(0x60); // RTS
    code
}

#[cfg(test)]
mod tests {
    use super::*;

    fn koala() -> Vec<u8> {
        let mut f = 0x6000_u16.to_le_bytes().to_vec();
        f.extend((0..8000_u32).map(|i| (i * 31 % 251) as u8));
        f.extend((0..1000_u32).map(|i| (i * 17 % 256) as u8));
        f.extend((0..1000_u32).map(|i| (i % 16) as u8));
        f.push(0x0B);
        f
    }

    fn art(padded: bool) -> Vec<u8> {
        let mut f = 0x2000_u16.to_le_bytes().to_vec();
        f.extend((0..8000_u32).map(|i| (i * 7 % 256) as u8));
        f.extend((0..1000_u32).map(|i| (i * 29 % 256) as u8));
        if padded {
            f.extend([0; 7]);
        }
        f
    }

    /// Just enough 6502 to run the loader: the opcodes it uses, flat 64 KB RAM (I/O
    /// registers are plain bytes). Returns memory once the program jumps to itself.
    fn run(prg: &[u8], dd00: u8) -> Vec<u8> {
        let mut m = vec![0_u8; 0x1_0000];
        let load = usize::from(u16::from_le_bytes([prg[0], prg[1]]));
        m[load..load + prg.len() - 2].copy_from_slice(&prg[2..]);
        m[0xDD00] = dd00;
        (m[0xD020], m[0xD021]) = (0x0E, 0x06); // power-on light blue border, blue background
        let (mut pc, mut a, mut x, mut y, mut sp, mut z) =
            (SYS_ADDRESS, 0_u8, 0_u8, 0_u8, 0xFF_u8, false);
        let rd16 =
            |m: &[u8], at: u16| u16::from_le_bytes([m[usize::from(at)], m[usize::from(at) + 1]]);
        for _ in 0..1_000_000 {
            let op = m[usize::from(pc)];
            let b1 = m[usize::from(pc) + 1];
            let w = rd16(&m, pc + 1);
            let zp_ind =
                |m: &[u8], zp: u8, y: u8| rd16(m, u16::from(zp)).wrapping_add(u16::from(y));
            match op {
                0x78 => pc += 1,                                               // SEI
                0xA9 => (a, z, pc) = (b1, b1 == 0, pc + 2),                    // LDA #
                0xAD => (a, pc) = (m[usize::from(w)], pc + 3),                 // LDA abs
                0x29 => (a, pc) = (a & b1, pc + 2),                            // AND #
                0x09 => (a, pc) = (a | b1, pc + 2),                            // ORA #
                0x8D => (m[usize::from(w)], pc) = (a, pc + 3),                 // STA abs
                0x85 => (m[usize::from(b1)], pc) = (a, pc + 2),                // STA zp
                0xA2 => (x, pc) = (b1, pc + 2),                                // LDX #
                0xA0 => (y, pc) = (b1, pc + 2),                                // LDY #
                0xB1 => (a, pc) = (m[usize::from(zp_ind(&m, b1, y))], pc + 2), // LDA (zp),Y
                0x91 => {
                    let at = zp_ind(&m, b1, y);
                    m[usize::from(at)] = a;
                    pc += 2;
                } // STA (zp),Y
                0xC8 => (y, z, pc) = (y.wrapping_add(1), y == 0xFF, pc + 1),   // INY
                0xCA => (x, z, pc) = (x.wrapping_sub(1), x == 1, pc + 1),      // DEX
                0xE6 => {
                    m[usize::from(b1)] = m[usize::from(b1)].wrapping_add(1);
                    z = m[usize::from(b1)] == 0;
                    pc += 2;
                } // INC zp
                0xD0 => {
                    pc += 2;
                    if !z {
                        pc = pc.wrapping_add_signed(i16::from(b1 as i8));
                    }
                } // BNE
                0x20 => {
                    let ret = (pc + 2).to_le_bytes();
                    m[0x100 + usize::from(sp)] = ret[1];
                    m[0x100 + usize::from(sp - 1)] = ret[0];
                    sp -= 2;
                    pc = w;
                } // JSR
                0x60 => {
                    sp += 2;
                    pc = u16::from_le_bytes([
                        m[0x100 + usize::from(sp - 1)],
                        m[0x100 + usize::from(sp)],
                    ]) + 1;
                } // RTS
                0x4C if w == pc => return m,                                   // JMP * : done
                0x4C => pc = w,                                                // JMP
                _ => panic!("unexpected opcode ${op:02X} at ${pc:04X}"),
            }
        }
        panic!("the loader did not reach its final loop");
    }

    #[test]
    fn basic_line_sys_matches_the_loader_address() {
        let prg = display_prg(BitmapFormat::Koala, &koala()).expect("legal");
        assert_eq!(&prg[..2], &[0x01, 0x08]);
        assert_eq!(
            &prg[2..14],
            &[
                0x0B, 0x08, 0x0A, 0x00, 0x9E, b'2', b'0', b'6', b'1', 0, 0, 0
            ]
        );
        assert_eq!(SYS_ADDRESS, 2061);
        assert_eq!(PAYLOAD_OFFSET, 257);
    }

    #[test]
    fn payload_is_the_file_verbatim() {
        for (format, file) in [
            (BitmapFormat::Koala, koala()),
            (BitmapFormat::ArtStudio, art(false)),
            (BitmapFormat::ArtStudio, art(true)),
        ] {
            let prg = display_prg(format, &file).expect("legal");
            assert_eq!(prg[PAYLOAD_OFFSET..], file[..]);
            // Everything between the loader and the payload is zero padding.
            let end = 2 + usize::from(SYS_ADDRESS - LOAD_ADDRESS) + loader(format).len();
            assert!(prg[end..PAYLOAD_OFFSET].iter().all(|&b| b == 0));
            // The payload ends well below the copy destinations.
            assert!(usize::from(PAYLOAD_ADDRESS) + file.len() <= usize::from(SCREEN_ADDRESS));
        }
    }

    #[test]
    fn koala_loader_copies_every_block_and_sets_the_registers() {
        let file = koala();
        let m = run(
            &display_prg(BitmapFormat::Koala, &file).expect("legal"),
            0x97,
        );
        assert_eq!(m[0x6000..0x6000 + 8000], file[2..8002]);
        assert_eq!(m[0x5C00..0x5C00 + 1000], file[8002..9002]);
        assert_eq!(m[0xD800..0xD800 + 1000], file[9002..10_002]);
        assert_eq!(m[0xDD00], 0x96, "bank 1, other port bits kept");
        assert_eq!((m[0xD018], m[0xD011], m[0xD016]), (0x78, 0x3B, 0x18));
        assert_eq!((m[0xD020], m[0xD021]), (0x00, 0x0B), "black border");
    }

    #[test]
    fn art_studio_loader_copies_bitmap_and_screen() {
        for padded in [false, true] {
            let file = art(padded);
            let m = run(
                &display_prg(BitmapFormat::ArtStudio, &file).expect("legal"),
                0x97,
            );
            assert_eq!(m[0x6000..0x6000 + 8000], file[2..8002]);
            assert_eq!(m[0x5C00..0x5C00 + 1000], file[8002..9002]);
            assert!(
                m[0xD800..0xDC00].iter().all(|&b| b == 0),
                "Colour RAM untouched"
            );
            assert_eq!(m[0xDD00], 0x96);
            assert_eq!((m[0xD018], m[0xD011], m[0xD016]), (0x78, 0x3B, 0x08));
            assert_eq!((m[0xD020], m[0xD021]), (0, 0));
        }
    }

    #[test]
    fn rejects_files_the_reader_rejects() {
        let err = display_prg(BitmapFormat::Koala, &[0; 100]).expect_err("short");
        assert_eq!(err.rule_number(), Some(6));
        let mut bad = koala();
        bad[10_002] = 0x1F;
        assert_eq!(
            display_prg(BitmapFormat::Koala, &bad)
                .expect_err("bg $1F")
                .rule_number(),
            Some(2)
        );
    }
}
