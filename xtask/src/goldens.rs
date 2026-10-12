//! `cargo xtask regen-goldens`: list golden changes, and accept them only with a reason
//! (#41).
//!
//! Runs the golden harness (`crates/convert64-core/tests/golden.rs`) so that it writes
//! candidates instead of failing: `insta` writes `*.snap.new` next to each changed format
//! snapshot (`INSTA_UPDATE=new`, `INSTA_FORCE_PASS=1`), and every solver case writes its
//! rendering to `target/golden-candidates/<fixture>/expected.png`
//! (`CONVERT64_GOLDEN_OUT`); composite scenes (`fixtures/composite/<scene>/`) write
//! `expected_1x.png` and `expected_attr.png` there. It then compares candidates with the
//! committed goldens.
//!
//! - Report mode (`--report`, or no flag): prints what would change, removes the
//!   `*.snap.new` files it created and exits 1 if anything differs. It changes no
//!   committed file, so it runs on a dirty tree: that is how you check uncommitted
//!   solver work. It stops if `*.snap.new` files already exist, because the harness
//!   would overwrite them.
//! - Accept mode (`--accept --reason "..."`): replaces the goldens, appends one line per
//!   changed fixture to `fixtures/CHANGELOG.md` (date, pull request, files, reason) and
//!   re-runs the harness, which must now pass. It refuses a dirty tree, because the
//!   goldens must come from committed code. The changelog cites the pull request number
//!   (`--pr`), not a commit hash: PRs are squash-merged, so branch hashes are not
//!   reachable from main.

use convert64_core::palette::{Rgb8, oklab_of};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::repo_root;

/// One golden file whose candidate differs.
struct Change {
    fixture: String,
    file: String,
    detail: String,
    apply: Apply,
}

enum Apply {
    /// Rename `<snap>.new` over `<snap>`.
    Snapshot(PathBuf),
    /// Copy a candidate PNG over the golden.
    Png { from: PathBuf, to: PathBuf },
}

/// Whether a run only reads (report) or writes goldens and the changelog (accept).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    /// List differences; changes no committed file. Allowed on a dirty tree.
    Report,
    /// Replace goldens and append to the changelog. Needs a clean tree.
    Accept,
}

/// Which pull request a changelog line cites.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PrRef {
    /// A pull request number.
    Number(u32),
    /// Not opened yet: `--set-pr N` fills it in.
    Pending,
}

/// The placeholder `--pr pending` writes and `--set-pr` replaces.
pub(crate) const PENDING: &str = "#pending";

impl std::str::FromStr for PrRef {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        if s == "pending" {
            return Ok(Self::Pending);
        }
        s.trim_start_matches('#')
            .parse()
            .map(Self::Number)
            .map_err(|_| format!("--pr takes a pull request number or `pending`, not {s:?}"))
    }
}

impl std::fmt::Display for PrRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Number(n) => write!(f, "#{n}"),
            Self::Pending => f.write_str(PENDING),
        }
    }
}

/// `--set-pr N`: rewrites every `#pending` changelog entry to `#N`.
pub(crate) fn set_pr(root: &Path, n: u32) -> Result<(), String> {
    let path = root.join("fixtures").join("CHANGELOG.md");
    let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let (out, count) = replace_pending(&text, n);
    if count == 0 {
        return Err(format!(
            "{} has no {PENDING} entries; nothing to set",
            path.display()
        ));
    }
    fs::write(&path, out).map_err(|e| format!("{}: {e}", path.display()))?;
    println!(
        "{count} changelog entr{} now cite #{n}; commit fixtures/CHANGELOG.md.",
        if count == 1 { "y" } else { "ies" }
    );
    Ok(())
}

/// Replaces `#pending` in changelog entry lines (`- <date> #pending …`) with `#n`.
fn replace_pending(text: &str, n: u32) -> (String, usize) {
    let mut count = 0;
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let is_entry = line.starts_with("- ") && line.split(' ').nth(2) == Some(PENDING);
        if is_entry {
            out.push_str(&line.replacen(PENDING, &format!("#{n}"), 1));
            count += 1;
        } else {
            out.push_str(line);
        }
    }
    (out, count)
}

