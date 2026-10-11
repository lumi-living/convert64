//! End-to-end tests of `convert64 validate` on files built from the byte layouts in
//! `docs/reference/file-formats.md`.

use std::path::PathBuf;
use std::process::{Command, Output};

/// A legal Koala file: load address, 8000 bitmap, 1000 screen, 1000 colour, background.
fn koala(load: u16, background: u8) -> Vec<u8> {
    let mut f = load.to_le_bytes().to_vec();
    f.extend((0..8000_u32).map(|i| (i * 31 % 251) as u8));
    f.extend((0..1000_u32).map(|i| (i * 17 % 256) as u8));
    f.extend((0..1000_u32).map(|i| (i % 16) as u8));
    f.push(background);
    f
}

/// A legal Art Studio file.
fn art_studio(load: u16) -> Vec<u8> {
    let mut f = load.to_le_bytes().to_vec();
    f.extend((0..8000_u32).map(|i| (i * 7 % 256) as u8));
    f.extend((0..1000_u32).map(|i| (i * 29 % 256) as u8));
    f
}

/// Writes `bytes` to a fresh file in the system temp directory.
fn temp_file(name: &str, bytes: &[u8]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("convert64-cli-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join(name);
    std::fs::write(&path, bytes).expect("write temp file");
    path
}

fn convert64(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_convert64"))
        .args(args)
        .output()
        .expect("run convert64")
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

#[test]
fn legal_koala_prints_legal_and_exits_0() {
    let path = temp_file("legal.kla", &koala(0x6000, 0));
    let out = convert64(&["validate", path.to_str().expect("utf-8 path")]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(stdout(&out), "legal\n");
}

#[test]
fn legal_art_studio_prints_legal_and_exits_0() {
    let path = temp_file("legal.art", &art_studio(0x2000));
    let out = convert64(&["validate", path.to_str().expect("utf-8 path")]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(stdout(&out), "legal\n");
}

#[test]
fn corrupted_koala_lists_every_violation_and_exits_1() {
    // Background byte $1F (rule 2) and a load address of $1000, which puts the bitmap
    // in the char-ROM shadow and off its 8 KB boundary (rule 5, twice).
    let path = temp_file("corrupted.kla", &koala(0x1000, 0x1F));
    let out = convert64(&["validate", path.to_str().expect("utf-8 path")]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let text = stdout(&out);
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines[0].starts_with("illegal: 3 violations in "), "{text}");
    assert!(lines[0].ends_with("(Koala)"), "{text}");
    assert_eq!(
        lines[1],
        "rule 2: file: background: colour value 31 ($1F) is not a palette index 0-15"
    );
    assert_eq!(
        lines[2],
        "rule 5: bitmap at $1000: bitmap must start on a $2000-byte boundary"
    );
    assert_eq!(
        lines[3],
        "rule 5: bitmap at $1000: $1000-$2F3F overlaps the char-ROM shadow $1000-$1FFF, where the VIC-II reads ROM"
    );
    assert_eq!(lines.len(), 4, "{text}");
}

#[test]
fn json_output_for_legal_and_illegal_files() {
    let legal = temp_file("json-legal.kla", &koala(0x6000, 6));
    let out = convert64(&["validate", "--json", legal.to_str().expect("utf-8 path")]);
    assert_eq!(out.status.code(), Some(0));
    let text = stdout(&out);
    assert!(
        text.contains(
            r#""format":"Koala","mode":"multicolor bitmap","legal":true,"violations":[]"#
        ),
        "{text}"
    );

    let bad = temp_file("json-bad.kla", &koala(0x6100, 0));
    let out = convert64(&["validate", "--json", bad.to_str().expect("utf-8 path")]);
    assert_eq!(out.status.code(), Some(1));
    let text = stdout(&out);
    assert!(text.contains(r#""legal":false"#), "{text}");
    assert!(
        text.contains(r#"{"rule":5,"location":{"kind":"address","block":"bitmap","address":24832},"message":"bitmap must start on a $2000-byte boundary"}"#),
        "{text}"
    );
}

#[test]
fn wrong_size_is_rule_6() {
    let path = temp_file("short.kla", &[0; 10_000]);
    let out = convert64(&["validate", path.to_str().expect("utf-8 path")]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stdout(&out).contains("rule 6: file: Koala files are 10003 bytes; this one is 10000"),
        "{}",
        stdout(&out)
    );
}

#[test]
fn format_is_guessed_from_size_or_forced() {
    let path = temp_file("noext.bin", &art_studio(0x2000));
    let out = convert64(&["validate", path.to_str().expect("utf-8 path")]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");

    // Forcing Koala on an Art Studio-sized file is a size violation.
    let out = convert64(&[
        "validate",
        "--format",
        "kla",
        path.to_str().expect("utf-8 path"),
    ]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
}

#[test]
fn unreadable_or_unknown_files_exit_2() {
    let out = convert64(&["validate", "/nonexistent/convert64/picture.kla"]);
    assert_eq!(out.status.code(), Some(2));
    let path = temp_file("mystery.bin", &[0; 123]);
    let out = convert64(&["validate", path.to_str().expect("utf-8 path")]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("cannot tell the format"));
}
