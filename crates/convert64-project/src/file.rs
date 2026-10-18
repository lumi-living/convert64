//! The `.convert64` project file (#163, ADR-009): the whole document the app and the CLI
//! share, as versioned JSON.
//!
//! A project holds the target mode, the supersample factor, the palette, the layers
//! (source, transform, opacity, blend, visibility, recipe, adjustments, mask), the region
//! overrides, the cell steering (pins, locks, allow and deny lists) and the solve
//! options. Each layer's source is either **linked** (a PNG path, relative to the
//! project file) or **embedded** (the PNG's bytes in base64), chosen per layer. A
//! project may also list registry commands, run in order after loading, as a recipe
//! file does; saving writes their result and no commands.
//!
//! Version-1 recipe files ([`crate::recipe::RecipeFile`], `"version": 1`) are read too:
//! [`ProjectFile::from_recipe`] maps one onto a project, so every recipe opens and saves
//! as a project. An older project file is migrated on load ([`ProjectFile::migrate`])
//! and says so once ([`ProjectFile::migration_note`]). The schema is documented in
//! `docs/reference/project-file.md`.
//!
//! Nothing here opens a file: linked sources, masks and `.vpl` palettes are read through
//! a [`Store`], so the browser build loads projects from bytes it was handed (#192).
//!
//! ```json
//! {
//!   "schema_version": 3,
//!   "target": "multicolor-bitmap",
//!   "palette": "pepto-pal",
//!   "layers": [{ "name": "title", "source": { "path": "title.png" },
//!                "recipe": { "method": { "kind": "bayer4" }, "strength": 0.8 } }]
//! }
//! ```

use crate::base64;
use crate::codec::{decode_mask, encode_mask};
use crate::command::JsonCommand;
use crate::error::{ProjectError, invalid};
use crate::recipe::RecipeFile;
use crate::store::Store;
use convert64_core::colour::ColourSet;
use convert64_core::layer::{Adjustments, Blend, Downscale, Recipe, Resample, Snap, Transform};
use convert64_core::mask::{MaskRaster, Modifiers, SourceEntry};
use convert64_core::mode::Mode;
use convert64_core::model::CellPos;
use convert64_core::palette::{Palette, Rgb8};
use convert64_core::solve::steering::{Pin, Steering};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The project-file schema version this build writes. Version 1 is the recipe file, which
/// has `"version"` instead of `"schema_version"`; project files from
/// [`OLDEST_SCHEMA_VERSION`] up are read and migrated ([`ProjectFile::migrate`]).
///
/// - 2 (#163): the project file.
/// - 3 (#236): `supersample` as ADR-012 made it. Every version-2 file was saved while
///   the default was 4 and wrote it, so loading one sets `supersample` to 1, the default
///   since #227: one look for every project.
pub const SCHEMA_VERSION: u32 = 3;

/// The oldest project-file `schema_version` this build reads.
pub const OLDEST_SCHEMA_VERSION: u32 = 2;

/// The project file's extension.
pub const EXTENSION: &str = "convert64";

/// A `.convert64` project file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectFile {
    /// [`SCHEMA_VERSION`] once loaded: an older file is migrated ([`Self::migrate`]).
    pub schema_version: u32,
    /// Target mode.
    pub target: Mode,
    /// Supersample factor, 1–8.
    #[serde(default = "default_supersample")]
    pub supersample: u8,
    /// The palette the picture is solved and previewed with.
    #[serde(default)]
    pub palette: PaletteSpec,
    /// Layers, bottom first.
    pub layers: Vec<LayerEntry>,
    /// Region overrides, applied in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub regions: Vec<RegionEntry>,
    /// Cell pins, locks and allow/deny lists.
    #[serde(default, skip_serializing_if = "SteeringSpec::is_empty")]
    pub steering: SteeringSpec,
    /// How the picture is solved.
    #[serde(default)]
    pub solve: SolveSpec,
    /// What transform commands snap to (ADR-011): `off` (the default, not written),
    /// `c64-pixel` or `cell`.
    #[serde(default, skip_serializing_if = "is_off")]
    pub snap: Snap,
    /// Registry commands run in order after loading (a recipe file's commands).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<JsonCommand>,
    /// The `schema_version` the file had before [`Self::migrate`] updated it, if it was
    /// older (#236). Neither read nor written: it is how a loaded file says it was
    /// migrated ([`Self::migration_note`]).
    #[serde(skip)]
    pub migrated_from: Option<u32>,
}

const fn default_supersample() -> u8 {
    convert64_core::SuperGrid::DEFAULT_N
}

const fn one() -> f32 {
    1.0
}

fn is_off(s: &Snap) -> bool {
    *s == Snap::Off
}

const fn yes() -> bool {
    true
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if passes a reference"
)]
const fn is_true(v: &bool) -> bool {
    *v
}

