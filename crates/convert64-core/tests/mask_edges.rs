//! ADR-012 (#227): on the 1× default grid a layer's colour is resampled straight to the
//! target, but its mask is resampled at 4× and box-reduced, because a mask edge on the
//! 1× grid alone was measured off by up to 0.3 in coverage. This checks the measurement:
//! a white masked layer over a dark backdrop, composited at 1× and at 4× (the reference; the ADR's
//! measurement against 8× gives the same picture, 4× within 0.014 of it),
//! must agree to 0.02 in linear light on every target pixel, for hard, diagonal,
//! feathered, downscaled, rotated and gradient masks.
use convert64_core::geom::Vec2;
use convert64_core::layer::{Layer, SourceImage, Transform};
use convert64_core::mask::{
    Combine, Gradient, GradientKind, Mask, MaskRaster, MaskSource, Modifiers, SourceEntry,
};
use convert64_core::mode::Mode;
use convert64_core::{Compositor, SuperGrid};
use std::sync::Arc;

fn solid(w: u32, h: u32, v: f32) -> Arc<SourceImage> {
    Arc::new(SourceImage::from_linear(w, h, vec![[v, v, v, 1.0]; (w * h) as usize]).unwrap())
}

fn scene(w: u32, h: u32, mask: Vec<f32>, m: Modifiers, t: Transform, grad: bool) -> Vec<Layer> {
    let bottom = Layer::new("b", solid(320, 200, 0.05));
    let mut top = Layer::new("t", solid(w, h, 0.8));
    let mut mk = Mask::new(MaskRaster::from_values(w, h, mask).unwrap());
    if grad {
        mk = Mask::new(MaskRaster::from_values(w, h, vec![1.0; (w * h) as usize]).unwrap());
        mk.push_source(SourceEntry {
            source: MaskSource::Gradient(
                Gradient::new(
                    GradientKind::Linear,
                    Vec2::new(100.0, 0.0),
                    Vec2::new(220.0, 0.0),
                    1.0,
                )
                .unwrap(),
            ),
            combine: Combine::Multiply,
        });
    }
    mk.set_modifiers(m);
    top.set_mask(Some(mk)).unwrap();
    top.set_transform(t);
    vec![bottom, top]
}

#[test]
fn a_mask_edge_at_1x_matches_4x() {
    let (w, h) = (320u32, 200u32);
    let hard: Vec<f32> = (0..w * h)
        .map(|i| if i % w < 160 { 1.0 } else { 0.0 })
        .collect();
    let diag: Vec<f32> = (0..w * h)
        .map(|i| {
            if (i % w) as f32 > 100.0 + 0.6 * (i / w) as f32 {
                1.0
            } else {
                0.0
            }
        })
        .collect();
    let (bw, bh) = (1024u32, 640u32);
    let big: Vec<f32> = (0..bw * bh)
        .map(|i| {
            if (i % bw) as f32 > 300.0 + 0.6 * (i / bw) as f32 {
                1.0
            } else {
                0.0
            }
        })
        .collect();
    let fit = Transform::new(
        Vec2::ZERO,
        Vec2::new(320.0 / bw as f32, 200.0 / bh as f32),
        false,
        false,
    )
    .unwrap();
    let nudge = Transform::translated(Vec2::new(0.25, 0.0)).unwrap();
    let cases: Vec<(&str, Vec<Layer>)> = vec![
        (
            "hard edge, 1:1",
            scene(
                w,
                h,
                hard.clone(),
                Modifiers::default(),
                Transform::translated(Vec2::ZERO).unwrap(),
                false,
            ),
        ),
        (
            "hard edge, +0.25 px",
            scene(w, h, hard.clone(), Modifiers::default(), nudge, false),
        ),
        (
            "diagonal edge, +0.25 px",
            scene(w, h, diag.clone(), Modifiers::default(), nudge, false),
        ),
        (
            "diagonal edge, feather 1",
            scene(
                w,
                h,
                diag.clone(),
                Modifiers::feathered(1.0).unwrap(),
                nudge,
                false,
            ),
        ),
        (
            "diagonal edge, feather 3",
            scene(
                w,
                h,
                diag.clone(),
                Modifiers::feathered(3.0).unwrap(),
                nudge,
                false,
            ),
        ),
        (
            "diagonal, 1024 fitted",
            scene(bw, bh, big, Modifiers::default(), fit, false),
        ),
        (
            "diagonal, rotated 3°",
            scene(
                w,
                h,
                diag,
                Modifiers::default(),
                nudge.with_rotation(3.0).unwrap(),
                false,
            ),
        ),
        (
            "linear gradient, +0.25 px",
            scene(w, h, hard, Modifiers::default(), nudge, true),
        ),
    ];
    let mut worst = Vec::new();
    for mode in [Mode::MulticolorBitmap, Mode::HiresBitmap] {
        for (name, layers) in &cases {
            let mut out = Vec::new();
            for n in [1u8, 4] {
                let mut c = Compositor::new(SuperGrid::new(mode, n).unwrap());
                c.update(layers, &[]);
                out.push(
                    c.frame()
                        .colour()
                        .iter()
                        .map(|p| p[0])
                        .collect::<Vec<f32>>(),
                );
            }
            let d: Vec<f32> = out[0]
                .iter()
                .zip(&out[1])
                .map(|(x, y)| (x - y).abs())
                .collect();
            let max = d.iter().copied().fold(0.0, f32::max);
            let rms = (d.iter().map(|v| v * v).sum::<f32>() / d.len() as f32).sqrt();
            println!("{mode:<18} {name:<28} 1x vs 4x: max {max:.4} rms {rms:.5}");
            if max > 0.02 {
                worst.push(format!("{mode} {name}: {max}"));
            }
        }
    }
    assert!(worst.is_empty(), "over 0.02: {worst:?}");
}
