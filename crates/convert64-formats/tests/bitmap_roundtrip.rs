//! Koala and Art Studio round trips on the six fixtures (#158): our own solves
//! `write → read → identical`, and the png2c64 baselines `read → write` byte for byte.

mod common;

use common::{FIXTURES, MODES, baseline_bytes, solve_all, solved};
use convert64_core::{validate, validate_placement};
use convert64_formats::{BitmapFile, BitmapFormat};

/// Every fixture solved in both modes, written in that mode's format, reads back as the
/// same `BitmapImage`, at the default and the other format's load address and under
/// every extension the format accepts.
#[test]
fn solves_write_read_identical() {
    solve_all();
    for (name, _) in FIXTURES {
        for mode in MODES {
            let image = solved(name, mode);
            assert_eq!(validate(&*image), Ok(()), "{name} {mode}");
            let format = BitmapFormat::for_mode(mode).expect("bitmap mode");
            let bytes = format.write(&image).expect("mode matches");
            assert_eq!(bytes.len(), format.sizes()[0], "{name} {mode}");
            for ext in format.extensions() {
                let read = BitmapFormat::from_extension(ext)
                    .expect("known extension")
                    .read(&bytes)
                    .unwrap_or_else(|e| panic!("{name} {mode} .{ext}: {e}"));
                assert_eq!(read.image, *image, "{name} {mode} .{ext}");
                assert_eq!(read.load_address, format.default_load_address());
                assert_eq!(validate_placement(&read.placement()), Ok(()));
            }
            for load_address in [0x2000, 0x6000] {
                let file = BitmapFile {
                    format,
                    load_address,
                    image: (*image).clone(),
                };
                let bytes = file.to_bytes().expect("mode matches");
                assert_eq!(bytes[..2], load_address.to_le_bytes());
                assert_eq!(format.read(&bytes).expect("reads"), file, "{name} {mode}");
            }
            // The other format refuses the picture rather than writing a wrong file.
            let other = match format {
                BitmapFormat::Koala => BitmapFormat::ArtStudio,
                BitmapFormat::ArtStudio => BitmapFormat::Koala,
            };
            assert!(other.write(&image).is_err(), "{name} {mode}");
        }
    }
}

/// The png2c64 baselines read and write back byte for byte, and the file size alone
/// identifies their format.
#[test]
fn baselines_read_write_byte_identical() {
    for (name, format) in FIXTURES {
        let bytes = baseline_bytes(name, format);
        assert_eq!(BitmapFormat::from_size(bytes.len()), Some(format), "{name}");
        let file = format
            .read(&bytes)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(file.to_bytes().expect("writes"), bytes, "{name}");
        assert_eq!(format.write(&file.image).expect("writes"), bytes, "{name}");
    }
}

/// Truncated and oversized copies of real files are refused with rule 6 and a message
/// that says which.
#[test]
fn truncated_and_oversized_files_are_refused() {
    for (name, format) in FIXTURES {
        let bytes = baseline_bytes(name, format);
        let short = format
            .read(&bytes[..bytes.len() - 1])
            .expect_err("truncated");
        assert_eq!(short.rule_number(), Some(6), "{name}");
        assert!(
            short.to_string().contains("truncated: 1 byte short"),
            "{short}"
        );
        let mut long = bytes.clone();
        long.extend([0; 100]);
        let long = format.read(&long).expect_err("oversized");
        assert_eq!(long.rule_number(), Some(6), "{name}");
        assert!(long.to_string().contains("oversized"), "{long}");
    }
}
