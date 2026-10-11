# Tooling

## Toolchain

| Need | Tool |
|---|---|
| Rust | stable, pinned in `rust-toolchain.toml`; `cargo fmt`, `cargo clippy -D warnings`, `cargo test` |
| UI | egui via eframe (`cargo run -p convert64-app`) |
| Web build | `trunk` targeting `wasm32-unknown-unknown` (`trunk serve` in `crates/convert64-app`) |
| MCP | `rmcp` (official Rust MCP SDK), stdio transport |
| Release | `cargo-dist` for native binaries; GitHub Pages for the web build |
| Snapshots | `insta` (`cargo insta review`) |
| Verification | VICE `x64sc` for every exported picture; a C64 Ultimate or real hardware when available |

## Reference material

Everything the code must agree with is in [`docs/reference/`](reference/README.md). Do not rely on memory for palette values, cell rules or byte layouts — read the file.

## AI-assisted development

This project is developed with AI coding agents alongside humans. If you use one:

- Point it at `CLAUDE.md` (the non-negotiables) and `docs/reference/` (the facts). The file name follows the Claude Code convention but the content is agent-agnostic.
- Have it check egui/eframe, rmcp and trunk APIs against current docs rather than training data.
- Treat its solver output like anyone else's: golden tests, OKLab-MSE delta against the png2c64 baseline, and a VICE load before merge.

No private prompts, skills or MCP servers are required to work on Convert64.
