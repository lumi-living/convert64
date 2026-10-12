//! Solver benchmark (#109) and the nudge statistics test. The benchmark is ignored; run
//! it in release, with and without `parallel`:
//!
//! ```sh
//! cargo test --release -p convert64-core --test solve_bench -- --ignored --nocapture
//! cargo test --release -p convert64-core --features parallel --test solve_bench -- --ignored --nocapture
//! ```
//!
//! It solves the `portrait` (multicolor) and `lineart` (hi-res) fixtures at 4× with
//! Floyd–Steinberg and Bayer 4×4, with the reference solver (the default, #181) and the
//! native one, and prints the median of several runs for the full solve and for the
//! interactive re-solve after a 0.25 px nudge.

use convert64_core::geom::Vec2;
use convert64_core::layer::{DitherKind, Layer, Recipe, SourceImage, Transform};
use convert64_core::mode::Mode;
use convert64_core::palette::Palette;
use convert64_core::solve::{
    Input, InputOptions, SolveMode, SolveOptions, Solver, SolverKind, Steering,
};
use convert64_core::{Compositor, SuperGrid};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn layer(name: &str, kind: DitherKind, dx: f32) -> Layer {
    let path = format!(
        "{}/../../fixtures/{name}/source.png",
        env!("CARGO_MANIFEST_DIR")
    );
    let img = image::open(&path).unwrap().to_rgba8();
    let (w, h) = img.dimensions();
    let src = Arc::new(SourceImage::from_srgba8(w, h, img.as_raw()).unwrap());
    let mut l = Layer::new(name, src);
    l.set_transform(
        Transform::new(
            Vec2::new(dx, 0.0),
            Vec2::new(320.0 / w as f32, 200.0 / h as f32),
            false,
            false,
        )
        .unwrap(),
    );
    l.set_recipe(Recipe::with_method(kind, 0.8).unwrap());
    l
}

fn median(mut v: Vec<Duration>) -> Duration {
    v.sort();
    v[v.len() / 2]
}

fn ms(d: Duration) -> String {
    format!("{:.1} ms", d.as_secs_f64() * 1000.0)
}

#[test]
#[ignore = "benchmark: run in release with --ignored --nocapture"]
fn solve_timings() {
    let threads = if cfg!(feature = "parallel") {
        "parallel"
    } else {
        "sequential"
    };
    let solver = Solver::new(&Palette::pepto_pal());
    for (name, mode) in [
        ("portrait", Mode::MulticolorBitmap),
        ("lineart", Mode::HiresBitmap),
    ] {
        for (kind, solver_kind) in [DitherKind::FloydSteinberg, DitherKind::Bayer4]
            .into_iter()
            .flat_map(|k| [(k, SolverKind::Reference), (k, SolverKind::Native)])
        {
            let opts = SolveOptions {
                solver: solver_kind,
                ..SolveOptions::default()
            };
            let grid = SuperGrid::new(mode, 4).unwrap();
            let mut c = Compositor::new(grid);
            c.update(&[layer(name, kind, 0.0)], &[]);
            let frame = c.frame().clone();
            let full = median(
                (0..5)
                    .map(|_| {
                        let t = Instant::now();
                        solver.solve(&frame, &Steering::new(), None, &opts);
                        t.elapsed()
                    })
                    .collect(),
            );
            let base = solver.solve(&frame, &Steering::new(), None, &opts);
            let mut times = Vec::new();
            for i in 1..=5 {
                let dirty = c.update(&[layer(name, kind, 0.25 * i as f32)], &[]);
                let t = Instant::now();
                solver.solve(
                    c.frame(),
                    &Steering::new(),
                    Some(&base),
                    &SolveOptions {
                        mode: SolveMode::Interactive { dirty },
                        ..opts.clone()
                    },
                );
                times.push(t.elapsed());
            }
            println!(
                "solve: {name} {mode} {kind:?} {solver_kind:?} {threads}: full {}, \
                 interactive after a 0.25 px nudge {}",
                ms(full),
                ms(median(times))
            );
        }
    }
}

/// A 0.25 px nudge on the gradient fixture changes at least one cell's statistics.
#[test]
fn a_quarter_pixel_nudge_changes_cell_statistics() {
    let t = convert64_core::solve::MetricTables::new(&Palette::pepto_pal());
    let grid = SuperGrid::new(Mode::MulticolorBitmap, 4).unwrap();
    let mut c = Compositor::new(grid);
    c.update(&[layer("gradient", DitherKind::None, 0.0)], &[]);
    let before = Input::new(c.frame(), &t, InputOptions::default());
    let dirty = c.update(&[layer("gradient", DitherKind::None, 0.25)], &[]);
    let after = Input::new(c.frame(), &t, InputOptions::default());
    assert!(!dirty.is_empty(), "the nudge must touch cells");
    let changed = dirty
        .iter()
        .filter(|&cell| before.cell_distances(cell) != after.cell_distances(cell))
        .count();
    assert!(changed > 0, "no cell's statistics changed");
}
