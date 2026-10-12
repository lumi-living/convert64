//! The convert64 MCP server: the core command registry as MCP tools (ADR-005).
//!
//! A minimal, synchronous implementation of the MCP stdio transport (newline-delimited
//! JSON-RPC 2.0) covering `initialize`, `ping`, `tools/list` and `tools/call`. It runs
//! recipes through the same [`Session`] as `convert64 recipe apply`, so an agent gets
//! exactly what the CLI and the app get. It is written against `serde_json` alone because
//! `rmcp` needs the `tokio` runtime, which is not yet an approved dependency; moving to
//! `rmcp` later changes the transport, not the tools.
//!
//! Tools: `project_open` (alias `recipe_open`), `project_save`, one tool per registry
//! command (`layer_add`, `recipe_set`, `mask_key`, `cell_pin`, … with a project file's
//! JSON arguments), `undo`, `redo`, `solve`, `clash_report`, `mask_export`,
//! `composite_export` and `attributes_export`.

use convert64_cli::schema::{command_schema, layer_schema, object_schema};
use convert64_core::command::REGISTRY;
use convert64_project::{Embed, JsonCommand, LayerRef, Session};
use serde_json::{Value, json};
use std::path::Path;

/// The MCP protocol revision this server implements.
pub const PROTOCOL_VERSION: &str = "2025-06-18";

/// One MCP session: at most one open recipe.
#[derive(Default)]
pub struct Server {
    session: Option<Session>,
}

fn tools() -> Value {
    let mut list = vec![
        json!({
            "name": "project_open",
            "description": "Open a .convert64 project (or a version-1 recipe file) and run its commands.",
            "inputSchema": object_schema(json!({ "path": { "type": "string" } }), &["path"]),
        }),
        json!({
            "name": "recipe_open",
            "description": "Open a convert64 recipe file (JSON: layers and commands) and run its commands; the same as project_open.",
            "inputSchema": object_schema(json!({ "path": { "type": "string" } }), &["path"]),
        }),
        json!({
            "name": "project_save",
            "description": "Save the open project as a .convert64 file. Sources stay linked or embedded as loaded; embed \"all\" makes it stand alone, \"none\" links every source.",
            "inputSchema": object_schema(json!({
                "path": { "type": "string" },
                "embed": { "type": "string", "enum": ["keep", "all", "none"], "default": "keep" }
            }), &["path"]),
        }),
    ];
    for c in REGISTRY {
        list.push(json!({
            "name": c.name,
            "description": c.summary,
            "inputSchema": command_schema(c.name).unwrap_or_else(|| json!({ "type": "object" })),
        }));
    }
    for (name, description) in [
        ("undo", "Undo the last command."),
        ("redo", "Redo the last undone command."),
    ] {
        list.push(json!({ "name": name, "description": description,
                          "inputSchema": { "type": "object" } }));
    }
    list.push(json!({
        "name": "mask_export",
        "description": "Write a layer's evaluated mask (sources and modifiers applied) on the super grid as a 16-bit greyscale PNG.",
        "inputSchema": object_schema(json!({ "layer": layer_schema(), "path": { "type": "string" } }), &["layer", "path"]),
    }));
    list.push(json!({
        "name": "solve",
        "description": "Solve the picture into legal C64 cells. full: background search and refinement; otherwise the interactive fast path re-solves only changed cells.",
        "inputSchema": object_schema(json!({ "full": { "type": "boolean", "default": true } }), &[]),
    }));
    list.push(json!({
        "name": "clash_report",
        "description": "The last solve's clash report: summary (clash count, total error, worst cell) and the worst cells' wanted, chosen and dropped colours.",
        "inputSchema": object_schema(json!({ "worst": { "type": "integer", "minimum": 0, "default": 10 } }), &[]),
    }));
    list.push(json!({
        "name": "composite_export",
        "description": "Write the composite on the target grid (sRGB, alpha = coverage) as a PNG.",
        "inputSchema": object_schema(json!({ "path": { "type": "string" } }), &["path"]),
    }));
    list.push(json!({
        "name": "attributes_export",
        "description": "Write the attribute map's debug render (hue = dither method, lightness = strength) as a PNG.",
        "inputSchema": object_schema(json!({ "path": { "type": "string" } }), &["path"]),
    }));
    Value::Array(list)
}

