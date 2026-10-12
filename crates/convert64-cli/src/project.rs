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
//! Version-1 recipe files (`crate::recipe::RecipeFile`, `"version": 1`) are read too:
//! [`ProjectFile::from_recipe`] maps one onto a project, so every recipe opens and saves
//! as a project. The schema is documented in `docs/reference/project-file.md`.
//!
//! ```json
//! {
//!   "schema_version": 2,
//!   "target": "multicolor-bitmap",
//!   "palette": "pepto-pal",
//!   "layers": [{ "name": "title", "source": { "path": "title.png" },
//!                "recipe": { "method": { "kind": "bayer4" }, "strength": 0.8 } }]
//! }
//! ```

use crate::base64;
use crate::recipe::{JsonCommand, RecipeError, RecipeFile, SolveSpec};
use convert64_core::colour::ColourSet;
use convert64_core::layer::{Adjustments, Blend, Recipe, SourceImage, Transform};
use convert64_core::mask::{MaskRaster, Modifiers, SourceEntry};
use convert64_core::mode::Mode;
use convert64_core::model::CellPos;
use convert64_core::palette::{Palette, Rgb8, linear_to_srgb8};
use convert64_core::solve::steering::{Pin, Steering};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};

/// The project-file schema version this build reads and writes. Version 1 is the recipe
/// file, which has `"version"` instead of `"schema_version"`.
pub const SCHEMA_VERSION: u32 = 2;

/// The project file's extension.
pub const EXTENSION: &str = "convert64";

/// A `.convert64` project file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectFile {
    /// Must be [`SCHEMA_VERSION`].
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
    /// Registry commands run in order after loading (a recipe file's commands).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<JsonCommand>,
}

const fn default_supersample() -> u8 {
    convert64_core::SuperGrid::DEFAULT_N
}

const fn one() -> f32 {
    1.0
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
    /// The palette, reading a `.vpl` file relative to `base`.
    pub fn resolve(&self, base: &Path) -> Result<Palette, RecipeError> {
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
                let bytes = std::fs::read(&path)
                    .map_err(|e| RecipeError::Io(path.clone(), e.to_string()))?;
                Palette::from_vpl(&bytes)
                    .map_err(|e| invalid(format!("{}: {e}", path.display())))?
            }
        })
    }

    /// The preset or file named on a command line: `pepto`, `pepto-pal`, `pepto-ntsc`,
    /// `colodore`, or a path ending in `.vpl`.
    pub fn from_arg(arg: &str) -> Result<Self, RecipeError> {
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

fn parse_hex(text: &str) -> Result<Rgb8, RecipeError> {
    let hex = text.strip_prefix('#').unwrap_or(text);
    if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(invalid(format!("{text:?} is not a #RRGGBB colour")));
    }
    let v = u32::from_str_radix(hex, 16).map_err(invalid)?;
    Ok(Rgb8::from_hex(v))
}

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
    pub fn to_steering(&self) -> Result<Steering, RecipeError> {
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
                recipe: Recipe::default(),
                adjustments: Adjustments::default(),
                mask: None,
            }],
            regions: Vec::new(),
            steering: SteeringSpec::default(),
            solve: SolveSpec::default(),
            commands: Vec::new(),
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
                    recipe: l.recipe,
                    adjustments: l.adjustments,
                    mask: None,
                })
                .collect(),
            regions: Vec::new(),
            steering: SteeringSpec::default(),
            solve: r.solve,
            commands: r.commands,
        }
    }

    /// Parses a project file, or a version-1 recipe file (migrated with
    /// [`Self::from_recipe`]).
    pub fn parse(text: &str) -> Result<Self, RecipeError> {
        let json = |e: serde_json::Error| RecipeError::Json(e.to_string());
        let v: serde_json::Value = serde_json::from_str(text).map_err(json)?;
        if let Some(n) = v.get("schema_version") {
            if n.as_u64() != Some(u64::from(SCHEMA_VERSION)) {
                return Err(invalid(format!(
                    "project schema_version {n} is not supported (this build reads \
                     {SCHEMA_VERSION}, and version-1 recipe files)"
                )));
            }
            serde_json::from_value(v).map_err(json)
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

    /// Reads and parses a project or recipe file.
    pub fn read(path: &Path) -> Result<Self, RecipeError> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| RecipeError::Io(path.to_owned(), e.to_string()))?;
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

pub(crate) fn invalid(e: impl std::fmt::Display) -> RecipeError {
    RecipeError::Invalid(e.to_string())
}

/// `p` made absolute against the current directory and normalised lexically (`.` and
/// `..` removed), without touching the file system.
pub fn absolute(p: &Path) -> PathBuf {
    let joined = if p.is_absolute() {
        p.to_owned()
    } else {
        std::env::current_dir().unwrap_or_default().join(p)
    };
    let mut out = PathBuf::new();
    for c in joined.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push(c);
                }
            }
            _ => out.push(c),
        }
    }
    out
}

/// `target` relative to the directory `dir`, `/`-separated so project files move
/// between systems; `target` itself when they share no root (another Windows drive).
pub fn relative(dir: &Path, target: &Path) -> PathBuf {
    let (dir, target) = (absolute(dir), absolute(target));
    let a: Vec<_> = dir.components().collect();
    let b: Vec<_> = target.components().collect();
    let common = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    if common == 0 || (common == 1 && matches!(a[0], Component::Prefix(_))) {
        return target;
    }
    let parts: Vec<String> = std::iter::repeat_n("..".to_owned(), a.len() - common)
        .chain(
            b[common..]
                .iter()
                .map(|c| c.as_os_str().to_string_lossy().into_owned()),
        )
        .collect();
    PathBuf::from(parts.join("/"))
}

