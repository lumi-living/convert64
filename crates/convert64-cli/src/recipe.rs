//! Recipe files: a layered scene plus a list of commands, in JSON (ADR-005).
//!
//! `convert64 recipe apply` and the MCP server both run recipes through [`Session`], which
//! drives the core's [`Editor`] and [`Compositor`] exactly as the app does, so a recipe
//! gives the same result through every front-end. JSON and file paths live here, not in
//! `convert64-core`, which does no I/O.
//!
//! A [`Session`] also opens and saves `.convert64` project files ([`crate::project`],
//! ADR-009); a recipe file is read as a project whose commands run after loading.
//!
//! ```json
//! {
//!   "version": 1,
//!   "target": "multicolor-bitmap",
//!   "supersample": 4,
//!   "layers": [{ "name": "photo", "source": "photo.png" }],
//!   "commands": [
//!     { "command": "mask_key", "layer": "photo",
//!       "key": { "channel": "chroma", "centre": [0.0, 1.0, 0.0], "range": 0.1, "softness": 0.05 },
//!       "combine": "max" },
//!     { "command": "mask_set_modifier", "layer": 0, "modifier": { "feather": 3.0 } }
//!   ]
//! }
//! ```

use crate::project::{
    self, Embed, LayerEntry, MaskSpec, PaletteSpec, ProjectFile, RegionEntry, SCHEMA_VERSION,
    SourceSpec, SteeringSpec,
};
use convert64_core::command::{
    CellAllow, CellClear, CellDeny, CellLock, CellPin, Command, Dirty, LayerViaMask, MaskClear,
    MaskFill, MaskFromAlpha, MaskGradient, MaskKey, MaskLoad, MaskSetModifier, MaskStroke,
    ModifierChange, Selection,
};
use convert64_core::geom::Vec2;
use convert64_core::layer::{Adjustments, Blend, Layer, Recipe, SourceImage, Transform};
use convert64_core::mask::{Combine, Gradient, Key, Mask, MaskRaster, PaintMode};
use convert64_core::mode::Mode;
use convert64_core::palette::linear_to_srgb8;
use convert64_core::region::{self, Region};
use convert64_core::solve::{Solution, SolveMode, SolveOptions, Solver};
use convert64_core::{Compositor, Editor, Palette, Project, SuperGrid};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The recipe-file format version this build reads.
pub const VERSION: u32 = 1;

/// A layer by stack index (0 = bottom) or by name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum LayerRef {
    /// Stack index, bottom first.
    Index(usize),
    /// Layer name.
    Name(String),
}

/// One layer of a recipe file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayerSpec {
    /// Display name, also usable as a [`LayerRef`].
    pub name: String,
    /// PNG source, relative to the recipe file.
    pub source: PathBuf,
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
}

const fn one() -> f32 {
    1.0
}

const fn default_supersample() -> u8 {
    SuperGrid::DEFAULT_N
}

/// A recipe file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecipeFile {
    /// Must be [`VERSION`].
    pub version: u32,
    /// Target mode.
    pub target: Mode,
    /// Supersample factor, 1–8.
    #[serde(default = "default_supersample")]
    pub supersample: u8,
    /// Layers, bottom first.
    pub layers: Vec<LayerSpec>,
    /// Commands, run in order.
    #[serde(default)]
    pub commands: Vec<JsonCommand>,
    /// How the picture is solved.
    #[serde(default)]
    pub solve: SolveSpec,
}

/// Solve settings in a recipe or project file (the per-pixel knobs live in each layer's
/// recipe).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SolveSpec {
    /// Background: `auto` (default) or `{"fixed": n}`.
    #[serde(default)]
    pub background: convert64_core::solve::Background,
    /// Refinement hysteresis ratio, e.g. 0.97; absent = off.
    #[serde(default)]
    pub hysteresis: Option<f32>,
    /// Same-luma pair bias per pair (positive penalises, negative rewards).
    #[serde(default)]
    pub luma_pair_bias: f32,
    /// Refinement rounds, 0–2.
    #[serde(default = "default_refinements")]
    pub refinements: u8,
    /// Backgrounds carried from the search into dithering, at least 1.
    #[serde(default = "default_finalists")]
    pub finalists: u8,
}

fn default_refinements() -> u8 {
    SolveOptions::default().refinements
}

fn default_finalists() -> u8 {
    SolveOptions::default().finalists
}