pub(crate) fn run(accept: bool, reason: Option<&str>, pr: Option<PrRef>) -> Result<(), String> {
    let reason = match (accept, reason.map(str::trim)) {
        (true, None | Some("")) => {
            return Err("--accept needs --reason \"why the goldens change\"".to_owned());
        }
        (true, Some(r)) => Some(r.to_owned()),
        (false, _) => None,
    };
    let pr = match (accept, pr) {
        (true, None) => {
            return Err(
                "--accept needs --pr <number>, or --pr pending before the PR exists".to_owned(),
            );
        }
        (_, pr) => pr,
    };
    let mode = if accept { Mode::Accept } else { Mode::Report };
    let root = repo_root();
    let log = fs::read_to_string(root.join("fixtures").join("CHANGELOG.md")).unwrap_or_default();
    if replace_pending(&log, 0).1 > 0 {
        println!(
            "note: fixtures/CHANGELOG.md has {PENDING} entries; run `cargo xtask regen-goldens \
             --set-pr N` once the pull request exists."
        );
    }
    let dirty = check_tree(&root, mode)?;
    let head = git(&root, &["rev-parse", "--short=12", "HEAD"])?;
    let commit = if dirty {
        format!("{head} plus uncommitted changes")
    } else {
        head
    };
    if let Some(pending) = pending_snapshots(&root)?.first() {
        return Err(format!(
            "{} is a pending insta snapshot; review or delete it first (the harness would \
             overwrite it)",
            pending.display()
        ));
    }

    let candidates = root.join("target").join("golden-candidates");
    let _ = fs::remove_dir_all(&candidates);
    run_harness(&root, &candidates, false)?;

    let changes = collect_changes(&root, &candidates)?;
    if changes.is_empty() {
        remove_pending_snapshots(&root)?;
        println!("All goldens match the current code ({commit}).");
        return Ok(());
    }
    println!("Goldens that differ from the current code ({commit}):");
    for c in &changes {
        println!("  {}/{}: {}", c.fixture, c.file, c.detail);
    }

    let Some(reason) = reason else {
        remove_pending_snapshots(&root)?;
        return Err(format!(
            "{} golden file(s) differ; rerun with --accept --reason \"...\" to update them",
            changes.len()
        ));
    };

    for c in &changes {
        match &c.apply {
            Apply::Snapshot(new) => {
                let target = new.with_extension("");
                fs::rename(new, &target).map_err(|e| format!("{}: {e}", target.display()))?;
            }
            Apply::Png { from, to } => {
                fs::copy(from, to).map_err(|e| format!("{}: {e}", to.display()))?;
            }
        }
    }
    append_changelog(&root, &changes, pr.expect("checked above"), &reason)?;
    run_harness(&root, &candidates, true)?;
    println!(
        "Accepted {} change(s); fixtures/CHANGELOG.md updated. Review and commit the result.",
        changes.len()
    );
    Ok(())
}

