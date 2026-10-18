//! Golden-test harness (`docs/TAD.md` §6, #40). One format case, one display-PRG case
//! and one solver case per fixture:
//!
//! - **Format, bit-exact.** The fixture's png2c64 baseline (`baseline.kla`/`.art`) is
//!   read into the model and serialised again. The bytes are snapshotted with `insta` as
//!   an annotated hex dump (`fixtures/<name>/expected_<ext>.snap`). Every byte is in the
//!   snapshot, so any change fails, and the diff names the section and cell that moved.
//!   The re-serialisation is the formats crate's writer (`BitmapFile::to_bytes`).
//! - **Display PRG, bit-exact.** `convert64_formats::prg::display_prg` is run on the
//!   baseline file twice: both runs must give the same bytes, and the picture file
//!   embedded at `PAYLOAD_OFFSET` must equal the baseline. The PRG is snapshotted as a
//!   hex dump (`fixtures/<name>/expected_prg.snap`): the BASIC line and loader in rows
//!   of 16 bytes, then the embedded file in the format case's layout. `cargo xtask
//!   vice-check` runs these PRGs in VICE.
//! - **Solver, perceptual.** The fixture's `source.png` is composited at 4× to fill the
//!   screen, with png2c64's preprocessing (`--gamma 1.5`, as [`Gamma::PNG2C64`]) applied
//!   as a layer adjustment, and solved with the recipe closest to the png2c64 baseline
//!   settings (see [`baseline_recipe`]). The result is rendered through the palette and
//!   compared by OKLab mean squared error with `fixtures/<name>/expected.png` (it must
//!   stay within `mse_vs_golden_max`) and with the preprocessed source, box-filtered to
//!   the target grid: the picture both converters actually solved (#142). The png2c64
//!   baseline is scored against the same source by the same code, per pixel and after
//!   a blur that integrates dither patterns the way the eye does ([`blur_mse`]). Neither
//!   is a bar any more (#156): both said we beat png2c64 while the side-by-side sheet
//!   showed png2c64 winning by eye (per-pixel MSE rewards flat wrong colours, blurred
//!   MSE rewards banded areas with the right regional average). The gate is a human
//!   review of `sheet-vs-png2c64.png` (CONTRIBUTING.md, TAD §6). `thresholds.toml`
//!   records both png2c64 values and the test checks they still hold. Each case prints `fixture, mode, mse_vs_golden,
//!   mse_vs_png2c64, mse_vs_source, png2c64_vs_source, blur_mse, png2c64_blur_mse` so
//!   PRs can report the delta to png2c64 (`CLAUDE.md`).
//!
//! Golden candidates: with `CONVERT64_GOLDEN_OUT=<dir>` set, each solver case also writes
//! its rendering to `<dir>/<fixture>/expected.png`. `cargo xtask regen-goldens` uses this
//! (and `INSTA_UPDATE=new`) to list and accept changes.

use convert64_core::Palette;
use convert64_core::composite::TargetFrame;
use convert64_core::geom::Vec2;
use convert64_core::layer::{
    Adjustments, DitherKind, Gamma, Layer, Recipe, SourceImage, Transform,
};
use convert64_core::model::IndexedBitmap;
use convert64_core::palette::{LinearRgb, Rgb8, linear_to_srgb8, oklab_of, srgb8_to_linear};
use convert64_core::solve::{SolveOptions, Solver, SolverKind, Steering};
use convert64_core::validate;
use convert64_core::{Compositor, SuperGrid};
use convert64_formats::prg::{LOAD_ADDRESS, PAYLOAD_ADDRESS, PAYLOAD_OFFSET, SYS_ADDRESS};
use convert64_formats::{BitmapFile, BitmapFormat};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

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
    file.to_bytes()
        .expect("the baseline's mode fits its format")
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

/// The native solver (E5, #157). The default is the reference solver (#181), whose bar
/// is the png2c64 parity test; the native solver keeps its goldens here, and the variant
/// tests below exercise its knobs.
fn native() -> SolveOptions {
    SolveOptions {
        solver: SolverKind::Native,
        ..SolveOptions::default()
    }
}

/// The recipe closest to the png2c64 baseline settings (`fixtures/README.md`):
/// png2c64 ran `--dither checker --dither-strength 0.7` for the multicolor fixtures (our
/// `checker` since #114) and `--dither none` for `lineart`.
fn baseline_recipe(format: BitmapFormat) -> Recipe {
    match format {
        BitmapFormat::Koala => Recipe::with_method(DitherKind::Checker, 0.7),
        BitmapFormat::ArtStudio => Recipe::with_method(DitherKind::None, 0.0),
    }
    .expect("valid recipe")
}