impl Default for SolveSpec {
    fn default() -> Self {
        Self {
            background: convert64_core::solve::Background::default(),
            hysteresis: None,
            luma_pair_bias: 0.0,
            refinements: default_refinements(),
            finalists: default_finalists(),
        }
    }
}

impl SolveSpec {
    /// Checks the values a solve cannot take.
    pub fn check(&self) -> Result<(), RecipeError> {
        if let Some(h) = self.hysteresis
            && !(h > 0.0 && h <= 1.0)
        {
            return Err(invalid(format!(
                "solve: hysteresis must be in (0, 1], got {h}"
            )));
        }
        if !self.luma_pair_bias.is_finite() {
            return Err(invalid("solve: luma_pair_bias must be finite"));
        }
        if self.refinements > 2 {
            return Err(invalid(format!(
                "solve: refinements must be 0–2, got {}",
                self.refinements
            )));
        }
        if self.finalists == 0 || self.finalists > 16 {
            return Err(invalid(format!(
                "solve: finalists must be 1–16, got {}",
                self.finalists
            )));
        }
        Ok(())
    }
}

/// A command as JSON: `{"command": "<name>", ...arguments}`. Names are the core
/// registry's (`convert64_core::command::REGISTRY`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
pub enum JsonCommand {
    /// Soft round brush along a polyline.
    MaskStroke {
        /// Layer.
        layer: LayerRef,
        /// Polyline, layer pixels.
        points: Vec<Vec2>,
        /// Radius, layer pixels.
        radius: f32,
        /// `0..=1`.
        #[serde(default)]
        hardness: f32,
        /// `0..=1`.
        #[serde(default = "one")]
        flow: f32,
        /// `add` or `subtract`.
        #[serde(default)]
        mode: PaintMode,
    },
    /// Polygon (lasso) fill.
    MaskFill {
        /// Layer.
        layer: LayerRef,
        /// Polygon, layer pixels.
        polygon: Vec<Vec2>,
        /// `0..=1`.
        #[serde(default = "one")]
        value: f32,
        /// `add` or `subtract`.
        #[serde(default)]
        mode: PaintMode,
    },
    /// Gradient source.
    MaskGradient {
        /// Layer.
        layer: LayerRef,
        /// The gradient.
        gradient: Gradient,
        /// `max`, `min` or `multiply`.
        #[serde(default)]
        combine: Combine,
    },
    /// Key source.
    MaskKey {
        /// Layer.
        layer: LayerRef,
        /// The key.
        key: Key,
        /// `max`, `min` or `multiply`.
        #[serde(default)]
        combine: Combine,
    },
    /// One modifier: `{"invert": true}`, `{"levels": {...}}`, `{"snap": "expand"}` or
    /// `{"feather": 3.0}`.
    MaskSetModifier {
        /// Layer.
        layer: LayerRef,
        /// The change.
        modifier: ModifierChange,
    },
    /// Greyscale PNG (8 or 16 bit) of the layer's size, relative to the recipe file.
    MaskLoad {
        /// Layer.
        layer: LayerRef,
        /// PNG path.
        path: PathBuf,
    },
    /// Whole raster to one value (0 clears, 1 fills).
    MaskClear {
        /// Layer.
        layer: LayerRef,
        /// `0..=1`.
        #[serde(default)]
        value: f32,
    },
    /// PNG alpha into the mask.
    MaskFromAlpha {
        /// Layer.
        layer: LayerRef,
    },
    /// Pin colours on cells (`{"x": 3, "y": 4}` positions, colours 0–15).
    CellPin(CellPin),
    /// Lock cells (`"locked": false` unlocks).
    CellLock(CellLock),
    /// Clear all steering on cells.
    CellClear(CellClear),
    /// Restrict cells to colours.
    CellAllow(CellAllow),
    /// Forbid colours in cells.
    CellDeny(CellDeny),
    /// New layer sharing the source pixels with a selection as its mask.
    LayerViaMask {
        /// The layer to split.
        source_layer: LayerRef,
        /// The selection.
        selection: Selection,
        /// Remove the selection from the original too.
        #[serde(default)]
        cut: bool,
    },
}

/// Why a recipe could not run.
#[derive(Debug)]
pub enum RecipeError {
    /// Reading or writing a file failed.
    Io(PathBuf, String),
    /// The JSON is malformed or has the wrong shape.
    Json(String),
    /// A value is not allowed.
    Invalid(String),
}