/// Decodes PNG (or other supported image) bytes into a layer source.
pub fn decode_source(bytes: &[u8], what: &str) -> Result<SourceImage, RecipeError> {
    let img = image::load_from_memory(bytes)
        .map_err(|e| invalid(format!("{what}: {e}")))?
        .to_rgba8();
    let (w, h) = img.dimensions();
    SourceImage::from_srgba8(w, h, img.as_raw()).map_err(|e| invalid(format!("{what}: {e}")))
}

/// Decodes greyscale PNG bytes (any bit depth; colour becomes luma) into a mask raster.
pub fn decode_mask(bytes: &[u8], what: &str) -> Result<MaskRaster, RecipeError> {
    let img = image::load_from_memory(bytes)
        .map_err(|e| invalid(format!("{what}: {e}")))?
        .to_luma16();
    let (w, h) = img.dimensions();
    MaskRaster::from_u16(w, h, img.as_raw()).map_err(|e| invalid(format!("{what}: {e}")))
}

fn png_bytes(img: image::DynamicImage) -> Vec<u8> {
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png)
        .unwrap_or_else(|e| unreachable!("PNG encoding into memory: {e}"));
    out.into_inner()
}

/// A mask raster as a 16-bit greyscale PNG (its on-disk form, ADR-008).
pub fn encode_mask(raster: &MaskRaster) -> Vec<u8> {
    let (w, h) = raster.size();
    let img = image::ImageBuffer::<image::Luma<u16>, _>::from_raw(w, h, raster.to_u16())
        .expect("sized by the raster");
    png_bytes(image::DynamicImage::ImageLuma16(img))
}

/// A layer source as an 8-bit sRGB RGBA PNG (for sources with no original file).
pub fn encode_source(src: &SourceImage) -> Vec<u8> {
    let (w, h) = src.size();
    let rgba = src
        .pixels()
        .iter()
        .flat_map(|p| {
            [
                linear_to_srgb8(p[0]),
                linear_to_srgb8(p[1]),
                linear_to_srgb8(p[2]),
                (p[3].clamp(0.0, 1.0) * 255.0).round() as u8,
            ]
        })
        .collect();
    let img = image::RgbaImage::from_raw(w, h, rgba).expect("sized by the source");
    png_bytes(image::DynamicImage::ImageRgba8(img))
}

/// The raster for a [`RasterSpec`]: linked paths are relative to `base`; a fill is
/// `size` large.
pub fn load_raster(
    spec: &RasterSpec,
    base: &Path,
    size: (u32, u32),
    what: &str,
) -> Result<MaskRaster, RecipeError> {
    let raster = match spec {
        RasterSpec::Fill(f) => MaskRaster::filled(size.0, size.1, f.fill)
            .map_err(|e| invalid(format!("{what}: {e}")))?,
        RasterSpec::Linked(l) => {
            let path = base.join(&l.path);
            let bytes =
                std::fs::read(&path).map_err(|e| RecipeError::Io(path.clone(), e.to_string()))?;
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

    #[test]
    fn relative_paths_walk_up_and_down() {
        let root = absolute(Path::new("/proj"));
        assert_eq!(
            relative(&root.join("art"), &root.join("src/title.png")),
            PathBuf::from("../src/title.png")
        );
        assert_eq!(
            relative(&root, &root.join("a/b.png")),
            PathBuf::from("a/b.png")
        );
        assert_eq!(
            relative(&root.join("x/./y/.."), &root.join("x/c.png")),
            PathBuf::from("c.png")
        );
    }

    #[test]
    fn palettes_by_name_and_by_colours() {
        for (arg, name) in [
            ("pepto", "Pepto (PAL)"),
            ("pepto-pal", "Pepto (PAL)"),
            ("pepto-ntsc", "Pepto (NTSC)"),
            ("colodore", "Colodore"),
        ] {
            let p = PaletteSpec::from_arg(arg).unwrap();
            assert_eq!(p.resolve(Path::new(".")).unwrap().name(), name);
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
            .resolve(Path::new("."))
            .unwrap();
        assert_eq!(p.rgb_table(), pepto.rgb_table());
        let short = PaletteSpec::Custom(vec!["#000000".into()]);
        assert!(short.resolve(Path::new(".")).is_err());
        let bad = PaletteSpec::Custom(vec!["#00000G".into(); 16]);
        assert!(bad.resolve(Path::new(".")).is_err());
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
        let newer = r#"{"schema_version":3,"target":"hires-bitmap","layers":[]}"#;
        assert!(
            ProjectFile::parse(newer)
                .unwrap_err()
                .to_string()
                .contains("schema_version 3")
        );
        let v1 =
            r#"{"version":1,"target":"hires-bitmap","layers":[{"name":"a","source":"a.png"}]}"#;
        let p = ProjectFile::parse(v1).unwrap();
        assert_eq!(p.schema_version, SCHEMA_VERSION);
        assert_eq!(p.layers[0].source, SourceSpec::Path("a.png".into()));
        let v2 = r#"{"schema_version":2,"target":"hires-bitmap","layers":[],"surprise":1}"#;
        assert!(ProjectFile::parse(v2).is_err(), "unknown fields are errors");
    }
}
