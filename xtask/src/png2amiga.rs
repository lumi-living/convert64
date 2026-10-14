//! `cargo xtask png2amiga` (#261, ADR-013): build the reference png2amiga and render
//! everything the parity tests compare with.
//!
//! - `build`: clones <https://github.com/tinic/png2amiga> into `target/xtask/png2amiga-src`
//!   (once; a blob-less clone), checks out [`PIN`], fetches the three submodules the
//!   command-line tool needs and builds it with CMake into
//!   `target/xtask/png2amiga-build/<variant>`. The `reference` variant is GCC with strict
//!   IEEE arithmetic (`-ffp-contract=off`, appended by a compiler launcher after
//!   png2amiga's own `-ffp-contract=fast`) and the maths shim
//!   `docs/reference/png2amiga-reference/crmath.c` linked in; the others exist for the
//!   strict-versus-default table in `docs/reference/png2amiga-reference.md`.
//! - `renders`: runs a png2amiga binary (default: the `reference` build) on every fixture
//!   with its baseline's settings (`baseline.kla`/`.art`, `baseline.png`, the README
//!   section), with `--match-range` (`baseline_match_range.png`) and, on the portrait,
//!   `--sharpen ±0.5`; and on the dither-preview test card (multicolor and hi-res) and the
//!   portrait for every method png2amiga's C64 path accepts
//!   (`docs/reference/dither-previews/png2amiga-*/<id>.png`).
//!
//! png2amiga writes the picture as Koala (`.koa`) or hi-res (`.hir`) files whose bytes
//! are Convert64's Koala and Art Studio layouts; the PNGs are those files decoded and
//! drawn at 320 × 200 through Pepto, so every render is exactly png2amiga's indices.

use crate::baselines::output;
use crate::repo_root;
use convert64_core::Palette;
use convert64_core::solve::png2amiga_ref::dither::Method;
use convert64_formats::{BitmapFile, read_art_studio, read_koala};
use image::{Rgb, RgbImage};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The pinned png2amiga commit (v1.111.2, 2026-10-05). Moving it is a deliberate PR that
/// regenerates every render (ADR-013).
pub(crate) const PIN: &str = "4a8777653536ac07074a650ee86860347c8755fb";
const REPO: &str = "https://github.com/tinic/png2amiga";
/// The submodules png2amiga's CMake build needs (its sentinel files); the Amiga debugger
/// submodule is not needed for the command-line tool.
const SUBMODULES: [&str; 3] = [
    "third_party/libwebp",
    "third_party/ssimulacra2",
    "third_party/constixel",
];

/// A build of png2amiga.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum Variant {
    /// GCC, strict IEEE (`-ffp-contract=off`) and the correctly rounded maths shim: the
    /// reference.
    Reference,
    /// GCC, strict IEEE, the platform's libm.
    Strict,
    /// GCC with png2amiga's own flags (`-ffp-contract=fast`).
    Default,
    /// The system `c++` (Apple clang) with png2amiga's own flags.
    Clang,
}

impl Variant {
    fn dir(self) -> &'static str {
        match self {
            Self::Reference => "reference",
            Self::Strict => "strict",
            Self::Default => "default",
            Self::Clang => "clang",
        }
    }
}

fn xtask_dir() -> PathBuf {
    repo_root().join("target/xtask")
}

fn src_dir() -> PathBuf {
    xtask_dir().join("png2amiga-src")
}

/// The binary a variant builds to.
pub(crate) fn binary(variant: Variant) -> PathBuf {
    xtask_dir()
        .join("png2amiga-build")
        .join(variant.dir())
        .join("png2amiga")
}

fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    output(Command::new("git").arg("-C").arg(dir).args(args))
}

/// Clones (once), checks out the pin and its submodules.
fn checkout() -> Result<PathBuf, String> {
    let src = src_dir();
    if !src.join(".git").exists() {
        fs::create_dir_all(xtask_dir()).map_err(|e| e.to_string())?;
        println!("cloning {REPO} into {}", src.display());
        output(
            Command::new("git")
                .args(["clone", "--filter=blob:none", "--no-checkout", REPO])
                .arg(&src),
        )?;
    }
    if git(&src, &["cat-file", "-e", &format!("{PIN}^{{commit}}")]).is_err() {
        git(&src, &["fetch", "origin"])?;
    }
    git(&src, &["checkout", "--quiet", "--detach", PIN])?;
    let mut args = vec!["submodule", "update", "--init", "--force"];
    args.extend(SUBMODULES);
    git(&src, &args)?;
    Ok(src)
}