impl std::fmt::Display for RecipeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(p, e) => write!(f, "{}: {e}", p.display()),
            Self::Json(e) => write!(f, "recipe JSON: {e}"),
            Self::Invalid(e) => f.write_str(e),
        }
    }
}

impl std::error::Error for RecipeError {}

fn invalid(e: impl std::fmt::Display) -> RecipeError {
    RecipeError::Invalid(e.to_string())
}

/// Decodes a PNG into a layer source.
pub fn load_source(path: &Path) -> Result<SourceImage, RecipeError> {
    let img = image::open(path)
        .map_err(|e| RecipeError::Io(path.to_owned(), e.to_string()))?
        .to_rgba8();
    let (w, h) = img.dimensions();
    SourceImage::from_srgba8(w, h, img.as_raw()).map_err(invalid)
}

/// Decodes a greyscale PNG (any bit depth; colour is converted to luma) into a mask
/// raster.
pub fn load_mask(path: &Path) -> Result<MaskRaster, RecipeError> {
    let img = image::open(path)
        .map_err(|e| RecipeError::Io(path.to_owned(), e.to_string()))?
        .to_luma16();
    let (w, h) = img.dimensions();
    MaskRaster::from_u16(w, h, img.as_raw()).map_err(invalid)
}

fn save(path: &Path, img: &image::DynamicImage) -> Result<(), RecipeError> {
    img.save(path)
        .map_err(|e| RecipeError::Io(path.to_owned(), e.to_string()))
}

/// Where a layer's pixels came from, so a saved project can link or embed them.
#[derive(Clone, Debug)]
enum Origin {
    /// A file (absolute path) and its bytes as loaded.
    Linked(PathBuf, Arc<Vec<u8>>),
    /// Bytes embedded in the project file.
    Embedded(Arc<Vec<u8>>),
}

/// A project or recipe being edited: the project, its history, the compositor and the
/// solver.
pub struct Session {
    editor: Editor,
    compositor: Compositor,
    base: PathBuf,
    palette_spec: PaletteSpec,
    palette: Palette,
    solver: Solver,
    last: Option<Solution>,
    spec: SolveSpec,
    origins: Vec<(Arc<SourceImage>, Origin)>,
    /// Cells changed since the last solve (from the compositor's dirty-cell API).
    dirty: convert64_core::CellSet,
}

impl Session {
    /// Loads a project file, or a version-1 recipe file, and runs its commands.
    pub fn open(path: &Path) -> Result<Self, RecipeError> {
        let file = ProjectFile::read(path)?;
        let base = path.parent().map(Path::to_owned).unwrap_or_default();
        Self::from_project(file, &base)
    }

    /// Builds a session from a parsed recipe, resolving paths against `base`.
    pub fn from_file(file: RecipeFile, base: &Path) -> Result<Self, RecipeError> {
        if file.version != VERSION {
            return Err(invalid(format!(
                "recipe version {} is not supported (this build reads {VERSION})",
                file.version
            )));
        }
        Self::from_project(ProjectFile::from_recipe(file), base)
    }

