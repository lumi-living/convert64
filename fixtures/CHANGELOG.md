# Golden fixture changelog

Every change to a golden file (`fixtures/<name>/expected_*.snap`, `fixtures/<name>/expected.png`) is recorded here, one line per fixture: date, the commit whose code produced the new goldens, the files, and why. Lines are appended by `cargo xtask regen-goldens --accept --reason "..."`; see [CONTRIBUTING.md](../CONTRIBUTING.md#golden-tests). Do not edit or remove existing lines.

- 2026-10-11 `ed223a3b2113` **gradient** (expected_kla.snap, expected.png): initial goldens from #40, the identity solver stub on the png2c64 baseline
- 2026-10-11 `ed223a3b2113` **lineart** (expected_art.snap, expected.png): initial goldens from #40, the identity solver stub on the png2c64 baseline
- 2026-10-11 `ed223a3b2113` **photo** (expected_kla.snap, expected.png): initial goldens from #40, the identity solver stub on the png2c64 baseline
- 2026-10-11 `ed223a3b2113` **portrait** (expected_kla.snap, expected.png): initial goldens from #40, the identity solver stub on the png2c64 baseline
- 2026-10-11 `ed223a3b2113` **terrain** (expected_kla.snap, expected.png): initial goldens from #40, the identity solver stub on the png2c64 baseline
- 2026-10-11 `ed223a3b2113` **title** (expected_kla.snap, expected.png): initial goldens from #40, the identity solver stub on the png2c64 baseline
