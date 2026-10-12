//! The `convert64` front-end as a library: project and recipe files and JSON commands,
//! shared by the `convert64` binary and the MCP server (`convert64-mcp`) so both run the
//! core command registry the same way (ADR-005).

mod base64;
pub mod project;
pub mod recipe;