    /// Builds a session from a parsed project, resolving paths against `base`, and runs
    /// its commands.
    pub fn from_project(file: ProjectFile, base: &Path) -> Result<Self, RecipeError> {
        if file.schema_version != SCHEMA_VERSION {
            return Err(invalid(format!(
                "project schema_version {} is not supported (this build reads {SCHEMA_VERSION})",
                file.schema_version
            )));
        }
        let grid = SuperGrid::new(file.target, file.supersample).map_err(invalid)?;
        file.solve.check()?;
        let palette_spec = match file.palette {
            PaletteSpec::Vpl(p) => PaletteSpec::Vpl(project::absolute(&base.join(p))),
            p => p,
        };
        let palette = palette_spec.resolve(base)?;
        let mut origins: Vec<(Arc<SourceImage>, Origin)> = Vec::new();
        let mut layers: Vec<Layer> = Vec::with_capacity(file.layers.len());
        for (i, entry) in file.layers.into_iter().enumerate() {
            let what = format!("layer {i} ({:?})", entry.name);
            let src = match entry.source {
                SourceSpec::Path(p) | SourceSpec::Linked(project::Linked { path: p }) => {
                    let path = project::absolute(&base.join(p));
                    let known = origins.iter().find_map(|(s, o)| match o {
                        Origin::Linked(q, _) if *q == path => Some(Arc::clone(s)),
                        _ => None,
                    });
                    if let Some(s) = known {
                        s
                    } else {
                        let bytes = std::fs::read(&path)
                            .map_err(|e| RecipeError::Io(path.clone(), e.to_string()))?;
                        let s =
                            Arc::new(project::decode_source(&bytes, &path.display().to_string())?);
                        origins.push((Arc::clone(&s), Origin::Linked(path, Arc::new(bytes))));
                        s
                    }
                }
                SourceSpec::Embedded(e) => {
                    let bytes = crate::base64::decode(&e.png_base64)
                        .map_err(|e| invalid(format!("{what}: {e}")))?;
                    let s = Arc::new(project::decode_source(&bytes, &what)?);
                    origins.push((Arc::clone(&s), Origin::Embedded(Arc::new(bytes))));
                    s
                }
                SourceSpec::Shared(project::Shared { layer }) => layers
                    .get(layer)
                    .map(|l| Arc::clone(l.source()))
                    .ok_or_else(|| {
                        invalid(format!(
                            "{what}: shares the pixels of layer {layer}, which is not below it"
                        ))
                    })?,
            };
            let size = src.size();
            let mut layer = Layer::new(entry.name, src);
            layer.set_transform(entry.transform);
            layer
                .set_opacity(entry.opacity)
                .map_err(|e| invalid(format!("{what}: {e}")))?;
            layer.set_blend(entry.blend);
            layer.set_visible(entry.visible);
            layer.set_recipe(entry.recipe);
            layer.set_adjustments(entry.adjustments);
            if let Some(m) = entry.mask {
                let raster = project::load_raster(&m.raster, base, size, &format!("{what} mask"))?;
                let mut mask = Mask::new(raster);
                mask.set_sources(m.sources);
                mask.set_modifiers(m.modifiers);
                layer
                    .set_mask(Some(mask))
                    .map_err(|e| invalid(format!("{what}: {e}")))?;
            }
            layers.push(layer);
        }
        let mut regions = Vec::with_capacity(file.regions.len());
        for (i, r) in file.regions.into_iter().enumerate() {
            let what = format!("region {i} ({:?})", r.name);
            let raster = project::load_raster(
                &r.mask,
                base,
                (region::CANVAS_WIDTH, region::CANVAS_HEIGHT),
                &format!("{what} mask"),
            )?;
            regions.push(
                Region::new(r.name, raster, r.recipe)
                    .map_err(|e| invalid(format!("{what}: {e}")))?,
            );
        }
        let steering = file.steering.to_steering()?;
        let mut session = Self {
            editor: Editor::new(Project {
                layers,
                regions,
                steering,
            }),
            compositor: Compositor::new(grid),
            base: base.to_owned(),
            solver: Solver::new(&palette),
            palette_spec,
            palette,
            last: None,
            spec: file.solve,
            origins,
            dirty: convert64_core::CellSet::all(),
        };
        for (i, cmd) in file.commands.into_iter().enumerate() {
            session
                .run(cmd)
                .map_err(|e| invalid(format!("command {i}: {e}")))?;
        }
        Ok(session)
    }

    /// The project as a project file whose paths are relative to `dir`, the directory
    /// it will be saved in. Commands run so far are part of the layers, masks and
    /// steering it holds; it lists no commands.
    pub fn to_project_file(&self, dir: &Path, embed: Embed) -> Result<ProjectFile, RecipeError> {
        let p = self.project();
        let mut layers = Vec::with_capacity(p.layers.len());
        for (i, l) in p.layers.iter().enumerate() {
            let shared = p.layers[..i]
                .iter()
                .position(|k| Arc::ptr_eq(k.source(), l.source()));
            let origin = self
                .origins
                .iter()
                .find(|(s, _)| Arc::ptr_eq(s, l.source()))
                .map(|(_, o)| o);
            let embedded = |bytes: &[u8]| {
                SourceSpec::Embedded(project::Embedded {
                    png_base64: crate::base64::encode(bytes),
                })
            };
            let source = match (shared, origin, embed) {
                (Some(layer), _, _) => SourceSpec::Shared(project::Shared { layer }),
                (None, Some(Origin::Linked(path, _)), Embed::Keep | Embed::None) => {
                    SourceSpec::Linked(project::Linked {
                        path: project::relative(dir, path),
                    })
                }
                (None, _, Embed::None) => {
                    return Err(invalid(format!(
                        "layer {i} ({:?}) has no source file to link; save it embedded",
                        l.name()
                    )));
                }
                (None, Some(Origin::Linked(_, bytes) | Origin::Embedded(bytes)), _) => {
                    embedded(bytes)
                }
                (None, None, _) => embedded(&project::encode_source(l.source())),
            };
            layers.push(LayerEntry {
                name: l.name().to_owned(),
                source,
                visible: l.visible(),
                transform: l.transform(),
                opacity: l.opacity(),
                blend: l.blend(),
                recipe: *l.recipe(),
                adjustments: l.adjustments().clone(),
                mask: l.mask().map(|m| MaskSpec {
                    raster: project::raster_spec(m.raster()),
                    sources: m.sources().to_vec(),
                    modifiers: m.modifiers(),
                }),
            });
        }
        Ok(ProjectFile {
            schema_version: SCHEMA_VERSION,
            target: self.target(),
            supersample: self.compositor.grid().n(),
            palette: match &self.palette_spec {
                PaletteSpec::Vpl(path) => PaletteSpec::Vpl(project::relative(dir, path)),
                other => other.clone(),
            },
            layers,
            regions: p
                .regions
                .iter()
                .map(|r| RegionEntry {
                    name: r.name().to_owned(),
                    mask: project::raster_spec(r.mask()),
                    recipe: *r.recipe(),
                })
                .collect(),
            steering: SteeringSpec::from_steering(&p.steering),
            solve: self.spec.clone(),
            commands: Vec::new(),
        })
    }

