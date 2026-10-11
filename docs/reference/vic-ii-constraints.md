# VIC-II constraints

Everything Convert64 outputs must satisfy this document. The validator in `convert64-core` is the executable form of it.

## 1. Palette

The C64 has exactly 16 colours. They cannot be changed. Convert64 ships the three presets below and lets users load `.vpl` palettes, but **index semantics never change**: a palette only decides which RGB value an index is shown as.

| Index | Name | Pepto (PAL) | Pepto (NTSC) | Colodore (default) | Luma |
|---|---|---|---|---|---|
| 0 | Black | `#000000` | `#000000` | `#000000` | 0 |
| 1 | White | `#FFFFFF` | `#FFFFFF` | `#FFFFFF` | 31 |
| 2 | Red | `#68372B` | `#67372B` | `#813338` | 10 |
| 3 | Cyan | `#70A4B2` | `#70A3B1` | `#75CEC8` | 20 |
| 4 | Purple | `#6F3D86` | `#6F3D86` | `#8E3C97` | 12 |
| 5 | Green | `#588D43` | `#588C42` | `#56AC4D` | 16 |
| 6 | Blue | `#352879` | `#342879` | `#2E2C9B` | 8 |
| 7 | Yellow | `#B8C76F` | `#B7C66E` | `#EDF171` | 24 |
| 8 | Orange | `#6F4F25` | `#6F4E25` | `#8E5029` | 12 |
| 9 | Brown | `#433900` | `#423800` | `#553800` | 8 |
| 10 | Light Red | `#9A6759` | `#996659` | `#C46C71` | 16 |
| 11 | Dark Grey | `#444444` | `#434343` | `#4A4A4A` | 10 |
| 12 | Medium Grey | `#6C6C6C` | `#6B6B6B` | `#7B7B7B` | 15 |
| 13 | Light Green | `#9AD284` | `#9AD183` | `#A9FF9F` | 24 |
| 14 | Light Blue | `#6C5EB5` | `#6B5EB5` | `#706DEB` | 15 |
| 15 | Light Grey | `#959595` | `#959595` | `#B2B2B2` | 20 |

Sources:
- **Pepto (PAL)** and **Pepto (NTSC)**: Pepto's measurements (pepto.de/projects/colorvic) as shipped by VICE in `data/C64/pepto-pal.vpl` and `data/C64/pepto-ntsc.vpl`.
- **Colodore (default)**: the output of colodore.com's palette generator at its default settings (brightness 50, contrast 100, saturation 50). Convert64 reproduces the generator exactly, so other slider settings are available too. For example, VICE's `data/C64/colodore.vpl` is the generator at brightness 47, contrast 100, saturation 70.

*Correction (2026-10-11):* earlier versions of this table labelled the Colodore column "Hex (Pepto)". The 16 values themselves are unchanged. The port of colodore.com's `renderPalette()` in `convert64-core` (`palette::colodore`) reproduces them byte for byte at the default settings, and reproduces VICE's `colodore.vpl` at brightness 47 and saturation 70. The Pepto columns were added from the VICE files named above.

Luma is on a 0–31 scale. Colodore's generator uses the same levels with white at 32, which groups the colours identically.

**Luminance groups** (nine levels). Colours in the same group blend cleanly when dithered or alternated on PAL displays, which the quantiser can reward:

| Luma | Colours |
|---|---|
| 0 | Black |
| 8 | Blue, Brown |
| 10 | Red, Dark Grey |
| 12 | Purple, Orange |
| 15 | Medium Grey, Light Blue |
| 16 | Green, Light Red |
| 20 | Cyan, Light Grey |
| 24 | Yellow, Light Green |
| 31 | White |

## 2. Display modes

Mode bits: `$D011` bit 5 = BMM (bitmap), bit 6 = ECM; `$D016` bit 4 = MCM (multicolor). Screen is 40×25 cells of 8×8 pixels = 1000 cells. Multicolor modes have double-wide pixels (effective 160×200, 2:1 pixel aspect).

