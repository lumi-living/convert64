//! `convert64 batch`: every PNG and project in a directory, converted in parallel with
//! per-file settings from a TOML manifest, in one report.
//!
//! ```toml
//! out = "build"                 # output directory, relative to the manifest
//!
//! [defaults]                    # every file; any `convert` option, kebab-case
//! dither = "bayer4"
//! outputs = ["kla", "prg"]      # kla, art, prg, png, raw; default: the mode's picture format
//!
//! [files."title.png"]           # one file, relative to the directory
//! mode = "hires"
//! strength = 0.8
//!
//! [files."draft.png"]
//! skip = true
//! ```

use crate::convert::{self, ConvertJob, FormatArg, ModeArg, Overrides};
use crate::report::{Fail, INPUT_ERROR, NOT_LEGAL, Run};
use clap::ValueEnum;
use convert64_project::ProjectFile;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

/// The manifest's name when `--manifest` is not given.
pub(crate) const DEFAULT_MANIFEST: &str = "batch.toml";

/// Settings for one file: `convert`'s options plus what to write.
#[derive(Clone, Debug, Default)]
struct Entry {
    overrides: Overrides,
    outputs: Option<Vec<FormatArg>>,
    max_clashes: Option<usize>,
    zoom: Option<u8>,
    skip: bool,
}

impl Entry {
    /// `self` over `base`.
    fn or(self, base: &Self) -> Self {
        Self {
            overrides: self.overrides.or(&base.overrides),
            outputs: self.outputs.or_else(|| base.outputs.clone()),
            max_clashes: self.max_clashes.or(base.max_clashes),
            zoom: self.zoom.or(base.zoom),
            skip: self.skip,
        }
    }

    /// Parses a manifest table: the batch keys, then `convert`'s options (unknown keys
    /// are errors).
    fn from_table(mut t: toml::Table, what: &str) -> Result<Self, Fail> {
        let bad = |e: &dyn std::fmt::Display| Fail::input(format!("{what}: {e}"));
        let outputs = match t.remove("outputs") {
            None => None,
            Some(v) => {
                let names: Vec<String> = v.try_into().map_err(|e| bad(&e))?;
                Some(
                    names
                        .iter()
                        .map(|n| {
                            FormatArg::from_str(n, true)
                                .map_err(|_| bad(&format!("unknown output format {n:?}")))
                        })
                        .collect::<Result<_, _>>()?,
                )
            }
        };
        let max_clashes = t
            .remove("max-clashes")
            .map(toml::Value::try_into::<usize>)
            .transpose()
            .map_err(|e| bad(&e))?;
        let zoom = t
            .remove("zoom")
            .map(toml::Value::try_into::<u8>)
            .transpose()
            .map_err(|e| bad(&e))?;
        let skip = t
            .remove("skip")
            .map(toml::Value::try_into::<bool>)
            .transpose()
            .map_err(|e| bad(&e))?
            .unwrap_or(false);
        let overrides: Overrides = toml::Value::Table(t).try_into().map_err(|e| bad(&e))?;
        Ok(Self {
            overrides,
            outputs,
            max_clashes,
            zoom,
            skip,
        })
    }
}

/// A parsed manifest.
#[derive(Debug, Default)]
struct Manifest {
    out: Option<PathBuf>,
    defaults: Entry,
    files: BTreeMap<String, Entry>,
}

