//! `convert64 recipe apply` (#77): a recipe with a key, a gradient and a feather gives
//! the same mask through the CLI binary as through the core in-process (the app's path).

use convert64_core::command::{
    Command, MaskClear, MaskGradient, MaskKey, MaskSetModifier, ModifierChange,
};
use convert64_core::geom::Vec2;
use convert64_core::layer::{Layer, Transform};
use convert64_core::mask::{Combine, Gradient, GradientKind, Key, KeyChannel};
use convert64_core::mode::Mode;
use convert64_core::{Compositor, Editor, Project, SuperGrid};
use std::path::{Path, PathBuf};
use std::process::Command as Process;
use std::sync::Arc;

/// A 64 × 40 picture: green on the left, red on the right, with a vertical ramp.
fn write_source(dir: &Path) -> PathBuf {
    let img = image::RgbaImage::from_fn(64, 40, |x, y| {
        let v = (y * 6) as u8;
        if x < 32 {
            image::Rgba([v / 4, 200, v / 3, 255])
        } else {
            image::Rgba([210, v / 2, 40, 255])
        }
    });
    let path = dir.join("photo.png");
    img.save(&path).unwrap();
    path
}

const RECIPE: &str = r#"{
  "version": 1,
  "target": "multicolor-bitmap",
  "supersample": 2,
  "layers": [
    { "name": "photo", "source": "photo.png",
      "transform": { "scale": { "x": 5.0, "y": 5.0 } } }
  ],
  "commands": [
    { "command": "mask_clear", "layer": "photo", "value": 0.0 },
    { "command": "mask_key", "layer": "photo",
      "key": { "channel": "chroma", "centre": [0.0, 0.58, 0.0], "range": 0.12, "softness": 0.1 },
      "combine": "max" },
    { "command": "mask_gradient", "layer": 0,
      "gradient": { "kind": "radial", "from": { "x": 20.0, "y": 20.0 }, "to": { "x": 20.0, "y": 0.0 }, "softness": 0.5 },
      "combine": "multiply" },
    { "command": "mask_set_modifier", "layer": 0, "modifier": { "feather": 2.0 } }
  ]
}"#;

fn temp(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The app's path: core commands on an `Editor`, composited in-process.
fn in_process(source: &Path) -> Vec<u16> {
    let img = image::open(source).unwrap().to_rgba8();
    let src = convert64_core::SourceImage::from_srgba8(64, 40, img.as_raw()).unwrap();
    let mut layer = Layer::new("photo", Arc::new(src));
    layer.set_transform(Transform::new(Vec2::ZERO, Vec2::new(5.0, 5.0), false, false).unwrap());
    let mut e = Editor::new(Project {
        layers: vec![layer],
        regions: Vec::new(),
    });
    for c in [
        Command::MaskClear(MaskClear {
            layer: 0,
            value: 0.0,
        }),
        Command::MaskKey(MaskKey {
            layer: 0,
            key: Key::new(KeyChannel::Chroma, [0.0, 0.58, 0.0], 0.12, 0.1).unwrap(),
            combine: Combine::Max,
        }),
        Command::MaskGradient(MaskGradient {
            layer: 0,
            gradient: Gradient::new(
                GradientKind::Radial,
                Vec2::new(20.0, 20.0),
                Vec2::new(20.0, 0.0),
                0.5,
            )
            .unwrap(),
            combine: Combine::Multiply,
        }),
        Command::MaskSetModifier(MaskSetModifier {
            layer: 0,
            modifier: ModifierChange::Feather(2.0),
        }),
    ] {
        e.execute(c).unwrap();
    }
    let grid = SuperGrid::new(Mode::MulticolorBitmap, 2).unwrap();
    let mut c = Compositor::new(grid);
    c.composite(&e.project().layers);
    let (rect, vals) = c.evaluated_mask(0).unwrap();
    let (w, h) = grid.size();
    let mut out = vec![0u16; (w * h) as usize];
    for y in rect.y0..rect.y1 {
        for x in rect.x0..rect.x1 {
            let v = vals[((y - rect.y0) * rect.width() + x - rect.x0) as usize];
            out[(y * w + x) as usize] = (v.clamp(0.0, 1.0) * 65535.0).round() as u16;
        }
    }
    out
}

#[test]
fn cli_recipe_apply_matches_the_in_process_mask() {
    let dir = temp("recipe-cli");
    let source = write_source(&dir);
    std::fs::write(dir.join("recipe.json"), RECIPE).unwrap();
    let out = Process::new(env!("CARGO_BIN_EXE_convert64"))
        .args([
            "recipe",
            "apply",
            "recipe.json",
            "--mask",
            "photo=cli.png",
            "--composite",
            "c.png",
            "--attributes",
            "a.png",
        ])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("1 layer(s), 4 command(s) applied"),
        "{stdout}"
    );
    let cli = image::open(dir.join("cli.png")).unwrap().to_luma16();
    assert_eq!(cli.dimensions(), (320, 400));
    let want = in_process(&source);
    assert_eq!(cli.as_raw(), &want, "CLI and in-process masks differ");
    // The mask really is shaped by the key, the gradient and the feather.
    assert!(want.contains(&0) && want.iter().any(|&v| v > 60_000));
    assert!(
        want.iter().any(|&v| v > 1_000 && v < 60_000),
        "feathered values"
    );
    assert!(dir.join("c.png").exists() && dir.join("a.png").exists());
}

#[test]
fn library_session_matches_too() {
    let dir = temp("recipe-lib");
    let source = write_source(&dir);
    std::fs::write(dir.join("recipe.json"), RECIPE).unwrap();
    let mut s = convert64_cli::recipe::Session::open(&dir.join("recipe.json")).unwrap();
    let (_, _, got) = s.evaluated_mask(0).unwrap();
    assert_eq!(got, in_process(&source));
}

#[test]
fn a_failing_command_exits_2_and_names_it() {
    let dir = temp("recipe-bad");
    write_source(&dir);
    let bad = RECIPE.replace(r#""feather": 2.0"#, r#""feather": 99.0"#);
    std::fs::write(dir.join("recipe.json"), bad).unwrap();
    let out = Process::new(env!("CARGO_BIN_EXE_convert64"))
        .args(["recipe", "apply", "recipe.json"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("command 3"));
}
