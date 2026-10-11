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

## Format references

Byte layouts live in [`docs/reference/file-formats.md`](docs/reference/file-formats.md). When in doubt, test-load the file in VICE or the original editor and attach a screenshot to the PR.

## Licence

By contributing you agree your contribution is licensed under the MIT licence of this repository.