fn mode_of(format: BitmapFormat) -> convert64_core::mode::Mode {
    match format {
        BitmapFormat::Koala => convert64_core::mode::Mode::MulticolorBitmap,
        BitmapFormat::ArtStudio => convert64_core::mode::Mode::HiresBitmap,
    }
}

/// A fixture's source composited at 4× to fill the screen, and the real solver's output.
struct Solved {
    frame: TargetFrame,
    output: IndexedBitmap,
    solution: convert64_core::solve::Solution,
}

/// The fixture solved with the png2c64-like recipe (see [`baseline_recipe`]).
fn solved(name: &str, format: BitmapFormat) -> Arc<Solved> {
    solved_with(
        name,
        format,
        "",
        baseline_recipe(format),
        &SolveOptions::default(),
        Gamma::PNG2C64,
    )
}

/// Composites and solves a fixture with `recipe` and `opts`, its source preprocessed
/// with `gamma`, once per test binary; `variant` names the combination in the cache.
fn solved_with(
    name: &str,
    format: BitmapFormat,
    variant: &str,
    recipe: Recipe,
    opts: &SolveOptions,
    gamma: Gamma,
) -> Arc<Solved> {
    solved_layer(name, format, variant, opts, &|layer| {
        layer.set_recipe(recipe);
        // png2c64's preprocessing (`Gamma::PNG2C64`), so both converters solve (and are
        // scored against) the same picture (#142).
        layer.set_adjustments(Adjustments {
            gamma,
            ..Adjustments::default()
        });
    })
}

/// [`solved_with`] for a layer `setup` makes: the fixture fitted to the screen, then
/// `setup`'s recipe, adjustments and resampling.
fn solved_layer(
    name: &str,
    format: BitmapFormat,
    variant: &str,
    opts: &SolveOptions,
    setup: &dyn Fn(&mut Layer),
) -> Arc<Solved> {
    static CACHE: OnceLock<Mutex<HashMap<String, Arc<Solved>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Mutex::default);
    let key = format!("{name}/{variant}");
    if let Some(s) = cache.lock().expect("cache").get(&key) {
        return Arc::clone(s);
    }
    // `card` is the dither-preview test card (`docs/reference/dither-previews/`).
    let path = if name == "card" {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/reference/dither-previews/testcard.png")
    } else {
        fixture_dir(name).join("source.png")
    };
    let img = image::open(&path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .to_rgba8();
    let (w, h) = img.dimensions();
    let src = Arc::new(SourceImage::from_srgba8(w, h, img.as_raw()).expect("decoded PNG"));
    let mut layer = Layer::new(name, src);
    layer.set_transform(
        Transform::new(
            Vec2::ZERO,
            Vec2::new(320.0 / w as f32, 200.0 / h as f32),
            false,
            false,
        )
        .expect("fit to screen"),
    );
    setup(&mut layer);
    let grid = SuperGrid::with_default_factor(mode_of(format)).expect("bitmap grid");
    let mut c = Compositor::new(grid);
    // The solver's front end (ADR-014 §2): png2amiga's for `--solver png2amiga`.
    c.set_pipeline(opts.solver.pipeline());
    c.update(&[layer], &[]);
    let frame = c.frame().clone();
    let solution = Solver::new(&Palette::pepto_pal()).solve(&frame, &Steering::new(), None, opts);
    let output = solution.image.to_indexed();
    let s = Arc::new(Solved {
        frame,
        output,
        solution,
    });
    cache.lock().expect("cache").insert(key, Arc::clone(&s));
    s
}

/// A recipe variant on a fixture: solved, validated, rendered, written as a candidate
/// `expected_<variant>.png` and compared with the committed one within the fixture's
/// `mse_vs_golden_max`. Returns the solve for extra checks.
fn check_variant(
    name: &str,
    format: BitmapFormat,
    variant: &str,
    recipe: Recipe,
    opts: &SolveOptions,
) -> Arc<Solved> {
    check_variant_from(name, format, variant, recipe, opts, Gamma::PNG2C64)
}

/// [`check_variant`] with the source preprocessed by `gamma`.
fn check_variant_from(
    name: &str,
    format: BitmapFormat,
    variant: &str,
    recipe: Recipe,
    opts: &SolveOptions,
    gamma: Gamma,
) -> Arc<Solved> {
    let (s, result) = variant_result(name, format, variant, recipe, opts, gamma);
    if let Err(e) = result {
        panic!("{e}");
    }
    s
}

/// [`check_variant_from`] without panicking: the solve, and why it fails if it does.
/// The candidate is written either way.
fn variant_result(
    name: &str,
    format: BitmapFormat,
    variant: &str,
    recipe: Recipe,
    opts: &SolveOptions,
    gamma: Gamma,
) -> (Arc<Solved>, Result<(), String>) {
    let s = solved_with(name, format, variant, recipe, opts, gamma);
    judge_variant(name, format, variant, s)
}

