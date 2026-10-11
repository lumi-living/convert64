## What

<!-- one or two sentences -->

## Why / issue

Closes #

## Checks

- [ ] Output still legal VIC-II data (validator passes; loaded in VICE if a format changed)
- [ ] `cargo fmt && cargo clippy -- -D warnings && cargo test` pass
- [ ] `convert64-core` still builds for `wasm32-unknown-unknown`
- [ ] Golden fixtures added/updated with a reason if solver output changed ([`cargo xtask regen-goldens --accept --reason --pr`](https://github.com/lumi-living/convert64/blob/main/CONTRIBUTING.md#golden-tests); paste the `mse_vs_png2c64` lines)
- [ ] ADR added/updated if architecture, format or scope changed
