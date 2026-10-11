//! Golden-test harness (`docs/TAD.md` §6, #40). One format case, one display-PRG case
//! and one solver case per fixture:
//!
//! - **Format, bit-exact.** The fixture's png2c64 baseline (`baseline.kla`/`.art`) is
//!   read into the model and serialised again. The bytes are snapshotted with `insta` as
//!   an annotated hex dump (`fixtures/<name>/expected_<ext>.snap`). Every byte is in the
//!   snapshot, so any change fails, and the diff names the section and cell that moved.
//!   When E6's writers land, they replace the re-serialisation here.
//! - **Display PRG, bit-exact.** `convert64_formats::prg::display_prg` is run on the
//!   baseline file twice: both runs must give the same bytes, and the picture file
//!   embedded at `PAYLOAD_OFFSET` must equal the baseline. The PRG is snapshotted as a
//!   hex dump (`fixtures/<name>/expected_prg.snap`): the BASIC line and loader in rows
//!   of 16 bytes, then the embedded file in the format case's layout. `cargo xtask
//!   vice-check` runs these PRGs in VICE.
//! - **Solver, perceptual.** The solver's output is rendered through the palette and
//!   compared with `fixtures/<name>/expected.png` by OKLab mean squared error. The test
//!   fails above `mse_vs_golden_max` in `fixtures/<name>/thresholds.toml`, and prints
//!   `fixture, mode, mse_vs_golden, mse_vs_png2c64` so PRs can report the delta to
//!   png2c64 (`CLAUDE.md`).
//!
//! The solver does not exist yet (E5, #5). Until it does, [`solve`] is an identity stub
//! fed the png2c64 baseline, which exercises the whole comparison path; replace its body
//! with the real solver and the source image as input.
//!
//! Golden candidates: with `CONVERT64_GOLDEN_OUT=<dir>` set, each solver case also writes
//! its rendering to `<dir>/<fixture>/expected.png`. `cargo xtask regen-goldens` uses this
//! (and `INSTA_UPDATE=new`) to list and accept changes.

use convert64_core::Palette;
use convert64_core::model::IndexedBitmap;
use convert64_core::palette::{Rgb8, oklab_of};
use convert64_core::validate;
use convert64_formats::prg::{LOAD_ADDRESS, PAYLOAD_ADDRESS, PAYLOAD_OFFSET, SYS_ADDRESS};
use convert64_formats::{BitmapFile, BitmapFormat};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

fn fixture_dir(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(name)
}

fn baseline_file(format: BitmapFormat) -> &'static str {
    match format {
        BitmapFormat::Koala => "baseline.kla",
        BitmapFormat::ArtStudio => "baseline.art",
    }
}

fn read_baseline(name: &str, format: BitmapFormat) -> (Vec<u8>, BitmapFile) {
    let path = fixture_dir(name).join(baseline_file(format));
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let file = format
        .read(&bytes)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    (bytes, file)
}

// ---------------------------------------------------------------------------------------
// Format: bit-exact.

/// Serialises a picture file in its format's byte layout
/// (`docs/reference/file-formats.md`).
fn serialise(file: &BitmapFile) -> Vec<u8> {
    let mut out = file.load_address.to_le_bytes().to_vec();
    out.extend(file.image.bitmap());
    out.extend(file.image.screen_bytes());
    if file.format == BitmapFormat::Koala {
        out.extend(file.image.colour_bytes());
        out.push(file.image.background().map_or(0, |c| c.get()));
    }
    out
}

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// An annotated hex dump: one line per bitmap cell, one per Screen/Colour RAM row, so a
/// one-byte change shows up as one changed line that names its cell or row.
fn hex_dump(format: BitmapFormat, bytes: &[u8]) -> String {
    let mut s = String::new();
    let _ = writeln!(
        s,
        "{} file, {} bytes, load address ${:02x}{:02x}",
        format,
        bytes.len(),
        bytes[1],
        bytes[0]
    );
    let bitmap = &bytes[2..8002];
    for (i, cell) in bitmap.chunks(8).enumerate() {
        let _ = writeln!(s, "bitmap cell ({:2}, {:2})  {}", i % 40, i / 40, hex(cell));
    }
    for (row, line) in bytes[8002..9002].chunks(40).enumerate() {
        let _ = writeln!(s, "screen row {row:2}  {}", hex(line));
    }
    if format == BitmapFormat::Koala {
        for (row, line) in bytes[9002..10_002].chunks(40).enumerate() {
            let _ = writeln!(s, "colour row {row:2}  {}", hex(line));
        }
        let _ = writeln!(s, "background  {:02x}", bytes[10_002]);
    }
    s
}