/// A solved variant validated, rendered, written as a candidate `expected_<variant>.png`
/// and compared with the committed one within the fixture's `mse_vs_golden_max`.
fn judge_variant(
    name: &str,
    format: BitmapFormat,
    variant: &str,
    s: Arc<Solved>,
) -> (Arc<Solved>, Result<(), String>) {
    assert_eq!(validate(&s.output), Ok(()), "{name}/{variant}: not legal");
    let rendered = render(&s.output, &Palette::pepto_pal());
    let file = format!("expected_{variant}.png");
    if let Some(out) = std::env::var_os("CONVERT64_GOLDEN_OUT") {
        save_png(&PathBuf::from(out).join(name).join(&file), &rendered);
    }
    let Some(golden) = load_png(&fixture_dir(name).join(&file)) else {
        let e =
            format!("{name}: fixtures/{name}/{file} is missing; run `cargo xtask regen-goldens`");
        return (s, Err(e));
    };
    let mse = oklab_mse(&rendered, &golden);
    let reference = reference(&s.frame);
    let source = oklab_mse(&rendered, &reference);
    let blurred = blur_mse(&rendered, &reference);
    let threshold = thresholds(name).mse_vs_golden_max(mode_name(format));
    println!(
        "{name}/{variant}, mse_vs_golden={mse:.6}, mse_vs_source={source:.6}, blur_mse={blurred:.6}"
    );
    let result = if mse <= threshold {
        Ok(())
    } else {
        Err(format!(
            "{name}/{variant}: mse_vs_golden {mse:.6} exceeds {threshold}; if intended, run \
             `cargo xtask regen-goldens --accept --reason \"...\" --pr pending`"
        ))
    };
    (s, result)
}

/// ADR-014 §4: the line-art recipe, an opt-in chosen by eye on #273's cause sheet (tile
/// 13): hi-res `none` at gamma 1.5, the layer resampled with png2amiga's widened
/// Lanczos-3, black point 0.5, and the cells limited to Pepto's greys (black, dark grey,
/// grey, light grey, white), on the png2amiga solver (#276): its brute-force cell pairs
/// reproduce tile 13, where the reference's global Floyd–Steinberg replacement of each
/// cell's pair gave 87.6 % of it (`sheet-lineart-recipe.md`).
mod line_art {
    use super::{BitmapFormat, judge_variant, solved_layer};
    use convert64_core::ColourIndex;
    use convert64_core::colour::ColourSet;
    use convert64_core::layer::FitMode;
    use convert64_core::layer::{
        Adjustments, BlackPoint, DitherKind, Gamma, Method, Metric, PatternPhase, Recipe, Resample,
    };
    use convert64_core::solve::{SolveOptions, SolverKind};

    /// Pepto's greys by lightness: black, dark grey, grey, light grey, white.
    pub(super) const GREYS: [u8; 5] = [0, 11, 12, 15, 1];

    #[test]
    fn lineart_recipe() {
        let s = solved_layer(
            "lineart",
            BitmapFormat::ArtStudio,
            "lineart-recipe",
            &SolveOptions {
                solver: SolverKind::Png2amiga,
                ..SolveOptions::default()
            },
            &|layer| {
                let greys: ColourSet = GREYS
                    .iter()
                    .map(|&c| ColourIndex::new(c).expect("a palette index"))
                    .collect();
                let recipe = Recipe::new(
                    Method::new(DitherKind::None),
                    0.0,
                    PatternPhase::default(),
                    Metric::default(),
                    greys,
                    FitMode::default(),
                )
                .expect("a valid recipe");
                layer.set_recipe(recipe);
                layer.set_adjustments(Adjustments {
                    gamma: Gamma::PNG2C64,
                    black_point: BlackPoint::new(0.5).expect("in range"),
                    ..Adjustments::default()
                });
                layer.set_resample(Resample::Lanczos);
            },
        );
        assert_eq!(s.solution.solver, SolverKind::Png2amiga);
        assert_eq!(s.solution.fallback, None);
        let used: std::collections::BTreeSet<u8> =
            s.output.pixels().iter().map(|c| c.get()).collect();
        assert!(used.iter().all(|c| GREYS.contains(c)), "{used:?}");
        let (_, result) = judge_variant("lineart", BitmapFormat::ArtStudio, "lineart-recipe", s);
        if let Err(e) = result {
            panic!("{e}");
        }
    }
}

/// Solver knobs (#123) on `portrait`.
/// #114: the `gradient` fixture dithered with every method id, so a change to any
/// matrix, noise function or kernel fails a test. Solved from the raw source with a
/// fixed black background and no refinement (fast, and the pattern is what is under
/// test), at full strength with continuous ratios (`mix_levels = 0`) so every
/// threshold of a matrix shows. These have no png2c64 bar: effects (`radial`,
/// `crosshatch`, `value-noise`) are not quantisers, and png2c64 has no equivalent of
/// most ids.
mod methods {
    use super::{BitmapFormat, DitherKind, Gamma, Recipe, SolveOptions, variant_result};
    use convert64_core::ColourIndex;
    use convert64_core::layer::Method;
    use convert64_core::solve::Background;