/// A palette: a built-in preset, 16 colours, or a VICE `.vpl` file.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PaletteSpec {
    /// Pepto's PAL palette (the default; `"pepto"` is accepted too).
    #[default]
    #[serde(alias = "pepto")]
    PeptoPal,
    /// Pepto's NTSC palette.
    PeptoNtsc,
    /// Colodore at colodore.com's default sliders.
    Colodore,
    /// 16 colours in index order, `"#RRGGBB"`.
    Custom(Vec<String>),
    /// A VICE palette file, relative to the project file.
    Vpl(PathBuf),
}

impl PaletteSpec {
    /// The palette, reading a `.vpl` file relative to `base` from `store`.
    pub fn resolve(&self, base: &Path, store: &dyn Store) -> Result<Palette, ProjectError> {
        Ok(match self {
            Self::PeptoPal => Palette::pepto_pal(),
            Self::PeptoNtsc => Palette::pepto_ntsc(),
            Self::Colodore => Palette::colodore(50.0, 100.0, 50.0),
            Self::Custom(colours) => {
                if colours.len() != 16 {
                    return Err(invalid(format!(
                        "a custom palette needs 16 colours, found {}",
                        colours.len()
                    )));
                }
                let mut rgb = [Rgb8::new(0, 0, 0); 16];
                for (slot, text) in rgb.iter_mut().zip(colours) {
                    *slot = parse_hex(text)?;
                }
                Palette::from_rgb("Custom", rgb)
            }
            Self::Vpl(path) => {
                let path = base.join(path);
                let bytes = store.read(&path)?;
                Palette::from_vpl(&bytes)
                    .map_err(|e| invalid(format!("{}: {e}", path.display())))?
            }
        })
    }

    /// The preset or file named on a command line: `pepto`, `pepto-pal`, `pepto-ntsc`,
    /// `colodore`, or a path ending in `.vpl`.
    pub fn from_arg(arg: &str) -> Result<Self, ProjectError> {
        if Path::new(arg)
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("vpl"))
        {
            return Ok(Self::Vpl(PathBuf::from(arg)));
        }
        serde_json::from_value(serde_json::Value::String(arg.to_owned())).map_err(|_| {
            invalid(format!(
                "unknown palette {arg:?}: use pepto, pepto-ntsc, colodore or a .vpl file"
            ))
        })
    }
}

fn parse_hex(text: &str) -> Result<Rgb8, ProjectError> {
    let hex = text.strip_prefix('#').unwrap_or(text);
    if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(invalid(format!("{text:?} is not a #RRGGBB colour")));
    }
    let v = u32::from_str_radix(hex, 16).map_err(invalid)?;
    Ok(Rgb8::from_hex(v))
}

/// Solve settings in a recipe or project file (the per-pixel knobs live in each layer's
/// recipe). Since #193 the core's document holds them ([`SolveSettings`]), so the
/// `solve_set` and `background_pin` commands can change them; the file form is the same.
///
/// [`SolveSettings`]: convert64_core::document::SolveSettings
pub type SolveSpec = convert64_core::document::SolveSettings;

/// A PNG file, relative to the project file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Linked {
    /// The path, `/`-separated.
    pub path: PathBuf,
}

/// A PNG file's bytes, in base64.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Embedded {
    /// Standard base64 with padding.
    pub png_base64: String,
}

/// The pixels of an earlier layer, shared (as `layer_via_mask` shares them).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Shared {
    /// The earlier layer's index.
    pub layer: usize,
}

/// Where a layer's pixels come from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SourceSpec {
    /// A bare path, as recipe files write it: the same as [`Linked`].
    Path(PathBuf),
    /// `{"path": "title.png"}`.
    Linked(Linked),
    /// `{"png_base64": "iVBORw0KGgo…"}`.
    Embedded(Embedded),
    /// `{"layer": 0}`.
    Shared(Shared),
}

/// One layer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayerEntry {
    /// Display name.
    pub name: String,
    /// The pixels.
    pub source: SourceSpec,
    /// Whether the layer takes part in the composite.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub visible: bool,
    /// Placement in canvas pixels.
    #[serde(default)]
    pub transform: Transform,
    /// Opacity, `0..=1`.
    #[serde(default = "one")]
    pub opacity: f32,
    /// Blend mode.
    #[serde(default)]
    pub blend: Blend,
    /// How the layer is sampled when shrunk a long way (#235); written only when not
    /// `auto`, so files without it load and save unchanged.
    #[serde(default, skip_serializing_if = "Downscale::is_auto")]
    pub downscale: Downscale,
    /// The kernel the layer's colour is resampled with (ADR-014 §3): `mitchell` (the
    /// default) or `lanczos`; written only when not `mitchell`.
    #[serde(default, skip_serializing_if = "Resample::is_mitchell")]
    pub resample: Resample,
    /// Dither recipe.
    #[serde(default)]
    pub recipe: Recipe,
    /// Colour adjustments.
    #[serde(default)]
    pub adjustments: Adjustments,
    /// The mask; none means fully opaque.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mask: Option<MaskSpec>,
}

