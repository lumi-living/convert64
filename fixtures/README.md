# Fixtures

Source images for golden tests. Each folder holds `source.png` (the input) and a `README.md` stating origin, licence and what the image tests. Golden outputs (`expected.*`) arrive with #40–#41; png2c64 baselines (`baseline.*`) are described below.

| Fixture | Mode exercised | Why |
|---|---|---|
| [portrait](portrait/) | multicolor bitmap | skin tones, same-luma dither pairs, cell clashes |
| [terrain](terrain/) | charset / tiles | 960 candidate chars vs the 256 budget, per-tile colours |
| [title](title/) | multicolor bitmap | smooth sky gradient + detailed foreground; per-layer recipes |
| [gradient](gradient/) | all | deterministic regression chart with exact Pepto swatches |
| [photo](photo/) | multicolor bitmap | dynamic range, natural texture, metric choice |
| [lineart](lineart/) | hi-res bitmap | two colours per cell, thin strokes |

Palette test files (`.vpl`) live in [palettes/](palettes/), generated from the reference table.

Rules for new fixtures are in [CONTRIBUTING.md](../CONTRIBUTING.md): rights-clear only, PNG, ≤ 2 MB, 16:10, and a README. Keep `fixtures/_local/` (git-ignored) for anything you cannot redistribute.

## png2c64 baselines

Each fixture has a baseline made by [png2c64](https://github.com/tinic/png2c64), the solver-quality bar named in `docs/PDD.md`. Convert64's solver is compared against it (`docs/TAD.md` §6).

| File | What it is |
|---|---|
| `baseline.png` | png2c64's 320×200 preview, rendered with its Pepto palette (equal to `Palette::pepto_pal()`) |
| `baseline.kla` | multicolor fixtures: Koala file, load address `$6000` |
| `baseline.art` | `lineart` only: Art Studio file, load address `$2000` |
| `vice.png` | VICE `x64sc` screenshot of the baseline (384×272 with border), from `cargo xtask vice-check` |

Regenerate all of them with one command:

```sh
cargo xtask png2c64-baselines --png2c64 /path/to/png2c64
```

The settings and the png2c64 version and commit used are recorded in each fixture's README, together with the result of `convert64 validate`.

**Where the `.kla`/`.art` bytes come from.** png2c64's command-line tool writes the PNG and a display PRG. Its Koala and Art Studio writers are only reachable from its web API. The display PRG holds exactly the bytes those writers would emit: the displayer loads at `$0801` and the picture data sits at `$2000`, which is file offset 6145. The data is in Koala order (bitmap, screen, colour, background) or Art Studio order (bitmap, screen). The task cuts that block out and puts the format's load address in front. It then checks the cut by decoding the file and comparing every pixel with png2c64's own PNG, and fails on any difference.

**Build used for the committed baselines.** png2c64 commit `1265329a573b1dfa3c2e0f3473d2aa15ffd7ac3f` (version 1.0.0.76), built with Homebrew GCC 15.3.0 and CMake 4.4.4 on macOS (arm64); its own 30 tests passed. GCC 16.2 does not build that commit, because png2c64 compiles with warnings as errors and GCC 16 reports two ignored return values in its interactive mode.

**Checked in VICE.** `cargo xtask vice-check` runs each baseline in VICE `x64sc`, through the display PRG `convert64-formats` builds from it, and compares the screenshot, `vice.png`, with the baseline. All six agree on 100 % of pixels; each fixture README records the VICE version, command and result. See `docs/TAD.md` §6.
