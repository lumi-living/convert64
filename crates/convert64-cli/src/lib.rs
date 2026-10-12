//! The `convert64` front-end as a library: the registry commands' argument schemas and
//! examples, shared by the `convert64` binary and the MCP server (`convert64-mcp`) so
//! both describe the core command registry the same way (ADR-005). The project file,
//! JSON commands and the editing session are in `convert64-project` (#192).

pub mod schema;
