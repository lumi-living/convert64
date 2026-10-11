# VICE palette files (test input)

Unmodified copies of three palette files from the VICE emulator, used only as input for the `.vpl` parser tests in `convert64-core` (`palette::vpl`). They are read at compile time by the tests and are not part of any Convert64 binary.

| File | VICE path | SHA-256 |
|---|---|---|
| `pepto-pal.vpl` | `vice/data/C64/pepto-pal.vpl` | `8601a72d62687450e5b015fd78974d6fdf008ec17ee25c8a234b959668ae032a` |
| `pepto-ntsc.vpl` | `vice/data/C64/pepto-ntsc.vpl` | `10763039b2282e24f737c6ffae40c92e68407e13a5eda76acfd67612591d2365` |
| `colodore.vpl` | `vice/data/C64/colodore.vpl` | `3ac258746a7efe8ca1e5c863dab8bdcdc297049a99ca127e51423f1fc957273a` |

Fetched on 2026-10-11 from the VICE Subversion mirror at <https://github.com/VICE-Team/svn-mirror> (trunk at commit `e9dd3c16934a`).

## Licence

These three files come from VICE and are distributed under VICE's licence, the **GNU General Public License, version 2 or later**, not under this repository's MIT licence. The palette values they contain are measurements published by Philip "Pepto" Timmermann (pepto.de, colodore.com); the same values are restated as facts in [`docs/reference/vic-ii-constraints.md`](../../../docs/reference/vic-ii-constraints.md) §1. Do not copy these files into any Convert64 crate or release artifact.