fn read_manifest(path: &Path) -> Result<Manifest, Fail> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| Fail::input(format!("{}: {e}", path.display())))?;
    let mut t: toml::Table = text
        .parse()
        .map_err(|e| Fail::input(format!("{}: {e}", path.display())))?;
    let what = path.display().to_string();
    let base = path.parent().map(Path::to_owned).unwrap_or_default();
    let out = t
        .remove("out")
        .map(toml::Value::try_into::<String>)
        .transpose()
        .map_err(|e| Fail::input(format!("{what}: out: {e}")))?
        .map(|o| base.join(o));
    let table = |v: toml::Value, key: &str| -> Result<toml::Table, Fail> {
        match v {
            toml::Value::Table(t) => Ok(t),
            _ => Err(Fail::input(format!("{what}: {key} must be a table"))),
        }
    };
    let defaults = match t.remove("defaults") {
        Some(v) => Entry::from_table(table(v, "defaults")?, &format!("{what}: [defaults]"))?,
        None => Entry::default(),
    };
    let mut files = BTreeMap::new();
    if let Some(v) = t.remove("files") {
        for (name, v) in table(v, "files")? {
            let entry = Entry::from_table(
                table(v, &format!("files.{name:?}"))?,
                &format!("{what}: [files.{name:?}]"),
            )?;
            files.insert(name, entry);
        }
    }
    if let Some(k) = t.keys().next() {
        return Err(Fail::input(format!(
            "{what}: unknown key {k:?} (expected out, [defaults], [files.\"…\"])"
        )));
    }
    Ok(Manifest {
        out,
        defaults,
        files,
    })
}

/// What `batch` was asked to do.
pub(crate) struct BatchJob<'a> {
    pub(crate) dir: &'a Path,
    pub(crate) manifest: Option<&'a Path>,
    pub(crate) out: Option<&'a Path>,
    pub(crate) jobs: Option<usize>,
    pub(crate) overrides: &'a Overrides,
}

/// One file to convert.
struct Task {
    /// As listed: relative to the directory.
    name: String,
    input: PathBuf,
    outputs: Vec<PathBuf>,
    entry: Entry,
}

fn kind(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "png" => Some("png"),
        "convert64" => Some("project"),
        _ => None,
    }
}

/// The mode a file will be solved in, for its default output format.
fn mode_of(input: &Path, entry: &Entry) -> Result<ModeArg, Fail> {
    if let Some(m) = entry.overrides.mode {
        return Ok(m);
    }
    if kind(input) == Some("project") {
        let file = ProjectFile::read(input).map_err(Fail::input)?;
        return Ok(match file.target {
            convert64_core::mode::Mode::HiresBitmap => ModeArg::Hires,
            _ => ModeArg::Koala,
        });
    }
    Ok(ModeArg::Koala)
}