    /// The method's reference id, as recipes spell it.
    fn id(kind: DitherKind) -> String {
        let text = toml::to_string(&Method::new(kind)).expect("method");
        text.split('"').nth(1).expect("kind = \"id\"").to_owned()
    }

    #[test]
    fn gradient_with_every_method() {
        let opts = SolveOptions {
            background: Background::Fixed(ColourIndex::BLACK),
            refinements: 0,
            finalists: 1,
            ..super::native()
        };
        // Every method is solved and its candidate written before any failure is
        // reported, so one `regen-goldens` run picks them all up.
        let failures: Vec<String> = DitherKind::ALL
            .into_iter()
            .filter_map(|kind| {
                let r = Recipe::with_method(kind, 1.0)
                    .and_then(|r| r.with_mix_levels(0))
                    .expect("recipe");
                let variant = format!("m-{}", id(kind));
                variant_result(
                    "gradient",
                    BitmapFormat::Koala,
                    &variant,
                    r,
                    &opts,
                    Gamma::IDENTITY,
                )
                .1
                .err()
            })
            .collect();
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }
}

/// #115: the palette-aware methods on `portrait` from png2c64's preprocessed source, at
/// png2c64's strength (0.7), next to Ostromoukhov and checker for comparison. The
/// harness prints each one's per-pixel and blurred OKLab MSE against the source.
mod aware {
    use super::{BitmapFormat, DitherKind, Recipe, check_variant};

    fn portrait(kind: DitherKind, variant: &str) {
        let r = Recipe::with_method(kind, 0.7).expect("recipe");
        check_variant(
            "portrait",
            BitmapFormat::Koala,
            variant,
            r,
            &super::native(),
        );
    }

    #[test]
    fn portrait_yliluoma1() {
        portrait(DitherKind::Yliluoma1, "yliluoma1");
    }

    #[test]
    fn portrait_yliluoma2() {
        portrait(DitherKind::Yliluoma2, "yliluoma2");
    }

    #[test]
    fn portrait_yliluoma3() {
        portrait(DitherKind::Yliluoma3, "yliluoma3");
    }

    #[test]
    fn portrait_knoll() {
        portrait(DitherKind::Knoll, "knoll");
    }

    #[test]
    fn portrait_luma() {
        portrait(DitherKind::Luma, "luma");
    }

    #[test]
    fn portrait_pair() {
        portrait(DitherKind::Pair, "pair");
    }

    #[test]
    fn portrait_ostromoukhov() {
        portrait(DitherKind::Ostromoukhov, "ostromoukhov");
    }

    #[test]
    fn portrait_checker() {
        portrait(DitherKind::Checker, "checker");
    }
}

/// #120: the `blur` and `ssim` cell metrics on `title`, with the baseline recipe.
mod cell_metrics {
    use super::{BitmapFormat, baseline_recipe, check_variant};
    use convert64_core::layer::CellMetric;

    fn title(metric: CellMetric, variant: &str) {
        let r = baseline_recipe(BitmapFormat::Koala).with_cell_metric(metric);
        check_variant("title", BitmapFormat::Koala, variant, r, &super::native());
    }

    #[test]
    fn title_blur() {
        title(CellMetric::Blur, "cell-blur");
    }

    #[test]
    fn title_ssim() {
        title(CellMetric::Ssim, "cell-ssim");
    }
}

/// #156: png2c64's ordered semantics, the chroma guard and bridging colours, on the
/// dither-preview test card, and E5's semantics kept reproducible.
mod ordered_semantics {
    use super::{BitmapFormat, DitherKind, Gamma, Recipe, check_variant, solved_with};
    use convert64_core::ColourIndex;
    use convert64_core::layer::OrderedSemantics;

    /// The card's grey ramp (display rows 0–47: whole cells) solved with `kind`.
    fn ramp(kind: DitherKind) -> std::sync::Arc<super::Solved> {
        solved_with(
            "card",
            BitmapFormat::Koala,
            &format!("card-{}", kind.id()),
            Recipe::with_method(kind, 1.0).expect("recipe"),
            &super::native(),
            Gamma::PNG2C64,
        )
    }

    /// The greys from darkest to lightest.
    const GREYS: [ColourIndex; 5] = [
        ColourIndex::BLACK,
        ColourIndex::DARK_GREY,
        ColourIndex::MEDIUM_GREY,
        ColourIndex::LIGHT_GREY,
        ColourIndex::WHITE,
    ];

