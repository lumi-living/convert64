//! `cargo xtask png2c64-baselines`: regenerate the png2c64 baselines (#39), archived in
//! `fixtures/archive/png2c64/` and ungated since ADR-013 (#261).
//!
//! png2c64's command-line tool writes a 320×200 PNG preview plus a display PRG; its
//! Koala and Art Studio writers (`prg::koala_raw`, `prg::hires_raw`) are only reachable
//! from its web API. The display PRG carries exactly the same bytes: the displayer
//! loads at `$0801` and the picture data sits at `$2000`, i.e. at file offset
//! `2 + $2000 - $0801 = 6145`, in Koala order (bitmap, screen, colour, background) or
//! Art Studio order (bitmap, screen). This task cuts that block out, prefixes the
//! format's load address, and proves the cut by decoding the result and comparing it
//! pixel for pixel with png2c64's own PNG.

use convert64_core::Palette;
use convert64_formats::{BitmapFile, read_art_studio, read_koala};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::repo_root;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Multicolor,
    Hires,
}

/// Every fixture and the mode its baseline is made in.
const FIXTURES: [(&str, Kind); 6] = [
    ("portrait", Kind::Multicolor),
    ("terrain", Kind::Multicolor),
    ("title", Kind::Multicolor),
    ("gradient", Kind::Multicolor),
    ("photo", Kind::Multicolor),
    ("lineart", Kind::Hires),
];

/// png2c64 arguments per kind. Every setting is spelled out, including png2c64's
/// defaults, so a change of default upstream cannot silently change a baseline.
/// The palette is Pepto, Convert64's default preset (`docs/PDD.md`); png2c64's `pepto`
/// table equals `Palette::pepto_pal()` byte for byte. Hi-res line art uses no dither so
/// anti-aliased strokes threshold cleanly.
fn args(kind: Kind) -> &'static [&'static str] {
    match kind {
        Kind::Multicolor => &[
            "--mode",
            "multicolor",
            "--palette",
            "pepto",
            "--dither",
            "checker",
            "--dither-strength",
            "0.7",
            "--error-clamp",
            "0.1",
            "--gamma",
            "1.5",
        ],
        Kind::Hires => &[
            "--mode",
            "hires",
            "--palette",
            "pepto",
            "--dither",
            "none",
            "--gamma",
            "1.5",
        ],
    }
}

/// File offset of the picture data inside png2c64's display PRG.
const PRG_DATA_OFFSET: usize = 2 + 0x2000 - 0x0801;

struct Outcome {
    name: &'static str,
    file: &'static str,
    total_error: String,
    validate: String,
}

pub(crate) fn run(png2c64: &Path, only: &[String]) -> Result<(), String> {
    let png2c64 = fs::canonicalize(png2c64)
        .map_err(|e| format!("png2c64 binary {}: {e}", png2c64.display()))?;
    let version = output(Command::new(&png2c64).arg("--version"))?
        .trim()
        .to_owned();
    let commit = png2c64_commit(&png2c64);
    println!("using {version}, commit {commit}");

    let mut outcomes = Vec::new();
    for (name, kind) in FIXTURES {
        if !only.is_empty() && !only.iter().any(|o| o == name) {
            continue;
        }
        let outcome = baseline(&png2c64, name, kind)?;
        write_readme_section(&outcome, kind, &version, &commit)?;
        println!(
            "{:<9} {:<13} total error {:<10} validate: {}",
            outcome.name, outcome.file, outcome.total_error, outcome.validate
        );
        outcomes.push(outcome);
    }
    if outcomes.is_empty() {
        return Err(format!("no fixture matches {only:?}"));
    }
    Ok(())
}

