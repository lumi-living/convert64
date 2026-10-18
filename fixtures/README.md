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

Layered compositor scenes (E4) live in [composite/](composite/): each `scene.toml` stacks the sources above with transforms, masks, blend modes and regions, and the golden harness compares the 1× composite and the attribute-map debug render.

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

**The parity bar (#181, #190).** `convert64 convert source.png` with a baseline's settings (below in each fixture README) reproduces `baseline.png` on ≥ 99 % of its pixels: all six are 100 % except `title` at 99.997 %. That's because a plain picture takes png2c64's own front end and the reference solver (`docs/TAD.md` §2, §6). CI checks it in the `png2c64 parity` step, together with png2c64's `--sharpen 0.5` and `--sharpen -0.5` renders of the portrait (`fixtures/portrait/baseline_sharpen_*.png`, #228) at ≥ 99.9 %. `cargo xtask front-end-sheet` shows what the supersampled composite would give instead.

**Checked in VICE.** `cargo xtask vice-check` runs each baseline in VICE `x64sc`, through the display PRG `convert64-formats` builds from it, and compares the screenshot, `vice.png`, with the baseline. All six agree on 100 % of pixels; each fixture README records the VICE version, command and result. See `docs/TAD.md` §6.

## png2amiga baselines (the png2amiga solver's references)

[png2amiga/](png2amiga/) holds [png2amiga](https://github.com/tinic/png2amiga)'s C64 renders of each fixture's `source.png` (#261), the references of the png2amiga port's parity tests. Since [ADR-014](../docs/adr/ADR-014-png2c64-stays-reference.md) they are not gated as the fixtures' baselines (png2c64's above are); they test the port, not quality. Per fixture, `png2amiga/<name>/`:

| File | What it is |
|---|---|
| `baseline.kla` / `baseline.art` | png2amiga's `.koa` / `.hir` file (Koala / Art Studio layout) |
| `baseline.png` | that file drawn at 320 × 200 through Pepto |
| `baseline_match_range.png` | with `--match-range` (png2amiga's hull) |
| `baseline_refine.png` | with `--cell-refine` |
| `portrait/baseline_sharpen_±0.5.png` | with `--sharpen 0.5` / `--sharpen -0.5` |
| `vice.png` | VICE `x64sc` screenshot of the baseline from when it was gated (#273), 100 % |

**Flags.** Convert64's defaults, spelled out (png2amiga's own differ): multicolor `--mode c64-multicolor --c64-palette pepto --c64-metric mse --dither checker --dither-strength 0.7 --error-clamp 0.1 --gamma 1.5`, hi-res (lineart) `--mode c64-hires --c64-palette pepto --c64-metric mse --dither none --gamma 1.5`. Each `png2amiga/<name>/README.md` records the exact command, the tool version and png2amiga's own SSIMULACRA2 of each render.

**The pinned build.** png2amiga commit `4a8777653536ac07074a650ee86860347c8755fb` (v1.111.2), Homebrew GCC 15.3.0 on macOS (arm64), `-ffp-contract=off` appended after png2amiga's own `-ffp-contract=fast` and the maths shim `docs/reference/png2amiga-reference/crmath.c` ([png2amiga-reference.md](../docs/reference/png2amiga-reference.md)). Regenerate the renders (and the method previews in `docs/reference/dither-previews/png2amiga-*/`) on a developer machine with `cargo xtask png2amiga build` then `cargo xtask png2amiga renders`. CI never builds png2amiga: its tests read these committed files.

**The parity tests.** `crates/convert64-core/tests/png2amiga_parity.rs` (224 cases, all 100 %) and `crates/convert64-cli/tests/png2amiga_convert.rs` (`convert64 convert --solver png2amiga`, 20 cases, all 100 %), in CI's `png2amiga parity` step.

## Eye-ranked pairs

[eye-rankings.toml](eye-rankings.toml) holds the pairs of pictures Jaakko ranked by eye on #273's review sheets; `cargo xtask metric-check` scores candidate quality metrics against them (`docs/TAD.md` §6). No metric is a gate unless it agrees with the eye on ≥ 90 % of them (ADR-014 §7). Edit it only to add pairs a human judged.
