# Contributing to Convert64

Thanks for looking. The project is early; the fastest way to help right now is **fixture images and golden outputs** (see below), bug reports against the solver, and format corrections.

## Ground rules

- **Scope:** Convert64 is a converter, not a pixel editor. Features that steer conversion are welcome; freehand painting tools will be declined (see [ADR-003](docs/adr/ADR-003-converter-not-pixel-editor.md)).
- **Hardware truth wins.** Any output must be legal VIC-II data. If the validator and your eyes disagree, the validator is right until proven otherwise with a VICE screenshot.
- **Core stays pure.** `convert64-core` has no `std::fs`, no I/O, no platform code; it must compile to WASM unchanged.
- **Decisions are recorded.** Anything that changes architecture, formats, or scope gets an ADR in `docs/adr/` (copy `ADR-000-template.md`). Small stuff goes in the PR description.

## Workflow

1. Open or pick an issue. Epics are labelled `epic`; pick a child issue, not the epic.
2. Branch from `main`: `feat/<short-name>`, `fix/<short-name>`, `docs/<short-name>`.
3. `cargo fmt`, `cargo clippy -- -D warnings`, `cargo test` must pass; UI: `pnpm lint && pnpm check`.
4. Solver changes must include or update a golden test in `fixtures/` (input PNG + expected `.kla`/`.art`/`.png`). Perceptual improvements are fine — update the golden file and say *why* in the PR.
5. PR against `main`; fill in the template. One reviewer approval; squash-merge.

## Fixture images

We need source images whose rights allow redistribution under MIT (your own art, CC0, or explicitly permitted). Put them in `fixtures/<name>/source.png` with a `README.md` stating origin and licence. Useful kinds: portraits, terrain tile sheets, title paintings, gradients, photos, high-contrast line art.

## Format references

Byte layouts for `.kla`, `.art`, `.spd` and FLI variants follow retropixels' documented formats; charset bins follow CharPad's export layout; sprite data is 64-byte aligned. When in doubt, test-load the file in VICE or the original editor.

## Licence

By contributing you agree your contribution is licensed under the MIT licence of this repository.