impl Server {
    /// Handles one line of input; returns the response line, or `None` for a
    /// notification.
    pub fn handle_line(&mut self, line: &str) -> Option<String> {
        let reply = match serde_json::from_str::<Value>(line) {
            Ok(msg) => self.handle(&msg)?,
            Err(e) => error_reply(&Value::Null, -32700, &format!("parse error: {e}")),
        };
        Some(reply.to_string())
    }

    /// Handles one JSON-RPC message; `None` for a notification.
    pub fn handle(&mut self, msg: &Value) -> Option<Value> {
        let id = msg.get("id")?.clone();
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        Some(match method {
            "initialize" => {
                let requested = params.get("protocolVersion").and_then(Value::as_str);
                let version = match requested {
                    Some(v @ ("2025-06-18" | "2025-03-26" | "2024-11-05")) => v,
                    _ => PROTOCOL_VERSION,
                };
                ok_reply(
                    &id,
                    json!({
                        "protocolVersion": version,
                        "capabilities": { "tools": { "listChanged": false } },
                        "serverInfo": { "name": "convert64-mcp", "version": env!("CARGO_PKG_VERSION") },
                        "instructions": "Open a project with project_open, edit it with the registry tools \
                                         (layer_*, recipe_set, adjustments_set, mode_set, palette_set, \
                                         background_pin, solve_set, mask_*, cell_*), solve, read the clash \
                                         report, and save with project_save. Layers are referenced by index or name."
                    }),
                )
            }
            "ping" => ok_reply(&id, json!({})),
            "tools/list" => ok_reply(&id, json!({ "tools": tools() })),
            "tools/call" => {
                let name = params.get("name").and_then(Value::as_str).unwrap_or("");
                let args = params
                    .get("arguments")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                let result = match self.call(name, args) {
                    Ok(text) => {
                        json!({ "content": [{ "type": "text", "text": text }], "isError": false })
                    }
                    Err(text) => {
                        json!({ "content": [{ "type": "text", "text": text }], "isError": true })
                    }
                };
                ok_reply(&id, result)
            }
            other => error_reply(&id, -32601, &format!("method not found: {other}")),
        })
    }

    fn session(&mut self) -> Result<&mut Session, String> {
        self.session
            .as_mut()
            .ok_or_else(|| "no project is open; call project_open first".to_owned())
    }

