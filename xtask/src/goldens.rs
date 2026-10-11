//! `cargo xtask regen-goldens`: list golden changes, and accept them only with a reason
//! (#41).
//!
//! Runs the golden harness (`crates/convert64-core/tests/golden.rs`) so that it writes
//! candidates instead of failing: `insta` writes `*.snap.new` next to each changed format
//! snapshot (`INSTA_UPDATE=new`, `INSTA_FORCE_PASS=1`), and every solver case writes its
//! rendering to `target/golden-candidates/<fixture>/expected.png`
//! (`CONVERT64_GOLDEN_OUT`). It then compares candidates with the committed goldens.
//!
//! - Without `--accept`: prints what would change, removes the `*.snap.new` files and
//!   exits 1 if anything differs.
//! - With `--accept --reason "..."`: replaces the goldens, appends one line per changed
//!   fixture to `fixtures/CHANGELOG.md` (date, commit, files, reason) and re-runs the
//!   harness, which must now pass.
//!
//! It refuses to run on a dirty git tree: the changelog records the commit whose code
//! produced the goldens, which is only true when nothing is uncommitted.

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

pub(crate) fn run(accept: bool, reason: Option<&str>) -> Result<(), String> {
    let reason = match (accept, reason.map(str::trim)) {
        (true, None | Some("")) => {
            return Err("--accept needs --reason \"why the goldens change\"".to_owned());
        }
        (true, Some(r)) => Some(r.to_owned()),
        (false, _) => None,
    };
    let root = repo_root();
    ensure_clean_tree(&root)?;
    let commit = git(&root, &["rev-parse", "--short=12", "HEAD"])?;

    let candidates = root.join("target").join("golden-candidates");
    let _ = fs::remove_dir_all(&candidates);
    remove_pending_snapshots(&root)?;
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
    append_changelog(&root, &changes, &commit, &reason)?;
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

fn ensure_clean_tree(root: &Path) -> Result<(), String> {
    let status = git(root, &["status", "--porcelain"])?;
    if status.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "the git tree is dirty; commit or stash first, because fixtures/CHANGELOG.md records \
             the commit that produced the goldens:\n{status}"
        ))
    }
}

fn fixture_dirs(root: &Path) -> Result<Vec<PathBuf>, String> {
    let fixtures = root.join("fixtures");
    let mut dirs: Vec<PathBuf> = fs::read_dir(&fixtures)
        .map_err(|e| format!("{}: {e}", fixtures.display()))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.join("thresholds.toml").exists())
        .collect();
    dirs.sort();
    Ok(dirs)
}

fn remove_pending_snapshots(root: &Path) -> Result<(), String> {
    for dir in fixture_dirs(root)? {
        for entry in fs::read_dir(&dir).map_err(|e| e.to_string())?.flatten() {
            let path = entry.path();
            if path.to_string_lossy().ends_with(".snap.new") {
                fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            }
        }
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

fn pixels(path: &Path) -> Result<Vec<Rgb8>, String> {
    let img = image::open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .to_rgb8();
    Ok(img
        .pixels()
        .map(|p| Rgb8::new(p.0[0], p.0[1], p.0[2]))
        .collect())
}

fn collect_changes(root: &Path, candidates: &Path) -> Result<Vec<Change>, String> {
    let mut changes = Vec::new();
    for dir in fixture_dirs(root)? {
        let fixture = dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
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
        // Solver goldens: compare pixels of the candidate and the committed PNG.
        let candidate = candidates.join(&fixture).join("expected.png");
        let golden = dir.join("expected.png");
        if !candidate.exists() {
            continue;
        }
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
                mse(&old, &new)
            )
        } else {
            "new golden".to_owned()
        };
        changes.push(Change {
            fixture,
            file: "expected.png".to_owned(),
            detail,
            apply: Apply::Png {
                from: candidate,
                to: golden,
            },
        });
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
fn changelog_lines(changes: &[Change], date: &str, commit: &str, reason: &str) -> Vec<String> {
    let mut by_fixture: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for c in changes {
        by_fixture.entry(&c.fixture).or_default().push(&c.file);
    }
    by_fixture
        .into_iter()
        .map(|(fixture, files)| {
            format!(
                "- {date} `{commit}` **{fixture}** ({}): {reason}",
                files.join(", ")
            )
        })
        .collect()
}

fn append_changelog(
    root: &Path,
    changes: &[Change],
    commit: &str,
    reason: &str,
) -> Result<(), String> {
    let path = root.join("fixtures").join("CHANGELOG.md");
    let mut text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    if !text.ends_with('\n') {
        text.push('\n');
    }
    for line in changelog_lines(changes, &today(), commit, reason) {
        text.push_str(&line);
        text.push('\n');
    }
    fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))
}

/// Today's UTC date as `YYYY-MM-DD`.
fn today() -> String {
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
    use super::{Apply, Change, changelog_lines, civil_date, run};
    use std::path::PathBuf;

    #[test]
    fn civil_dates() {
        assert_eq!(civil_date(0), "1970-01-01");
        assert_eq!(civil_date(11_016), "2000-02-29");
        assert_eq!(civil_date(20_737), "2026-10-11");
    }

    #[test]
    fn accept_without_reason_is_rejected_before_anything_runs() {
        let err = run(true, None).expect_err("needs a reason");
        assert!(err.contains("--reason"), "{err}");
        let err = run(true, Some("   ")).expect_err("blank reason");
        assert!(err.contains("--reason"), "{err}");
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
            "abc123",
            "new dither",
        );
        assert_eq!(
            lines,
            [
                "- 2026-10-11 `abc123` **portrait** (expected_kla.snap, expected.png): new dither",
                "- 2026-10-11 `abc123` **title** (expected.png): new dither",
            ]
        );
    }
}
