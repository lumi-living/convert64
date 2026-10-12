//! GUI/CLI/MCP parity (#166, #193, CLAUDE.md non-negotiable 4): every command in the
//! core's registry runs from the command line (`convert64 apply`) and as an MCP tool
//! (`convert64_mcp::Server`, in-process), each giving the same saved project as the
//! command run in-process on the core (the app's path); through the MCP server, undo
//! and redo give back the project before and after. A registry command the CLI or the
//! MCP server cannot run, or that `convert64 commands` does not describe, fails this test.

use convert64_core::command::REGISTRY;
use convert64_project::{Embed, JsonCommand, Session};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Output;

fn temp(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn convert64(dir: &Path, args: &[&str]) -> Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_convert64"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap()
}

fn ok(out: &Output) -> Value {
    assert!(
        out.status.success(),
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap_or(Value::Null)
}

/// A two-layer 64 × 40 project (the photo and a copy of it) with a half-transparent
/// source and a greyscale `mask.png`.
fn project(dir: &Path) -> PathBuf {
    let img = image::RgbaImage::from_fn(64, 40, |x, y| {
        let a = if x < 48 { 255 } else { 96 };
        if x < 32 {
            image::Rgba([(y * 4) as u8, 200, 60, a])
        } else {
            image::Rgba([210, (y * 3) as u8, 40, a])
        }
    });
    img.save(dir.join("photo.png")).unwrap();
    image::GrayImage::from_fn(64, 40, |x, _| image::Luma([(x * 4) as u8]))
        .save(dir.join("mask.png"))
        .unwrap();
    ok(&convert64(
        dir,
        &["convert", "photo.png", "--save-project", "base.convert64"],
    ));
    ok(&convert64(
        dir,
        &[
            "apply",
            "base.convert64",
            "--command",
            r#"{"command":"layer_duplicate","layer":0}"#,
            "--in-place",
        ],
    ));
    dir.join("base.convert64")
}

/// Runs an MCP tool on `server`; its text, or a panic naming the tool.
fn mcp(server: &mut convert64_mcp::Server, tool: &str, args: Value) -> String {
    server
        .call(tool, args)
        .unwrap_or_else(|e| panic!("MCP {tool}: {e}"))
}

#[test]
fn every_registry_command_runs_from_the_cli_and_mcp_as_in_the_core() {
    let dir = temp("parity");
    let base = project(&dir);
    let base_text = std::fs::read_to_string(&base).unwrap();

    // The MCP server has a tool per registry command, with the schema `convert64
    // commands` gives.
    let mut server = convert64_mcp::Server::default();
    let tools = server
        .handle(&json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }))
        .unwrap();
    let tools = tools["result"]["tools"].as_array().unwrap().clone();

    // `convert64 commands` describes exactly the registry.
    let listed = ok(&convert64(&dir, &["commands", "--json"]));
    let listed = listed["commands"].as_array().unwrap();
    let names: Vec<&str> = listed.iter().map(|c| c["name"].as_str().unwrap()).collect();
    let registry: Vec<&str> = REGISTRY.iter().map(|c| c.name).collect();
    assert_eq!(names, registry);

    for c in REGISTRY {
        let entry = listed.iter().find(|e| e["name"] == c.name).unwrap();
        assert_eq!(
            entry["schema"]["type"], "object",
            "{}: no argument schema",
            c.name
        );
        let example = &entry["example"];
        assert!(example.is_object(), "{}: no example", c.name);
        let tool = tools
            .iter()
            .find(|t| t["name"] == c.name)
            .unwrap_or_else(|| panic!("{}: no MCP tool", c.name));
        assert_eq!(tool["inputSchema"], entry["schema"], "{}", c.name);

        // The CLI's path: `convert64 apply`, saved.
        let out = format!("{}.convert64", c.name);
        let text = example.to_string();
        let r = ok(&convert64(
            &dir,
            &[
                "apply",
                "base.convert64",
                "--command",
                &text,
                "-o",
                &out,
                "--json",
            ],
        ));
        assert_eq!(r["commands_applied"], serde_json::json!([c.name]));

        // The app's path: the same command on the core, in-process.
        let mut s = Session::open(&base).unwrap();
        let cmd: JsonCommand = serde_json::from_value(example.clone()).unwrap();
        s.run_at(cmd, &dir).unwrap();
        let in_process = s.to_project_file(&dir, Embed::Keep).unwrap().to_json();
        // Clearing steering on an unsteered cell is the one example with nothing to do.
        if c.name != "cell_clear" {
            assert_ne!(
                in_process, base_text,
                "{}: the example changes nothing",
                c.name
            );
        }
        assert_eq!(
            std::fs::read_to_string(dir.join(&out)).unwrap(),
            in_process,
            "{}: CLI",
            c.name
        );

        // The agent's path: the MCP tool, then undo and redo.
        let mut args = example.clone();
        args.as_object_mut().unwrap().remove("command");
        let saved = |server: &mut convert64_mcp::Server, name: &str| {
            let path = dir.join(name);
            mcp(server, "project_save", json!({ "path": path }));
            std::fs::read_to_string(path).unwrap()
        };
        mcp(&mut server, "project_open", json!({ "path": base }));
        mcp(&mut server, c.name, args);
        let mcp_name = format!("{}-mcp.convert64", c.name);
        assert_eq!(saved(&mut server, &mcp_name), in_process, "{}: MCP", c.name);
        mcp(&mut server, "undo", json!({}));
        assert_eq!(
            saved(&mut server, "undone.convert64"),
            base_text,
            "{}: MCP undo",
            c.name
        );
        mcp(&mut server, "redo", json!({}));
        assert_eq!(
            saved(&mut server, "redone.convert64"),
            in_process,
            "{}: MCP redo",
            c.name
        );
    }

    // A command outside the registry is refused, and nothing is written.
    let r = convert64(
        &dir,
        &[
            "apply",
            "base.convert64",
            "--command",
            r#"{"command":"paint_pixel","x":1,"y":1}"#,
            "-o",
            "never.convert64",
        ],
    );
    assert_eq!(r.status.code(), Some(2));
    assert!(!dir.join("never.convert64").exists());
}