fn git(root: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|e| format!("git {}: {e}", args.join(" ")))?;
    if !out.status.success() {
        return Err(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// Checks the working tree for `mode`. Returns whether it is dirty; accept mode fails on
/// a dirty tree.
fn check_tree(root: &Path, mode: Mode) -> Result<bool, String> {
    let status = git(root, &["status", "--porcelain"])?;
    match (mode, status.is_empty()) {
        (_, true) => Ok(false),
        (Mode::Report, false) => Ok(true),
        (Mode::Accept, false) => Err(format!(
            "the git tree is dirty; commit or stash first, so the goldens come from committed \
             code (report mode works on a dirty tree):\n{status}"
        )),
    }
}

/// Fixture directories with goldens: `fixtures/<name>/` with a `thresholds.toml`, and
/// layered composite scenes `fixtures/composite/<scene>/` with a `scene.toml`.
fn fixture_dirs(root: &Path) -> Result<Vec<PathBuf>, String> {
    let fixtures = root.join("fixtures");
    let list = |dir: &Path, marker: &str| -> Result<Vec<PathBuf>, String> {
        Ok(fs::read_dir(dir)
            .map_err(|e| format!("{}: {e}", dir.display()))?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.join(marker).exists())
            .collect())
    };
    let mut dirs = list(&fixtures, "thresholds.toml")?;
    let composite = fixtures.join("composite");
    if composite.is_dir() {
        dirs.extend(list(&composite, "scene.toml")?);
    }
    dirs.sort();
    Ok(dirs)
}

fn pending_snapshots(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut found = Vec::new();
    for dir in fixture_dirs(root)? {
        for entry in fs::read_dir(&dir).map_err(|e| e.to_string())?.flatten() {
            let path = entry.path();
            if path.to_string_lossy().ends_with(".snap.new") {
                found.push(path);
            }
        }
    }
    found.sort();
    Ok(found)
}

fn remove_pending_snapshots(root: &Path) -> Result<(), String> {
    for path in pending_snapshots(root)? {
        fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(())
}

/// Runs the golden tests. In candidate mode mismatches do not fail the run; in `verify`
/// mode the run must pass.
fn run_harness(root: &Path, candidates: &Path, verify: bool) -> Result<(), String> {
    let mut cmd = Command::new(env!("CARGO"));
    cmd.args([
        "test",
        "--quiet",
        "-p",
        "convert64-core",
        "--test",
        "golden",
    ])
    .current_dir(root)
    .env_remove("CI");
    if !verify {
        cmd.env("INSTA_UPDATE", "new")
            .env("INSTA_FORCE_PASS", "1")
            .env("CONVERT64_GOLDEN_OUT", candidates);
    }
    let out = cmd.output().map_err(|e| format!("cargo test: {e}"))?;
    if verify && !out.status.success() {
        return Err(format!(
            "the golden harness still fails after accepting:\n{}",
            String::from_utf8_lossy(&out.stdout)
        ));
    }
    if !verify && !candidates.exists() {
        return Err(format!(
            "the golden harness wrote no candidates (does it compile?):\n{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(())
}

/// RGBA pixels; alpha matters for composite goldens, whose alpha is coverage.
fn pixels(path: &Path) -> Result<Vec<(Rgb8, u8)>, String> {
    let img = image::open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .to_rgba8();
    Ok(img
        .pixels()
        .map(|p| (Rgb8::new(p.0[0], p.0[1], p.0[2]), p.0[3]))
        .collect())
}

fn collect_changes(root: &Path, candidates: &Path) -> Result<Vec<Change>, String> {
    let fixtures = root.join("fixtures");
    let mut changes = Vec::new();
    for dir in fixture_dirs(root)? {
        // `portrait`, or `composite/<scene>` for layered scenes.
        let fixture = dir
            .strip_prefix(&fixtures)
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        // Format snapshots: insta wrote a .snap.new for every mismatch.
        let mut news: Vec<PathBuf> = fs::read_dir(&dir)
            .map_err(|e| e.to_string())?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.to_string_lossy().ends_with(".snap.new"))
            .collect();
        news.sort();
        for new in news {
            let file = new
                .with_extension("")
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            changes.push(Change {
                fixture: fixture.clone(),
                file,
                detail: "format snapshot changed".to_owned(),
                apply: Apply::Snapshot(new),
            });
        }
        // Image goldens: every PNG the harness wrote for this fixture (`expected.png` for
        // solver cases; `expected_1x.png` and `expected_attr.png` for composite scenes),
        // compared pixel by pixel with the committed file.
        let cand_dir = candidates.join(&fixture);
        let Ok(entries) = fs::read_dir(&cand_dir) else {
            continue;
        };
        let mut pngs: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "png"))
            .collect();
        pngs.sort();
        for candidate in pngs {
            let file = candidate
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let golden = dir.join(&file);
            let new = pixels(&candidate)?;
            let detail = if golden.exists() {
                let old = pixels(&golden)?;
                let differing = old.iter().zip(&new).filter(|(a, b)| a != b).count();
                if differing == 0 && old.len() == new.len() {
                    continue;
                }
                format!(
                    "{differing} of {} pixels differ, OKLab MSE {:.6}",
                    new.len(),
                    mse(
                        &old.iter().map(|p| p.0).collect::<Vec<_>>(),
                        &new.iter().map(|p| p.0).collect::<Vec<_>>()
                    )
                )
            } else {
                "new golden".to_owned()
            };
            changes.push(Change {
                fixture: fixture.clone(),
                file,
                detail,
                apply: Apply::Png {
                    from: candidate,
                    to: golden,
                },
            });
        }
    }
    Ok(changes)
}

fn mse(a: &[Rgb8], b: &[Rgb8]) -> f64 {
    let n = a.len().min(b.len()).max(1);
    let sum: f64 = a
        .iter()
        .zip(b)
        .map(|(&x, &y)| {
            let (p, q) = (oklab_of(x), oklab_of(y));
            [p.l - q.l, p.a - q.a, p.b - q.b]
                .iter()
                .map(|d| f64::from(*d) * f64::from(*d))
                .sum::<f64>()
        })
        .sum();
    sum / n as f64
}

/// One changelog line per fixture, listing every file that changed in it.
fn changelog_lines(changes: &[Change], date: &str, pr: PrRef, reason: &str) -> Vec<String> {
    let mut by_fixture: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for c in changes {
        by_fixture.entry(&c.fixture).or_default().push(&c.file);
    }
    by_fixture
        .into_iter()
        .map(|(fixture, files)| {
            format!(
                "- {date} {pr} **{fixture}** ({}): {reason}",
                files.join(", ")
            )
        })
        .collect()
}

fn append_changelog(
    root: &Path,
    changes: &[Change],
    pr: PrRef,
    reason: &str,
) -> Result<(), String> {
    let path = root.join("fixtures").join("CHANGELOG.md");
    let mut text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    if !text.ends_with('\n') {
        text.push('\n');
    }
    for line in changelog_lines(changes, &today(), pr, reason) {
        text.push_str(&line);
        text.push('\n');
    }
    fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))
}

