//! Compositor benchmark (#87): 10 full-canvas layers, each with a mask (raster plus a
//! gradient source) feathered at σ = 8 target px, in all four blend modes, at 4× on a
//! multicolor and a hi-res target. `criterion` is not an approved dependency, so this is
//! an ignored test timed with `std::time::Instant`. Run it in release, with and without
//! the `parallel` feature:
//!
//! ```sh
//! cargo test --release -p convert64-core --test bench -- --ignored --nocapture
//! cargo test --release -p convert64-core --features parallel --test bench -- --ignored --nocapture
//! ```
//!
//! It prints the median of several runs for each case and checks the E4 budget (#4:
//! "10 layers at 4× under 10 ms native") on the **warm recomposite**: all ten layers
//! resampled and their masks feathered already, every pixel re-blended and re-filtered,
//! which is what a slider drag on a blend or opacity control costs.

use convert64_core::geom::Vec2;
use convert64_core::layer::{Blend, Layer, SourceImage, Transform};
use convert64_core::mask::{
    Combine, Gradient, GradientKind, Mask, MaskRaster, MaskSource, Modifiers, SourceEntry,
};
use convert64_core::mode::Mode;
use convert64_core::{Compositor, SuperGrid};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The E4 budget for compositing 10 masked layers at 4×.
const BUDGET: Duration = Duration::from_millis(10);

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

fn median(mut v: Vec<Duration>) -> Duration {
    v.sort();
    v[v.len() / 2]
}

fn ms(d: Duration) -> String {
    format!("{:.2} ms", d.as_secs_f64() * 1000.0)
}

/// Times `f` `runs` times after `setup`, returning the median.
fn time<S>(runs: usize, mut setup: impl FnMut() -> S, mut f: impl FnMut(&mut S)) -> Duration {
    let runs = (0..runs)
        .map(|_| {
            let mut s = setup();
            let t = Instant::now();
            f(&mut s);
            t.elapsed()
        })
        .collect();
    median(runs)
}

#[test]
#[ignore = "benchmark: run in release with --ignored --nocapture"]
fn composite_10_masked_layers_at_4x() {
    let layers = scene();
    let parallel = if cfg!(feature = "parallel") {
        "parallel"
    } else {
        "sequential"
    };
    println!(
        "bench: 10 layers 320x200, masks (raster + gradient, feather 8), 4 blend modes, 4x, {parallel}"
    );
    let mut warm_worst = Duration::ZERO;
    for mode in [Mode::MulticolorBitmap, Mode::HiresBitmap] {
        let grid = SuperGrid::new(mode, 4).unwrap();
        let (w, h) = grid.size();

        let cold = time(
            3,
            || Compositor::new(grid),
            |c| {
                c.composite(&layers);
            },
        );

        let mut warm_c = Compositor::new(grid);
        warm_c.composite(&layers);
        let warm = time(
            9,
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
        let mut k = 0.0f32;
        let nudge = time(
            9,
            || (),
            |()| {
                k += 0.25;
                moved[4].set_transform(Transform::translated(Vec2::new(k, 0.0)).unwrap());
                nudge_c.update(&moved, &[]);
            },
        );

        // A feather slider: re-evaluates one mask (no resample) and recomposites.
        let mut slide = layers.clone();
        let mut slide_c = Compositor::new(grid);
        slide_c.update(&slide, &[]);
        let mut sigma = 8.0f32;
        let feather = time(
            9,
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

        println!(
            "bench: {mode} {w}x{h}: cold {}, warm recomposite {}, nudge one layer {}, feather slider {}",
            ms(cold),
            ms(warm),
            ms(nudge),
            ms(feather)
        );
        warm_worst = warm_worst.max(warm);
    }
    println!(
        "bench: warm recomposite worst {} vs budget {}: {}",
        ms(warm_worst),
        ms(BUDGET),
        if warm_worst <= BUDGET {
            "within"
        } else {
            "OVER"
        }
    );
}