| Mode | ECM/BMM/MCM | Logical res | Pixel aspect | Colours per cell | Global colours | Per-cell colour sources |
|---|---|---|---|---|---|---|
| Standard text / hi-res charset | 0/0/0 | 320×200 | 1:1 | 2 per 8×8 | bg `$D021` | fg = Colour RAM (0–15) |
| Multicolor text / charset | 0/0/1 | 160×200 | 2:1 | 4 per 4×8 | bg `$D021`, MC1 `$D022`, MC2 `$D023` | char colour = Colour RAM bits 0–2 (**0–7 only**; bit 3 set selects multicolor for that char) |
| ECM text | 1/0/0 | 320×200 | 1:1 | 2 per 8×8 | 4 bgs `$D021–$D024` | fg = Colour RAM; bg chosen by char code bits 6–7 → **only 64 unique chars** |
| Hi-res bitmap | 0/1/0 | 320×200 | 1:1 | 2 per 8×8 | none | bit 1 = Screen RAM high nybble, bit 0 = Screen RAM low nybble |
| Multicolor bitmap | 0/1/1 | 160×200 | 2:1 | 4 per 4×8 | bg `$D021` | `01` = Screen high nybble, `10` = Screen low nybble, `11` = Colour RAM |

### Bitmap memory layout
Bitmap data is organised by cell, not by scanline:
```
addr = base + (cell_row * 320) + (cell_col * 8) + pixel_row
```
Cell (0,0) = bytes 0–7, cell (1,0) = bytes 8–15, … cell (0,1) = bytes 320–327. Bit 7 is the leftmost pixel.

### Charset modes
- A character set is 256 × 8 bytes = 2048 bytes; each char is 8 rows, bit 7 leftmost.
- Screen RAM (1000 bytes) holds char codes; Colour RAM (1000 nybbles at `$D800`, fixed, cannot be relocated or double-buffered) holds the per-cell colour.
- Unique-character budget: **256** (64 in ECM) — a global constraint across the whole screen/map, unlike the per-cell colour limits.
- In multicolor charset mode a single screen can mix hi-res chars (Colour RAM 0–7) and multicolor chars (8–15).

## 3. Sprites

- 8 hardware sprites, 24×21 pixels (hi-res) or 12×21 double-wide pixels (multicolor). Max 8 per scanline without multiplexing.
- 64 bytes each: 21 rows × 3 bytes = 63 bytes of pixels + 1 padding/attribute byte. Must start on a 64-byte boundary. Pointer = `(address − vic_bank_base) / 64`, stored at `screen_base + $03F8 + n`.
- Hi-res encoding: `0` transparent, `1` sprite colour (`$D027+n`, 0–15).
- Multicolor encoding (bit pairs): `00` transparent, `01` MC1 `$D025`, `10` sprite colour `$D027+n`, `11` MC2 `$D026`. **MC1 and MC2 are shared by all eight sprites.**
- Overlay: a hi-res sprite placed at the same coordinates as a multicolor sprite yields up to 7 colours (3 + 1 + transparent, plus the per-sprite colour of each). Lower sprite number has priority.
- Expansion (`$D017` Y, `$D01D` X) doubles size without adding data.

## 4. FLI family (phase 3)

- **FLI**: Screen RAM is re-pointed every raster line, so the `01`/`10` colours change per 4×1 row while Colour RAM (`11`) stays per 4×8 and bg stays global. Data: 8 × 1000 bytes Screen RAM + 8000 bitmap + 1000 Colour RAM + bg. The first three character columns of each row show garbage ("FLI bug") and are normally hidden by sprites or masked.
- **AFLI**: the hi-res equivalent (2 colours per 8×1).
- **IFLI / MCI / NUFLI**: two frames alternated at 50 Hz; perceived colour is the blend; flicker on real hardware, needs a blended preview.

## 5. Memory and alignment (for exports and PRG stubs)

| Data | Alignment | Notes |
|---|---|---|
| Character set | 2 KB (`$0800`) | `$D018` bits 1–3 select `base + n*$0800` |
| Screen RAM | 1 KB (`$0400`) | `$D018` bits 4–7 select `base + n*$0400` |
| Bitmap | 8 KB (`$2000`) | `$D018` bit 3 selects which half of the 16 KB bank |
| Sprite | 64 B | pointer = offset / 64 |
| Colour RAM | fixed `$D800–$DBE7` | not in a VIC bank |

