# File formats

Byte layouts Convert64 reads and writes. Offsets are decimal unless prefixed `$`. All multi-byte values are little-endian. "Load address" means the two-byte PRG header most C64 picture formats carry.

## Bitmap pictures

### Koala Painter (`.kla`, `.koa`) — multicolor bitmap
```
Offset  Size  Content
0       2     Load address $6000 ($00 $60)
2       8000  Bitmap
8002    1000  Screen RAM   (per cell: high nybble = bit pair 01, low nybble = 10)
9002    1000  Colour RAM   (per cell: low nybble = bit pair 11)
10002   1     Background colour (bit pair 00)
Total   10003
```

### Art Studio (`.art`, `.aas`) — hi-res bitmap
```
0       2     Load address $2000
2       8000  Bitmap
8002    1000  Screen RAM (high nybble = bit 1 colour, low nybble = bit 0 colour)
Total   9002
```
Some files carry 7 trailing padding bytes (9009 total); readers should accept both.

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
Separate files with no headers, plus a small manifest:
```
<name>.bitmap.bin   8000 bytes
<name>.screen.bin   1000 bytes
<name>.colour.bin   1000 bytes   (multicolor only)
<name>.bg.bin       1 byte       (multicolor only)
<name>.json         mode, palette preset, suggested load addresses
```

### FLI (`.fli`, raw layout as used by retropixels/Convert64)
```
0       2     Load address
2       8000  Screen RAM × 8 (one 1000-byte block per raster line offset 0–7)
8002    8000  Bitmap
16002   1000  Colour RAM
17002   1     Background colour
```
AFLI omits Colour RAM and background. The first three columns of every row are unusable on hardware.

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

---
*Layouts cross-checked against retropixels' documented writers, the Koala/Art Studio/Doodle conventions used by VICE and common viewers, and community documentation of SpritePad/CharPad project files. Corrections welcome with a VICE or hardware screenshot.*
