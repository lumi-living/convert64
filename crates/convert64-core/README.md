# convert64-core

The pure heart of Convert64: document model, command registry, compositor, attribute map, per-mode solvers and the VIC-II validator. Everything else (`convert64-formats`, `convert64-cli`, `convert64-mcp`, `convert64-app`) is a front-end over this crate.

## No I/O, no platform code

This crate never touches `std::fs`, the network, threads-by-default, or anything platform-specific. Callers hand it bytes and get bytes or data structures back. That is what lets the same crate run natively, in the browser (`wasm32-unknown-unknown`) and inside the MCP server without a single `cfg` at the call sites. CI builds it for wasm32 with default features; if that job ever fails, the fix is to move the offending code out of core, not to gate it.

## The `parallel` feature

Parallelism is opt-in:

```toml
convert64-core = { workspace = true, features = ["parallel"] }
```

With the feature on, [`par::for_each`](src/par.rs) and [`par::map`](src/par.rs) dispatch to `rayon`; with it off (the default, and the only option on wasm32) they are plain sequential iterators with identical signatures. Algorithms call the helpers in `par` and never mention `rayon` directly, so a native build with `--features parallel` and a wasm build compile the same source.

```sh
cargo build -p convert64-core                                  # sequential, default
cargo build -p convert64-core --features parallel              # rayon on the host
cargo build -p convert64-core --target wasm32-unknown-unknown  # must always pass
```