fn check_format(name: &str, format: BitmapFormat) {
    let (original, file) = read_baseline(name, format);
    let bytes = serialise(&file);
    assert_eq!(
        bytes.len(),
        format.sizes()[0],
        "{name}: serialised size does not match the layout"
    );
    // Padding bytes of Art Studio files are not part of the picture.
    assert_eq!(
        bytes,
        original[..bytes.len()],
        "{name}: read/serialise round trip changed bytes"
    );
    let ext = baseline_file(format).rsplit('.').next().unwrap_or("bin");
    let mut settings = insta::Settings::clone_current();
    settings.set_snapshot_path(fixture_dir(name));
    settings.set_prepend_module_to_snapshot(false);
    settings.set_omit_expression(true);
    settings.bind(|| {
        insta::assert_snapshot!(format!("expected_{ext}"), hex_dump(format, &bytes));
    });
}

// ---------------------------------------------------------------------------------------
// Display PRG: bit-exact.

/// The display PRG as an annotated hex dump: the BASIC line, loader and padding in rows
/// of 16 bytes with their C64 addresses, then the embedded picture file as
/// [`hex_dump`] lays it out.
fn prg_dump(format: BitmapFormat, prg: &[u8]) -> String {
    let mut s = String::new();
    let _ = writeln!(
        s,
        "Display PRG ({format}), {} bytes, load address ${:02x}{:02x}, SYS {}, \
         picture file at offset {PAYLOAD_OFFSET} (${PAYLOAD_ADDRESS:04x})",
        prg.len(),
        prg[1],
        prg[0],
        SYS_ADDRESS
    );
    for (i, row) in prg[2..PAYLOAD_OFFSET].chunks(16).enumerate() {
        let addr = usize::from(LOAD_ADDRESS) + i * 16;
        let _ = writeln!(s, "stub ${addr:04x}  {}", hex(row));
    }
    s.push_str(&hex_dump(format, &prg[PAYLOAD_OFFSET..]));
    s
}

fn check_prg(name: &str, format: BitmapFormat) {
    let (original, _) = read_baseline(name, format);
    let build = || {
        convert64_formats::prg::display_prg(format, &original)
            .unwrap_or_else(|e| panic!("{name}: {e}"))
    };
    let prg = build();
    assert_eq!(prg, build(), "{name}: the display PRG differs between runs");
    assert_eq!(
        prg[PAYLOAD_OFFSET..],
        original[..],
        "{name}: the display PRG does not carry the baseline file verbatim"
    );
    let mut settings = insta::Settings::clone_current();
    settings.set_snapshot_path(fixture_dir(name));
    settings.set_prepend_module_to_snapshot(false);
    settings.set_omit_expression(true);
    settings.bind(|| {
        insta::assert_snapshot!("expected_prg", prg_dump(format, &prg));
    });
}

// ---------------------------------------------------------------------------------------
// Solver: perceptual.

/// The solver under test. Identity stub until E5 (#5): returns its input. Replace with
/// the real bitmap solver, fed the fixture's `source.png`.
fn solve(input: &IndexedBitmap) -> IndexedBitmap {
    input.clone()
}

/// Renders a bitmap picture at the 320×200 display size (multicolor pixels doubled).
fn render(img: &IndexedBitmap, palette: &Palette) -> Vec<Rgb8> {
    let x_scale = 320 / img.width();
    let mut out = Vec::with_capacity(320 * 200);
    for y in 0..200 {
        for x in 0..320 {
            out.push(palette.rgb(img.pixel(x / x_scale, y)));
        }
    }
    out
}

