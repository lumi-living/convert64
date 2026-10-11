//! The MCP server over real stdio (#77): opening a recipe of layers, then `mask_key`,
//! `mask_gradient` and a feather through tools gives the same mask as running the same
//! commands from a recipe file in-process (`convert64 recipe apply`'s path).

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn temp(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_source(dir: &Path) {
    let img = image::RgbaImage::from_fn(64, 40, |x, y| {
        let v = (y * 6) as u8;
        if x < 32 {
            image::Rgba([v / 4, 200, v / 3, 255])
        } else {
            image::Rgba([210, v / 2, 40, 255])
        }
    });
    img.save(dir.join("photo.png")).unwrap();
}

const LAYERS: &str = r#""version": 1, "target": "multicolor-bitmap", "supersample": 2,
  "layers": [{ "name": "photo", "source": "photo.png", "transform": { "scale": { "x": 5.0, "y": 5.0 } } }]"#;

fn key() -> Value {
    json!({ "channel": "chroma", "centre": [0.0, 0.58, 0.0], "range": 0.12, "softness": 0.1 })
}

fn gradient() -> Value {
    json!({ "kind": "radial", "from": { "x": 20.0, "y": 20.0 }, "to": { "x": 20.0, "y": 0.0 }, "softness": 0.5 })
}

#[test]
fn mcp_tools_match_a_recipe_file() {
    let dir = temp("mcp-parity");
    write_source(&dir);
    std::fs::write(dir.join("layers.json"), format!("{{{LAYERS}}}")).unwrap();
    let full = json!({
        "commands": [
            { "command": "mask_clear", "layer": "photo", "value": 0.0 },
            { "command": "mask_key", "layer": "photo", "key": key(), "combine": "max" },
            { "command": "mask_gradient", "layer": 0, "gradient": gradient(), "combine": "multiply" },
            { "command": "mask_set_modifier", "layer": 0, "modifier": { "feather": 2.0 } }
        ]
    });
    let mut recipe: Value = serde_json::from_str(&format!("{{{LAYERS}}}")).unwrap();
    recipe["commands"] = full["commands"].clone();
    std::fs::write(dir.join("recipe.json"), recipe.to_string()).unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_convert64-mcp"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut id = 0;
    let mut call = |method: &str, params: Value| -> Value {
        id += 1;
        let msg = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        writeln!(stdin, "{msg}").unwrap();
        stdin.flush().unwrap();
        let mut line = String::new();
        stdout.read_line(&mut line).unwrap();
        let reply: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(reply["id"], id);
        reply
    };
    let init = call(
        "initialize",
        json!({ "protocolVersion": "2025-06-18", "capabilities": {},
                                          "clientInfo": { "name": "parity", "version": "0" } }),
    );
    assert_eq!(init["result"]["serverInfo"]["name"], "convert64-mcp");
    let tool = |name: &str, args: Value| json!({ "name": name, "arguments": args });
    let layers_path = dir.join("layers.json").display().to_string();
    let mcp_png = dir.join("mcp.png").display().to_string();
    for (name, args) in [
        ("recipe_open", json!({ "path": layers_path })),
        ("mask_clear", json!({ "layer": "photo", "value": 0.0 })),
        (
            "mask_key",
            json!({ "layer": "photo", "key": key(), "combine": "max" }),
        ),
        (
            "mask_gradient",
            json!({ "layer": 0, "gradient": gradient(), "combine": "multiply" }),
        ),
        (
            "mask_set_modifier",
            json!({ "layer": 0, "modifier": { "feather": 2.0 } }),
        ),
        ("mask_export", json!({ "layer": "photo", "path": mcp_png })),
    ] {
        let r = call("tools/call", tool(name, args));
        assert_eq!(r["result"]["isError"], false, "{name}: {r}");
    }
    drop(stdin);
    child.wait().unwrap();

    let mcp = image::open(&mcp_png).unwrap().to_luma16();
    let mut session = convert64_cli::recipe::Session::open(&dir.join("recipe.json")).unwrap();
    let (w, h, want) = session.evaluated_mask(0).unwrap();
    assert_eq!(mcp.dimensions(), (w, h));
    assert_eq!(mcp.as_raw(), &want, "MCP and recipe-file masks differ");
    assert!(
        want.iter().any(|&v| v > 1_000 && v < 60_000),
        "feathered values"
    );
}