/// One value everywhere.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fill {
    /// `0..=1`.
    pub fill: f32,
}

/// A mask raster: one value, a greyscale PNG (8 or 16 bit), or an embedded 16-bit PNG.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RasterSpec {
    /// `{"fill": 1.0}`.
    Fill(Fill),
    /// `{"path": "mask.png"}`.
    Linked(Linked),
    /// `{"png_base64": "…"}`.
    Embedded(Embedded),
}

/// A layer mask (ADR-008): the raster, procedural sources and modifiers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaskSpec {
    /// The painted raster, the layer's size.
    pub raster: RasterSpec,
    /// Gradients and keys, applied in order after the raster.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<SourceEntry>,
    /// Invert, levels, snap and feather.
    #[serde(default)]
    pub modifiers: Modifiers,
}

/// A region override: a canvas-space (320 × 200) mask with its own recipe.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegionEntry {
    /// Display name.
    pub name: String,
    /// The mask, 320 × 200.
    pub mask: RasterSpec,
    /// The recipe it imposes.
    #[serde(default)]
    pub recipe: Recipe,
}

/// A pinned cell.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PinEntry {
    /// Column, 0–39.
    pub x: u16,
    /// Row, 0–24.
    pub y: u16,
    /// 1–4 colours.
    pub colours: ColourSet,
    /// The whole set (true) or colours the set must contain (false).
    #[serde(default)]
    pub full: bool,
}

/// A cell with a colour list (allow or deny).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CellColours {
    /// Column, 0–39.
    pub x: u16,
    /// Row, 0–24.
    pub y: u16,
    /// The colours.
    pub colours: ColourSet,
}

/// Cell steering (UX §7), in cell order.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SteeringSpec {
    /// Pinned cells.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pins: Vec<PinEntry>,
    /// Locked cells.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub locks: Vec<CellPos>,
    /// Cells restricted to colours.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allow: Vec<CellColours>,
    /// Cells with forbidden colours.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deny: Vec<CellColours>,
}

impl SteeringSpec {
    /// Whether nothing is steered.
    pub fn is_empty(&self) -> bool {
        self.pins.is_empty()
            && self.locks.is_empty()
            && self.allow.is_empty()
            && self.deny.is_empty()
    }

    /// The steering as data.
    pub fn from_steering(s: &Steering) -> Self {
        let cells = || (0..25).flat_map(|y| (0..40).map(move |x| CellPos::new(x, y)));
        Self {
            pins: s
                .pins()
                .map(|(c, p)| PinEntry {
                    x: c.x,
                    y: c.y,
                    colours: p.colours,
                    full: p.full,
                })
                .collect(),
            locks: cells().filter(|c| s.locks().contains(*c)).collect(),
            allow: cells()
                .filter(|c| s.allow(*c) != ColourSet::ALL)
                .map(|c| CellColours {
                    x: c.x,
                    y: c.y,
                    colours: s.allow(c),
                })
                .collect(),
            deny: cells()
                .filter(|c| !s.deny(*c).is_empty())
                .map(|c| CellColours {
                    x: c.x,
                    y: c.y,
                    colours: s.deny(c),
                })
                .collect(),
        }
    }

    /// The steering, checking every cell is on the 40 × 25 screen and every pin names
    /// 1–4 colours.
    pub fn to_steering(&self) -> Result<Steering, ProjectError> {
        let cell = |x: u16, y: u16| {
            if x < 40 && y < 25 {
                Ok(CellPos::new(x, y))
            } else {
                Err(invalid(format!(
                    "steering: cell ({x}, {y}) is off the 40 × 25 screen"
                )))
            }
        };
        let mut s = Steering::new();
        for p in &self.pins {
            if p.colours.is_empty() || p.colours.len() > 4 {
                return Err(invalid(format!(
                    "steering: the pin on ({}, {}) needs 1–4 colours",
                    p.x, p.y
                )));
            }
            s.set_pin(
                cell(p.x, p.y)?,
                Pin {
                    colours: p.colours,
                    full: p.full,
                },
            );
        }
        for c in &self.locks {
            s.set_lock(cell(c.x, c.y)?, true);
        }
        for a in &self.allow {
            s.set_allow(cell(a.x, a.y)?, a.colours);
        }
        for d in &self.deny {
            s.set_deny(cell(d.x, d.y)?, d.colours);
        }
        Ok(s)
    }
}