    /// Saves the project to `path` (see [`Self::to_project_file`]).
    pub fn save(&self, path: &Path, embed: Embed) -> Result<(), RecipeError> {
        let dir = path.parent().map(Path::to_owned).unwrap_or_default();
        let text = self.to_project_file(&dir, embed)?.to_json();
        std::fs::write(path, text).map_err(|e| RecipeError::Io(path.to_owned(), e.to_string()))
    }

    /// The target mode.
    pub fn target(&self) -> Mode {
        self.compositor.grid().target()
    }

    /// The palette the picture is solved and previewed with.
    pub const fn palette(&self) -> &Palette {
        &self.palette
    }

    /// The solve settings.
    pub const fn solve_spec(&self) -> &SolveSpec {
        &self.spec
    }

    /// The project.
    pub fn project(&self) -> &Project {
        self.editor.project()
    }

    /// The editor (for undo/redo).
    pub fn editor_mut(&mut self) -> &mut Editor {
        &mut self.editor
    }

    /// The layer index a reference names.
    pub fn resolve(&self, r: &LayerRef) -> Result<usize, RecipeError> {
        let layers = &self.project().layers;
        match r {
            LayerRef::Index(i) if *i < layers.len() => Ok(*i),
            LayerRef::Index(i) => Err(invalid(format!("there is no layer {i}"))),
            LayerRef::Name(n) => layers
                .iter()
                .position(|l| l.name() == n)
                .ok_or_else(|| invalid(format!("there is no layer named {n:?}"))),
        }
    }

    /// The core command for a JSON command (reading any file it names, relative to the
    /// project or recipe file).
    pub fn to_command(&self, cmd: JsonCommand) -> Result<Command, RecipeError> {
        self.to_command_at(cmd, &self.base)
    }

    /// The core command for a JSON command, reading any file it names relative to `base`.
    pub fn to_command_at(&self, cmd: JsonCommand, base: &Path) -> Result<Command, RecipeError> {
        Ok(match cmd {
            JsonCommand::MaskStroke {
                layer,
                points,
                radius,
                hardness,
                flow,
                mode,
            } => Command::MaskStroke(MaskStroke {
                layer: self.resolve(&layer)?,
                points,
                radius,
                hardness,
                flow,
                mode,
            }),
            JsonCommand::MaskFill {
                layer,
                polygon,
                value,
                mode,
            } => Command::MaskFill(MaskFill {
                layer: self.resolve(&layer)?,
                polygon,
                value,
                mode,
            }),
            JsonCommand::MaskGradient {
                layer,
                gradient,
                combine,
            } => Command::MaskGradient(MaskGradient {
                layer: self.resolve(&layer)?,
                gradient,
                combine,
            }),
            JsonCommand::MaskKey {
                layer,
                key,
                combine,
            } => Command::MaskKey(MaskKey {
                layer: self.resolve(&layer)?,
                key,
                combine,
            }),
            JsonCommand::MaskSetModifier { layer, modifier } => {
                Command::MaskSetModifier(MaskSetModifier {
                    layer: self.resolve(&layer)?,
                    modifier,
                })
            }
            JsonCommand::MaskLoad { layer, path } => Command::MaskLoad(MaskLoad {
                layer: self.resolve(&layer)?,
                raster: load_mask(&base.join(path))?,
            }),
            JsonCommand::MaskClear { layer, value } => Command::MaskClear(MaskClear {
                layer: self.resolve(&layer)?,
                value,
            }),
            JsonCommand::MaskFromAlpha { layer } => Command::MaskFromAlpha(MaskFromAlpha {
                layer: self.resolve(&layer)?,
            }),
            JsonCommand::CellPin(c) => Command::CellPin(c),
            JsonCommand::CellLock(c) => Command::CellLock(c),
            JsonCommand::CellClear(c) => Command::CellClear(c),
            JsonCommand::CellAllow(c) => Command::CellAllow(c),
            JsonCommand::CellDeny(c) => Command::CellDeny(c),
            JsonCommand::LayerViaMask {
                source_layer,
                selection,
                cut,
            } => Command::LayerViaMask(LayerViaMask {
                source_layer: self.resolve(&source_layer)?,
                selection,
                cut,
            }),
        })
    }