/// Today's UTC date as `YYYY-MM-DD`.
pub(crate) fn today() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    civil_date(i64::try_from(secs / 86_400).unwrap_or(0))
}

/// Days since 1970-01-01 to a proleptic Gregorian date (Howard Hinnant's algorithm).
fn civil_date(days: i64) -> String {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
mod tests {
    use super::{
        Apply, Change, Mode, PrRef, changelog_lines, check_tree, civil_date, replace_pending, run,
    };
    use std::path::{Path, PathBuf};
    use std::process::Command;

    fn git(dir: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args([
                "-c",
                "user.name=test",
                "-c",
                "user.email=test@example.invalid",
            ])
            .args(args)
            .output()
            .expect("git runs")
            .status
            .success();
        assert!(ok, "git {args:?}");
    }

    /// A throwaway git repository with one committed file.
    fn temp_repo(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("convert64-xtask-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        git(&dir, &["-c", "init.defaultBranch=main", "init", "-q"]);
        std::fs::write(dir.join("a.txt"), "one\n").expect("write");
        git(&dir, &["add", "a.txt"]);
        git(&dir, &["commit", "-q", "-m", "init"]);
        dir
    }

    #[test]
    fn report_mode_runs_on_a_dirty_tree_and_accept_mode_refuses() {
        let repo = temp_repo("dirty");
        assert_eq!(check_tree(&repo, Mode::Report), Ok(false));
        assert_eq!(check_tree(&repo, Mode::Accept), Ok(false));

        std::fs::write(repo.join("a.txt"), "two\n").expect("uncommitted change");
        assert_eq!(check_tree(&repo, Mode::Report), Ok(true));
        let err = check_tree(&repo, Mode::Accept).expect_err("accept refuses");
        assert!(err.contains("dirty") && err.contains("a.txt"), "{err}");

        std::fs::write(repo.join("new.txt"), "untracked\n").expect("untracked file");
        assert_eq!(check_tree(&repo, Mode::Report), Ok(true));
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn civil_dates() {
        assert_eq!(civil_date(0), "1970-01-01");
        assert_eq!(civil_date(11_016), "2000-02-29");
        assert_eq!(civil_date(20_737), "2026-10-11");
    }

    #[test]
    fn accept_without_reason_is_rejected_before_anything_runs() {
        let err = run(true, None, Some(PrRef::Number(1))).expect_err("needs a reason");
        assert!(err.contains("--reason"), "{err}");
        let err = run(true, Some("   "), Some(PrRef::Number(1))).expect_err("blank reason");
        assert!(err.contains("--reason"), "{err}");
        let err = run(true, Some("why"), None).expect_err("needs a PR number");
        assert!(err.contains("--pr"), "{err}");
    }

    #[test]
    fn one_changelog_line_per_fixture() {
        let change = |fixture: &str, file: &str| Change {
            fixture: fixture.to_owned(),
            file: file.to_owned(),
            detail: String::new(),
            apply: Apply::Snapshot(PathBuf::new()),
        };
        let lines = changelog_lines(
            &[
                change("title", "expected.png"),
                change("portrait", "expected_kla.snap"),
                change("portrait", "expected.png"),
            ],
            "2026-10-11",
            PrRef::Number(123),
            "new dither",
        );
        assert_eq!(
            lines,
            [
                "- 2026-10-11 #123 **portrait** (expected_kla.snap, expected.png): new dither",
                "- 2026-10-11 #123 **title** (expected.png): new dither",
            ]
        );
    }

    #[test]
    fn pending_entries_are_set_once_the_pr_exists() {
        let change = |fixture: &str| Change {
            fixture: fixture.to_owned(),
            file: "expected.png".to_owned(),
            detail: String::new(),
            apply: Apply::Snapshot(PathBuf::new()),
        };
        let lines = changelog_lines(&[change("title")], "2026-10-12", PrRef::Pending, "why");
        assert_eq!(
            lines,
            ["- 2026-10-12 #pending **title** (expected.png): why"]
        );
        let text = format!(
            "# Log\n\nMentions #pending in prose.\n- 2026-10-11 #70 **a** (x): y\n{}\n",
            lines[0]
        );
        let (out, n) = replace_pending(&text, 141);
        assert_eq!(n, 1);
        assert!(out.contains("- 2026-10-12 #141 **title** (expected.png): why"));
        assert!(
            out.contains("Mentions #pending in prose."),
            "prose is untouched"
        );
        assert_eq!("pending".parse::<PrRef>(), Ok(PrRef::Pending));
        assert_eq!("#12".parse::<PrRef>(), Ok(PrRef::Number(12)));
        assert!("soon".parse::<PrRef>().is_err());
    }
}