/// How a saved project refers to its layers' sources.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Embed {
    /// Each layer as it was loaded: linked stays linked, embedded stays embedded.
    #[default]
    Keep,
    /// Every source embedded, so the project file stands alone.
    All,
    /// Every source linked; fails for a source that has no file.
    None,
}

/// The error for a `schema_version` this build does not read.
fn unsupported(n: impl std::fmt::Display) -> ProjectError {
    invalid(format!(
        "project schema_version {n} is not supported (this build reads \
         {OLDEST_SCHEMA_VERSION}–{SCHEMA_VERSION}, and version-1 recipe files)"
    ))
}

impl ProjectFile {
    /// A project of one PNG layer scaled to fill the 320 × 200 canvas, the default
    /// recipe and the default palette.
    pub fn from_png(target: Mode, name: &str, path: &Path, size: (u32, u32)) -> Self {
        let (w, h) = size;
        let transform = Transform::new(
            convert64_core::Vec2::ZERO,
            convert64_core::Vec2::new(320.0 / w as f32, 200.0 / h as f32),
            false,
            false,
        )
        .expect("positive finite scale");
        Self {
            schema_version: SCHEMA_VERSION,
            target,
            supersample: default_supersample(),
            palette: PaletteSpec::default(),
            layers: vec![LayerEntry {
                name: name.to_owned(),
                source: SourceSpec::Linked(Linked {
                    path: path.to_owned(),
                }),
                visible: true,
                transform,
                opacity: 1.0,
                blend: Blend::default(),
                downscale: Downscale::Auto,
                resample: Resample::Mitchell,
                recipe: Recipe::default(),
                adjustments: Adjustments::default(),
                mask: None,
            }],
            regions: Vec::new(),
            steering: SteeringSpec::default(),
            solve: SolveSpec::default(),
            snap: Snap::Off,
            commands: Vec::new(),
            migrated_from: None,
        }
    }

    /// A version-1 recipe file as a project: linked sources, the default palette, and
    /// the recipe's commands to run after loading.
    pub fn from_recipe(r: RecipeFile) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            target: r.target,
            supersample: r.supersample,
            palette: PaletteSpec::default(),
            layers: r
                .layers
                .into_iter()
                .map(|l| LayerEntry {
                    name: l.name,
                    source: SourceSpec::Path(l.source),
                    visible: true,
                    transform: l.transform,
                    opacity: l.opacity,
                    blend: l.blend,
                    downscale: Downscale::Auto,
                    resample: Resample::Mitchell,
                    recipe: l.recipe,
                    adjustments: l.adjustments,
                    mask: None,
                })
                .collect(),
            regions: Vec::new(),
            steering: SteeringSpec::default(),
            solve: r.solve,
            snap: Snap::Off,
            commands: r.commands,
            migrated_from: None,
        }
    }

    /// Brings a file of an older `schema_version` up to [`SCHEMA_VERSION`] and notes the
    /// version it had in [`Self::migrated_from`]; a current file is left as it is.
    /// [`Self::parse`] and [`Session::load`](crate::Session::load) call it.
    ///
    /// 2 → 3 (#236): `supersample` becomes 1, whatever it was. Version-2 files were saved
    /// while the default was 4, so a stored 4 meant "the default"; nothing is released
    /// yet, and one look for every project is simpler than a setting. Version-1 recipe
    /// files are not migrated: they are written by hand, and write `supersample` only
    /// when they mean it.
    ///
    /// # Errors
    /// A `schema_version` this build does not read.
    pub fn migrate(&mut self) -> Result<(), ProjectError> {
        let from = self.schema_version;
        if !(OLDEST_SCHEMA_VERSION..=SCHEMA_VERSION).contains(&from) {
            return Err(unsupported(from));
        }
        if from < 3 {
            // ADR-012's 1× resampling for every project.
            self.supersample = default_supersample();
        }
        if from < SCHEMA_VERSION {
            self.schema_version = SCHEMA_VERSION;
            self.migrated_from = Some(from);
        }
        Ok(())
    }

    /// What loading changed, to tell the user once (the CLI's `notes`, the app's status
    /// bar): `Some` for a file [`Self::migrate`] updated.
    pub fn migration_note(&self) -> Option<String> {
        self.migrated_from
            .map(|v| format!("Updated project from v{v}: now uses 1× resampling"))
    }

    /// Parses a project file (migrated with [`Self::migrate`] if older), or a version-1
    /// recipe file (mapped with [`Self::from_recipe`]).
    pub fn parse(text: &str) -> Result<Self, ProjectError> {
        let json = |e: serde_json::Error| ProjectError::Json(e.to_string());
        let v: serde_json::Value = serde_json::from_str(text).map_err(json)?;
        if let Some(n) = v.get("schema_version") {
            let readable = |n: &u64| {
                (u64::from(OLDEST_SCHEMA_VERSION)..=u64::from(SCHEMA_VERSION)).contains(n)
            };
            if !n.as_u64().is_some_and(|n| readable(&n)) {
                return Err(unsupported(n));
            }
            let mut file: Self = serde_json::from_value(v).map_err(json)?;
            file.migrate()?;
            Ok(file)
        } else if v.get("version").is_some() {
            let r: RecipeFile = serde_json::from_value(v).map_err(json)?;
            if r.version != crate::recipe::VERSION {
                return Err(invalid(format!(
                    "recipe version {} is not supported (this build reads {})",
                    r.version,
                    crate::recipe::VERSION
                )));
            }
            Ok(Self::from_recipe(r))
        } else {
            Err(invalid(
                "neither a project file (\"schema_version\") nor a recipe file (\"version\")",
            ))
        }
    }

    /// Reads and parses a project or recipe file from `store`.
    pub fn read_from(store: &dyn Store, path: &Path) -> Result<Self, ProjectError> {
        let bytes = store.read(path)?;
        let text = String::from_utf8(bytes)
            .map_err(|e| ProjectError::Io(path.to_owned(), e.to_string()))?;
        Self::parse(&text)
    }

    /// Reads and parses a project or recipe file.
    #[cfg(feature = "fs")]
    pub fn read(path: &Path) -> Result<Self, ProjectError> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| ProjectError::Io(path.to_owned(), e.to_string()))?;
        Self::parse(&text)
    }

    /// The project as pretty-printed JSON with a trailing newline.
    pub fn to_json(&self) -> String {
        let mut s = serde_json::to_string_pretty(self)
            .unwrap_or_else(|e| unreachable!("plain data serialises: {e}"));
        s.push('\n');
        s
    }
}

