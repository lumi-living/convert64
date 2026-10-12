# File formats

Byte layouts Convert64 reads and writes. Offsets are decimal unless prefixed `$`. All multi-byte values are little-endian. "Load address" means the two-byte PRG header most C64 picture formats carry.

## Bitmap pictures

Koala and Art Studio share one shape: a two-byte load address, then the 8000 bitmap bytes, then Screen RAM, then (Koala only) Colour RAM and the background. Bitmap bytes are in VIC-II order, one 8-byte cell after another along each row (`offset = cell_row × 320 + cell_col × 8 + pixel_row`), bit 7 leftmost ([vic-ii-constraints.md §2](vic-ii-constraints.md)). Screen and Colour RAM are 1000 bytes, one per cell, row by row (`cell_row × 40 + cell_col`). "In memory" is where each block lands when the file is loaded at its conventional address; a viewer copies Screen and Colour RAM from there to the addresses it displays from.

### Koala Painter (`.kla`, `.koa`) — multicolor bitmap
```
Offset         Size  In memory at $6000   Content
0     $0000    2     —                    Load address, little-endian: $00 $60 = $6000
2     $0002    8000  $6000–$7F3F          Bitmap: 2 bits per pixel, 4 pixels per byte
8002  $1F42    1000  $7F40–$8327          Screen RAM: high nybble = bit pair 01, low nybble = bit pair 10
9002  $232A    1000  $8328–$870F          Colour RAM: low nybble = bit pair 11
10002 $2712    1     $8710                Background colour ($D021) = bit pair 00
Total          10003
```

### Art Studio (`.art`, `.aas`) — hi-res bitmap
```
Offset         Size  In memory at $2000   Content
0     $0000    2     —                    Load address, little-endian: $00 $20 = $2000
2     $0002    8000  $2000–$3F3F          Bitmap: 1 bit per pixel, 8 pixels per byte
8002  $1F42    1000  $3F40–$4327          Screen RAM: high nybble = colour of set bits (1), low nybble = clear bits (0)
[9002 $232A    7     $4328–$432E          Padding, in some files]
Total          9002 (9009 padded)
```

**As read** (`BitmapFormat::read`, `read_koala`, `read_art_studio`):
- The format comes from the extension, case-insensitively and with or without a dot (`kla`/`koa` Koala, `art`/`aas` Art Studio: `BitmapFormat::from_extension`), or from the size alone, which no two formats share (`BitmapFormat::from_size`).
- The size must be exactly 10003 (Koala) or 9002 or 9009 (Art Studio). Anything else is refused as rule 6 with a message that says whether the file is truncated or oversized and by how much, e.g. `Koala files are 10003 bytes; this one is 10002 (truncated: 1 byte short)`.
- Any load address is accepted and kept (`BitmapFile::load_address`): Koala at `$6000` and Art Studio at `$2000` are the conventions, and the layout after the header does not depend on the address, so a file saved elsewhere reads the same. The round-trip tests use both `$6000` and `$2000` for both formats.
- Koala Colour RAM keeps only its low nybble, as the hardware does: memory dumps of `$D800` carry random high bits. A background byte above `$0F` is refused (rule 2).
- Art Studio's seven padding bytes are ignored, whatever they hold.

**As written** (`BitmapFormat::write`, `BitmapFile::to_bytes`):
- `BitmapFormat::write` writes the format's conventional load address (`$6000` Koala, `$2000` Art Studio); `BitmapFile::to_bytes` writes the file's own `load_address`, so a file read at another address writes back unchanged.
- Koala: 10003 bytes. Colour RAM bytes have clear high nybbles.
- Art Studio: 9002 bytes, never padded.
- Each format refuses a picture of the other mode (`WriteError`) rather than writing a file a viewer would misread.

