//! Assembler and C header emitters (#161): every emitter, in every data style, on every
//! fixture, gives the bytes the binary writers give; golden text snapshots of each
//! emitter; and, where an assembler or C compiler is installed, the real tool agrees.

mod common;

use common::{FIXTURES, baseline};
use convert64_core::model::BitmapImage;
use convert64_formats::BitmapFormat;
use convert64_formats::asm::{Assembler, DataStyle, EmitOptions, emit};
use std::path::{Path, PathBuf};
use std::process::Command;

const STYLES: [DataStyle; 3] = [
    DataStyle::Bytes,
    DataStyle::IncludeRaw,
    DataStyle::IncludePicture,
];

/// The picture file's body: what assembling the emitted source must produce.
fn body(image: &BitmapImage) -> Vec<u8> {
    let format = BitmapFormat::for_mode(image.mode()).expect("bitmap");
    format.write(image).expect("writes")[2..].to_vec()
}

/// Parses a `$xx` / `0xNN` / decimal number.
fn number(s: &str) -> u32 {
    let s = s.trim();
    if let Some(h) = s.strip_prefix('$') {
        u32::from_str_radix(h, 16).expect("hex")
    } else if let Some(h) = s.strip_prefix("0x") {
        u32::from_str_radix(h, 16).expect("hex")
    } else {
        s.parse().expect("decimal")
    }
}

/// A just-enough "assembler" for the emitted sources: byte lines and include lines, in
/// order. `files` are the files `emit` returned next to the source.
fn data_of(asm: Assembler, source: &str, files: &[(PathBuf, Vec<u8>)]) -> Vec<u8> {
    let find = |name: &str| -> Vec<u8> {
        files
            .iter()
            .find(|(p, _)| p.to_str() == Some(name))
            .unwrap_or_else(|| panic!("{asm}: no returned file {name}"))
            .1
            .clone()
    };
    let mut out = Vec::new();
    for line in source.lines().map(str::trim) {
        if asm == Assembler::CHeader {
            if line.starts_with("0x") {
                out.extend(
                    line.split(',')
                        .filter(|s| !s.trim().is_empty())
                        .map(|s| u8::try_from(number(s)).expect("byte")),
                );
            }
            continue;
        }
        let (directive, rest) = line.split_once(' ').unwrap_or((line, ""));
        match directive {
            ".byte" | "!byte" => {
                out.extend(
                    rest.split(',')
                        .map(|s| u8::try_from(number(s)).expect("byte")),
                );
            }
            ".import" | "!binary" | ".binary" | ".incbin" => {
                let rest = rest.strip_prefix("c64 ").map_or(rest, |r| r);
                let rest = rest.strip_prefix("binary ").map_or(rest, |r| r);
                let (name, skip) = rest
                    .trim_start_matches('"')
                    .split_once('"')
                    .expect("quoted");
                let mut bytes = find(name);
                let skip_header = directive == ".import" && line.contains(" c64 ")
                    || skip.trim_start_matches(',').trim() == "2";
                if skip_header {
                    bytes.drain(..2);
                }
                out.extend(bytes);
            }
            _ => {}
        }
    }
    out
}

#[test]
fn every_emitter_gives_the_writers_bytes() {
    for (name, format) in FIXTURES {
        let image = baseline(name, format);
        let want = body(&image);
        for asm in Assembler::ALL {
            for style in STYLES {
                let files = emit(asm, &image, &EmitOptions::new(name).with_data(style))
                    .unwrap_or_else(|e| panic!("{name} {asm}: {e}"));
                assert_eq!(
                    files[0].0.to_str(),
                    Some(format!("{name}.{}", asm.extension()).as_str())
                );
                let source = std::str::from_utf8(&files[0].1).expect("UTF-8 source");
                assert_eq!(data_of(asm, source, &files), want, "{name} {asm} {style:?}");
                let origin = format!("${:04x}", format.default_load_address());
                assert!(source.contains(&origin), "{name} {asm}: no {origin}");
                if asm == Assembler::CHeader || style == DataStyle::Bytes {
                    assert_eq!(files.len(), 1, "{name} {asm} {style:?}: source only");
                }
            }
        }
    }
}

/// Golden text: each emitter on `lineart` (hi-res) as byte lines, and on `portrait`
/// (multicolor) with each include style.
#[test]
fn golden_sources() {
    let source = |name: &str, format, asm, style| {
        let files = emit(
            asm,
            &baseline(name, format),
            &EmitOptions::new(name).with_data(style),
        )
        .expect("emits");
        String::from_utf8(files[0].1.clone()).expect("UTF-8")
    };
    for asm in Assembler::ALL {
        insta::assert_snapshot!(
            format!("lineart_bytes_{}", asm.id()),
            source("lineart", BitmapFormat::ArtStudio, asm, DataStyle::Bytes)
        );
        if asm == Assembler::CHeader {
            continue;
        }
        insta::assert_snapshot!(
            format!("portrait_include_raw_{}", asm.id()),
            source("portrait", BitmapFormat::Koala, asm, DataStyle::IncludeRaw)
        );
        insta::assert_snapshot!(
            format!("portrait_include_picture_{}", asm.id()),
            source(
                "portrait",
                BitmapFormat::Koala,
                asm,
                DataStyle::IncludePicture
            )
        );
    }
}