/// `cargo xtask png2amiga build`.
pub(crate) fn build(variant: Variant, compiler: &str) -> Result<(), String> {
    let src = checkout()?;
    let root = repo_root();
    let out = binary(variant);
    let bdir = out.parent().expect("a build directory").to_path_buf();
    fs::create_dir_all(&bdir).map_err(|e| format!("{}: {e}", bdir.display()))?;
    let cc = compiler.replace("g++", "gcc");
    let mut cmake = Command::new("cmake");
    cmake
        .arg("-S")
        .arg(&src)
        .arg("-B")
        .arg(&bdir)
        .arg("-DCMAKE_BUILD_TYPE=Release");
    match variant {
        Variant::Clang => {
            cmake.args(["-DCMAKE_CXX_COMPILER=c++", "-DCMAKE_C_COMPILER=cc"]);
        }
        _ => {
            cmake
                .arg(format!("-DCMAKE_CXX_COMPILER={compiler}"))
                .arg(format!("-DCMAKE_C_COMPILER={cc}"));
        }
    }
    if matches!(variant, Variant::Reference | Variant::Strict) {
        // png2amiga adds `-ffp-contract=fast` as a target option, after any
        // CMAKE_CXX_FLAGS; a launcher appending ours makes it the last word.
        let launcher = bdir.join("strict-launcher.sh");
        fs::write(
            &launcher,
            "#!/bin/sh\n# Appends -ffp-contract=off after png2amiga's own -ffp-contract=fast.\n\
             exec \"$@\" -ffp-contract=off\n",
        )
        .map_err(|e| e.to_string())?;
        output(Command::new("chmod").arg("+x").arg(&launcher))?;
        cmake.arg(format!(
            "-DCMAKE_CXX_COMPILER_LAUNCHER={}",
            launcher.display()
        ));
    }
    if variant == Variant::Reference {
        let shim = root.join("docs/reference/png2amiga-reference/crmath.c");
        let obj = bdir.join("crmath.o");
        output(
            Command::new(&cc)
                .args(["-O1", "-fno-builtin", "-ffp-contract=off", "-c"])
                .arg(&shim)
                .arg("-o")
                .arg(&obj),
        )?;
        cmake.arg(format!("-DCMAKE_EXE_LINKER_FLAGS={}", obj.display()));
    }
    println!("configuring {} ({})", variant.dir(), bdir.display());
    output(&mut cmake)?;
    println!("building (a few minutes)");
    output(Command::new("cmake").arg("--build").arg(&bdir).args([
        "--target",
        "png2amiga",
        "-j",
        "8",
    ]))?;
    let version = output(Command::new(&out).arg("--version"))?;
    println!("{} → {}", version.trim(), out.display());
    Ok(())
}

/// The kind of picture a render is.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Multicolor,
    Hires,
}

/// Every fixture and its baseline's mode.
const FIXTURES: [(&str, Kind); 6] = [
    ("portrait", Kind::Multicolor),
    ("terrain", Kind::Multicolor),
    ("title", Kind::Multicolor),
    ("gradient", Kind::Multicolor),
    ("photo", Kind::Multicolor),
    ("lineart", Kind::Hires),
];

/// The baselines' png2amiga arguments: Convert64's defaults (#39; unchanged by ADR-013,
/// whose defaults are a separate decision), every setting spelled out.
pub(crate) fn baseline_args(multicolor: bool) -> Vec<&'static str> {
    if multicolor {
        vec![
            "--mode",
            "c64-multicolor",
            "--c64-palette",
            "pepto",
            "--c64-metric",
            "mse",
            "--dither",
            "checker",
            "--dither-strength",
            "0.7",
            "--error-clamp",
            "0.1",
            "--gamma",
            "1.5",
        ]
    } else {
        vec![
            "--mode",
            "c64-hires",
            "--c64-palette",
            "pepto",
            "--c64-metric",
            "mse",
            "--dither",
            "none",
            "--gamma",
            "1.5",
        ]
    }
}

/// A preview set: where it goes, its input, its mode and its arguments besides the method.
pub(crate) struct PreviewSet {
    pub dir: &'static str,
    pub input: &'static str,
    pub hires: bool,
    pub args: &'static [&'static str],
}