    /// Runs a tool; `Err` becomes an MCP tool error (`isError: true`).
    pub fn call(&mut self, name: &str, args: Value) -> Result<String, String> {
        let str_arg = |k: &str| -> Result<String, String> {
            args.get(k)
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| format!("missing string argument {k:?}"))
        };
        match name {
            "project_open" | "recipe_open" => {
                let path = str_arg("path")?;
                let s = Session::open(Path::new(&path)).map_err(|e| e.to_string())?;
                let text = format!(
                    "opened {path}: {} layer(s): {}",
                    s.project().layers.len(),
                    s.project()
                        .layers
                        .iter()
                        .map(|l| l.name().to_owned())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                self.session = Some(s);
                Ok(text)
            }
            "project_save" => {
                let path = str_arg("path")?;
                let embed = match args.get("embed").and_then(Value::as_str).unwrap_or("keep") {
                    "keep" => Embed::Keep,
                    "all" => Embed::All,
                    "none" => Embed::None,
                    other => return Err(format!("embed must be keep, all or none, not {other:?}")),
                };
                self.session()?
                    .save(Path::new(&path), embed)
                    .map_err(|e| e.to_string())?;
                Ok(format!("project -> {path}"))
            }
            "undo" => Ok(match self.session()?.editor_mut().undo() {
                Some(d) => format!("undone ({d:?})"),
                None => "nothing to undo".to_owned(),
            }),
            "redo" => match self.session()?.editor_mut().redo() {
                Some(r) => r
                    .map(|d| format!("redone ({d:?})"))
                    .map_err(|e| e.to_string()),
                None => Ok("nothing to redo".to_owned()),
            },
            "mask_export" => {
                let path = str_arg("path")?;
                let layer: LayerRef = serde_json::from_value(
                    args.get("layer")
                        .cloned()
                        .ok_or("missing argument \"layer\"")?,
                )
                .map_err(|e| e.to_string())?;
                let s = self.session()?;
                let i = s.resolve(&layer).map_err(|e| e.to_string())?;
                s.write_mask(i, Path::new(&path))
                    .map_err(|e| e.to_string())?;
                Ok(format!("mask of layer {i} -> {path}"))
            }
            "solve" => {
                let full = args.get("full").and_then(Value::as_bool).unwrap_or(true);
                let sol = self.session()?.solve(full);
                let s = sol.report.summary();
                Ok(serde_json::to_string(&json!({
                    "background": sol.background,
                    "error": sol.error,
                    "clashes": s.clashes,
                    "worst": s.worst,
                }))
                .expect("serialisable"))
            }
            "clash_report" => {
                let worst = args.get("worst").and_then(Value::as_u64).unwrap_or(10);
                let v = self
                    .session()?
                    .clash_report_json(usize::try_from(worst).unwrap_or(usize::MAX))
                    .ok_or("nothing solved yet; call solve first")?;
                Ok(serde_json::to_string(&v).expect("serialisable"))
            }
            "composite_export" => {
                let path = str_arg("path")?;
                self.session()?
                    .write_composite(Path::new(&path))
                    .map_err(|e| e.to_string())?;
                Ok(format!("composite -> {path}"))
            }
            "attributes_export" => {
                let path = str_arg("path")?;
                self.session()?
                    .write_attributes(Path::new(&path))
                    .map_err(|e| e.to_string())?;
                Ok(format!("attribute map -> {path}"))
            }
            cmd if REGISTRY.iter().any(|c| c.name == cmd) => {
                let mut obj = match args {
                    Value::Object(m) => m,
                    Value::Null => serde_json::Map::new(),
                    _ => return Err("arguments must be an object".to_owned()),
                };
                obj.insert("command".to_owned(), Value::String(cmd.to_owned()));
                let parsed: JsonCommand = serde_json::from_value(Value::Object(obj))
                    .map_err(|e| format!("{cmd}: {e}"))?;
                let dirty = self.session()?.run(parsed).map_err(|e| e.to_string())?;
                Ok(format!("{cmd} done ({dirty:?})"))
            }
            other => Err(format!("unknown tool {other:?}")),
        }
    }
}

fn ok_reply(id: &Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error_reply(id: &Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(id: u64, method: &str, params: Value) -> Value {
        json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
    }

    #[test]
    fn handshake_and_tool_list() {
        let mut s = Server::default();
        let r = s
            .handle(&req(1, "initialize", json!({ "protocolVersion": "2025-06-18", "capabilities": {},
                                                    "clientInfo": { "name": "t", "version": "0" } })))
            .unwrap();
        assert_eq!(r["result"]["protocolVersion"], "2025-06-18");
        assert!(r["result"]["capabilities"]["tools"].is_object());
        assert!(
            s.handle(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))
                .is_none()
        );
        let r = s.handle(&req(2, "tools/list", json!({}))).unwrap();
        let names: Vec<&str> = r["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        for n in [
            "mask_key",
            "mask_gradient",
            "recipe_open",
            "project_open",
            "project_save",
            "mask_export",
            "undo",
        ] {
            assert!(names.contains(&n), "{n} missing from {names:?}");
        }
        for c in REGISTRY {
            assert!(names.contains(&c.name));
        }
        assert_eq!(
            s.handle(&req(3, "nope", json!({}))).unwrap()["error"]["code"],
            -32601
        );
        assert_eq!(
            serde_json::from_str::<Value>(&s.handle_line("{not json").unwrap()).unwrap()["error"]["code"],
            -32700
        );
    }

    #[test]
    fn tool_errors_are_reported_not_fatal() {
        let mut s = Server::default();
        let r = s
            .handle(&req(
                1,
                "tools/call",
                json!({ "name": "mask_key", "arguments": { "layer": 0 } }),
            ))
            .unwrap();
        assert_eq!(r["result"]["isError"], true);
        let r = s
            .handle(&req(
                2,
                "tools/call",
                json!({ "name": "recipe_open", "arguments": { "path": "/nonexistent.json" } }),
            ))
            .unwrap();
        assert_eq!(r["result"]["isError"], true);
    }
}