    #[test]
    fn the_grey_ramp_has_no_chromatic_pixels() {
        for kind in [
            DitherKind::Bayer4,
            DitherKind::Checker,
            DitherKind::BlueNoise,
        ] {
            let s = ramp(kind);
            for y in 0..48u16 {
                for x in 0..160u16 {
                    let c = s.output.pixel(x, y);
                    assert!(GREYS.contains(&c), "{kind:?}: ({x}, {y}) is {c}");
                }
            }
        }
    }

    #[test]
    fn no_grey_ramp_cell_skips_an_intermediate_grey() {
        for kind in [
            DitherKind::Bayer4,
            DitherKind::Checker,
            DitherKind::BlueNoise,
        ] {
            let s = ramp(kind);
            for cy in 0..6u16 {
                for cx in 0..40u16 {
                    let mut used = [false; 5];
                    for y in cy * 8..cy * 8 + 8 {
                        for x in cx * 4..cx * 4 + 4 {
                            let c = s.output.pixel(x, y);
                            if let Some(k) = GREYS.iter().position(|g| *g == c) {
                                used[k] = true;
                            }
                        }
                    }
                    let (lo, hi) = (
                        used.iter().position(|u| *u).unwrap_or(0),
                        used.iter().rposition(|u| *u).unwrap_or(0),
                    );
                    assert!(
                        used[lo..=hi].iter().all(|u| *u),
                        "{kind:?}: cell ({cx}, {cy}) uses {used:?}"
                    );
                }
            }
        }
    }

    /// E5's semantics reproduce the portrait golden as it was before #156.
    #[test]
    fn legacy_semantics_reproduce_the_e5_portrait() {
        let r = Recipe::with_method(DitherKind::Bayer2, 0.7)
            .expect("recipe")
            .with_ordered_semantics(OrderedSemantics::Legacy);
        check_variant(
            "portrait",
            BitmapFormat::Koala,
            "legacy",
            r,
            &super::native(),
        );
    }
}

/// #142: mix levels, pre-smoothing and the gradient swatches.
mod flat_areas {
    use super::{
        BitmapFormat, DitherKind, Gamma, Recipe, baseline_recipe, check_variant, solved_with,
    };
    use convert64_core::colour::ColourIndex;

    /// The swatch band of `gradient` is the 16 Pepto colours, 10 multicolor pixels each:
    /// solved from the raw source (png2c64's gamma would move them off the palette),
    /// every pixel of the cell rows wholly inside the band, away from the swatch edges,
    /// is its swatch's index.
    #[test]
    fn gradient_swatches_quantise_exactly() {
        let s = solved_with(
            "gradient",
            BitmapFormat::Koala,
            "raw",
            baseline_recipe(BitmapFormat::Koala),
            &super::native(),
            Gamma::IDENTITY,
        );
        for y in 152..200 {
            // The column on either side of a swatch edge blends through the resampler,
            // and png2c64's ordered semantics (#156) dither a blend, as they should.
            for x in (0..160).filter(|x| !matches!(x % 10, 0 | 9)) {
                assert_eq!(
                    s.output.pixel(x, y),
                    ColourIndex::ALL[(x / 10) as usize],
                    "swatch pixel ({x}, {y})"
                );
            }
        }
    }

    /// E5's continuous per-pixel ratio is still there as `mix_levels = 0` (legacy
    /// semantics, #156).
    #[test]
    fn portrait_continuous_ratio() {
        let r = Recipe::with_method(DitherKind::Bayer2, 0.7)
            .and_then(|r| r.with_mix_levels(0))
            .expect("recipe")
            .with_ordered_semantics(convert64_core::layer::OrderedSemantics::Legacy);
        check_variant(
            "portrait",
            BitmapFormat::Koala,
            "levels0",
            r,
            &super::native(),
        );
    }

    /// Pre-smoothing of the mix ratio (off by default; legacy semantics, #156).
    #[test]
    fn portrait_smoothed_ratio() {
        let r = Recipe::with_method(DitherKind::Bayer2, 0.7)
            .expect("recipe")
            .with_smooth(true)
            .with_ordered_semantics(convert64_core::layer::OrderedSemantics::Legacy);
        check_variant(
            "portrait",
            BitmapFormat::Koala,
            "smooth",
            r,
            &super::native(),
        );
    }
}

mod knobs {
    use super::{BitmapFormat, DitherKind, Recipe, SolveOptions, check_variant};

    fn fs() -> Recipe {
        Recipe::with_method(DitherKind::FloydSteinberg, 1.0).expect("recipe")
    }

    #[test]
    fn portrait_fs_cell_clamped() {
        check_variant(
            "portrait",
            BitmapFormat::Koala,
            "fs-clamped",
            fs(),
            &super::native(),
        );
    }

