//! Compositor benchmark (#87, #98): 10 full-canvas layers, each with a mask (raster plus
//! a gradient source) feathered at σ = 8 target px, in all four blend modes, at 4× on a
//! multicolor and a hi-res target. `criterion` is not an approved dependency, so this is
//! an ignored test timed with `std::time::Instant`. Run it in release, with and without
//! the `parallel` feature:
//!
//! ```sh
//! cargo test --release -p convert64-core --test bench -- --ignored --nocapture
//! cargo test --release -p convert64-core --features parallel --test bench -- --ignored --nocapture
//! ```
//!
//! It prints one row per case with the median of several runs for each target mode, and
//! the budget that applies to it in this build (sequential or parallel):
//!
//! - **cold composite**: a new compositor composites the scene: every layer resampled,
//!   every mask evaluated and feathered, everything blended. #98: ≤ 60 ms (parallel);
//!   #139: ≤ 60 ms single-threaded too (the wasm and app path: the first frame after
//!   load on the web).
//! - **warm recomposite**: everything re-blended and re-filtered with every layer cached,
//!   what a blend or opacity slider on the bottom layer costs. E4 (#4): ≤ 10 ms
//!   (parallel); #98: ≤ 12 ms multicolor / ≤ 16 ms hi-res single-threaded (the wasm and
//!   app path).
//! - **nudge one layer**: `update` after moving layer 4 by 0.25 px: one resample, one
//!   feather, and the layers from 4 up re-blended. #98: ≤ 10 ms multicolor / ≤ 12 ms
//!   hi-res (parallel).
//! - **feather slider**: `update` after changing layer 7's feather: one mask
//!   re-evaluated, no resample.
//!
//! The same four cases again with every layer **rotated** (#198, ADR-011: layer `i` by
//! `3 + 7·i` degrees about its centre), which takes the non-separable path (inverse-mapped
//! Catmull-Rom sampling with exact edge coverage). The budgets above were set for the
//! axis-aligned path and are printed against these rows too, for comparison; the nudge
//! row is a rotation drag (`update` after turning layer 4 by a further 0.25°).
//!
//! The budgets are for the CI macOS runner class (the `bench` workflow); the check is
//! printed, not asserted, because shared runners are noisy.

use convert64_core::geom::Vec2;
use convert64_core::layer::{Blend, Layer, SourceImage, Transform};
use convert64_core::mask::{
    Combine, Gradient, GradientKind, Mask, MaskRaster, MaskSource, Modifiers, SourceEntry,
};
use convert64_core::mode::Mode;
use convert64_core::{Compositor, SuperGrid};
use std::sync::Arc;
use std::time::{Duration, Instant};

const PARALLEL: bool = cfg!(feature = "parallel");

/// The budget for a case in this build, `[multicolor, hi-res]` in ms, and its source.
fn budget(case: &str) -> Option<([f64; 2], &'static str)> {
    match (case, PARALLEL) {
        ("cold composite", true) => Some(([60.0, 60.0], "#98")),
        ("cold composite", false) => Some(([60.0, 60.0], "#139")),
        ("warm recomposite", true) => Some(([10.0, 10.0], "E4")),
        ("warm recomposite", false) => Some(([12.0, 16.0], "#98")),
        ("nudge one layer", true) => Some(([10.0, 12.0], "#98")),
        _ => None,
    }
}
/// [`scene`] with layer `i` turned `3 + 7·i` degrees about its centre (#198).
fn rotated_scene() -> Vec<Layer> {
    let mut layers = scene();
    for (i, l) in layers.iter_mut().enumerate() {
        let t = l.transform().with_rotation(3.0 + 7.0 * i as f32).unwrap();
        l.set_transform(t);
    }
    layers
}

fn scene() -> Vec<Layer> {
    let blends = [
        Blend::Normal,
        Blend::Multiply,
        Blend::Screen,
        Blend::Overlay,
    ];
    (0..10)
        .map(|i| {
            let px: Vec<u8> = (0..320 * 200)
                .flat_map(|p| {
                    let (x, y) = ((p % 320) as u32, (p / 320) as u32);
                    [
                        (x * 7 + i * 25) as u8,
                        (y * 5 + i * 13) as u8,
                        ((x ^ y) + i * 3) as u8,
                        255,
                    ]
                })
                .collect();
            let src = Arc::new(SourceImage::from_srgba8(320, 200, &px).unwrap());
            let mut l = Layer::new(format!("layer {i}"), src);
            let f = i as f32;
            l.set_transform(
                Transform::translated(Vec2::new(f * 0.37 - 1.0, f * 0.21 - 1.0)).unwrap(),
            );
            l.set_blend(blends[i as usize % 4]);
            l.set_opacity(0.9).unwrap();
            let raster: Vec<f32> = (0..320 * 200)
                .map(|p| {
                    if (p % 320 + p / 320 + i * 17) % 97 < 60 {
                        1.0
                    } else {
                        0.4
                    }
                })
                .collect();
            let mut m = Mask::new(MaskRaster::from_values(320, 200, raster).unwrap());
            m.push_source(SourceEntry {
                source: MaskSource::Gradient(
                    Gradient::new(
                        GradientKind::Radial,
                        Vec2::new(40.0 + f * 25.0, 100.0),
                        Vec2::new(40.0 + f * 25.0 + 150.0, 100.0),
                        0.7,
                    )
                    .unwrap(),
                ),
                combine: Combine::Multiply,
            });
            m.set_modifiers(Modifiers::feathered(8.0).unwrap());
            l.set_mask(Some(m)).unwrap();
            l
        })
        .collect()
}

