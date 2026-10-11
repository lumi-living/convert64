# Palette fixtures (`.vpl`)

Test input for the `.vpl` parser in `convert64-core` (`palette::vpl`). They use VICE's palette file syntax but contain no VICE material. They are MIT-licensed like the rest of the repository; see [LICENSE](../../LICENSE).

| File | Source of the values | Syntax it exercises |
|---|---|---|
| `pepto-pal.vpl` | [`docs/reference/vic-ii-constraints.md`](../../docs/reference/vic-ii-constraints.md) §1, column "Pepto (PAL)" | lowercase two-digit hex, a comment and a blank line per entry |
| `pepto-ntsc.vpl` | same table, column "Pepto (NTSC)" | uppercase hex without leading zeros, double spaces, optional dither nybble |
| `colodore-default.vpl` | same table, column "Colodore (default)" | compact: comment then entry, no blank lines |
| `colodore-b47-s70.vpl` | `convert64-core`'s own Colodore port, `Palette::colodore(47, 100, 70)` | compact |

The first three were generated from the reference table's columns on 2026-10-11. The fourth was generated from the port's output. The tests assert that the parsed palettes equal `Palette::pepto_pal()`, `Palette::pepto_ntsc()`, `Palette::colodore(50, 100, 50)` and `Palette::colodore(47, 100, 70)`, names included. If the reference table changes, regenerate these files from it.