/// The method previews: the card with Convert64's settings at strength 1 (multicolor
/// and hi-res), the portrait with png2amiga's own C64 defaults (gamma 1, error clamp
/// 0.35, the blur metric) at strength 1.
pub(crate) const PREVIEWS: [PreviewSet; 3] = [
    PreviewSet {
        dir: "png2amiga-card",
        input: "docs/reference/dither-previews/testcard.png",
        hires: false,
        args: &[
            "--mode",
            "c64-multicolor",
            "--c64-palette",
            "pepto",
            "--c64-metric",
            "mse",
            "--dither-strength",
            "1.0",
            "--error-clamp",
            "0.1",
            "--gamma",
            "1.5",
        ],
    },
    PreviewSet {
        dir: "png2amiga-card-hires",
        input: "docs/reference/dither-previews/testcard.png",
        hires: true,
        args: &[
            "--mode",
            "c64-hires",
            "--c64-palette",
            "pepto",
            "--c64-metric",
            "mse",
            "--dither-strength",
            "1.0",
            "--error-clamp",
            "0.1",
            "--gamma",
            "1.5",
        ],
    },
    PreviewSet {
        dir: "png2amiga-portrait",
        input: "fixtures/portrait/source.png",
        hires: false,
        args: &[
            "--mode",
            "c64-multicolor",
            "--c64-palette",
            "pepto",
            "--c64-metric",
            "blur",
            "--dither-strength",
            "1.0",
            "--error-clamp",
            "0.35",
            "--gamma",
            "1.0",
        ],
    },
];

