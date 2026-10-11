# ADR-005: One command engine behind UI, CLI and MCP

**Status:** Accepted
**Date:** 2026-10-11
**Deciders:** Jaakko Haaparanta

## Context

Convert64 must be driven three ways: interactively (Tauri/Svelte UI), headlessly (CLI in Makefiles), and by AI agents (an MCP server, so a Claude Code session inside a game repo can open a project, tweak a layer's dither recipe, re-solve, read the clash report and export). ADR-001 already requires GUI/CLI parity. [PhotoCraft](https://github.com/storytold/photocraft) demonstrates the clean way to get all three from one codebase: a pure-data document model plus a registry of commands, with the UI, CLI, a JSON control channel and an MCP server as thin front-ends over the same registry.

## Decision

- `convert64-core` exposes a **document model** (`Project`: layers, recipes, pins/locks, target mode, palette) and a **command registry**: every user-visible action (`add_layer`, `set_layer_transform`, `set_dither_recipe`, `pin_cell`, `lock_cell`, `solve`, `export`, …) is a named command with a serde-serialisable argument struct and result. Commands are the only way to mutate a project; undo/redo is command history.
- Front-ends are thin: the egui app calls commands in-process, the CLI maps subcommands and flags to commands, and `convert64-mcp` exposes the registry as MCP tools over stdio with JSON-schema generated from the argument structs.
- Solver and export results are returned as data (clash report, timing, output bytes or paths) so an agent can reason about them without screenshots.

## Options considered

| Option | Verdict |
|---|---|
| A. Separate code paths per front-end (UI calls core APIs directly; CLI has its own flag parsing; MCP written later) | Rejected — parity drifts; MCP becomes a bolt-on |
| B. Command registry in core; UI, CLI, MCP as front-ends (PhotoCraft pattern) | **Chosen** |
| C. MCP wraps the CLI binary (spawn `convert64` per call) | Fallback for M1 if B is late — cheap but stateless and slow for interactive loops |

## Consequences
- Easier: GUI/CLI parity is structural; MCP is a ~1-crate addition; undo/redo and project-file replay are free; scripting from any language via the JSON channel.
- Harder: every UI action must be designed as a command first (discipline, slightly more boilerplate); schema versioning of command args alongside the `.convert64` file format.
- Revisit when: command count grows past what a flat registry handles cleanly (namespaces), or a long-running job model (batch over 500 images) is needed.

## Action items
- [ ] M1: implement the registry and route the CLI and UI through it (part of E7/E8)
- [ ] M2: `convert64-mcp` (E17) using the official Rust MCP SDK; tools: `project.open/save`, `layer.*`, `recipe.set`, `cell.pin/lock`, `solve`, `clash.report`, `export`
- [ ] Evaluate `photocraft-psd` for layered PSD import (E16)
