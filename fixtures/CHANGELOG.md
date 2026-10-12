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
- 2026-10-11 #125 **composite/portrait-over-title** (expected_1x.png, expected_attr.png): half-res feather from σ ≥ 4 target px (#98)
- 2026-10-11 #135 **gradient** (expected.png): real solver replaces the identity stub; recipe closest to png2c64 (#111)
- 2026-10-11 #135 **lineart** (expected.png): real solver replaces the identity stub; recipe closest to png2c64 (#111)
- 2026-10-11 #135 **photo** (expected.png): real solver replaces the identity stub; recipe closest to png2c64 (#111)
- 2026-10-11 #135 **portrait** (expected.png): real solver replaces the identity stub; recipe closest to png2c64 (#111)
- 2026-10-11 #135 **terrain** (expected.png): real solver replaces the identity stub; recipe closest to png2c64 (#111)
- 2026-10-11 #135 **title** (expected.png): real solver replaces the identity stub; recipe closest to png2c64 (#111)
- 2026-10-12 #143 **portrait** (expected_adaptive.png, expected_fs-clamped.png, expected_fs-unclamped.png, expected_hysteresis.png, expected_luma0.png): solver knob variants on portrait (#123): cell_clamp on/off, max_luma_diff 0, adaptive 0.5, hysteresis 0.97