Round trips, tested in `crates/convert64-formats/tests/bitmap_roundtrip.rs` (#158): every fixture solved in both modes writes, reads back as the same `BitmapImage` under each extension and at both load addresses, and validates; every committed png2c64 baseline reads and writes back byte for byte.

### Advanced Art Studio (`.ocp`, `.mpic`) — multicolor bitmap
```
0       2     Load address $2000
2       8000  Bitmap
8002    1000  Screen RAM
9002    1     Border colour
9003    1     Background colour
9004    1000  Colour RAM  (common variant; some writers pad 14 bytes before Colour RAM — readers should detect by total length 10007 vs 10018)
```

### Doodle (`.dd`, `.jj`) — hi-res bitmap
```
0       2     Load address $5C00
2       1000  Screen RAM
1002    1     Border colour
1003    1     Background colour
1004    2     Load address $6000 (second segment)
1006    8000  Bitmap
Total   9006
```

### Raw bitmap set (Convert64 `--format raw`)
Separate files with no headers, plus a JSON manifest (`convert64_formats::raw`):
```
File                Size   Suggested address   Content
<name>.bitmap.bin   8000   $6000               Bitmap, VIC-II cell order (as in Koala/Art Studio)
<name>.screen.bin   1000   $5C00               Screen RAM (nybbles as in Koala/Art Studio)
<name>.colour.bin   1000   $D800               Colour RAM, high nybbles clear   (multicolor only)
<name>.bg.bin       1      — ($D021)           Background colour 0–15           (multicolor only)
<name>.json         —      —                   Manifest
```
The block files concatenated in this order are a Koala (multicolor) or Art Studio (hi-res) file without its two-byte load address. The suggested addresses are VIC bank 1, the layout the display PRG uses: `$DD00` bits 0–1 = `%10`, `$D018` = `$78`.

The manifest, as written for a hi-res set called `line` (pretty-printed with two-space indents, LF line endings, a trailing newline; a multicolor set lists four files):
```json
{
  "format": "convert64-raw-bitmap",
  "version": 1,
  "mode": "hires-bitmap",
  "palette": {
    "name": "Pepto (PAL)",
    "rgb": ["#000000", "#FFFFFF", "…14 more, index order"]
  },
  "files": [
    { "block": "bitmap", "file": "line.bitmap.bin", "size": 8000, "load_address": "$6000" },
    { "block": "screen", "file": "line.screen.bin", "size": 1000, "load_address": "$5C00" }
  ]
}
```

| Field | Value |
|---|---|
| `format` | always `convert64-raw-bitmap` |
| `version` | `1`; a reader refuses versions newer than it knows |
| `mode` | `multicolor-bitmap` or `hires-bitmap` |
| `palette.name` | the palette preset the picture was solved and previewed with |
| `palette.rgb` | its 16 colours in index order, `#RRGGBB` (uppercase when written; either case read) |
| `files[]` | one per block, in the order `bitmap`, `screen`, then `colour`, `background` for multicolor |
| `files[].file` | the file name, next to the manifest; a name with a directory is refused |
| `files[].size` | 8000, 1000, 1000, 1 |
| `files[].load_address` | the suggested address as `$XXXX`; absent for `background`, which is the `$D021` value |

Unknown fields are refused. Reading (`raw::read`, `raw::read_files`) checks the format, the version, that the files are exactly the mode's blocks, every size in the manifest and on disk (rule 6) and the background (rule 2), and keeps only the low nybble of Colour RAM bytes. Every fixture, solved in both modes, round-trips through a raw set to the identical `BitmapImage` (`crates/convert64-formats/tests/raw_roundtrip.rs`, #159).

### PNG preview (`convert64_formats::png`)
Not a C64 file: a picture of what the VIC-II shows, for people and diffs.
```
Size        320·zoom × 200·zoom pixels, zoom 1–8, both modes
Pixels      8-bit RGB (PNG colour type 2), no alpha, no gamma or colour-profile chunks
Colours     exactly the 16 RGB values of the palette object passed in (the solve's own)
Aspect      multicolor: each logical pixel 2·zoom wide, zoom tall; hi-res: zoom × zoom
Scaling     pixel repetition only; never filtered or resampled
```
`png::preview(&BitmapImage, &Palette, zoom)` and `png::preview_indexed(&IndexedBitmap, …)` return the PNG bytes; `png::rgb` returns the raw pixels. The PNG is RGB, not indexed: the `image` crate cannot write a palette PNG and TAD §4 allows no other PNG crate. The output is byte-stable for a given `image` version (its default encoder settings, no metadata). At 1× every png2c64 baseline's preview equals png2c64's own `baseline.png` pixel for pixel, and every solve's equals the golden harness's rendering (`crates/convert64-formats/tests/png_preview.rs`, #160).

### FLI (`.fli`, raw layout as used by retropixels/Convert64)
```
0       2     Load address
2       8000  Screen RAM × 8 (one 1000-byte block per raster line offset 0–7)
8002    8000  Bitmap
16002   1000  Colour RAM
17002   1     Background colour
```
AFLI omits Colour RAM and background. The first three columns of every row are unusable on hardware.

## Palettes

### VICE palette (`.vpl`)
Plain text, one colour per line, 16 entries in index order (black first). This is what `Palette::from_vpl` accepts, following VICE's own loader (`palette_load_core` in VICE `src/palette.c`):
```
# comment                      a line whose first non-blank character is '#'
# NAME:Pepto (PAL)             optional; Convert64 uses it as the palette name
# TYPE:VICII                   optional; informational only
                               blank lines are ignored
68 37 2b                       red green blue, hexadecimal, 00–FF each
68 37 2b 0                     optional 4th value: dither nybble 0–F (older files; ignored)
```
- Values are separated by spaces or tabs. Each is one or more hex digits with an optional `0x` prefix, so `0`, `00` and `0x00` are equal; case does not matter.
- Line endings may be LF or CRLF.
- Exactly 16 entries are required. Fewer or more is an error.
- Anything after the colour and optional dither value is an error in Convert64. VICE only warns about it.

## Sprites

### Raw sprite block (64 bytes, 64-byte aligned)
```
0–62    21 rows × 3 bytes, bit 7 leftmost; hi-res 1 bit/pixel, multicolor 2 bits/double-pixel
63      Attribute byte (SpritePad convention; the hardware ignores it):
        bit 7 multicolor flag, bit 6 overlay flag, bits 0–3 sprite colour
```

### SpritePad project (`.spd`)
v1 (what Convert64 writes — simplest, widely readable):
```
0       3     "SPD"
3       1     Version $01
4       1     Sprite count − 1
5       1     Animation count − 1
6       1     Background colour
7       1     Multicolor 1
8       1     Multicolor 2
9       ...   Sprite blocks, 64 bytes each (attribute byte as above)
```
v2 (read-only support planned):
```
0       3     "SPD"
3       1     Version $02
4       2     Sprite count
6       2     Animation count
8       1     Background colour
9       1     Multicolor 1
10      1     Multicolor 2
11      1     Flags (bit 0 tile mode)
12      ...   Sprite blocks, then animation data
```
*Verify against SpritePad before relying on a reader; the header layouts above are community documentation, not an official spec.*

## Character sets, tiles and maps

### Raw charset export (CharPad-compatible layout)
```
charset.bin        n × 8 bytes (usually 2048)
charattribs.bin    n × 1 byte  (per-char colour / material, if the colour mode needs it)
tiles.bin          tiles × (w × h) char indices
tileattribs.bin    tiles × 1–2 bytes (colour + material)
map.bin            width × height × (1 | 2) bytes of tile indices (8-bit if ≤ 256 tiles, else 16-bit)
screen.bin         1000 bytes (when exporting a single 40×25 screen)
colour.bin         1000 bytes (per-cell colours for a single screen)
```

### CharPad project (`.ctm`) — read/write is best effort (phase 3)
v8/v9 header, as documented by the community:
```
0       3     "CTM"
3       1     Version ($08 / $09)
4       1     Background colour
5       1     Multicolor 1
6       1     Multicolor 2
7       1     Char colour (meaning depends on colour method)
8       1     Colour method: 0 global, 1 per tile, 2 per cell
9       1     Flags: bit 0 = multicolor
10      2     Char count − 1
12      2     Tile count − 1
14      1     Tile width − 1
15      1     Tile height − 1
16      2     Map width (tiles)
18      2     Map height (tiles)
20      ...   Char data (8 B each), char attribs, tile data, tile attribs, map data
```
The format has changed across versions and the authoritative definition is CharPad itself; always test-load exported `.ctm` files.

## PRG display stubs

A `.prg` is a 2-byte load address followed by data. Convert64's PRG exports prepend a tiny machine-code viewer (BASIC SYS line + a loop that sets `$DD00`, `$D011`, `$D016`, `$D018`, copies Colour RAM to `$D800` and sets `$D020/$D021`) in front of the picture data so the file runs directly in VICE or on hardware. Data placement follows the conventional load addresses in [vic-ii-constraints.md §5](vic-ii-constraints.md#5-memory-and-alignment-for-exports-and-prg-stubs).

### Koala / Art Studio display PRG (`convert64_formats::prg::display_prg`)

The picture file is embedded **verbatim**, its own 2-byte load address included, at a fixed file offset, so `prg[257..]` is the `.kla`/`.art` file byte for byte. `cargo xtask vice-check` runs these PRGs in VICE; the golden harness snapshots them (`fixtures/<name>/expected_prg.snap`).

```
Offset  C64 address    Size  Content
0       —              2     Load address $0801 ($01 $08)
2       $0801–$080C    12    BASIC: 10 SYS2061
                               0B 08     link to $080B (end of program)
                               0A 00     line number 10
                               9E        SYS token
                               32 30 36 31  "2061"
                               00        end of line
                               00 00     end of program
14      $080D–         —     Loader (main program, then the copy routine); Koala 120 bytes
                             ($080D–$0884), Art Studio 96 bytes ($080D–$086C)
        … –$08FF             Zero padding up to $0900
257     $0900–         n     The picture file verbatim: Koala 10003 bytes ($0900–$3012),
                             Art Studio 9002 or 9009 bytes ($0900–$2C29 / $2C30)
Total                        257 + n: Koala 10260, Art Studio 9259 (9266 padded)
```

The loader, at `SYS 2061` (`$080D`):

```
SEI                         IRQs off: no KERNAL interrupt writes to the screen
LDA $DD00 / AND #$FC / ORA #$02 / STA $DD00
                            VIC bank 1 ($4000–$7FFF); the other port A bits are kept
copy $0902 → $6000, 32 pages   bitmap (8000 bytes)
copy $2842 → $5C00,  4 pages   Screen RAM (1000 bytes)
copy $2C2A → $D800,  4 pages   Colour RAM (1000 bytes), Koala only
LDA #$78 / STA $D018        Screen RAM at bank + $1C00 (bits 4–7 = 7), bitmap at bank + $2000 (bit 3)
LDA #$3B / STA $D011        bitmap mode, display on, 25 rows, Y scroll 3
LDA #$18 / STA $D016        Koala: multicolor, 40 columns, X scroll 0
LDA #$08 / STA $D016        Art Studio: hi-res, 40 columns, X scroll 0
LDA $3012 / STA $D021       Koala: background (bit pair 00) from the file's last byte
LDA #$00 / STA $D021        Art Studio: $D021 is not shown in hi-res bitmap mode; black
LDA #$00 / STA $D020        border black (neither format stores a border colour)
JMP *                       loop forever
```

Each "copy" is `LDA #<src / STA $FB / LDA #>src / STA $FC / LDA #<dst / STA $FD / LDA #>dst / STA $FE / LDX #pages / JSR copy`, and the copy routine right after the main program is:

```
copy: LDY #$00
loop: LDA ($FB),Y / STA ($FD),Y / INY / BNE loop
      INC $FC / INC $FE / DEX / BNE loop
      RTS
```

Copy routine address: `$0874` (Koala), `$085C` (Art Studio). Zero page `$FB`–`$FE` is free for user programs.

**Why these destinations.** The payload occupies `$0900`–`$3012` at most. Bank 1 puts the bitmap at `$6000` and Screen RAM at `$5C00`, both above the payload, so the copies never overlap their source and a plain forward copy is safe; bank 1 also has no character-ROM shadow. The file's own load address (`$6000` Koala, `$2000` Art Studio) is ignored: the loader always copies. Whole pages are copied, so the bytes past each block land in memory the VIC-II does not show (`$7F40`–`$7FFF` after the bitmap, `$5FE8`–`$5FFF` after Screen RAM, `$DBE8`–`$DBFF` after Colour RAM).

## Assembler source we emit

Every export can be written as source for the four common cross-assemblers. The data is identical; only directives differ.

| Assembler | Program counter | Byte data | Include binary | Skip PRG header |
|---|---|---|---|---|
| KickAssembler | `*=$4800 "Charset"` | `.byte $01,$02` | `.import binary "file.bin"` | `.import c64 "file.prg"` |
| ACME | `*=$4800` | `!byte $01,$02` | `!binary "file.bin"` | `!binary "file.prg",,2` |
| 64tass | `*=$4800` | `.byte $01,$02` | `.binary "file.bin"` | `.binary "file.prg",2` |
| ca65 | `.segment "CHARSET"` | `.byte $01,$02` | `.incbin "file.bin"` | `.incbin "file.prg",2` |

Emitted labels follow `lowercase_with_underscores`; constants `UPPERCASE_WITH_UNDERSCORES`. A sprite export also emits pointer helpers, e.g. KickAssembler:
```
.function spritePtr(addr) { .return (addr - (addr & $c000)) / 64 }
```
C header exports emit `static const unsigned char name[] = { ... };` with a size constant.

### Bitmap pictures (`convert64_formats::asm::emit`)

`emit(Assembler, &BitmapImage, &EmitOptions)` returns `Vec<(PathBuf, Vec<u8>)>`: the source file first, then any file it includes. Paths are bare file names.

| Target (`Assembler`, id) | Source file | Program counter | Label definition | Constant |
|---|---|---|---|---|
| KickAssembler (`kickass`) | `<name>.asm` | `* = $6000 "<name>"` | `name_bitmap:` | `.const NAME_BITMAP_SIZE = 8000` |
| ACME (`acme`) | `<name>.a` | `* = $6000` | `name_bitmap` | `NAME_BITMAP_SIZE = 8000` |
| 64tass (`64tass`) | `<name>.tas` | `* = $6000` | `name_bitmap` | `NAME_BITMAP_SIZE = 8000` |
| ca65 (`ca65`) | `<name>.s` | `.segment "NAME"` | `name_bitmap:` | `NAME_BITMAP_SIZE = 8000` |
| C header (`c`) | `<name>.h` | — | `static const unsigned char name_bitmap[NAME_BITMAP_SIZE]` | `#define NAME_BITMAP_SIZE 8000` |

- **Data and addresses.** The data is the picture file's body in its own order, at the format's load address: Koala `bitmap` `$6000`, `screen` `$7F40`, `colour` `$8328`, `background` `$8710`; Art Studio `bitmap` `$2000`, `screen` `$3F40`. Assembling the source therefore gives the bytes `BitmapFormat::write` writes after its load address, and KickAssembler, ACME (`-f cbm`) and 64tass, which write a load address in front, produce the `.kla` / `.art` file itself. `EmitOptions::load_address` moves the data; ca65 leaves placement to the linker (the header comment shows a matching `ld65` config).
- **Names.** `EmitOptions::name` is the file stem. Labels are it lowercased with every character other than an ASCII letter or digit turned into `_` (prefixed `pic_` if it starts with a digit): `<name>_bitmap`, `<name>_screen`, `<name>_colour`, `<name>_background`. Constants are the same uppercased: `<NAME>_LOAD_ADDRESS`, `<NAME>_<BLOCK>_SIZE` per block, `<NAME>_BACKGROUND_COLOUR` (multicolor). The ca65 segment is `<NAME>`, the C include guard `<NAME>_H`.
- **Data styles** (`DataStyle`). `Bytes`: byte lines, 16 bytes per line, lowercase hex (`$0b`, or `0x0b` in C). `IncludeRaw`: one binary include per block of the [raw bitmap set](#raw-bitmap-set-convert64---format-raw) (`<name>.bitmap.bin`, …), which are returned with the source. `IncludePicture`: one include of `<name>.kla` / `<name>.art` that skips its load address, the other labels defined as offsets from `<name>_bitmap`; the picture file is returned with the source. A C header always holds the bytes, since C has no standard binary include.
- **Header comment.** Name, mode and layout, then the address and size of each block, then how to assemble.

Tests (`crates/convert64-formats/tests/asm_emit.rs`, #161): every target and data style on every fixture baseline yields the binary writer's bytes (read back by a small parser of the emitted directives); golden text snapshots of each target (`tests/snapshots/asm_emit__*.snap`); where ACME, 64tass, ca65/ld65, KickAssembler (`KICKASS_JAR`) or a C compiler (`cc`) is installed, the real tool's output is compared too, otherwise skipped.

---
*Layouts cross-checked against retropixels' documented writers, the Koala/Art Studio/Doodle conventions used by VICE and common viewers, and community documentation of SpritePad/CharPad project files. Corrections welcome with a VICE or hardware screenshot.*
