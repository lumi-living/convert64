//! #198 (ADR-011): a sub-degree rotation of a real fixture reaches the solver. The
//! `gradient` fixture's source fills the screen; turned by 0.25° about its centre, at
//! least one cell's statistics (the mean of its target pixels' colour and coverage)
//! must change, or a fine rotation could not steer the solve.

use convert64_core::geom::Vec2;
use convert64_core::layer::{Layer, SourceImage, Transform};
use convert64_core::mode::Mode;
use convert64_core::{Compositor, SuperGrid, TargetFrame};
use std::path::Path;
use std::sync::Arc;

fn gradient() -> Layer {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/gradient/source.png");
    let img = image::open(&path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .to_rgba8();
    let (w, h) = img.dimensions();
    let src = Arc::new(SourceImage::from_srgba8(w, h, img.as_raw()).expect("decoded PNG"));
    let mut layer = Layer::new("gradient", src);
    layer.set_transform(
        Transform::new(
            Vec2::ZERO,
            Vec2::new(320.0 / w as f32, 200.0 / h as f32),
            false,
            false,
        )
        .expect("fit to screen"),
    );
    layer
}

/// Per cell, the mean colour and coverage of its target pixels.
fn cell_stats(grid: SuperGrid, frame: &TargetFrame) -> Vec<[f64; 4]> {
    let (tw, th) = grid.target_size();
    let (cw, ch) = grid.cell_size();
    let (cx, cy) = grid.cells();
    let mut stats = vec![[0.0f64; 4]; (cx * cy) as usize];
    for y in 0..th {
        for x in 0..tw {
            let i = (y * tw + x) as usize;
            let c = frame.colour()[i];
            let s = &mut stats[((y / ch) * cx + x / cw) as usize];
            for k in 0..3 {
                s[k] += f64::from(c[k]);
            }
            s[3] += f64::from(frame.coverage()[i]);
        }
    }
    let n = f64::from(cw * ch);
    for s in &mut stats {
        for v in s.iter_mut() {
            *v /= n;
        }
    }
    stats
}

#[test]
fn a_quarter_degree_turn_changes_cell_statistics_on_gradient() {
    for mode in [Mode::MulticolorBitmap, Mode::HiresBitmap] {
        let grid = SuperGrid::with_default_factor(mode).expect("bitmap grid");
        let layer = gradient();
        let mut turned = layer.clone();
        turned.set_transform(layer.transform().with_rotation(0.25).expect("finite"));
        let mut c = Compositor::new(grid);
        c.composite(&[layer]);
        let before = cell_stats(grid, c.frame());
        c.composite(&[turned]);
        let after = cell_stats(grid, c.frame());
        let changed = before
            .iter()
            .zip(&after)
            .filter(|(a, b)| a.iter().zip(b.iter()).any(|(x, y)| (x - y).abs() > 1e-4))
            .count();
        println!("{mode}: {changed} of 1000 cells changed by more than 1e-4");
        assert!(changed >= 1, "{mode}: a 0.25° turn changed no cell");
    }
}