/// The raster for a [`RasterSpec`]: linked paths are relative to `base`, read from
/// `store`; a fill is `size` large.
pub fn load_raster(
    spec: &RasterSpec,
    base: &Path,
    store: &dyn Store,
    size: (u32, u32),
    what: &str,
) -> Result<MaskRaster, ProjectError> {
    let raster = match spec {
        RasterSpec::Fill(f) => MaskRaster::filled(size.0, size.1, f.fill)
            .map_err(|e| invalid(format!("{what}: {e}")))?,
        RasterSpec::Linked(l) => {
            let path = base.join(&l.path);
            let bytes = store.read(&path)?;
            decode_mask(&bytes, &path.display().to_string())?
        }
        RasterSpec::Embedded(e) => {
            let bytes =
                base64::decode(&e.png_base64).map_err(|e| invalid(format!("{what}: {e}")))?;
            decode_mask(&bytes, what)?
        }
    };
    if raster.size() != size {
        return Err(invalid(format!(
            "{what}: the mask is {}×{}, it must be {}×{}",
            raster.width(),
            raster.height(),
            size.0,
            size.1
        )));
    }
    Ok(raster)
}

/// A raster as data: one value when every value is the same, else an embedded 16-bit PNG.
pub fn raster_spec(raster: &MaskRaster) -> RasterSpec {
    let v = raster.values();
    match v.first() {
        Some(&first) if v.iter().all(|&x| x.to_bits() == first.to_bits()) => {
            RasterSpec::Fill(Fill { fill: first })
        }
        _ => RasterSpec::Embedded(Embedded {
            png_base64: base64::encode(&encode_mask(raster)),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::MemoryStore;

    #[test]
    fn palettes_by_name_and_by_colours() {
        for (arg, name) in [
            ("pepto", "Pepto (PAL)"),
            ("pepto-pal", "Pepto (PAL)"),
            ("pepto-ntsc", "Pepto (NTSC)"),
            ("colodore", "Colodore"),
        ] {
            let p = PaletteSpec::from_arg(arg).unwrap();
            assert_eq!(
                p.resolve(Path::new("."), &MemoryStore::new())
                    .unwrap()
                    .name(),
                name
            );
        }
        assert!(PaletteSpec::from_arg("vice").is_err());
        assert_eq!(
            PaletteSpec::from_arg("x/my.VPL").unwrap(),
            PaletteSpec::Vpl("x/my.VPL".into())
        );
        let pepto = Palette::pepto_pal();
        let custom =
            PaletteSpec::Custom(pepto.rgb_table().iter().map(ToString::to_string).collect());
        let json = serde_json::to_string(&custom).unwrap();
        assert!(
            json.starts_with(r##"{"custom":["#000000","#FFFFFF""##),
            "{json}"
        );
        let p = serde_json::from_str::<PaletteSpec>(&json)
            .unwrap()
            .resolve(Path::new("."), &MemoryStore::new())
            .unwrap();
        assert_eq!(p.rgb_table(), pepto.rgb_table());
        let short = PaletteSpec::Custom(vec!["#000000".into()]);
        assert!(short.resolve(Path::new("."), &MemoryStore::new()).is_err());
        let bad = PaletteSpec::Custom(vec!["#00000G".into(); 16]);
        assert!(bad.resolve(Path::new("."), &MemoryStore::new()).is_err());
    }

    #[test]
    fn steering_round_trips_and_is_checked() {
        let text = r#"{ "pins": [{ "x": 3, "y": 4, "colours": [2, 1], "full": true }],
                        "locks": [{ "x": 0, "y": 0 }, { "x": 39, "y": 24 }],
                        "allow": [{ "x": 5, "y": 5, "colours": [0, 1, 2] }],
                        "deny": [{ "x": 6, "y": 5, "colours": [7] }] }"#;
        let spec: SteeringSpec = serde_json::from_str(text).unwrap();
        let s = spec.to_steering().unwrap();
        assert!(s.pin(CellPos::new(3, 4)).unwrap().full);
        assert!(s.locks().contains(CellPos::new(39, 24)));
        assert_eq!(SteeringSpec::from_steering(&s), spec);
        let off: SteeringSpec =
            serde_json::from_str(r#"{ "locks": [{ "x": 40, "y": 0 }] }"#).unwrap();
        assert!(off.to_steering().is_err());
        let five: SteeringSpec =
            serde_json::from_str(r#"{ "pins": [{ "x": 1, "y": 1, "colours": [0, 1, 2, 3, 4] }] }"#)
                .unwrap();
        assert!(five.to_steering().is_err());
    }

    #[test]
    fn sources_parse_in_every_form() {
        for (text, want) in [
            (r#""a.png""#, SourceSpec::Path("a.png".into())),
            (
                r#"{"path":"a.png"}"#,
                SourceSpec::Linked(Linked {
                    path: "a.png".into(),
                }),
            ),
            (
                r#"{"png_base64":"AAAA"}"#,
                SourceSpec::Embedded(Embedded {
                    png_base64: "AAAA".into(),
                }),
            ),
            (r#"{"layer":0}"#, SourceSpec::Shared(Shared { layer: 0 })),
        ] {
            assert_eq!(serde_json::from_str::<SourceSpec>(text).unwrap(), want);
        }
        assert!(serde_json::from_str::<SourceSpec>(r#"{"url":"x"}"#).is_err());
    }

    #[test]
    fn masks_survive_png_encoding() {
        let vals: Vec<f32> = (0..12).map(|i| i as f32 / 11.0).collect();
        let r = MaskRaster::from_values(4, 3, vals).unwrap();
        let back = decode_mask(&encode_mask(&r), "mask").unwrap();
        assert_eq!(back.to_u16(), r.to_u16());
        // A second trip is exact: the values are now 16-bit.
        assert_eq!(decode_mask(&encode_mask(&back), "mask").unwrap(), back);
        let flat = MaskRaster::filled(4, 3, 0.3).unwrap();
        assert_eq!(raster_spec(&flat), RasterSpec::Fill(Fill { fill: 0.3 }));
        assert!(matches!(raster_spec(&r), RasterSpec::Embedded(_)));
    }

    #[test]
    fn version_detection() {
        assert!(ProjectFile::parse(r#"{"target":"hires-bitmap","layers":[]}"#).is_err());
        for unknown in [1, 4] {
            let text =
                format!(r#"{{"schema_version":{unknown},"target":"hires-bitmap","layers":[]}}"#);
            let e = ProjectFile::parse(&text).unwrap_err().to_string();
            assert!(e.contains(&format!("schema_version {unknown}")), "{e}");
        }
        let v1 = r#"{"version":1,"target":"hires-bitmap","supersample":2,
            "layers":[{"name":"a","source":"a.png"}]}"#;
        let p = ProjectFile::parse(v1).unwrap();
        assert_eq!(p.schema_version, SCHEMA_VERSION);
        assert_eq!(p.layers[0].source, SourceSpec::Path("a.png".into()));
        assert_eq!(p.supersample, 2, "a recipe's factor is kept");
        assert_eq!(p.migration_note(), None);
        let v2 = r#"{"schema_version":2,"target":"hires-bitmap","layers":[],"surprise":1}"#;
        assert!(ProjectFile::parse(v2).is_err(), "unknown fields are errors");
        let v3 = r#"{"schema_version":3,"target":"hires-bitmap","layers":[],"migrated_from":2}"#;
        assert!(
            ProjectFile::parse(v3).is_err(),
            "migrated_from is never read"
        );
    }

    /// #236: a version-2 file's `supersample` (4, the default it was saved with) becomes
    /// 1, and the file says so once; a version-3 file loads as written.
    #[test]
    fn version_2_files_migrate_to_1x() {
        for (stored, field) in [
            (4, r#""supersample":4,"#),
            (2, r#""supersample":2,"#),
            (4, ""),
        ] {
            let text =
                format!(r#"{{"schema_version":2,"target":"hires-bitmap",{field}"layers":[]}}"#);
            let p = ProjectFile::parse(&text).unwrap();
            assert_eq!(p.supersample, 1, "v2 with {stored}");
            assert_eq!(p.schema_version, SCHEMA_VERSION);
            assert_eq!(p.migrated_from, Some(2));
            assert_eq!(
                p.migration_note().as_deref(),
                Some("Updated project from v2: now uses 1× resampling")
            );
            let saved = p.to_json();
            assert!(saved.contains(r#""schema_version": 3"#), "{saved}");
            assert!(saved.contains(r#""supersample": 1"#), "{saved}");
            assert!(!saved.contains("migrated"), "{saved}");
            let again = ProjectFile::parse(&saved).unwrap();
            assert_eq!(again.migration_note(), None, "reported once");
        }
        let v3 = r#"{"schema_version":3,"target":"hires-bitmap","supersample":4,"layers":[]}"#;
        let p = ProjectFile::parse(v3).unwrap();
        assert_eq!((p.supersample, p.migrated_from), (4, None));
        assert_eq!(p.migration_note(), None);
        // A file built in memory with an old version migrates the same way.
        let mut q = p.clone();
        q.schema_version = 2;
        q.migrate().unwrap();
        assert_eq!((q.supersample, q.migrated_from), (1, Some(2)));
        q.schema_version = 9;
        assert!(q.migrate().is_err());
    }

    /// #198: the snap setting and a layer's rotation and anchor are optional, written
    /// only when set, and a file without them reads and writes exactly as before.
    #[test]
    fn snap_rotate_and_anchor_are_backward_compatible() {
        let old = r#"{"schema_version":2,"target":"hires-bitmap","layers":[
            {"name":"a","source":"a.png","transform":{"translate":{"x":1.5,"y":2.0},
             "scale":{"x":2.0,"y":2.0},"flip_h":true,"flip_v":false}}]}"#;
        let p = ProjectFile::parse(old).unwrap();
        assert_eq!(p.snap, Snap::Off);
        let t = p.layers[0].transform;
        assert_eq!((t.rotate(), t.anchor()), (0.0, Transform::CENTRE));
        let text = p.to_json();
        for absent in ["snap", "rotate", "anchor"] {
            assert!(!text.contains(absent), "{absent} written: {text}");
        }
        let mut q = p.clone();
        // A saved file is current, so it says nothing about a migration.
        q.migrated_from = None;
        q.snap = Snap::C64Pixel;
        q.layers[0].transform = t
            .with_rotation(-7.5)
            .unwrap()
            .with_anchor(convert64_core::Vec2::new(0.0, 1.0))
            .unwrap();
        let text = q.to_json();
        assert!(text.contains(r#""snap": "c64-pixel""#), "{text}");
        assert!(text.contains(r#""rotate": -7.5"#), "{text}");
        assert_eq!(ProjectFile::parse(&text).unwrap(), q);
        let bad = r#"{"schema_version":2,"target":"hires-bitmap","layers":[],"snap":"grid"}"#;
        assert!(ProjectFile::parse(bad).is_err());
    }

    /// #235: a layer's `downscale` is written only when it is not `auto`, so files
    /// without it read and write exactly as before.
    #[test]
    fn downscale_is_written_only_when_exact() {
        let old = r#"{"schema_version":3,"target":"hires-bitmap","layers":[
            {"name":"a","source":"a.png"}]}"#;
        let p = ProjectFile::parse(old).unwrap();
        assert_eq!(p.layers[0].downscale, Downscale::Auto);
        assert!(!p.to_json().contains("downscale"));
        let mut q = p.clone();
        q.layers[0].downscale = Downscale::Exact;
        let text = q.to_json();
        assert!(text.contains(r#""downscale": "exact""#), "{text}");
        assert_eq!(ProjectFile::parse(&text).unwrap(), q);
        let bad = old.replace(r#""a.png"}"#, r#""a.png","downscale":"area"}"#);
        assert!(ProjectFile::parse(&bad).is_err());
    }

    /// ADR-014 §3: a layer's `resample` is written only when `lanczos`, and a black
    /// point only when on; both round-trip, and the old `png2c64` solver name loads as
    /// the reference.
    #[test]
    fn resample_and_black_point_are_written_only_when_set() {
        use convert64_core::layer::BlackPoint;
        use convert64_core::solve::SolverKind;
        let old = r#"{"schema_version":3,"target":"hires-bitmap","layers":[
            {"name":"a","source":"a.png"}]}"#;
        let p = ProjectFile::parse(old).unwrap();
        assert_eq!(p.layers[0].resample, Resample::Mitchell);
        let text = p.to_json();
        assert!(
            !text.contains("resample") && !text.contains("black_point"),
            "{text}"
        );
        let mut q = p.clone();
        q.layers[0].resample = Resample::Lanczos;
        q.layers[0].adjustments.black_point = BlackPoint::new(0.5).unwrap();
        let text = q.to_json();
        assert!(text.contains(r#""resample": "lanczos""#), "{text}");
        assert!(text.contains(r#""black_point": 0.5"#), "{text}");
        assert_eq!(ProjectFile::parse(&text).unwrap(), q);
        let bad = old.replace(r#""a.png"}"#, r#""a.png","resample":"bicubic"}"#);
        assert!(ProjectFile::parse(&bad).is_err());
        let named = old.replace(r#""layers""#, r#""solve":{"solver":"png2c64"},"layers""#);
        assert_eq!(
            ProjectFile::parse(&named).unwrap().solve.solver,
            SolverKind::Reference
        );
    }

    /// #273: `solve.background_rule` is written only when not `black` (the default), and
    /// round-trips.
    #[test]
    fn the_background_rule_is_written_only_when_search() {
        use convert64_core::solve::BackgroundRule;
        let old = r#"{"schema_version":3,"target":"multicolor-bitmap","layers":[]}"#;
        let p = ProjectFile::parse(old).unwrap();
        assert_eq!(p.solve.background_rule, BackgroundRule::Black);
        let text = p.to_json();
        assert!(!text.contains("background_rule"), "{text}");
        let mut q = p.clone();
        q.migrated_from = None;
        q.solve.background_rule = BackgroundRule::Search;
        let text = q.to_json();
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["solve"]["background_rule"], "search", "{text}");
        assert_eq!(ProjectFile::parse(&text).unwrap(), q);
    }

    /// #261: `solve.refine` is written only when on; a hull `match_range` is written
    /// with its palette, and its `amount` only when not 1; an older box loads as a box;
    /// all of them round-trip.
    #[test]
    fn refine_and_hull_range_matching_round_trip() {
        use convert64_core::layer::{RangeMatch, RangeMethod};
        use convert64_core::solve::SolverKind;
        let old = r#"{"schema_version":3,"target":"multicolor-bitmap","layers":[
            {"name":"a","source":"a.png","adjustments":{"match_range":
             {"percentile":0.01,"l":[0.1,0.9],"a":[-0.2,0.2],"b":[-0.25,0.25]}}}]}"#;
        let p = ProjectFile::parse(old).unwrap();
        assert!(!p.solve.refine);
        let m = p.layers[0].adjustments.match_range.expect("a box");
        assert_eq!(m.method(), RangeMethod::Box);
        let text = p.to_json();
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert!(v["solve"].get("refine").is_none(), "{text}");
        let m = &v["layers"][0]["adjustments"]["match_range"];
        assert!(m.get("method").is_none() && m["l"].is_array(), "{text}");
        assert_eq!(ProjectFile::parse(&text).unwrap(), p);

        let mut q = p.clone();
        q.migrated_from = None;
        q.solve.refine = true;
        q.solve.solver = SolverKind::Png2amiga;
        let pal = convert64_core::Palette::pepto_pal();
        q.layers[0].adjustments.match_range = Some(RangeMatch::hull(&pal, 0.4).unwrap());
        let text = q.to_json();
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["solve"]["refine"], true, "{text}");
        assert_eq!(v["solve"]["solver"], "png2amiga", "{text}");
        let m = &v["layers"][0]["adjustments"]["match_range"];
        assert_eq!(m["method"], "hull", "{text}");
        assert_eq!(m["palette"].as_array().unwrap().len(), 16, "{text}");
        assert_eq!(m["palette"][0], "#000000", "{text}");
        assert!((m["amount"].as_f64().unwrap() - 0.4).abs() < 1e-6, "{text}");
        assert!(m.get("l").is_none(), "{text}");
        let back = ProjectFile::parse(&text).unwrap();
        assert_eq!(back, q);
        let m = back.layers[0].adjustments.match_range.unwrap();
        assert_eq!((m.method(), m.amount()), (RangeMethod::Hull, 0.4));

        // At amount 1 (png2amiga's), no `amount` is written.
        q.layers[0].adjustments.match_range = Some(RangeMatch::hull(&pal, 1.0).unwrap());
        let text = q.to_json();
        assert!(!text.contains("amount"), "{text}");
        assert_eq!(ProjectFile::parse(&text).unwrap(), q);
        let bad = text.replace(r##""#000000","##, "");
        assert!(ProjectFile::parse(&bad).is_err(), "15 colours");
    }
}