    /// Runs one JSON command.
    pub fn run(&mut self, cmd: JsonCommand) -> Result<Dirty, RecipeError> {
        let c = self.to_command(cmd)?;
        self.editor.execute(c).map_err(invalid)
    }

    /// Runs one JSON command whose file paths are relative to `base` (a commands file's
    /// directory, or the working directory for a command given on the command line).
    pub fn run_at(&mut self, cmd: JsonCommand, base: &Path) -> Result<Dirty, RecipeError> {
        let c = self.to_command_at(cmd, base)?;
        self.editor.execute(c).map_err(invalid)
    }

    fn composite(&mut self) {
        let p = self.editor.project();
        let changed = self.compositor.update(&p.layers, &p.regions);
        self.dirty = self.dirty.union(changed);
    }

    /// Solves the picture: `full` searches backgrounds and refines; otherwise the
    /// interactive fast path re-solves only cells changed since the last solve.
    pub fn solve(&mut self, full: bool) -> &Solution {
        self.composite();
        let p = self.editor.project();
        let mode = if full || self.last.is_none() {
            SolveMode::Full
        } else {
            SolveMode::Interactive { dirty: self.dirty }
        };
        let opts = SolveOptions {
            mode,
            background: self.spec.background,
            hysteresis: self.spec.hysteresis,
            luma_pair_bias: self.spec.luma_pair_bias,
            refinements: self.spec.refinements,
            finalists: self.spec.finalists,
            layer_origins: convert64_core::solve::dither::layer_origins(
                self.compositor.grid(),
                &p.layers,
            ),
            ..SolveOptions::default()
        };
        let sol = self.solver.solve(
            self.compositor.frame(),
            &p.steering,
            self.last.as_ref(),
            &opts,
        );
        self.dirty = convert64_core::CellSet::EMPTY;
        self.last.insert(sol)
    }

    /// The last solution, if any.
    pub fn solution(&self) -> Option<&Solution> {
        self.last.as_ref()
    }

    /// The last solve's clash report as JSON: the status-bar summary and the `worst`
    /// worst cells (wanted, chosen and dropped colours).
    pub fn clash_report_json(&self, worst: usize) -> Option<serde_json::Value> {
        let sol = self.last.as_ref()?;
        let order = sol.report.worst_first();
        let cells: Vec<_> = order
            .iter()
            .take(worst)
            .filter_map(|c| sol.report.cell(*c))
            .collect();
        Some(serde_json::json!({
            "background": sol.background,
            "error": sol.error,
            "summary": sol.report.summary(),
            "worst": cells,
        }))
    }

    /// Layer `layer`'s evaluated mask (sources and modifiers applied) on the whole super
    /// grid, as 16-bit values, row-major; 0 outside the layer. Width and height first.
    pub fn evaluated_mask(&mut self, layer: usize) -> Result<(u32, u32, Vec<u16>), RecipeError> {
        if self
            .project()
            .layers
            .get(layer)
            .and_then(Layer::mask)
            .is_none()
        {
            return Err(invalid(format!("layer {layer} has no mask")));
        }
        self.composite();
        let grid = self.compositor.grid();
        let (w, h) = grid.size();
        let (rect, vals) = self
            .compositor
            .evaluated_mask(layer)
            .ok_or_else(|| invalid(format!("layer {layer} is hidden")))?;
        let mut out = vec![0u16; w as usize * h as usize];
        for y in rect.y0..rect.y1 {
            for x in rect.x0..rect.x1 {
                let v = vals[((y - rect.y0) * rect.width() + x - rect.x0) as usize];
                out[(y * w + x) as usize] = (v.clamp(0.0, 1.0) * 65535.0).round() as u16;
            }
        }
        Ok((w, h, out))
    }

