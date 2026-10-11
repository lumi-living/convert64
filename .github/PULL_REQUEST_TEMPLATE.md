## What

<!-- one or two sentences -->

## Why / issue

Closes #

## Checks

- [ ] Output still legal VIC-II data (validator passes; loaded in VICE if a format changed)
- [ ] `cargo fmt && cargo clippy -- -D warnings && cargo test` pass
- [ ] `convert64-core` still builds for `wasm32-unknown-unknown`
- [ ] Golden fixtures added/updated with a reason if solver output changed
- [ ] ADR added/updated if architecture, format or scope changed
