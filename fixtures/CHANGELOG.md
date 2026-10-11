# Golden fixture changelog

Every change to a golden file (`fixtures/<name>/expected_*.snap`, `fixtures/<name>/expected*.png`, `fixtures/composite/<scene>/expected_*.png`) is recorded here, one line per fixture: date, the pull request that brought the new goldens, the files, and why. Lines are appended by `cargo xtask regen-goldens --accept --reason "..." --pr N`; see [CONTRIBUTING.md](../CONTRIBUTING.md#golden-tests). Do not edit or remove existing lines.

*Corrected 2026-10-11 (#99):* the first entries cited branch commit hashes, which squash merges make unreachable from main; they now cite the pull requests those commits were merged in (`ed223a3b2113` → #70, `0fcd8cf48298` → #89, `9ce9d72e12e6` → #93).

- 2026-10-11 #70 **gradient** (expected_kla.snap, expected.png): initial goldens from #40, the identity solver stub on the png2c64 baseline
- 2026-10-11 #70 **lineart** (expected_art.snap, expected.png): initial goldens from #40, the identity solver stub on the png2c64 baseline
- 2026-10-11 #70 **photo** (expected_kla.snap, expected.png): initial goldens from #40, the identity solver stub on the png2c64 baseline
- 2026-10-11 #70 **portrait** (expected_kla.snap, expected.png): initial goldens from #40, the identity solver stub on the png2c64 baseline
- 2026-10-11 #70 **terrain** (expected_kla.snap, expected.png): initial goldens from #40, the identity solver stub on the png2c64 baseline
- 2026-10-11 #70 **title** (expected_kla.snap, expected.png): initial goldens from #40, the identity solver stub on the png2c64 baseline
- 2026-10-11 #89 **gradient** (expected_prg.snap): new display-PRG snapshots from #81: convert64-formats::prg on the png2c64 baseline
- 2026-10-11 #89 **lineart** (expected_prg.snap): new display-PRG snapshots from #81: convert64-formats::prg on the png2c64 baseline
- 2026-10-11 #89 **photo** (expected_prg.snap): new display-PRG snapshots from #81: convert64-formats::prg on the png2c64 baseline
- 2026-10-11 #89 **portrait** (expected_prg.snap): new display-PRG snapshots from #81: convert64-formats::prg on the png2c64 baseline
- 2026-10-11 #89 **terrain** (expected_prg.snap): new display-PRG snapshots from #81: convert64-formats::prg on the png2c64 baseline
- 2026-10-11 #89 **title** (expected_prg.snap): new display-PRG snapshots from #81: convert64-formats::prg on the png2c64 baseline
- 2026-10-11 #93 **composite/portrait-over-title** (expected_1x.png, expected_attr.png): E4 #85: first composite scenes (portrait-over-title, title-multiply-hires): 1× composite and attribute-map debug render
- 2026-10-11 #93 **composite/title-multiply-hires** (expected_1x.png, expected_attr.png): E4 #85: first composite scenes (portrait-over-title, title-multiply-hires): 1× composite and attribute-map debug render
- 2026-10-11 #101 **composite/title-multiply-hires** (expected_attr.png): opacity now scales strength (#99)
