//! `convert64 apply` and `convert64 commands`: every command in the core's registry,
//! run headless on a project (ADR-005, CLAUDE.md non-negotiable 4).

use crate::report::{Fail, Run};
use convert64_cli::schema;
use convert64_core::command::REGISTRY;
use convert64_project::{Embed, JsonCommand, Session};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// What `apply` was asked to do.
pub(crate) struct ApplyJob<'a> {
    pub(crate) file: &'a Path,
    pub(crate) commands: &'a [String],
    pub(crate) commands_file: Option<&'a Path>,
    pub(crate) save: Option<(&'a Path, Embed)>,
}

/// One command or an array of them.
fn parse_commands(v: Value, what: &str) -> Result<Vec<JsonCommand>, Fail> {
    let list = match v {
        Value::Array(a) => a,
        other => vec![other],
    };
    list.into_iter()
        .enumerate()
        .map(|(i, c)| {
            serde_json::from_value(c).map_err(|e| Fail::input(format!("{what} command {i}: {e}")))
        })
        .collect()
}

/// Runs `apply`: loads the project, runs the commands, saves it.
pub(crate) fn run(run: &mut Run, job: &ApplyJob<'_>) -> Result<u8, Fail> {
    run.set("file", job.file.display().to_string());
    let mut queue: Vec<(JsonCommand, PathBuf)> = Vec::new();
    if let Some(f) = job.commands_file {
        let text =
            std::fs::read_to_string(f).map_err(|e| Fail::input(format!("{}: {e}", f.display())))?;
        let v: Value = serde_json::from_str(&text)
            .map_err(|e| Fail::input(format!("{}: {e}", f.display())))?;
        let base = f.parent().map(Path::to_owned).unwrap_or_default();
        for c in parse_commands(v, &f.display().to_string())? {
            queue.push((c, base.clone()));
        }
    }
    for (i, text) in job.commands.iter().enumerate() {
        let v: Value =
            serde_json::from_str(text).map_err(|e| Fail::input(format!("--command {i}: {e}")))?;
        for c in parse_commands(v, &format!("--command {i}:"))? {
            queue.push((c, PathBuf::new()));
        }
    }
    let mut session = run.time("load", || Session::open(job.file).map_err(Fail::input))?;
    let mut names = Vec::with_capacity(queue.len());
    run.time("apply", || {
        for (i, (cmd, base)) in queue.into_iter().enumerate() {
            let name = serde_json::to_value(&cmd)
                .ok()
                .and_then(|v| v["command"].as_str().map(str::to_owned))
                .unwrap_or_default();
            session
                .run_at(cmd, &base)
                .map_err(|e| Fail::input(format!("command {i} ({name}): {e}")))?;
            names.push(name);
        }
        Ok::<_, Fail>(())
    })?;
    run.set("commands_applied", &names);
    run.set("layers", session.project().layers.len());
    run.say(format!(
        "{}: {} command(s) applied, {} layer(s)",
        job.file.display(),
        names.len(),
        session.project().layers.len()
    ));
    if let Some((path, embed)) = job.save {
        let dir = path.parent().unwrap_or(Path::new(""));
        let text = session
            .to_project_file(dir, embed)
            .map_err(Fail::input)?
            .to_json();
        run.write(path, text.as_bytes())?;
        run.say(format!("project -> {}", path.display()));
    }
    Ok(0)
}

/// Lists the registry: names and summaries, with each command's arguments (`?` marks
/// optional ones); the JSON report has each command's schema and an example.
pub(crate) fn list(run: &mut Run) {
    let mut out = Vec::with_capacity(REGISTRY.len());
    for c in REGISTRY {
        let schema = schema::command_schema(c.name).unwrap_or(Value::Null);
        run.say(format!("{:<18} {}", c.name, c.summary));
        run.say(format!("{:<18}   ({})", "", schema::signature(&schema)));
        out.push(json!({
            "name": c.name,
            "summary": c.summary,
            "schema": schema,
            "example": schema::example(c.name),
        }));
    }
    run.set("commands", out);
}
