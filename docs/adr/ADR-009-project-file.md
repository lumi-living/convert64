# ADR-009: The `.convert64` project file — versioned JSON in the CLI library

**Status:** Accepted (amended 2026-10-13, #192: the format moved to `convert64-project`; see "Amended" below)
**Date:** 2026-10-12
**Deciders:** Jaakko Haaparanta

## Context

ADR-005 makes the CLI and the app thin front-ends over one document and one command registry, and the TAD's Makefile story (`convert64 convert title.convert64 --mode koala -o build/title.kla`) needs that document on disk. Until #163 the only file was E4's recipe file (`"version": 1`): layers with linked PNG sources, a list of commands to replay and a few solve options. It could not hold a palette, regions, steering, a hidden layer, a mask without replaying the commands that made it, or a source that travels with the file.

Three questions needed an answer: what the file holds, how sources are stored, and which crate owns the format.

## Decision

- **The file is the document, not a replay log.** A `.convert64` file holds the state: `schema_version`, target mode, supersample factor, palette, layers (source, transform, opacity, blend, visibility, recipe, adjustments, mask raster + sources + modifiers), regions, steering (pins, locks, allow, deny) and solve options. An optional `commands` list, run after loading, keeps recipe files' convenience; saving writes the result and no commands. Schema: [reference/project-file.md](../reference/project-file.md).
- **Versioned JSON with serde.** `schema_version: 2`; version 1 is the recipe file, which every build keeps reading by mapping it onto a project. Unknown fields are errors. Values go through the core's validating constructors (`try_from` on the serde types), so a file cannot describe an invalid layer.
- **Sources are linked or embedded, per layer.** Linked: a path relative to the project file, `/`-separated. Embedded: the original PNG's bytes in base64, so a project can stand alone (mailing it, attaching it to an issue). A layer may instead share an earlier layer's pixels (`{"layer": 0}`), which is what `layer_via_mask` creates. Saving keeps each layer's choice unless the caller asks to embed or link all.
- **Masks are 16-bit on disk**, as ADR-008 already says: a uniform raster is written as one exact value, anything else as an embedded 16-bit greyscale PNG.
- **The format lives in `convert64-cli`'s library** (`convert64_cli::project`), next to the recipe file and `Session`, not in `convert64-core`. Reading it needs PNG decoding and file paths; the core stays I/O-free and dependency-free beyond `serde` (CLAUDE.md non-negotiable 2). The core's `Project` stays the in-memory document commands act on; the target mode, palette and solve options live beside it in the session, as the compositor's grid and the solver already did.

## Options considered

### Option A: replay log (recipe file grown)
| Dimension | Assessment |
|---|---|
| Complexity | Low: already exists |
| Performance | Loading replays every stroke; grows without bound |
| Maintenance | Every command's argument schema becomes a file-format promise forever |
| Fit | Poor: an app session of hundreds of strokes saves as hundreds of commands |

### Option B: state snapshot in JSON, sources linked or embedded (chosen)
| Dimension | Assessment |
|---|---|
| Complexity | Moderate: one serde model mirroring the core types |
| Performance | Load is decode + one composite; masks are compact PNGs |
| Maintenance | Only the document's shape is a format promise; commands can change |
| Fit | Good: diffable for linked projects, self-contained when embedded |

### Option C: a zip container (JSON + PNG files)
| Dimension | Assessment |
|---|---|
| Complexity | Needs a zip crate, not in TAD §4 |
| Performance | Fine |
| Maintenance | Opaque to diff and to hand editing |
| Fit | The Makefile use case wants a hand-editable text file next to the art |

### Format in `convert64-core` instead of the CLI library
Rejected for now: the core would need PNG encoding and decoding for embedded sources and masks, and path handling for linked ones. The serde types themselves are I/O-free and could move into the core later if the app needs them without the CLI library (below).

## Trade-off analysis

Option B costs a second description of the document (the serde model) that must follow the core types, and base64 inflates embedded sources by a third. In return the file is hand-editable, diffable when linked, self-contained when embedded, and loads without replaying history. A zip would be smaller for embedded projects but loses the text file the Makefile workflow wants.

## Consequences
- Easier: `make` builds from a project file; agents and humans can write projects by hand; the app, CLI and MCP server load the same files through `Session`.
- Harder: every new piece of document state (a layer property, a region feature) needs a field here too, with a default so older files still load. A renamed or re-meant field needs `schema_version: 3` and a migration.
- Revisit when: the app (E8) needs to load projects in the wasm build. `convert64-cli` builds the core with `parallel`, so the app should not depend on it as is; move the serde model (and the PNG helpers, `image` builds for wasm) into a small I/O-free crate or the core then.

## Amended (2026-10-13, #192)

The "Revisit when" above came due: the app (E8) needs to load projects in its wasm build, and `convert64-cli` builds the core with `parallel` (rayon). The decision above stands (versioned JSON, the file as the document, linked/embedded/shared sources, 16-bit masks); only where the format lives changed:

- **`crates/convert64-project`** (`convert64_project`) now holds the serde model (`ProjectFile` and its parts, `SolveSpec`), the version-1 recipe file and its migration, the JSON form of registry commands (`JsonCommand`, `LayerRef`), the PNG and base64 codecs, and `Session` (the project loaded into the core's `Editor`, `Compositor` and `Solver`). The app, the CLI and the MCP server depend on it; the core does not.
- **No file access in the type layer.** Everything a project names (linked sources and masks, a `.vpl` palette, `mask_load`'s PNG, the project file itself) is read and written through a `Store` trait (`read`/`write` whole files by path). `MemoryStore` serves the browser and tests; `FsStore` and the path-taking conveniences (`Session::open`, `Session::save`, `ProjectFile::read`, the PNG writers) are behind the crate's `fs` feature, which the CLI and MCP server enable and the app does not.
- **The core's `parallel` feature** is the native front-ends' choice: `convert64-project` builds the core without it, so `cargo tree -p convert64-app --target wasm32-unknown-unknown -e features` shows no `rayon`.

File bytes, CLI behaviour and the schema are unchanged; `schema_version` stays 2.

## Action items
- [x] #163: `convert64_cli::project`, `Session::save`, `recipe apply --save/--embed/--link`, round-trip and migration tests, `docs/reference/project-file.md`
- [ ] #164: `convert64 convert` reads projects
- [x] #192: the model moves to `convert64-project`, file access behind `Store`, the app depends on it (load/save smoke test)
- [ ] E8: the app opens and saves `.convert64` files through the same model