pub(crate) fn output(cmd: &mut Command) -> Result<String, String> {
    let out = cmd.output().map_err(|e| format!("{cmd:?}: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "{cmd:?} failed ({}): {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The git commit of the checkout the binary was built in, if it is inside one.
pub(crate) fn png2c64_commit(binary: &Path) -> String {
    binary
        .ancestors()
        .find(|dir| dir.join(".git").exists())
        .and_then(|dir| {
            output(
                Command::new("git")
                    .arg("-C")
                    .arg(dir)
                    .args(["rev-parse", "HEAD"]),
            )
            .ok()
        })
        .map_or_else(|| "unknown".to_owned(), |s| s.trim().to_owned())
}

fn baseline(png2c64: &Path, name: &'static str, kind: Kind) -> Result<Outcome, String> {
    let root = repo_root();
    // Since ADR-013 (#261) png2c64's baselines live in the archive, ungated.
    let fixture = root.join("fixtures/archive/png2c64").join(name);
    let source = root.join("fixtures").join(name).join("source.png");
    let work = root.join("target").join("xtask").join("png2c64").join(name);
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work).map_err(|e| format!("{}: {e}", work.display()))?;
    fs::copy(&source, work.join("source.png"))
        .map_err(|e| format!("{name}/source.png: {e}"))?;

    let stdout = output(
        Command::new(png2c64)
            .args(args(kind))
            .args(["source.png", "baseline.png"])
            .current_dir(&work),
    )?;
    // png2c64 adds per-cell errors into one shared `f32` from several threads, so the
    // last digit varies between runs of an identical result. Two decimals are stable.
    let total_error = stdout
        .lines()
        .find_map(|l| l.split("total error: ").nth(1))
        .and_then(|s| s.trim().parse::<f64>().ok())
        .map_or_else(|| "not reported".to_owned(), |v| format!("{v:.2}"));

    // Cut the picture out of the display PRG.
    let prg = fs::read(work.join("baseline.prg")).map_err(|e| format!("{name} PRG: {e}"))?;
    let (data_len, load, file): (usize, u16, &'static str) = match kind {
        Kind::Multicolor => (10_001, 0x6000, "baseline.kla"),
        Kind::Hires => (9000, 0x2000, "baseline.art"),
    };
    if prg.len() != PRG_DATA_OFFSET + data_len {
        return Err(format!(
            "{name}: PRG is {} bytes, expected {} (displayer layout changed upstream?)",
            prg.len(),
            PRG_DATA_OFFSET + data_len
        ));
    }
    let mut bytes = load.to_le_bytes().to_vec();
    bytes.extend_from_slice(&prg[PRG_DATA_OFFSET..]);
    let read: BitmapFile = match kind {
        Kind::Multicolor => read_koala(&bytes),
        Kind::Hires => read_art_studio(&bytes),
    }
    .map_err(|e| format!("{name}: extracted data does not read: {e}"))?;
    check_against_png(name, &read, &work.join("baseline.png"))?;

    fs::write(fixture.join(file), &bytes).map_err(|e| format!("{name}/{file}: {e}"))?;
    fs::copy(work.join("baseline.png"), fixture.join("baseline.png"))
        .map_err(|e| format!("{name}/baseline.png: {e}"))?;

    let validate = validate(&fixture.join(file))?;
    Ok(Outcome {
        name,
        file,
        total_error,
        validate,
    })
}

/// Decodes the extracted file, renders it through Pepto PAL at 320×200 and requires
/// every pixel to equal png2c64's PNG.
fn check_against_png(name: &str, file: &BitmapFile, png: &Path) -> Result<(), String> {
    let preview = image::open(png)
        .map_err(|e| format!("{name}: {}: {e}", png.display()))?
        .to_rgb8();
    if preview.dimensions() != (320, 200) {
        return Err(format!(
            "{name}: PNG is {:?}, expected 320x200",
            preview.dimensions()
        ));
    }
    let palette = Palette::pepto_pal();
    let indexed = file.image.to_indexed();
    let x_scale = 320 / u32::from(indexed.width());
    for y in 0..200_u32 {
        for x in 0..320_u32 {
            let c = indexed.pixel(
                u16::try_from(x / x_scale).unwrap_or(0),
                u16::try_from(y).unwrap_or(0),
            );
            let rgb = palette.rgb(c);
            let want = preview.get_pixel(x, y).0;
            if [rgb.r, rgb.g, rgb.b] != want {
                return Err(format!(
                    "{name}: pixel ({x}, {y}) is {rgb} in the extracted file but {want:?} in png2c64's PNG"
                ));
            }
        }
    }
    Ok(())
}

/// Runs `convert64 validate <file>` and returns its first output line and exit code.
fn validate(file: &Path) -> Result<String, String> {
    let root = repo_root();
    let relative: PathBuf = file.strip_prefix(&root).unwrap_or(file).to_path_buf();
    let out = Command::new(env!("CARGO"))
        .args(["run", "--quiet", "-p", "convert64-cli", "--", "validate"])
        .arg(&relative)
        .current_dir(&root)
        .output()
        .map_err(|e| format!("convert64 validate: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout);
    let first = text.lines().next().unwrap_or("").to_owned();
    Ok(format!(
        "`{first}` (exit {})",
        out.status
            .code()
            .map_or_else(|| "?".to_owned(), |c| c.to_string())
    ))
}

const START: &str = "<!-- png2c64-baseline:start -->";
const END: &str = "<!-- png2c64-baseline:end -->";

fn write_readme_section(
    o: &Outcome,
    kind: Kind,
    version: &str,
    commit: &str,
) -> Result<(), String> {
    let path = repo_root()
        .join("fixtures/archive/png2c64")
        .join(o.name)
        .join("README.md");
    let readme = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mode = match kind {
        Kind::Multicolor => "multicolor bitmap (Koala)",
        Kind::Hires => "hi-res bitmap (Art Studio)",
    };
    let section = format!(
        "{START}\n## png2c64 baseline\n\n\
         Generated by `cargo xtask png2c64-baselines`; do not edit this section by hand.\n\n\
         - Tool: {version}, commit `{commit}` (<https://github.com/tinic/png2c64>)\n\
         - Mode: {mode}\n\
         - Command, run in a directory holding a copy of `source.png`:\n\n  \
           ```sh\n  png2c64 {} source.png baseline.png\n  ```\n\n\
         - Files: `baseline.png` is png2c64's 320×200 preview; `{}` is the picture data from \
           the display PRG png2c64 writes, with the format's load address in front (see \
           [fixtures/README.md](../../../README.md#png2c64-baselines-archived)).\n\
         - png2c64 total error (sum of per-cell OKLab errors, two decimals): {}\n\
         - `convert64 validate {}`: {}\n\
         {END}",
        args(kind).join(" "),
        o.file,
        o.total_error,
        o.file,
        o.validate
    );
    fs::write(&path, replace_section(&readme, &section))
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// `readme` with the marked section replaced by `section`, or `section` appended.
fn replace_section(readme: &str, section: &str) -> String {
    match (readme.find(START), readme.find(END)) {
        (Some(s), Some(e)) if e > s => {
            format!("{}{section}{}", &readme[..s], &readme[e + END.len()..])
        }
        _ => format!("{}\n\n{section}\n", readme.trim_end()),
    }
}

#[cfg(test)]
mod tests {
    use super::{END, PRG_DATA_OFFSET, START, replace_section};

    #[test]
    fn picture_data_offset_in_the_display_prg() {
        // Load address $0801 in the first two bytes, picture at $2000.
        assert_eq!(PRG_DATA_OFFSET, 6145);
    }

    #[test]
    fn section_is_appended_once_then_replaced() {
        let one = format!("{START}\nfirst\n{END}");
        let two = format!("{START}\nsecond\n{END}");
        let a = replace_section("# Title\n\nText.\n", &one);
        assert_eq!(a, format!("# Title\n\nText.\n\n{one}\n"));
        let b = replace_section(&a, &two);
        assert_eq!(b, format!("# Title\n\nText.\n\n{two}\n"));
        assert_eq!(replace_section(&b, &two), b);
    }
}