    #[test]
    fn portrait_fs_cell_unclamped() {
        check_variant(
            "portrait",
            BitmapFormat::Koala,
            "fs-unclamped",
            fs().with_cell_clamp(false),
            &super::native(),
        );
    }

    /// `max_luma_diff = 0` (PAL-blend safe): no cell's dither crosses a luma level.
    #[test]
    fn portrait_same_luma_only() {
        let r = Recipe::with_method(DitherKind::Bayer2, 0.7)
            .and_then(|r| r.with_max_luma_diff(0))
            .expect("recipe");
        let s = check_variant(
            "portrait",
            BitmapFormat::Koala,
            "luma0",
            r,
            &super::native(),
        );
        assert_eq!(s.solution.report.summary().luma_crossings, 0);
    }

    #[test]
    fn portrait_fs_adaptive() {
        let r = fs().with_adaptive(0.5).expect("recipe");
        check_variant(
            "portrait",
            BitmapFormat::Koala,
            "adaptive",
            r,
            &super::native(),
        );
    }

    #[test]
    fn portrait_fs_hysteresis() {
        let opts = SolveOptions {
            hysteresis: Some(0.97),
            ..super::native()
        };
        check_variant("portrait", BitmapFormat::Koala, "hysteresis", fs(), &opts);
    }
}

