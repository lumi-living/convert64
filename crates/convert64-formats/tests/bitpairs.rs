//! Bitpair policies on the six fixtures (#124): every policy writes a legal file that
//! shows the same picture; `Stable` is byte-exact against a golden and packs smaller
//! than `Frequency` on every png2c64 baseline.

mod common;

use common::{FIXTURES, MODES, baseline, solve_all, solved};
use convert64_core::model::BitmapImage;
use convert64_core::validate;
use convert64_formats::BitmapFormat;
use convert64_formats::bitpairs::{BitpairPolicy, packed_size_estimate, reassign};
use std::fmt::Write as _;

/// FNV-1a, 64 bit: a short fingerprint of a whole file for the golden.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

fn distinct(mut bytes: Vec<u8>) -> usize {
    bytes.sort_unstable();
    bytes.dedup();
    bytes.len()
}

/// Every policy: same pixels, legal, written file reads back as the reassigned image.
fn check_policies(name: &str, image: &BitmapImage) {
    let format = BitmapFormat::for_mode(image.mode()).expect("bitmap");
    for policy in BitpairPolicy::ALL {
        let r = reassign(image, policy);
        assert_eq!(r.to_indexed(), image.to_indexed(), "{name} {policy:?}");
        assert_eq!(validate(&r), Ok(()), "{name} {policy:?}");
        let bytes = format.write_with(image, policy).expect("writes");
        assert_eq!(
            format.read(&bytes).expect("reads").image,
            r,
            "{name} {policy:?}"
        );
    }
}

#[test]
fn every_policy_keeps_the_picture() {
    solve_all();
    for (name, format) in FIXTURES {
        check_policies(name, &baseline(name, format));
        for mode in MODES {
            check_policies(name, &solved(name, mode));
        }
    }
}

/// The exit of #124 on the png2c64 baselines: `Stable` packs Screen and Colour RAM
/// smaller than `Frequency` on every fixture, and `BruteForcePacked` never larger
/// than `Stable`. On `portrait`, `Stable` also has fewer distinct Screen RAM bytes.
/// The sizes are the fixed-Huffman DEFLATE estimate (`packed_size_estimate`).
#[test]
fn stable_packs_smaller_than_frequency() {
    for (name, format) in FIXTURES {
        let image = baseline(name, format);
        let [stable, frequency, packed] = [
            BitpairPolicy::Stable,
            BitpairPolicy::Frequency,
            BitpairPolicy::BruteForcePacked,
        ]
        .map(|p| reassign(&image, p));
        let size = |i: &BitmapImage| packed_size_estimate(i);
        println!(
            "{name}: as-is {}, stable {}, frequency {}, packed {} bytes",
            size(&image),
            size(&stable),
            size(&frequency),
            size(&packed)
        );
        assert!(size(&stable) < size(&frequency), "{name}");
        assert!(size(&packed) <= size(&stable), "{name}");
        if name == "portrait" {
            assert!(
                distinct(stable.screen_bytes()) < distinct(frequency.screen_bytes()),
                "portrait: distinct Screen RAM bytes"
            );
        }
    }
}

/// Byte-exact golden: each baseline written with each policy, as a fingerprint of the
/// whole file, the packed-size estimate and the number of distinct Screen RAM bytes.
#[test]
fn golden_policy_outputs() {
    let mut report = String::new();
    for (name, format) in FIXTURES {
        let image = baseline(name, format);
        for policy in BitpairPolicy::ALL {
            let file = format.write_with(&image, policy).expect("writes");
            let r = reassign(&image, policy);
            let _ = writeln!(
                report,
                "{name:<9} {:<9} fnv1a={:016x} packed={:>4} distinct_screen={:>3}",
                policy.id(),
                fnv1a(&file),
                packed_size_estimate(&r),
                distinct(r.screen_bytes())
            );
        }
    }
    insta::assert_snapshot!("bitpair_policies", report);
}