#[test]
fn apply_reads_command_files_and_saves_in_place() {
    let dir = temp("parity-files");
    project(&dir);
    std::fs::create_dir_all(dir.join("steps")).unwrap();
    std::fs::copy(dir.join("mask.png"), dir.join("steps/m.png")).unwrap();
    std::fs::write(
        dir.join("steps/steps.json"),
        r#"[
          { "command": "mask_load", "layer": 0, "path": "m.png" },
          { "command": "mask_set_modifier", "layer": "photo", "modifier": { "invert": true } }
        ]"#,
    )
    .unwrap();
    let before = std::fs::read_to_string(dir.join("base.convert64")).unwrap();
    // Without -o or --in-place the commands are only checked.
    let r = ok(&convert64(
        &dir,
        &[
            "apply",
            "base.convert64",
            "--commands",
            "steps/steps.json",
            "--command",
            r#"{"command":"cell_lock","cells":[{"x":0,"y":0}]}"#,
            "--json",
        ],
    ));
    assert_eq!(
        r["commands_applied"],
        serde_json::json!(["mask_load", "mask_set_modifier", "cell_lock"])
    );
    assert_eq!(r["files_written"], serde_json::json!([]));
    assert_eq!(
        std::fs::read_to_string(dir.join("base.convert64")).unwrap(),
        before
    );
    ok(&convert64(
        &dir,
        &[
            "apply",
            "base.convert64",
            "--commands",
            "steps/steps.json",
            "--in-place",
        ],
    ));
    let after: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("base.convert64")).unwrap())
            .unwrap();
    assert_eq!(after["layers"][0]["mask"]["modifiers"]["invert"], true);
    // A failing command names itself and leaves the file alone.
    let r = convert64(
        &dir,
        &[
            "apply",
            "base.convert64",
            "--command",
            r#"{"command":"mask_clear","layer":5}"#,
            "--in-place",
        ],
    );
    assert_eq!(r.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&r.stderr).contains("mask_clear"));
}
