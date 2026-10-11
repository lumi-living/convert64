# Contributing to Convert64

Thanks for looking. The project is early; the fastest way to help right now is **fixture images and golden outputs** (see below), bug reports against the solver, and format corrections.

## Ground rules

- **Scope:** Convert64 is a converter, not a pixel editor. Features that steer conversion are welcome; freehand painting tools will be declined (see [ADR-003](docs/adr/ADR-003-converter-not-pixel-editor.md)).
- **Hardware truth wins.** Any output must be legal VIC-II data as defined in [`docs/reference/vic-ii-constraints.md`](docs/reference/vic-ii-constraints.md). If the validator and your eyes disagree, the validator is right until proven otherwise with a VICE screenshot — then fix the reference and the validator together.
- **Core stays pure.** `convert64-core` has no `std::fs`, no I/O, no platform code; it must compile to WASM unchanged.
- **Decisions are recorded.** Anything that changes architecture, formats, or scope gets an ADR in `docs/adr/` (copy `ADR-000-template.md`). Small stuff goes in the PR description.

## Workflow

1. Open or pick an issue. Epics are labelled `epic`; pick a child issue, not the epic.
2. Branch from `main`: `feat/<short-name>`, `fix/<short-name>`, `docs/<short-name>`.
3. `cargo fmt`, `cargo clippy --workspace -- -D warnings`, `cargo test --workspace` must pass, and `convert64-core` + `convert64-app` must still build for `wasm32-unknown-unknown`.
4. Solver changes must include or update a golden test in `fixtures/` (input PNG + expected `.kla`/`.art`/`.png`). Perceptual improvements are fine — update the golden file and say *why* in the PR.
5. PR against `main`; fill in the template. One reviewer approval; squash-merge.

`main` is protected: merging needs a PR whose branch is up to date with `main` and green on the required CI checks `rust (ubuntu-latest)`, `rust (macos-latest)`, `rust (windows-latest)` and `wasm` (jobs in `.github/workflows/ci.yml`). Renaming those jobs breaks merging until branch protection is updated to match.

## Fixture images

We need source images whose rights allow redistribution under MIT (your own art, CC0, or explicitly permitted). Put them in `fixtures/<name>/source.png` with a `README.md` stating origin and licence. Useful kinds: portraits, terrain tile sheets, title paintings, gradients, photos, high-contrast line art.

Every file in the repository, test data included, must be MIT-compatible: no GPL or CC-BY-SA material, not even in tests.

## Golden tests

`cargo test -p convert64-core --test golden` runs three cases per fixture in `fixtures/`:

- **`format::<fixture>`, bit-exact.** The fixture's png2c64 baseline is read and serialised again. The bytes are compared with `fixtures/<fixture>/expected_kla.snap` (or `expected_art.snap`). That file is an `insta` snapshot holding an annotated hex dump: one line per bitmap cell, one per Screen or Colour RAM row. A failure prints a diff naming the cell or row that changed.
- **`prg::<fixture>`, bit-exact.** The display PRG `convert64-formats` builds from the baseline (the one `cargo xtask vice-check` runs in VICE) must be the same on every run and carry the baseline file verbatim. Its hex dump is `fixtures/<fixture>/expected_prg.snap`: the BASIC line and loader in rows of 16 bytes, then the embedded file.
- **`solver::<fixture>`, perceptual.** The solver's output is rendered through Pepto PAL and compared with `fixtures/<fixture>/expected.png` by mean squared OKLab distance. The test fails above `mse_vs_golden_max` in `fixtures/<fixture>/thresholds.toml`. Each case prints a line you can paste into your PR:

  ```
  portrait, multicolor-bitmap, mse_vs_golden=0.000000, mse_vs_png2c64=0.000000
  ```

  `mse_vs_png2c64` is the distance to png2c64's baseline, the delta `CLAUDE.md` asks solver PRs to report. Run with `-- --nocapture` to see the lines.

Until the solver exists (E5), the solver under test is an identity stub fed the png2c64 baseline, so every score is 0.

**Updating goldens** is allowed when a change is intended, and it must say why. Nothing in `fixtures/` is ever edited by hand.

```sh
cargo xtask regen-goldens --report                         # list what would change; exit 1 if anything does
cargo xtask regen-goldens --accept --reason "why"          # replace the goldens and log the reason
```

`--report` (also the default) changes no committed file, so it runs on uncommitted work. `--accept` needs a clean tree: commit your code change first, because the changelog records the commit that produced the goldens.

`--accept` writes the new `expected_*.snap` and `expected.png` files. It appends one line per changed fixture to [`fixtures/CHANGELOG.md`](fixtures/CHANGELOG.md), with the date, commit, files and reason, and re-runs the harness. Commit the result together with your PR, and quote the `mse_vs_png2c64` lines in its description.

## Format references

Byte layouts live in [`docs/reference/file-formats.md`](docs/reference/file-formats.md). When in doubt, test-load the file in VICE or the original editor and attach a screenshot to the PR.

## Licence

By contributing you agree your contribution is licensed under the MIT licence of this repository.