The VIC-II sees one 16 KB bank at a time, selected by CIA2 `$DD00` bits 0–1: `11` = bank 0 `$0000–$3FFF` (default), `10` = bank 1 `$4000–$7FFF`, `01` = bank 2 `$8000–$BFFF`, `00` = bank 3 `$C000–$FFFF`.

**Character ROM shadow:** in banks 0 and 2 the VIC reads the character ROM instead of RAM at `$1000–$1FFF` and `$9000–$9FFF`. Never place a custom charset or bitmap there. Bank 1 (`$4000–$7FFF`) avoids the problem entirely and is the recommended default for exported assembly snippets.

Conventional load addresses: Koala bitmap `$6000`; Art Studio bitmap `$2000`; Doodle screen `$5C00` + bitmap `$6000`.

## 6. Validator rules (what `convert64 validate` checks)

Rules 1–5 are implemented in `convert64-core` (`validate` module and the image-model constructors). Each is covered by hand-built tests in `crates/convert64-core/tests/rules.rs`, listed under the rule. Never weaken one of these tests to make a change pass.

1. Colours per cell never exceeded, counting global colours: 2 (hi-res modes) or 4 (multicolor modes).
   Tests: `rule1_mc_bitmap_rejects_five_colours_in_a_cell`, `rule1_mc_bitmap_rejects_four_own_colours_without_background_pixels`, `rule1_hires_bitmap_rejects_three_colours_in_a_cell`, `rule1_encoded_bitmaps_accept_any_bytes`.
2. All colour values are palette indices 0–15; multicolor-charset per-char colour is 0–7.
   Tests: `rule2_mc_charset_rejects_char_colour_9`, `rule2_mc_charset_rejects_hires_char_colour_above_7`, `rule2_mc_bitmap_rejects_background_16`, `rule2_mc_sprite_rejects_multicolor_register_16`, `rule2_any_mode_rejects_colour_index_16`.
3. Charset outputs respect the unique-character budget (256, or 64 for ECM) and reference only existing chars.
   Tests: `rule3_hires_charset_rejects_257_chars`, `rule3_mc_charset_rejects_257_chars`, `rule3_ecm_charset_rejects_65_chars`, `rule3_hires_charset_rejects_a_missing_char`, `rule3_mc_charset_rejects_a_missing_char`, `rule3_ecm_charset_rejects_a_missing_char`, `rule3_ecm_charset_accepts_any_background_bits`.
4. Sprite data is exactly 64 bytes per sprite; MC1/MC2 are single values for the whole set; overlay pairs share coordinates.
   Tests: `rule4_hires_sprite_rejects_63_byte_block`, `rule4_mc_sprite_rejects_partial_second_block`, `rule4_overlay_sprite_rejects_different_coordinates`, `rule4_overlay_sprite_rejects_hires_base`, `rule4_overlay_sprite_rejects_a_missing_sprite`.
5. Exported addresses satisfy the alignment table and avoid the char-ROM shadow ranges.
   Tests: `rule5_sprite_rejects_misaligned_block`, `rule5_charset_rejects_char_rom_shadow`, `rule5_charset_rejects_misaligned_address`, `rule5_bitmap_rejects_char_rom_shadow_in_bank_2`, `rule5_bitmap_rejects_misaligned_address`, `rule5_screen_rejects_misaligned_address`, `rule5_colour_ram_rejects_relocation`, `rule5_bank3_accepts_the_shadow_free_range`.
6. File sizes match the layouts in [file-formats.md](file-formats.md) byte for byte.

## 7. Timing facts the preview may use

PAL: 312 lines/frame, 63 cycles/line, ~50.12 Hz; NTSC: 263 lines, 65 cycles/line, ~59.83 Hz. Display area is raster lines 51–250 in 25-row mode. Convert64 renders the 320×200 display area plus a border; it does not emulate timing.

---
*Palette values: Philip "Pepto" Timmermann's measurements (pepto.de), as used by VICE and Colodore. Mode and register facts: the public VIC-II documentation (Christian Bauer's "The MOS 6567/6569 video controller" and the C64 Programmer's Reference Guide).*
