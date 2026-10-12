//! Raw bitmap set round trips on the six fixtures (#159): every fixture solved in both
//! modes is written as a raw set and read back from the manifest as the identical
//! picture; the png2c64 baselines too.

mod common;

use common::{FIXTURES, MODES, baseline, solve_all, solved};
use convert64_core::Palette;
use convert64_core::model::BitmapImage;
use convert64_formats::{BitmapFormat, raw};

fn round_trip(name: &str, image: &BitmapImage) {
    let palette = Palette::pepto_pal();
    let files = raw::write(image, name, &palette).unwrap_or_else(|e| panic!("{name}: {e}"));
    let blocks = raw::Block::for_mode(image.mode());
    assert_eq!(files.len(), blocks.len() + 1, "{name}: blocks and manifest");
    for ((path, bytes), block) in files.iter().zip(blocks) {
        assert_eq!(
            path.to_str(),
            Some(format!("{name}.{}", block.suffix()).as_str())
        );
        assert_eq!(bytes.len(), block.size(), "{name} {block:?}");
    }
    // The blocks in order are the Koala / Art Studio body.
    let body: Vec<u8> = files[..blocks.len()]
        .iter()
        .flat_map(|(_, b)| b.iter().copied())
        .collect();
    let format = BitmapFormat::for_mode(image.mode()).expect("bitmap");
    assert_eq!(body, format.write(image).expect("writes")[2..], "{name}");
    // Reading through a file-name lookup, as a caller with a directory would.
    let manifest = &files.last().expect("manifest").1;
    let set = raw::read(manifest, |file| {
        files
            .iter()
            .find(|(p, _)| p.to_str() == Some(file))
            .map(|(_, b)| b.clone())
    })
    .unwrap_or_else(|e| panic!("{name}: {e}"));
    assert_eq!(set.image, *image, "{name}");
    assert_eq!(set.palette, palette, "{name}");
    assert_eq!(set.manifest.mode, image.mode());
    // Writing what was read gives the same files.
    assert_eq!(
        raw::write(&set.image, name, &set.palette).expect("writes"),
        files,
        "{name}"
    );
}

#[test]
fn solves_round_trip_through_raw_sets() {
    solve_all();
    for (name, _) in FIXTURES {
        for mode in MODES {
            round_trip(name, &solved(name, mode));
        }
    }
}

#[test]
fn baselines_round_trip_through_raw_sets() {
    for (name, format) in FIXTURES {
        round_trip(name, &baseline(name, format));
    }
}