fn load_png(path: &Path) -> Option<Vec<Rgb8>> {
    let img = image::open(path).ok()?.to_rgb8();
    assert_eq!(
        img.dimensions(),
        (320, 200),
        "{} is not 320x200",
        path.display()
    );
    Some(
        img.pixels()
            .map(|p| Rgb8::new(p.0[0], p.0[1], p.0[2]))
            .collect(),
    )
}

fn save_png(path: &Path, pixels: &[Rgb8]) {
    let raw: Vec<u8> = pixels.iter().flat_map(|c| [c.r, c.g, c.b]).collect();
    let img = image::RgbImage::from_raw(320, 200, raw).expect("320x200 RGB");
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).expect("candidate dir");
    }
    img.save(path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
}

/// Mean squared OKLab distance between two 320×200 renderings.
fn oklab_mse(a: &[Rgb8], b: &[Rgb8]) -> f64 {
    assert_eq!(a.len(), b.len());
    let sum: f64 = a
        .iter()
        .zip(b)
        .map(|(&x, &y)| {
            let (p, q) = (oklab_of(x), oklab_of(y));
            let d = [p.l - q.l, p.a - q.a, p.b - q.b];
            d.iter().map(|v| f64::from(*v) * f64::from(*v)).sum::<f64>()
        })
        .sum();
    sum / a.len() as f64
}

/// `fixtures/<name>/thresholds.toml`.
///
/// ```toml
/// # comments are allowed
/// mse_vs_golden_max = 0.0002          # default for every mode of this fixture
///
/// [mode.multicolor-bitmap]            # optional per-mode override
/// mse_vs_golden_max = 0.0003
/// ```
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Thresholds {
    mse_vs_golden_max: f64,
    #[serde(default)]
    mode: std::collections::BTreeMap<String, ModeThresholds>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ModeThresholds {
    mse_vs_golden_max: f64,
}

impl Thresholds {
    fn parse(text: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(text)
    }

    /// The threshold for `mode`: its override if there is one, else the default.
    fn mse_vs_golden_max(&self, mode: &str) -> f64 {
        self.mode
            .get(mode)
            .map_or(self.mse_vs_golden_max, |m| m.mse_vs_golden_max)
    }
}

fn threshold(name: &str, mode: &str) -> f64 {
    let path = fixture_dir(name).join("thresholds.toml");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    Thresholds::parse(&text)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .mse_vs_golden_max(mode)
}

fn mode_name(format: BitmapFormat) -> &'static str {
    match format {
        BitmapFormat::Koala => "multicolor-bitmap",
        BitmapFormat::ArtStudio => "hires-bitmap",
    }
}

struct SolverScores {
    mse_vs_golden: f64,
    mse_vs_png2c64: f64,
    threshold: f64,
}

/// Scores `solver` on one fixture. Only the real solver's cases write golden
/// candidates (`write_candidate`), never the self-tests' deliberately broken solvers.
fn score(
    name: &str,
    format: BitmapFormat,
    solver: fn(&IndexedBitmap) -> IndexedBitmap,
    write_candidate: bool,
) -> SolverScores {
    let (_, file) = read_baseline(name, format);
    let output = solver(&file.image.to_indexed());
    assert_eq!(
        validate(&output),
        Ok(()),
        "{name}: solver output is not legal"
    );
    let palette = Palette::pepto_pal();
    let rendered = render(&output, &palette);
    let dir = fixture_dir(name);
    if write_candidate && let Some(out) = std::env::var_os("CONVERT64_GOLDEN_OUT") {
        save_png(
            &PathBuf::from(out).join(name).join("expected.png"),
            &rendered,
        );
    }
    let golden = load_png(&dir.join("expected.png")).unwrap_or_else(|| {
        panic!("{name}: fixtures/{name}/expected.png is missing; run `cargo xtask regen-goldens`")
    });
    let baseline = load_png(&dir.join("baseline.png")).expect("png2c64 baseline.png");
    SolverScores {
        mse_vs_golden: oklab_mse(&rendered, &golden),
        mse_vs_png2c64: oklab_mse(&rendered, &baseline),
        threshold: threshold(name, mode_name(format)),
    }
}