    /// Writes [`Self::evaluated_mask`] as a 16-bit greyscale PNG.
    pub fn write_mask(&mut self, layer: usize, path: &Path) -> Result<(), RecipeError> {
        let (w, h, v) = self.evaluated_mask(layer)?;
        let img =
            image::ImageBuffer::<image::Luma<u16>, _>::from_raw(w, h, v).expect("sized above");
        save(path, &image::DynamicImage::ImageLuma16(img))
    }

    /// The composite on the target grid as RGBA8 (straight sRGB, alpha = coverage).
    pub fn composite_rgba(&mut self) -> (u32, u32, Vec<u8>) {
        self.composite();
        let frame = self.compositor.frame();
        let (w, h) = frame.grid().target_size();
        let rgba = frame
            .colour()
            .iter()
            .zip(frame.coverage())
            .flat_map(|(c, a)| {
                let [r, g, b] = c.map(linear_to_srgb8);
                [r, g, b, (a * 255.0).round() as u8]
            })
            .collect();
        (w, h, rgba)
    }

    /// Writes [`Self::composite_rgba`] as a PNG.
    pub fn write_composite(&mut self, path: &Path) -> Result<(), RecipeError> {
        let (w, h, rgba) = self.composite_rgba();
        let img = image::RgbaImage::from_raw(w, h, rgba).expect("sized above");
        save(path, &image::DynamicImage::ImageRgba8(img))
    }