/// Runs `batch`: the worst exit code of any file (3, then 2, then 1), else 0.
pub(crate) fn run(run: &mut Run, job: &BatchJob<'_>) -> Result<u8, Fail> {
    run.set("dir", job.dir.display().to_string());
    let manifest_path = job.manifest.map(Path::to_owned).or_else(|| {
        let p = job.dir.join(DEFAULT_MANIFEST);
        p.exists().then_some(p)
    });
    let manifest = match &manifest_path {
        Some(p) => read_manifest(p)?,
        None => Manifest::default(),
    };
    run.set(
        "manifest",
        manifest_path.as_ref().map(|p| p.display().to_string()),
    );
    let out_dir = job
        .out
        .map(Path::to_owned)
        .or(manifest.out.clone())
        .unwrap_or_else(|| job.dir.to_owned());
    std::fs::create_dir_all(&out_dir).map_err(|e| Fail::write(&out_dir, e))?;
    run.set("out", out_dir.display().to_string());

    // Every PNG and project in the directory, plus every file the manifest names.
    let mut names: Vec<String> = std::fs::read_dir(job.dir)
        .map_err(|e| Fail::input(format!("{}: {e}", job.dir.display())))?
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| e.file_name().to_str().map(str::to_owned))
        .filter(|n| kind(Path::new(n)).is_some())
        .collect();
    for n in manifest.files.keys() {
        if !names.contains(n) {
            names.push(n.clone());
        }
    }
    names.sort();
    let stem = |n: &str| {
        Path::new(n)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    let base = Entry {
        overrides: job.overrides.clone(),
        ..Entry::default()
    };
    let mut tasks = Vec::new();
    let mut skipped = Vec::new();
    for name in names {
        let entry = manifest
            .files
            .get(&name)
            .cloned()
            .unwrap_or_default()
            .or(&manifest.defaults);
        // The command line wins over the manifest.
        let entry = Entry {
            overrides: base.overrides.clone().or(&entry.overrides),
            ..entry
        };
        if entry.skip {
            skipped.push(json!({ "input": name, "reason": "skip = true in the manifest" }));
            continue;
        }
        if kind(Path::new(&name)) == Some("png")
            && let Some(p) = project_twin(&name, &stem, &manifest, job.dir)
        {
            skipped.push(json!({ "input": name, "reason": format!("{p} is converted instead") }));
            continue;
        }
        let input = job.dir.join(&name);
        let formats = match &entry.outputs {
            Some(f) => f.clone(),
            None => match mode_of(&input, &entry) {
                Ok(ModeArg::Hires) => vec![FormatArg::Art],
                _ => vec![FormatArg::Kla],
            },
        };
        let outputs = formats
            .iter()
            .map(|f| out_dir.join(format!("{}.{}", stem(&name), f.extension())))
            .collect();
        tasks.push(Task {
            name,
            input,
            outputs,
            entry,
        });
    }
    run.set("skipped", &skipped);

    let workers = job
        .jobs
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, usize::from))
        .clamp(1, tasks.len().max(1));
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<Option<Value>>> = Mutex::new(vec![None; tasks.len()]);
    run.time("convert", || {
        std::thread::scope(|s| {
            for _ in 0..workers {
                s.spawn(|| {
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(t) = tasks.get(i) else { break };
                        let v = convert_one(t);
                        results.lock().expect("no panics while held")[i] = Some(v);
                    }
                });
            }
        });
    });
    let results: Vec<Value> = results
        .into_inner()
        .expect("workers joined")
        .into_iter()
        .map(|v| v.expect("every task ran"))
        .collect();

    let mut worst = 0u8;
    let mut failed = 0usize;
    for (t, v) in tasks.iter().zip(&results) {
        let code = v["exit_code"].as_u64().map_or(INPUT_ERROR, |c| c as u8);
        // 3 (write) is worse than 2 (input), than 1 (not legal), than 0.
        worst = worst.max(code);
        if code != 0 {
            failed += 1;
        }
        for f in v["files_written"].as_array().into_iter().flatten() {
            if let Some(f) = f.as_str() {
                run.wrote(Path::new(f));
            }
        }
        match v["error"].as_str() {
            Some(e) => run.say(format!("{}: error: {e}", t.name)),
            None => run.say(format!(
                "{} -> {} ({} clash{}){}",
                t.name,
                t.outputs
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", "),
                v["clashes"],
                if v["clashes"] == 1 { "" } else { "es" },
                if code == NOT_LEGAL {
                    " — over --max-clashes or not legal"
                } else {
                    ""
                }
            )),
        }
    }
    run.say(format!(
        "{} converted, {failed} failed, {} skipped",
        tasks.len() - failed,
        skipped.len()
    ));
    run.set("converted", tasks.len() - failed);
    run.set("failed", failed);
    run.set("files", results);
    Ok(worst)
}

/// A project in the batch with the same stem as the PNG `name`, which then is not
/// converted on its own (the project usually links it).
fn project_twin(
    name: &str,
    stem: &impl Fn(&str) -> String,
    manifest: &Manifest,
    dir: &Path,
) -> Option<String> {
    let project = format!("{}.convert64", stem(name));
    let parent = Path::new(name).parent().unwrap_or(Path::new(""));
    let rel = parent.join(&project).to_string_lossy().replace('\\', "/");
    let skipped = manifest.files.get(&rel).is_some_and(|e| e.skip);
    (dir.join(&rel).is_file() && !skipped).then_some(rel)
}

/// Converts one file into its JSON report (`convert`'s, never printed).
fn convert_one(t: &Task) -> Value {
    let mut r = Run::new("convert", true);
    let job = ConvertJob {
        input: &t.input,
        outputs: &t.outputs,
        format: None,
        overrides: &t.entry.overrides,
        max_clashes: t.entry.max_clashes,
        save_project: None,
        report: false,
        zoom: t.entry.zoom.unwrap_or(1),
    };
    let (code, error) = match convert::run(&mut r, &job) {
        Ok(c) => (c, None),
        Err(f) => (f.code, Some(f.message)),
    };
    let mut v = r.to_value(code, error.as_deref());
    v["input"] = json!(t.name);
    v
}