fn check_solver(name: &str, format: BitmapFormat) {
    let mode = mode_name(format);
    let s = score(name, format, solve, true);
    println!(
        "{name}, {mode}, mse_vs_golden={:.6}, mse_vs_png2c64={:.6}",
        s.mse_vs_golden, s.mse_vs_png2c64
    );
    assert!(
        s.mse_vs_golden <= s.threshold,
        "{name}, {mode}: mse_vs_golden {:.6} exceeds {} from fixtures/{name}/thresholds.toml \
         (mse_vs_png2c64 {:.6}). If the change is intended, run \
         `cargo xtask regen-goldens --accept --reason \"...\" --pr N`.",
        s.mse_vs_golden,
        s.threshold,
        s.mse_vs_png2c64
    );
}

macro_rules! golden_cases {
    ($($name:ident => $format:expr),* $(,)?) => {
        mod format {
            use convert64_formats::BitmapFormat;
            $(#[test] fn $name() { super::check_format(stringify!($name), $format); })*
        }
        mod prg {
            use convert64_formats::BitmapFormat;
            $(#[test] fn $name() { super::check_prg(stringify!($name), $format); })*
        }
        mod solver {
            use convert64_formats::BitmapFormat;
            $(#[test] fn $name() { super::check_solver(stringify!($name), $format); })*
        }
        /// Every fixture, for the harness self-tests.
        const ALL: &[(&str, BitmapFormat)] = &[$((stringify!($name), $format)),*];
    };
}

golden_cases! {
    portrait => BitmapFormat::Koala,
    terrain => BitmapFormat::Koala,
    title => BitmapFormat::Koala,
    gradient => BitmapFormat::Koala,
    photo => BitmapFormat::Koala,
    lineart => BitmapFormat::ArtStudio,
}

// ---------------------------------------------------------------------------------------
// Composite scenes (E4): layered scenes in `fixtures/composite/<scene>/scene.toml`,
// composited and compared with `expected_1x.png` (the composite box-filtered to the
// target grid; straight sRGB, alpha = coverage) and `expected_attr.png` (the attribute
// map's debug render). Comparison allows ±2 per channel so that last-bit differences in
// the platform's `sin`/`exp` cannot fail CI; `regen-goldens` compares exactly.

mod composite_scene {
    use convert64_core::geom::PxRect;
    use convert64_core::layer::{Adjustments, Blend, Layer, Recipe, SourceImage, Transform};
    use convert64_core::mask::{Mask, MaskRaster, Modifiers, SourceEntry};
    use convert64_core::mode::Mode;
    use convert64_core::palette::linear_to_srgb8;
    use convert64_core::region::Region;
    use convert64_core::{Compositor, SuperGrid};
    use serde::Deserialize;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    const fn one() -> f32 {
        1.0
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Scene {
        target: Mode,
        n: u8,
        layers: Vec<SceneLayer>,
        #[serde(default)]
        regions: Vec<SceneRegion>,
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct SceneLayer {
        name: String,
        /// A fixture name: its `source.png` is the layer's pixels.
        source: String,
        #[serde(default)]
        transform: Transform,
        #[serde(default = "one")]
        opacity: f32,
        #[serde(default)]
        blend: Blend,
        #[serde(default)]
        recipe: Recipe,
        #[serde(default)]
        adjustments: Adjustments,
        mask: Option<SceneMask>,
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct SceneMask {
        /// The raster's uniform value before sources.
        #[serde(default)]
        fill: f32,
        #[serde(default)]
        sources: Vec<SourceEntry>,
        #[serde(default)]
        modifiers: Modifiers,
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct SceneRegion {
        name: String,
        /// Canvas rectangle `[x0, y0, x1, y1]` where the region's mask is 1.
        rect: [u32; 4],
        recipe: Recipe,
    }

    fn fixtures() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
    }

    fn load_source(fixture: &str) -> Arc<SourceImage> {
        let path = fixtures().join(fixture).join("source.png");
        let img = image::open(&path)
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
            .to_rgba8();
        let (w, h) = img.dimensions();
        Arc::new(SourceImage::from_srgba8(w, h, img.as_raw()).expect("decoded PNG"))
    }

    /// Composites a scene: `(target width, height, composite RGBA8, attribute map RGBA8)`.
    pub(super) fn render(scene_dir: &Path) -> (u32, u32, Vec<u8>, Vec<u8>) {
        let text = std::fs::read_to_string(scene_dir.join("scene.toml")).expect("scene.toml");
        let scene: Scene = toml::from_str(&text).expect("valid scene.toml");
        let grid = SuperGrid::new(scene.target, scene.n).expect("valid grid");
        let mut sources = std::collections::HashMap::new();
        let layers: Vec<Layer> = scene
            .layers
            .into_iter()
            .map(|l| {
                let src = Arc::clone(
                    sources
                        .entry(l.source.clone())
                        .or_insert_with(|| load_source(&l.source)),
                );
                let (w, h) = src.size();
                let mut layer = Layer::new(l.name, src);
                layer.set_transform(l.transform);
                layer.set_opacity(l.opacity).expect("opacity");
                layer.set_blend(l.blend);
                layer.set_recipe(l.recipe);
                layer.set_adjustments(l.adjustments);
                if let Some(m) = l.mask {
                    let mut mask = Mask::new(MaskRaster::filled(w, h, m.fill).expect("mask"));
                    mask.set_sources(m.sources);
                    mask.set_modifiers(m.modifiers);
                    layer.set_mask(Some(mask)).expect("mask size");
                }
                layer
            })
            .collect();
        let regions: Vec<Region> = scene
            .regions
            .into_iter()
            .map(|r| {
                let rect = PxRect::new(r.rect[0], r.rect[1], r.rect[2], r.rect[3]);
                let vals = (0..320 * 200)
                    .map(|i| {
                        if rect.contains(i % 320, i / 320) {
                            1.0
                        } else {
                            0.0
                        }
                    })
                    .collect();
                Region::new(
                    r.name,
                    MaskRaster::from_values(320, 200, vals).unwrap(),
                    r.recipe,
                )
                .expect("region")
            })
            .collect();
        let mut c = Compositor::new(grid);
        let colour: Vec<u8> = c
            .composite_with(&layers, &regions)
            .downsample()
            .iter()
            .flat_map(|p| {
                let a = p[3];
                let rgb = if a > 0.0 {
                    [p[0] / a, p[1] / a, p[2] / a].map(linear_to_srgb8)
                } else {
                    [0; 3]
                };
                [
                    rgb[0],
                    rgb[1],
                    rgb[2],
                    (a.clamp(0.0, 1.0) * 255.0).round() as u8,
                ]
            })
            .collect();
        let attr: Vec<u8> = c
            .attribute_map()
            .debug_rgba()
            .into_iter()
            .flatten()
            .collect();
        let (w, h) = grid.target_size();
        (w, h, colour, attr)
    }

    fn save(path: &Path, w: u32, h: u32, rgba: &[u8]) {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).expect("candidate dir");
        }
        image::RgbaImage::from_raw(w, h, rgba.to_vec())
            .expect("RGBA size")
            .save(path)
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    }

    fn compare(name: &str, file: &str, w: u32, h: u32, got: &[u8]) {
        let path = fixtures().join("composite").join(name).join(file);
        let want = image::open(&path)
            .unwrap_or_else(|e| {
                panic!(
                    "{}: {e}; run `cargo xtask regen-goldens --accept --reason \"...\" --pr N`",
                    path.display()
                )
            })
            .to_rgba8();
        assert_eq!(want.dimensions(), (w, h), "{}", path.display());
        let worst = want
            .as_raw()
            .iter()
            .zip(got)
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap_or(0);
        let differing = want
            .as_raw()
            .chunks(4)
            .zip(got.chunks(4))
            .filter(|(a, b)| a != b)
            .count();
        println!("composite/{name}/{file}: {differing} pixels differ, worst channel {worst}");
        assert!(
            worst <= 2,
            "composite/{name}/{file}: a channel differs by {worst} (> 2); if intended, run \
             `cargo xtask regen-goldens --accept --reason \"...\" --pr N`"
        );
    }

    pub(super) fn check(name: &str) {
        let dir = fixtures().join("composite").join(name);
        let (w, h, colour, attr) = render(&dir);
        if let Some(out) = std::env::var_os("CONVERT64_GOLDEN_OUT") {
            let out = PathBuf::from(out).join("composite").join(name);
            save(&out.join("expected_1x.png"), w, h, &colour);
            save(&out.join("expected_attr.png"), w, h, &attr);
        }
        compare(name, "expected_1x.png", w, h, &colour);
        compare(name, "expected_attr.png", w, h, &attr);
    }
}

mod composite {
    #[test]
    fn portrait_over_title() {
        super::composite_scene::check("portrait-over-title");
    }

    #[test]
    fn title_multiply_hires() {
        super::composite_scene::check("title-multiply-hires");
    }
}

// ---------------------------------------------------------------------------------------
// Harness self-tests.

/// A deliberately different solver must fail every fixture's threshold.
#[test]
fn a_deliberate_solver_change_is_caught() {
    fn shifted(input: &IndexedBitmap) -> IndexedBitmap {
        // Every pixel moved to the next palette index: legal, and clearly different.
        let mut out = input.clone();
        let (w, h) = (out.width(), out.height());
        for y in 0..h {
            for x in 0..w {
                let c = out.pixel(x, y);
                let next = convert64_core::ColourIndex::from_low_nybble(c.get() + 1);
                out.set_pixel(x, y, next);
            }
        }
        if let Some(bg) = input.background() {
            // Keep the picture legal: rebuild with the shifted background.
            let next = convert64_core::ColourIndex::from_low_nybble(bg.get() + 1);
            return IndexedBitmap::new(out.mode(), out.pixels().to_vec(), Some(next))
                .expect("same size and mode");
        }
        out
    }
    for &(name, format) in ALL {
        let s = score(name, format, shifted, false);
        assert!(
            s.mse_vs_golden > s.threshold,
            "{name}: a shifted palette scored {:.6}, not above {}",
            s.mse_vs_golden,
            s.threshold
        );
    }
}

#[test]
fn identical_images_have_zero_mse() {
    let a = vec![Rgb8::new(10, 20, 30); 320 * 200];
    assert!(oklab_mse(&a, &a).abs() < f64::EPSILON);
    let mut b = a.clone();
    b[0] = Rgb8::new(255, 255, 255);
    assert!(oklab_mse(&a, &b) > 0.0);
}

#[test]
fn hex_dump_names_the_cell_of_a_changed_byte() {
    let (_, file) = read_baseline("portrait", BitmapFormat::Koala);
    let mut bytes = serialise(&file);
    let before = hex_dump(BitmapFormat::Koala, &bytes);
    // Byte 2 + 41*8 + 3 is row 3 of bitmap cell (1, 1).
    bytes[2 + 41 * 8 + 3] ^= 0xFF;
    let after = hex_dump(BitmapFormat::Koala, &bytes);
    let changed: Vec<(&str, &str)> = before
        .lines()
        .zip(after.lines())
        .filter(|(a, b)| a != b)
        .collect();
    assert_eq!(changed.len(), 1);
    assert!(
        changed[0].1.starts_with("bitmap cell ( 1,  1)"),
        "{}",
        changed[0].1
    );
}

#[test]
fn thresholds_parse_comments_and_nested_mode_tables() {
    let t = Thresholds::parse(
        "# Thresholds for a test fixture.\n\
         mse_vs_golden_max = 0.0002 # default\n\
         \n\
         [mode.multicolor-bitmap]\n\
         # looser for multicolor\n\
         mse_vs_golden_max = 0.0005\n",
    )
    .expect("valid thresholds");
    assert!((t.mse_vs_golden_max("multicolor-bitmap") - 0.0005).abs() < 1e-12);
    assert!((t.mse_vs_golden_max("hires-bitmap") - 0.0002).abs() < 1e-12);
    assert!(Thresholds::parse("mse_vs_golden_max = \"high\"").is_err());
    assert!(Thresholds::parse("mse_vs_golden_max = 0.1\ntypo = 1").is_err());
}

#[test]
fn every_committed_thresholds_file_parses() {
    for &(name, format) in ALL {
        let t = threshold(name, mode_name(format));
        assert!(t > 0.0 && t < 0.01, "{name}: {t}");
    }
}