    /// Writes the attribute map's debug render as a PNG.
    pub fn write_attributes(&mut self, path: &Path) -> Result<(), RecipeError> {
        self.composite();
        let frame = self.compositor.frame();
        let (w, h) = frame.grid().target_size();
        let rgba: Vec<u8> = frame
            .attributes()
            .debug_rgba()
            .into_iter()
            .flatten()
            .collect();
        let img = image::RgbaImage::from_raw(w, h, rgba).expect("sized above");
        save(path, &image::DynamicImage::ImageRgba8(img))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use convert64_core::ColourIndex;
    use convert64_core::layer::Levels;
    use convert64_core::mask::{
        GradientKind, KeyChannel, MaskSource, Modifiers, SnapToCells, SourceEntry,
    };
    use convert64_core::model::CellPos;

    fn every_command() -> Vec<JsonCommand> {
        let l = LayerRef::Name("photo".into());
        vec![
            JsonCommand::MaskStroke {
                layer: l.clone(),
                points: vec![Vec2::new(1.0, 2.0), Vec2::new(3.5, 4.0)],
                radius: 6.0,
                hardness: 0.25,
                flow: 0.5,
                mode: PaintMode::Subtract,
            },
            JsonCommand::MaskFill {
                layer: LayerRef::Index(0),
                polygon: vec![Vec2::ZERO, Vec2::new(5.0, 0.0), Vec2::new(0.0, 5.0)],
                value: 0.75,
                mode: PaintMode::Add,
            },
            JsonCommand::MaskGradient {
                layer: l.clone(),
                gradient: Gradient::new(
                    GradientKind::Radial,
                    Vec2::new(5.0, 5.0),
                    Vec2::new(9.0, 5.0),
                    0.5,
                )
                .unwrap(),
                combine: Combine::Multiply,
            },
            JsonCommand::MaskKey {
                layer: l.clone(),
                key: Key::new(KeyChannel::Chroma, [0.0, 1.0, 0.0], 0.1, 0.05).unwrap(),
                combine: Combine::Min,
            },
            JsonCommand::MaskSetModifier {
                layer: l.clone(),
                modifier: ModifierChange::Feather(3.0),
            },
            JsonCommand::MaskSetModifier {
                layer: l.clone(),
                modifier: ModifierChange::Invert(true),
            },
            JsonCommand::MaskSetModifier {
                layer: l.clone(),
                modifier: ModifierChange::Snap(SnapToCells::Expand),
            },
            JsonCommand::MaskSetModifier {
                layer: l.clone(),
                modifier: ModifierChange::Levels(Levels::new(0.1, 0.9, 1.2).unwrap()),
            },
            JsonCommand::MaskLoad {
                layer: l.clone(),
                path: "mask.png".into(),
            },
            JsonCommand::MaskClear {
                layer: l.clone(),
                value: 1.0,
            },
            JsonCommand::MaskFromAlpha { layer: l.clone() },
            JsonCommand::CellPin(CellPin {
                cells: vec![CellPos::new(3, 4)],
                colours: vec![ColourIndex::RED, ColourIndex::WHITE],
                full: true,
            }),
            JsonCommand::CellLock(CellLock {
                cells: vec![CellPos::new(0, 0), CellPos::new(39, 24)],
                locked: false,
            }),
            JsonCommand::CellClear(CellClear {
                cells: vec![CellPos::new(1, 1)],
            }),
            JsonCommand::CellAllow(CellAllow {
                cells: vec![CellPos::new(2, 2)],
                colours: vec![ColourIndex::BLACK],
            }),
            JsonCommand::CellDeny(CellDeny {
                cells: vec![CellPos::new(2, 2)],
                colours: vec![ColourIndex::YELLOW],
            }),
            JsonCommand::LayerViaMask {
                source_layer: l,
                selection: Selection {
                    polygons: vec![vec![Vec2::ZERO, Vec2::new(4.0, 0.0), Vec2::new(4.0, 4.0)]],
                    sources: vec![SourceEntry {
                        source: MaskSource::Gradient(
                            Gradient::new(GradientKind::Linear, Vec2::ZERO, Vec2::ONE, 1.0)
                                .unwrap(),
                        ),
                        combine: Combine::Max,
                    }],
                    modifiers: Modifiers::feathered(1.0).unwrap(),
                },
                cut: true,
            },
        ]
    }

    #[test]
    fn every_command_round_trips_through_json() {
        let all = every_command();
        let names: std::collections::BTreeSet<String> = all
            .iter()
            .map(|c| {
                serde_json::to_value(c).unwrap()["command"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            })
            .collect();
        let registry: std::collections::BTreeSet<String> = convert64_core::command::REGISTRY
            .iter()
            .map(|c| c.name.to_owned())
            .collect();
        assert_eq!(names, registry, "every registry command has a JSON form");
        for c in all {
            let text = serde_json::to_string(&c).unwrap();
            let back: JsonCommand = serde_json::from_str(&text).unwrap();
            assert_eq!(back, c, "{text}");
        }
    }

    #[test]
    fn recipe_files_carry_solver_knobs() {
        let text = r#"{
            "version": 1, "target": "multicolor-bitmap",
            "layers": [{ "name": "a", "source": "a.png",
                "recipe": { "method": { "kind": "floyd-steinberg" }, "strength": 1.0,
                            "cell_clamp": false, "max_luma_diff": 0, "adaptive": 0.5 } }],
            "solve": { "background": { "fixed": 6 }, "hysteresis": 0.97, "luma_pair_bias": -0.01 }
        }"#;
        let f: RecipeFile = serde_json::from_str(text).unwrap();
        let r = f.layers[0].recipe;
        assert!(!r.cell_clamp() && r.max_luma_diff() == 0 && r.adaptive() == 0.5);
        assert_eq!(f.solve.hysteresis, Some(0.97));
        let back: RecipeFile = serde_json::from_str(&serde_json::to_string(&f).unwrap()).unwrap();
        assert_eq!(back, f);
    }

    #[test]
    fn json_goes_through_the_core_constructors() {
        let bad = r#"{"command":"mask_set_modifier","layer":0,"modifier":{"feather":-1}}"#;
        // Feather is validated when the command runs; the shape itself parses.
        assert!(serde_json::from_str::<JsonCommand>(bad).is_ok());
        let bad_gradient = r#"{"command":"mask_gradient","layer":0,"gradient":{"kind":"linear","from":{"x":1,"y":1},"to":{"x":1,"y":1}}}"#;
        assert!(serde_json::from_str::<JsonCommand>(bad_gradient).is_err());
        let pin = r#"{"command":"cell_pin","cells":[{"x":3,"y":4}],"colours":[2,1],"full":true}"#;
        assert!(matches!(
            serde_json::from_str::<JsonCommand>(pin),
            Ok(JsonCommand::CellPin(_))
        ));
        let unknown = r#"{"command":"mask_paint_colour","layer":0}"#;
        assert!(serde_json::from_str::<JsonCommand>(unknown).is_err());
    }
}