/// The median, fastest and slowest of a case's runs.
#[derive(Clone, Copy, Default)]
struct Stat {
    median: Duration,
    min: Duration,
    max: Duration,
}

fn stat(mut v: Vec<Duration>) -> Stat {
    v.sort();
    Stat {
        median: v[v.len() / 2],
        min: v[0],
        max: v[v.len() - 1],
    }
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// Times `f` `runs` times after `setup`, after one untimed run: the process (thread
/// pool, allocator) is warm, so "cold" means a new compositor, not a new process.
fn time<S>(runs: usize, mut setup: impl FnMut() -> S, mut f: impl FnMut(&mut S)) -> Stat {
    f(&mut setup());
    let runs = (0..runs)
        .map(|_| {
            let mut s = setup();
            let t = Instant::now();
            f(&mut s);
            t.elapsed()
        })
        .collect();
    stat(runs)
}

#[test]
#[ignore = "benchmark: run in release with --ignored --nocapture"]
fn composite_10_masked_layers_at_4x() {
    let threads = if PARALLEL {
        let n = std::thread::available_parallelism().map_or(1, usize::from);
        format!("parallel, {n} threads")
    } else {
        "sequential".to_owned()
    };
    println!(
        "bench: 10 layers 320x200, masks (raster + gradient, feather 8), 4 blend modes, 4x, {threads}"
    );
    run(&scene(), "", |_, k| {
        Transform::translated(Vec2::new(k, 0.0)).unwrap()
    });
    // #198: every layer rotated; the nudge is a rotation drag on layer 4.
    run(&rotated_scene(), " (rotated)", |t, k| {
        t.with_rotation(31.0 + k).unwrap()
    });
}

/// Times the four cases on `layers` for both modes and prints them, each case name
/// followed by `suffix`. `nudge(t, k)` is layer 4's transform after `k` steps of the
/// nudge row, from its transform `t` in `layers`.
fn run(layers: &[Layer], suffix: &str, nudge: impl Fn(Transform, f32) -> Transform) {
    let layers = layers.to_vec();
    let cases = [
        "cold composite",
        "warm recomposite",
        "nudge one layer",
        "feather slider",
    ];
    let mut rows = [[Stat::default(); 2]; 4];
    let modes = [Mode::MulticolorBitmap, Mode::HiresBitmap];
    for (m, mode) in modes.into_iter().enumerate() {
        let grid = SuperGrid::new(mode, 4).unwrap();

        rows[0][m] = time(
            7,
            || Compositor::new(grid),
            |c| {
                c.composite(&layers);
            },
        );

        let mut warm_c = Compositor::new(grid);
        warm_c.composite(&layers);
        rows[1][m] = time(
            21,
            || (),
            |()| {
                warm_c.composite(&layers);
            },
        );

        // Interactive: nudge one layer 0.25 px; update() resamples it, re-feathers its
        // mask and recomposites its footprint (here the whole canvas).
        let mut nudge_c = Compositor::new(grid);
        nudge_c.update(&layers, &[]);
        let mut moved = layers.clone();
        let t4 = layers[4].transform();
        let mut k = 0.0f32;
        rows[2][m] = time(
            21,
            || (),
            |()| {
                k += 0.25;
                moved[4].set_transform(nudge(t4, k));
                nudge_c.update(&moved, &[]);
            },
        );

        // A feather slider: re-evaluates one mask (no resample) and recomposites.
        let mut slide = layers.clone();
        let mut slide_c = Compositor::new(grid);
        slide_c.update(&slide, &[]);
        let mut sigma = 8.0f32;
        rows[3][m] = time(
            21,
            || (),
            |()| {
                sigma = if sigma == 8.0 { 7.5 } else { 8.0 };
                slide[7]
                    .mask_mut()
                    .unwrap()
                    .set_modifiers(Modifiers::feathered(sigma).unwrap());
                slide_c.update(&slide, &[]);
            },
        );
    }
    let size = |mode: Mode| {
        let (w, h) = SuperGrid::new(mode, 4).unwrap().size();
        format!("{w}x{h}")
    };
    println!(
        "bench: {:<28} {:>12} {:>12}   budget",
        "",
        format!("mc {}", size(modes[0])),
        format!("hires {}", size(modes[1])),
    );
    for (case, row) in cases.iter().zip(rows) {
        let check = budget(case).map_or_else(String::new, |(b, source)| {
            let within = row.iter().zip(b).all(|(d, b)| ms(d.median) <= b);
            format!(
                "   {source} ≤ {} / {} ms: {}",
                b[0],
                b[1],
                if within { "within" } else { "OVER" }
            )
        });
        let name = format!("{case}{suffix}");
        println!(
            "bench: {name:<28} {:>9.2} ms {:>9.2} ms{check}",
            ms(row[0].median),
            ms(row[1].median)
        );
    }
    println!("bench: (medians; fastest and slowest run:)");
    for (case, row) in cases.iter().zip(rows) {
        let range = |s: Stat| format!("{:.1}–{:.1}", ms(s.min), ms(s.max));
        let name = format!("{case}{suffix}");
        println!(
            "bench: {name:<28} {:>12} {:>12}",
            range(row[0]),
            range(row[1])
        );
    }
}