/// The source as the solver saw it: the composite on the target grid, in sRGB, rendered
/// at 320×200 like a picture.
fn reference(frame: &TargetFrame) -> Vec<Rgb8> {
    let (tw, _) = frame.grid().target_size();
    let x_scale = 320 / tw as usize;
    let colours: Vec<Rgb8> = frame
        .colour()
        .iter()
        .map(|c| {
            let [r, g, b] = c.map(linear_to_srgb8);
            Rgb8::new(r, g, b)
        })
        .collect();
    (0..200)
        .flat_map(|y| (0..320).map(move |x| (x, y)))
        .map(|(x, y)| colours[y * tw as usize + x / x_scale])
        .collect()
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

/// The blur [`blur_mse`] applies: binomial taps (`[1 4 6 4 1] / 16`, σ = 1 display
/// pixel), run twice per axis (σ ≈ 1.4), enough to integrate a 2 × 2 multicolor pattern.
const BLUR_TAPS: [f64; 5] = [1.0, 4.0, 6.0, 4.0, 1.0];

/// One blur pass over a 320 × 200 linear image, horizontally or vertically, edges
/// clamped.
fn blur_pass(img: &[[f64; 3]], horizontal: bool) -> Vec<[f64; 3]> {
    (0..320 * 200)
        .map(|i| {
            let (x, y) = ((i % 320) as i64, (i / 320) as i64);
            let mut acc = [0.0; 3];
            for (k, w) in (-2i64..=2).zip(BLUR_TAPS) {
                let (sx, sy) = if horizontal {
                    ((x + k).clamp(0, 319), y)
                } else {
                    (x, (y + k).clamp(0, 199))
                };
                let p = img[(sy * 320 + sx) as usize];
                for c in 0..3 {
                    acc[c] += w * p[c] / 16.0;
                }
            }
            acc
        })
        .collect()
}

/// OKLab MSE after blurring both renderings in linear light (#142): what an eye at a
/// normal viewing distance sees of a dither pattern. Calm, correct-on-average patterns
/// score well; blotches that switch with the source's micro-texture do not.
fn blur_mse(a: &[Rgb8], b: &[Rgb8]) -> f64 {
    let blur = |img: &[Rgb8]| {
        let mut v: Vec<[f64; 3]> = img
            .iter()
            .map(|c| [c.r, c.g, c.b].map(|u| f64::from(srgb8_to_linear(u))))
            .collect();
        for horizontal in [true, false, true, false] {
            v = blur_pass(&v, horizontal);
        }
        v
    };
    let (ba, bb) = (blur(a), blur(b));
    let lab = |p: [f64; 3]| {
        #[expect(clippy::cast_possible_truncation, reason = "colour channels")]
        let c = LinearRgb {
            r: p[0] as f32,
            g: p[1] as f32,
            b: p[2] as f32,
        }
        .to_oklab();
        [c.l, c.a, c.b].map(f64::from)
    };
    ba.iter()
        .zip(&bb)
        .map(|(&p, &q)| {
            let (x, y) = (lab(p), lab(q));
            (0..3).map(|k| (x[k] - y[k]) * (x[k] - y[k])).sum::<f64>()
        })
        .sum::<f64>()
        / ba.len() as f64
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
    /// png2c64's OKLab MSE against the source (reported; checked to still hold).
    png2c64_mse_vs_source: Option<f64>,
    /// png2c64's blurred OKLab MSE against the source ([`blur_mse`]): the quality bar,
    /// ours must not be worse.
    png2c64_blur_mse_vs_source: Option<f64>,
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

fn thresholds(name: &str) -> Thresholds {
    let path = fixture_dir(name).join("thresholds.toml");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    Thresholds::parse(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn mode_name(format: BitmapFormat) -> &'static str {
    match format {
        BitmapFormat::Koala => "multicolor-bitmap",
        BitmapFormat::ArtStudio => "hires-bitmap",
    }
}

struct SolverScores {
    /// Percentage of pixels identical to png2c64's baseline.
    same_as_png2c64: f64,
    mse_vs_golden: f64,
    mse_vs_png2c64: f64,
    mse_vs_source: f64,
    png2c64_vs_source: f64,
    blur_mse: f64,
    png2c64_blur_mse: f64,
    threshold: f64,
}

/// Scores the solver's output on one fixture, after `post` (identity for the real case;
/// the self-test passes a deliberate change). Only the real case writes golden
/// candidates (`write_candidate`).
fn score(
    name: &str,
    format: BitmapFormat,
    post: fn(&IndexedBitmap) -> IndexedBitmap,
    write_candidate: bool,
) -> SolverScores {
    let s = solved(name, format);
    let output = post(&s.output);
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
    let source = reference(&s.frame);
    SolverScores {
        same_as_png2c64: 100.0
            * rendered
                .iter()
                .zip(&baseline)
                .filter(|(a, b)| a == b)
                .count() as f64
            / rendered.len() as f64,
        mse_vs_golden: oklab_mse(&rendered, &golden),
        mse_vs_png2c64: oklab_mse(&rendered, &baseline),
        mse_vs_source: oklab_mse(&rendered, &source),
        png2c64_vs_source: oklab_mse(&baseline, &source),
        blur_mse: blur_mse(&rendered, &source),
        png2c64_blur_mse: blur_mse(&baseline, &source),
        threshold: thresholds(name).mse_vs_golden_max(mode_name(format)),
    }
}

fn check_solver(name: &str, format: BitmapFormat) {
    let mode = mode_name(format);
    let s = score(name, format, |o| o.clone(), true);
    println!(
        "{name}, {mode}, mse_vs_golden={:.6}, mse_vs_png2c64={:.6}, mse_vs_source={:.6}, \
         png2c64_vs_source={:.6}, blur_mse={:.6}, png2c64_blur_mse={:.6}",
        s.mse_vs_golden,
        s.mse_vs_png2c64,
        s.mse_vs_source,
        s.png2c64_vs_source,
        s.blur_mse,
        s.png2c64_blur_mse
    );
    println!(
        "{name}, {mode}: {:.3} % of pixels identical to png2c64's baseline",
        s.same_as_png2c64
    );
    let t = thresholds(name);
    let bar = t.png2c64_mse_vs_source.unwrap_or_else(|| {
        panic!(
            "{name}: thresholds.toml needs png2c64_mse_vs_source = {:.6}",
            s.png2c64_vs_source
        )
    });
    let blur_bar = t.png2c64_blur_mse_vs_source.unwrap_or_else(|| {
        panic!(
            "{name}: thresholds.toml needs png2c64_blur_mse_vs_source = {:.6}",
            s.png2c64_blur_mse
        )
    });
    assert!(
        (s.png2c64_blur_mse - blur_bar).abs() <= blur_bar * 0.01,
        "{name}: png2c64 now scores {:.6} blurred against the source, thresholds.toml says \
         {blur_bar:.6}",
        s.png2c64_blur_mse
    );
    assert!(
        (s.png2c64_vs_source - bar).abs() <= bar * 0.01,
        "{name}: png2c64 now scores {:.6} against the source, thresholds.toml says {bar:.6}",
        s.png2c64_vs_source
    );
    // The default solver is the reference (#181): its bar is the png2c64 parity test
    // (tests/png2c64_parity.rs), so its scores here are reported, not gated (a golden
    // still records it, and `regen-goldens --report` still lists a change).
    if s.mse_vs_golden > s.threshold {
        println!(
            "{name}, {mode}: mse_vs_golden {:.6} exceeds {} (reported only)",
            s.mse_vs_golden, s.threshold
        );
    }
}

/// The native solver on a fixture with the png2c64-like recipe, against
/// `expected_native.png` within the fixture's `mse_vs_golden_max`: the native solver keeps
/// its goldens and their gate (#181).
fn check_native(name: &str, format: BitmapFormat) {
    check_variant(name, format, "native", baseline_recipe(format), &native());
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
        mod native {
            use convert64_formats::BitmapFormat;
            $(#[test] fn $name() { super::check_native(stringify!($name), $format); })*
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

    /// A scene's grid, layers (bottom first) and regions.
    pub(super) fn load(scene_dir: &Path) -> (SuperGrid, Vec<Layer>, Vec<Region>) {
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
        (grid, layers, regions)
    }

    /// The composite box-filtered to the target grid as RGBA8: straight sRGB, alpha =
    /// coverage.
    pub(super) fn colour_rgba8(c: &Compositor) -> Vec<u8> {
        c.canvas()
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
            .collect()
    }

    /// Composites a scene: `(target width, height, composite RGBA8, attribute map RGBA8)`.
    pub(super) fn render(scene_dir: &Path) -> (u32, u32, Vec<u8>, Vec<u8>) {
        let (grid, layers, regions) = load(scene_dir);
        let mut c = Compositor::new(grid);
        c.composite_with(&layers, &regions);
        let colour = colour_rgba8(&c);
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

    pub(super) fn scene_dir(name: &str) -> PathBuf {
        fixtures().join("composite").join(name)
    }

    pub(super) fn check(name: &str) {
        let dir = scene_dir(name);
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

    /// #98: feathers from σ = 4 target px are blurred at half resolution. Each scene is
    /// composited with that and with every feather at full resolution, and the error of
    /// the half-resolution blur is printed and bounded: below one 8-bit step in the
    /// evaluated masks (0–1) and at most one in the composite (sRGB and coverage). The
    /// attribute map can change more, but only on a feather's far tail: a layer claims
    /// every super pixel its mask is above zero on, and the half-resolution blur spreads
    /// that support by a pixel or two, so a few target pixels there change source or
    /// strength.
    #[test]
    fn half_res_feather_error() {
        use super::composite_scene::{colour_rgba8, load, scene_dir};
        use convert64_core::Compositor;
        for name in ["portrait-over-title", "title-multiply-hires"] {
            let (grid, layers, regions) = load(&scene_dir(name));
            let mut half = Compositor::new(grid);
            half.composite_with(&layers, &regions);
            let mut full = Compositor::new(grid);
            full.set_full_res_feather(true);
            full.composite_with(&layers, &regions);
            let mut mask = 0.0f32;
            for i in 0..layers.len() {
                if let (Some((ra, a)), Some((rb, b))) =
                    (half.evaluated_mask(i), full.evaluated_mask(i))
                {
                    assert_eq!(ra, rb);
                    for (x, y) in a.iter().zip(b) {
                        mask = mask.max((x - y).abs());
                    }
                }
            }
            let colour = colour_rgba8(&half)
                .iter()
                .zip(colour_rgba8(&full))
                .map(|(a, b)| a.abs_diff(b))
                .max()
                .unwrap_or(0);
            // The least full-resolution mask value over target pixel (tx, ty)'s super
            // pixels, over every masked layer.
            let n = u32::from(grid.n());
            let tail = |tx: u32, ty: u32| {
                let mut least = f32::INFINITY;
                for i in 0..layers.len() {
                    let Some((r, v)) = full.evaluated_mask(i) else {
                        continue;
                    };
                    for y in ty * n..(ty + 1) * n {
                        for x in tx * n..(tx + 1) * n {
                            if r.contains(x, y) {
                                least = least.min(v[((y - r.y0) * r.width() + x - r.x0) as usize]);
                            }
                        }
                    }
                }
                least
            };
            let (ha, fa) = (half.frame().attributes(), full.frame().attributes());
            let tw = grid.target_size().0;
            let (mut strength, mut small, mut sources, mut tail_max) = (0.0f32, 0.0f32, 0, 0.0f32);
            for (i, (a, b)) in ha.pixels().iter().zip(fa.pixels()).enumerate() {
                let d = (a.strength - b.strength).abs();
                if a.source != b.source {
                    sources += 1;
                } else if d >= 1.0 / 255.0 {
                    strength = strength.max(d);
                } else {
                    // Elsewhere strength follows the mask, within one 8-bit step.
                    small = small.max(d);
                    continue;
                }
                let (tx, ty) = (i as u32 % tw, i as u32 / tw);
                tail_max = tail_max.max(tail(tx, ty));
            }
            println!(
                "half-res feather error, composite/{name}: mask {mask:.5}, composite {colour} \
                 / 255; attribute strength {small:.5}, except {sources} target pixels that \
                 change source and strength by up to {strength:.3}, all where a mask is at \
                 most {tail_max:.4}"
            );
            assert!(mask < 1.0 / 255.0, "{name}: mask error {mask}");
            assert!(colour <= 1, "{name}: composite error {colour}");
            assert!(
                tail_max < 0.01,
                "{name}: attributes change inside a feather"
            );
        }
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
        let th = thresholds(name);
        let t = th.mse_vs_golden_max(mode_name(format));
        assert!(t > 0.0 && t < 0.01, "{name}: {t}");
        let bar = th.png2c64_mse_vs_source.expect("png2c64_mse_vs_source");
        assert!(bar > 0.0 && bar < 1.0, "{name}: {bar}");
    }
}