// ---------------------------------------------------------------------------------------
// Real tools, where installed (skipped otherwise; CI need not have them). KickAssembler
// runs when `KICKASS_JAR` names its jar.

fn tool(cmd: &str) -> bool {
    Command::new(cmd)
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success() || !o.stdout.is_empty() || !o.stderr.is_empty())
}

fn scratch(asm: Assembler, style: DataStyle, name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("asm")
        .join(format!("{}-{style:?}-{name}", asm.id()));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Writes `emit`'s files into a scratch directory, runs `build` there and returns the
/// bytes of `out.bin`.
fn assemble(
    asm: Assembler,
    style: DataStyle,
    name: &str,
    image: &BitmapImage,
    build: impl Fn(&Path, &str) -> Command,
) -> Vec<u8> {
    let dir = scratch(asm, style, name);
    let files = emit(asm, image, &EmitOptions::new(name).with_data(style)).expect("emits");
    for (p, b) in &files {
        std::fs::write(dir.join(p), b).expect("write");
    }
    let src = files[0].0.to_str().expect("utf-8 name").to_owned();
    let out = build(&dir, &src).current_dir(&dir).output().expect("runs");
    assert!(
        out.status.success(),
        "{asm} failed:\n{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::read(dir.join("out.bin")).expect("out.bin")
}

#[test]
fn installed_assemblers_agree() {
    let mut ran = Vec::new();
    for (name, format) in [
        ("portrait", BitmapFormat::Koala),
        ("lineart", BitmapFormat::ArtStudio),
    ] {
        let image = baseline(name, format);
        let want = body(&image);
        for style in STYLES {
            if tool("acme") {
                let got = assemble(Assembler::Acme, style, name, &image, |_, src| {
                    let mut c = Command::new("acme");
                    c.args(["-f", "plain", "-o", "out.bin", src]);
                    c
                });
                assert_eq!(got, want, "acme {name} {style:?}");
                ran.push("acme");
            }
            if tool("64tass") {
                let got = assemble(Assembler::Tass64, style, name, &image, |_, src| {
                    let mut c = Command::new("64tass");
                    c.args(["--nostart", "-o", "out.bin", src]);
                    c
                });
                assert_eq!(got, want, "64tass {name} {style:?}");
                ran.push("64tass");
            }
            if tool("ca65") && tool("ld65") {
                let seg = name.to_ascii_uppercase();
                let got = assemble(Assembler::Ca65, style, name, &image, |dir, src| {
                    std::fs::write(
                        dir.join("link.cfg"),
                        format!(
                            "MEMORY {{ PIC: start = ${:04x}, size = $4000, file = %O; }}\n\
                             SEGMENTS {{ {seg}: load = PIC, type = ro; }}\n",
                            format.default_load_address()
                        ),
                    )
                    .expect("cfg");
                    let mut c = Command::new("sh");
                    c.args([
                        "-c",
                        &format!("ca65 -o pic.o {src} && ld65 -C link.cfg -o out.bin pic.o"),
                    ]);
                    c
                });
                assert_eq!(got, want, "ca65 {name} {style:?}");
                ran.push("ca65");
            }
            if let Ok(jar) = std::env::var("KICKASS_JAR") {
                let got = assemble(Assembler::KickAssembler, style, name, &image, |_, src| {
                    let mut c = Command::new("java");
                    c.args(["-jar", &jar, src, "-binfile", "-o", "out.bin"]);
                    c
                });
                assert_eq!(got, want, "KickAssembler {name} {style:?}");
                ran.push("kickass");
            }
        }
        if tool("cc") {
            // A C program that includes the header and writes the arrays out.
            let style = DataStyle::Bytes;
            let got = assemble(Assembler::CHeader, style, name, &image, |dir, src| {
                let label = name.to_owned();
                let arrays: Vec<String> = ["bitmap", "screen", "colour", "background"]
                    .iter()
                    .take(if format == BitmapFormat::Koala { 4 } else { 2 })
                    .map(|b| format!("fwrite({label}_{b}, 1, sizeof {label}_{b}, f);"))
                    .collect();
                std::fs::write(
                    dir.join("main.c"),
                    format!(
                        "#include <stdio.h>\n#include \"{src}\"\nint main(void) {{\n\
                         FILE *f = fopen(\"out.bin\", \"wb\");\n{}\nreturn fclose(f);\n}}\n",
                        arrays.join("\n")
                    ),
                )
                .expect("main.c");
                let mut c = Command::new("sh");
                c.args(["-c", "cc -std=c99 -Wall -Werror -o main main.c && ./main"]);
                c
            });
            assert_eq!(got, want, "C header {name}");
            ran.push("cc");
        }
    }
    ran.dedup();
    println!("assembled with: {ran:?} (others not installed)");
}
