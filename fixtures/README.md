# Fixtures

Source images for golden tests. Each folder holds `source.png` (the input) and a `README.md` stating origin, licence and what the image tests. Golden outputs (`expected.*`) arrive with #40–#41; the png2amiga baselines (`baseline.*`, the reference since ADR-013) are described below.

| Fixture | Mode exercised | Why |
|---|---|---|
| [portrait](portrait/) | multicolor bitmap | skin tones, same-luma dither pairs, cell clashes |
| [terrain](terrain/) | charset / tiles | 960 candidate chars vs the 256 budget, per-tile colours |
| [title](title/) | multicolor bitmap | smooth sky gradient + detailed foreground; per-layer recipes |
| [gradient](gradient/) | all | deterministic regression chart with exact Pepto swatches |
| [photo](photo/) | multicolor bitmap | dynamic range, natural texture, metric choice |
| [lineart](lineart/) | hi-res bitmap | two colours per cell, thin strokes |

Palette test files (`.vpl`) live in [palettes/](palettes/), generated from the reference table.

[eye-rankings.toml](eye-rankings.toml) holds the pairs of pictures Jaakko ranked by eye on #273's review sheets; `cargo xtask metric-check` scores candidate quality metrics against them (TAD §6). Edit it only to add pairs a human judged.

Layered compositor scenes (E4) live in [composite/](composite/): each `scene.toml` stacks the sources above with transforms, masks, blend modes and regions, and the golden harness compares the 1× composite and the attribute-map debug render.

Rules for new fixtures are in [CONTRIBUTING.md](../CONTRIBUTING.md): rights-clear only, PNG, ≤ 2 MB, 16:10, and a README. Keep `fixtures/_local/` (git-ignored) for anything you cannot redistribute.

## png2amiga baselines

Each fixture has a baseline made by [png2amiga](https://github.com/tinic/png2amiga)'s C64 path, the reference since ADR-013 (#261; png2c64's are archived below). Convert64's reference solver is a port of it and must reproduce these (`docs/TAD.md` §6).

| File | What it is |
|---|---|
| `baseline.kla` | multicolor fixtures: png2amiga's `.koa` file (Koala, load address `$6000`) |
| `baseline.art` | `lineart` only: png2amiga's `.hir` file (Art Studio layout, load address `$2000`) |
| `baseline.png` | that file drawn at 320 × 200 through Pepto (equal to `Palette::pepto_pal()`) |
| `baseline_match_range.png` | with `--match-range` (png2amiga's per-(L, hue) chroma stretch) |
| `baseline_refine.png` | with `--cell-refine` (the Refine step) |
| `portrait/baseline_sharpen_±0.5.png` | with `--sharpen 0.5` / `--sharpen -0.5` |
| `vice.png` | VICE `x64sc` screenshot of the baseline (384×272 with border), from `cargo xtask vice-check` |

Regenerate all of them, and every method's previews in `docs/reference/dither-previews/png2amiga-*/`, with:

```sh
cargo xtask png2amiga build      # once: clones and builds the pinned reference
cargo xtask png2amiga renders
```

**The pinned build.** png2amiga commit `4a8777653536ac07074a650ee86860347c8755fb` (v1.111.2), built with Homebrew GCC 15.3.0 and CMake 4.4.4 on macOS (arm64), `CMAKE_BUILD_TYPE=Release`, with **`-ffp-contract=off`** appended after png2amiga's own `-ffp-contract=fast` (a compiler launcher) and the maths shim `docs/reference/png2amiga-reference/crmath.c` linked in. No other shim was needed. Why, and how much the default flags change: [png2amiga-reference.md](../docs/reference/png2amiga-reference.md). Moving the pin is a deliberate PR that regenerates every render (ADR-013).

**Settings.** The baselines use Convert64's defaults, spelled out (png2amiga's own differ: Floyd–Steinberg, error clamp 0.35, gamma 1.0, the `blur` metric, the Colodore palette): multicolor `--dither checker --dither-strength 0.7 --error-clamp 0.1 --gamma 1.5 --c64-metric mse --c64-palette pepto`, hi-res `--dither none --gamma 1.5 --c64-metric mse --c64-palette pepto`. Each fixture README records the exact command, the tool version and png2amiga's own SSIMULACRA2 of each render.

**The parity bar (#261).** The reference solver reproduces every render bit for bit (`crates/convert64-core/tests/png2amiga_parity.rs`, 224 cases, ≥ 99 % required), and `convert64 convert source.png` with a baseline's settings reproduces the fixture renders (`crates/convert64-cli/tests/png2amiga_convert.rs`). CI checks both in the `png2amiga parity` step.

**Checked in VICE.** `cargo xtask vice-check` runs each baseline in VICE `x64sc`, through the display PRG `convert64-formats` builds from it, and compares the screenshot, `vice.png`, with the baseline. All six agree on 100 % of pixels; each fixture README records the VICE version, command and result. See `docs/TAD.md` §6.

## png2c64 baselines (archived)

Before ADR-013 the baselines were [png2c64](https://github.com/tinic/png2c64)'s. They are kept in [archive/png2c64/](archive/png2c64/) (`baseline.kla` / `.art`, `baseline.png`, the portrait's `--sharpen` renders, the old `vice.png`), each with its README section, and no test gates them; `crates/convert64-core/tests/png2c64_parity.rs` still compares the png2c64 port with them when run with `--ignored`. Regenerate them with `cargo xtask png2c64-baselines --png2c64 /path/to/png2c64`, which now writes into the archive.

**Where the `.kla`/`.art` bytes come from.** png2c64's command-line tool writes the PNG and a display PRG. Its Koala and Art Studio writers are only reachable from its web API. The display PRG holds exactly the bytes those writers would emit: the displayer loads at `$0801` and the picture data sits at `$2000`, which is file offset 6145. The task cuts that block out, puts the format's load address in front and checks every pixel against png2c64's own PNG.

**Build used.** png2c64 commit `1265329a573b1dfa3c2e0f3473d2aa15ffd7ac3f` (version 1.0.0.76), built with Homebrew GCC 15.3.0 and CMake 4.4.4 on macOS (arm64); see [png2c64-reference.md](../docs/reference/png2c64-reference.md).