/// Runs png2amiga on `input` with `args`, writing the picture file into `work`, and
/// returns the file's bytes, its decoded picture and png2amiga's reported S2.
fn render(
    bin: &Path,
    input: &Path,
    args: &[&str],
    hires: bool,
    work: &Path,
) -> Result<(Vec<u8>, BitmapFile, Option<f64>), String> {
    let out = work.join(if hires { "out.hir" } else { "out.koa" });
    let _ = fs::remove_file(&out);
    let stdout = output(Command::new(bin).arg(input).arg(&out).args(args))?;
    let s2 = stdout
        .split("S2: ")
        .nth(1)
        .and_then(|s| s.split_whitespace().next())
        .and_then(|s| s.parse().ok());
    let bytes = fs::read(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    let file = if hires {
        read_art_studio(&bytes)
    } else {
        read_koala(&bytes)
    }
    .map_err(|e| format!("png2amiga's {} does not read: {e}", out.display()))?;
    Ok((bytes, file, s2))
}

/// A decoded picture drawn at 320 × 200 through Pepto (multicolor pixels doubled).
pub(crate) fn draw(file: &BitmapFile) -> RgbImage {
    let palette = Palette::pepto_pal();
    let indexed = file.image.to_indexed();
    let xs = 320 / u32::from(indexed.width());
    RgbImage::from_fn(320, 200, |x, y| {
        let c = indexed.pixel(
            u16::try_from(x / xs).unwrap_or(0),
            u16::try_from(y).unwrap_or(0),
        );
        let rgb = palette.rgb(c);
        Rgb([rgb.r, rgb.g, rgb.b])
    })
}

fn save_png(img: &RgbImage, path: &Path) -> Result<bool, String> {
    let mut bytes = Vec::new();
    img.write_to(
        &mut std::io::Cursor::new(&mut bytes),
        image::ImageFormat::Png,
    )
    .map_err(|e| format!("{}: {e}", path.display()))?;
    if fs::read(path).ok().as_deref() == Some(&bytes[..]) {
        return Ok(false);
    }
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    fs::write(path, &bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(true)
}

/// `cargo xtask png2amiga renders`.
pub(crate) fn renders(bin: Option<PathBuf>, skip_previews: bool) -> Result<(), String> {
    let bin = bin.unwrap_or_else(|| binary(Variant::Reference));
    let bin =
        fs::canonicalize(&bin).map_err(|e| format!("png2amiga binary {}: {e}", bin.display()))?;
    let version = output(Command::new(&bin).arg("--version"))?
        .trim()
        .to_owned();
    println!("using {version} ({})", bin.display());
    let root = repo_root();
    let work = xtask_dir().join("png2amiga-renders");
    fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let mut changed = Vec::new();
    for (name, kind) in FIXTURES {
        let dir = root.join("fixtures").join(name);
        let src = dir.join("source.png");
        let hires = kind == Kind::Hires;
        let args = baseline_args(!hires);
        let (bytes, file, s2) = render(&bin, &src, &args, hires, &work)?;
        let ext = if hires { "art" } else { "kla" };
        let picture = dir.join(format!("baseline.{ext}"));
        if fs::read(&picture).ok().as_deref() != Some(&bytes[..]) {
            fs::write(&picture, &bytes).map_err(|e| format!("{}: {e}", picture.display()))?;
            changed.push(format!("{name}/baseline.{ext}"));
        }
        if save_png(&draw(&file), &dir.join("baseline.png"))? {
            changed.push(format!("{name}/baseline.png"));
        }
        let mut extra = vec![
            ("match_range", vec!["--match-range"]),
            ("refine", vec!["--cell-refine"]),
        ];
        if name == "portrait" {
            extra.push(("sharpen_0.5", vec!["--sharpen", "0.5"]));
            extra.push(("sharpen_-0.5", vec!["--sharpen", "-0.5"]));
        }
        let mut s2s = vec![("baseline".to_owned(), s2)];
        for (suffix, more) in extra {
            let mut a = args.clone();
            a.extend(more);
            let (_, f, s) = render(&bin, &src, &a, hires, &work)?;
            let png = dir.join(format!("baseline_{suffix}.png"));
            if save_png(&draw(&f), &png)? {
                changed.push(format!("{name}/baseline_{suffix}.png"));
            }
            s2s.push((format!("baseline_{suffix}"), s));
        }
        write_readme_section(name, hires, &version, &args, &s2s)?;
        println!(
            "{name:<9} {}",
            s2s.iter()
                .map(|(n, s)| format!("{n} S2 {}", s.map_or("?".to_owned(), |v| format!("{v:.2}"))))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    if !skip_previews {
        for set in &PREVIEWS {
            let out = root.join("docs/reference/dither-previews").join(set.dir);
            let input = root.join(set.input);
            for m in Method::ALL {
                let mut args = set.args.to_vec();
                args.extend(["--dither", m.name()]);
                let (_, f, _) = render(&bin, &input, &args, set.hires, &work)?;
                if save_png(&draw(&f), &out.join(format!("{}.png", m.name())))? {
                    changed.push(format!("{}/{}.png", set.dir, m.name()));
                }
            }
            println!("{}: {} methods", set.dir, Method::ALL.len());
        }
    }
    if changed.is_empty() {
        println!("every render unchanged");
    } else {
        println!("{} files changed:", changed.len());
        for c in &changed {
            println!("  {c}");
        }
    }
    Ok(())
}

pub(crate) const START: &str = "<!-- png2amiga-baseline:start -->";
pub(crate) const END: &str = "<!-- png2amiga-baseline:end -->";

fn write_readme_section(
    name: &str,
    hires: bool,
    version: &str,
    args: &[&str],
    s2s: &[(String, Option<f64>)],
) -> Result<(), String> {
    let path = repo_root().join("fixtures").join(name).join("README.md");
    let readme = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let (mode, file, ext) = if hires {
        ("hi-res bitmap (Art Studio)", "baseline.art", "hir")
    } else {
        ("multicolor bitmap (Koala)", "baseline.kla", "koa")
    };
    let extra = if name == "portrait" {
        " `baseline_sharpen_0.5.png` and `baseline_sharpen_-0.5.png` add `--sharpen 0.5` and \
         `--sharpen -0.5`."
    } else {
        ""
    };
    let scores = s2s
        .iter()
        .map(|(n, s)| {
            format!(
                "`{n}` {}",
                s.map_or("not reported".to_owned(), |v| format!("{v:.2}"))
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let section = format!(
        "{START}\n## png2amiga baseline\n\n\
         Generated by `cargo xtask png2amiga renders`; do not edit this section by hand.\n\n\
         - Tool: {version}, commit `{PIN}` (<{REPO}>), the reference build \
           ([png2amiga-reference.md](../../docs/reference/png2amiga-reference.md))\n\
         - Mode: {mode}\n\
         - Command:\n\n  \
           ```sh\n  png2amiga source.png baseline.{ext} {}\n  ```\n\n\
         - Files: `{file}` is the picture png2amiga writes (`.{ext}`, the same bytes); \
           `baseline.png` is it drawn at 320 × 200 through Pepto. `baseline_match_range.png` \
           adds `--match-range`, `baseline_refine.png` `--cell-refine`.{extra}\n\
         - png2amiga's SSIMULACRA2 (its own, against its preprocessed input): {scores}\n\
         {END}",
        args.join(" ")
    );
    let new = match (readme.find(START), readme.find(END)) {
        (Some(s), Some(e)) if e > s => {
            format!("{}{section}{}", &readme[..s], &readme[e + END.len()..])
        }
        _ => format!("{}\n\n{section}\n", readme.trim_end()),
    };
    fs::write(&path, new).map_err(|e| format!("{}: {e}", path.display()))
}
