//! The editing session: a project loaded into the core's [`Editor`], [`Compositor`] and
//! [`Solver`], as the app, `convert64` and the MCP server all use it (ADR-005, ADR-009).
//!
//! A session loads a `.convert64` project file ([`ProjectFile`]) or a version-1 recipe,
//! runs its commands, composites, solves and saves it back. It never opens a file: every
//! linked source, mask, palette and the project file itself goes through the session's
//! [`Store`], so the same code runs natively ([`FsStore`](crate::FsStore), the `fs`
//! feature) and in the browser ([`MemoryStore`](crate::MemoryStore)).

use crate::codec;
use crate::command::{JsonCommand, LayerRef, RegionRef};
use crate::error::{ProjectError, invalid};
use crate::file::{
    self, Embed, LayerEntry, MaskSpec, PaletteSpec, ProjectFile, RegionEntry, SCHEMA_VERSION,
    SolveSpec, SourceSpec, SteeringSpec,
};
use crate::paths;
use crate::store::Store;
use convert64_core::command::{
    AdjustmentsSet, Command, Dirty, LayerAdd, LayerDuplicate, LayerNudge, LayerRemove, LayerRename,
    LayerReorder, LayerRotate, LayerSetBlend, LayerSetDownscale, LayerSetOpacity,
    LayerSetTransform, LayerSetVisible, LayerViaMask, MaskClear, MaskFill, MaskFromAlpha,
    MaskGradient, MaskKey, MaskLoad, MaskSetModifier, MaskSourceRemove, MaskSourceSetCombine,
    MaskStroke, PaletteSet, RecipeSet, RegionAdd, RegionClear, RegionFill, RegionGradient,
    RegionRemove, RegionRename, RegionReorder, RegionSetRecipe, RegionStroke,
};
use convert64_core::layer::{Layer, SourceImage};
use convert64_core::mask::{Mask, MaskRaster};
use convert64_core::mode::Mode;
use convert64_core::palette::linear_to_srgb8;
use convert64_core::region::{self, Region};
use convert64_core::solve::{Solution, SolveMode, Solver};
use convert64_core::{Compositor, Editor, Palette, Project, SuperGrid};
use std::path::{Path, PathBuf};
use std::sync::Arc;

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
    /// The Source view's compositor at [`Session::PREVIEW_N`], built on first use, when
    /// the project's factor is below it (ADR-012: the solve's grid is 1× by default).
    preview: Option<Compositor>,
    base: PathBuf,
    /// Every palette the session has resolved with the spec it came from, latest last,
    /// so a saved project names its palette as it was given (preset, colours, `.vpl`).
    palette_specs: Vec<(Palette, PaletteSpec)>,
    solver: Solver,
    /// The palette `solver` was built for.
    solver_palette: Palette,
    last: Option<Solution>,
    /// The target and solve settings `last` was solved with; when they change, the next
    /// solve is a full one.
    last_settings: Option<(Mode, SolveSpec)>,
    origins: Vec<(Arc<SourceImage>, Origin)>,
    /// Cells changed since the last solve (from the compositor's dirty-cell API).
    dirty: convert64_core::CellSet,
    store: Box<dyn Store>,
    /// The clock the compositors time their mip builds with ([`Session::set_clock`]).
    clock: Option<convert64_core::composite::Clock>,
    /// What loading changed in an older file (#236), for the front-end to say once.
    notes: Vec<String>,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("base", &self.base)
            .field("target", &self.target())
            .field("layers", &self.project().layers.len())
            .finish_non_exhaustive()
    }
}

impl Session {
    /// Loads the project file, or version-1 recipe file, at `path` in `store` and runs
    /// its commands; the files it links are read from `store` relative to `path`.
    pub fn open_in(store: Box<dyn Store>, path: &Path) -> Result<Self, ProjectError> {
        let file = ProjectFile::read_from(store.as_ref(), path)?;
        let base = path.parent().map(Path::to_owned).unwrap_or_default();
        Self::load(file, &base, store)
    }

    /// Builds a session from a parsed project, reading the files it links from `store`
    /// relative to `base`, and runs its commands.
    /// An older file is migrated first ([`ProjectFile::migrate`]); [`Self::notes`] then
    /// says what changed.
    pub fn load(
        mut file: ProjectFile,
        base: &Path,
        store: Box<dyn Store>,
    ) -> Result<Self, ProjectError> {
        file.migrate()?;
        let notes = file.migration_note().into_iter().collect();
        let grid = SuperGrid::new(file.target, file.supersample).map_err(invalid)?;
        file.solve
            .check()
            .map_err(|e| invalid(format!("solve: {e}")))?;
        let palette_spec = match file.palette {
            PaletteSpec::Vpl(p) => PaletteSpec::Vpl(paths::absolute(&base.join(p))),
            p => p,
        };
        let palette = palette_spec.resolve(base, store.as_ref())?;
        let mut origins: Vec<(Arc<SourceImage>, Origin)> = Vec::new();
        let mut layers: Vec<Layer> = Vec::with_capacity(file.layers.len());
        for (i, entry) in file.layers.into_iter().enumerate() {
            let what = format!("layer {i} ({:?})", entry.name);
            let src = resolve_source(
                entry.source,
                base,
                store.as_ref(),
                &mut origins,
                &layers,
                &what,
            )?;
            let size = src.size();
            let mut layer = Layer::new(entry.name, src);
            layer.set_transform(entry.transform);
            layer
                .set_opacity(entry.opacity)
                .map_err(|e| invalid(format!("{what}: {e}")))?;
            layer.set_blend(entry.blend);
            layer.set_downscale(entry.downscale);
            layer.set_visible(entry.visible);
            layer.set_recipe(entry.recipe);
            layer.set_adjustments(entry.adjustments);
            if let Some(m) = entry.mask {
                let raster = file::load_raster(
                    &m.raster,
                    base,
                    store.as_ref(),
                    size,
                    &format!("{what} mask"),
                )?;
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
            let raster = file::load_raster(
                &r.mask,
                base,
                store.as_ref(),
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
                target: file.target,
                palette: palette.clone(),
                solve: file.solve,
                snap: file.snap,
            }),
            compositor: Compositor::new(grid),
            preview: None,
            base: base.to_owned(),
            solver: Solver::new(&palette),
            solver_palette: palette.clone(),
            palette_specs: vec![(palette, palette_spec)],
            last: None,
            last_settings: None,
            origins,
            dirty: convert64_core::CellSet::all(),
            store,
            clock: None,
            notes,
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
    pub fn to_project_file(&self, dir: &Path, embed: Embed) -> Result<ProjectFile, ProjectError> {
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
                SourceSpec::Embedded(file::Embedded {
                    png_base64: crate::base64::encode(bytes),
                })
            };
            let source = match (shared, origin, embed) {
                (Some(layer), _, _) => SourceSpec::Shared(file::Shared { layer }),
                (None, Some(Origin::Linked(path, _)), Embed::Keep | Embed::None) => {
                    SourceSpec::Linked(file::Linked {
                        path: paths::relative(dir, path),
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
                (None, None, _) => embedded(&codec::encode_source(l.source())),
            };
            layers.push(LayerEntry {
                name: l.name().to_owned(),
                source,
                visible: l.visible(),
                transform: l.transform(),
                opacity: l.opacity(),
                blend: l.blend(),
                downscale: l.downscale(),
                recipe: *l.recipe(),
                adjustments: l.adjustments().clone(),
                mask: l.mask().map(|m| MaskSpec {
                    raster: file::raster_spec(m.raster()),
                    sources: m.sources().to_vec(),
                    modifiers: m.modifiers(),
                }),
            });
        }
        Ok(ProjectFile {
            schema_version: SCHEMA_VERSION,
            target: p.target,
            supersample: self.compositor.grid().n(),
            palette: match self.palette_spec(&p.palette) {
                PaletteSpec::Vpl(path) => PaletteSpec::Vpl(paths::relative(dir, &path)),
                other => other,
            },
            layers,
            regions: p
                .regions
                .iter()
                .map(|r| RegionEntry {
                    name: r.name().to_owned(),
                    mask: file::raster_spec(r.mask()),
                    recipe: *r.recipe(),
                })
                .collect(),
            steering: SteeringSpec::from_steering(&p.steering),
            solve: p.solve.clone(),
            snap: p.snap,
            commands: Vec::new(),
            migrated_from: None,
        })
    }

    /// How a project file names `palette`: as the session was given it, else a preset it
    /// equals, else its 16 colours.
    fn palette_spec(&self, palette: &Palette) -> PaletteSpec {
        if let Some((_, spec)) = self.palette_specs.iter().rev().find(|(p, _)| p == palette) {
            return spec.clone();
        }
        if *palette == Palette::pepto_pal() {
            PaletteSpec::PeptoPal
        } else if *palette == Palette::pepto_ntsc() {
            PaletteSpec::PeptoNtsc
        } else if *palette == Palette::colodore(50.0, 100.0, 50.0) {
            PaletteSpec::Colodore
        } else {
            PaletteSpec::Custom(
                palette
                    .rgb_table()
                    .iter()
                    .map(ToString::to_string)
                    .collect(),
            )
        }
    }

    /// Saves the project to `path` in the session's store (see
    /// [`Self::to_project_file`]).
    pub fn save_to_store(&self, path: &Path, embed: Embed) -> Result<(), ProjectError> {
        let dir = path.parent().map(Path::to_owned).unwrap_or_default();
        let text = self.to_project_file(&dir, embed)?.to_json();
        self.store.write(path, text.as_bytes())
    }

    /// What loading changed, for the front-end to tell the user once: for a project file
    /// of an older `schema_version`, e.g. "Updated project from v2: now uses 1×
    /// resampling" ([`ProjectFile::migration_note`]). Empty for a current file or a
    /// recipe. Saving writes the current version, so the saved file has none.
    pub fn notes(&self) -> &[String] {
        &self.notes
    }

    /// The store the session reads and writes files through.
    pub fn store(&self) -> &dyn Store {
        self.store.as_ref()
    }

    /// The target mode.
    pub fn target(&self) -> Mode {
        self.project().target
    }

    /// The palette the picture is solved and previewed with.
    pub fn palette(&self) -> &Palette {
        &self.project().palette
    }

    /// The solve settings.
    pub fn solve_spec(&self) -> &SolveSpec {
        &self.project().solve
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
    pub fn resolve(&self, r: &LayerRef) -> Result<usize, ProjectError> {
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

    /// The region index a reference names.
    pub fn resolve_region(&self, r: &RegionRef) -> Result<usize, ProjectError> {
        let regions = &self.project().regions;
        match r {
            RegionRef::Index(i) if *i < regions.len() => Ok(*i),
            RegionRef::Index(i) => Err(invalid(format!("there is no region {i}"))),
            RegionRef::Name(n) => regions
                .iter()
                .position(|r| r.name() == n)
                .ok_or_else(|| invalid(format!("there is no region named {n:?}"))),
        }
    }

    /// The core command for a JSON command (reading any file it names, relative to the
    /// project or recipe file).
    pub fn to_command(&mut self, cmd: JsonCommand) -> Result<Command, ProjectError> {
        let base = self.base.clone();
        self.to_command_at(cmd, &base)
    }

    /// The core command for a JSON command, reading any file it names relative to `base`.
    pub fn to_command_at(
        &mut self,
        cmd: JsonCommand,
        base: &Path,
    ) -> Result<Command, ProjectError> {
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
                raster: self.load_mask(&base.join(path))?,
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
            JsonCommand::LayerAdd {
                name,
                source,
                index,
                visible,
                transform,
                opacity,
                blend,
                recipe,
                adjustments,
            } => {
                let what = format!("layer_add {name:?}");
                let src = resolve_source(
                    source,
                    base,
                    self.store.as_ref(),
                    &mut self.origins,
                    &self.editor.project().layers,
                    &what,
                )?;
                let mut layer = Layer::new(name, src);
                layer.set_transform(transform);
                layer
                    .set_opacity(opacity)
                    .map_err(|e| invalid(format!("{what}: {e}")))?;
                layer.set_blend(blend);
                layer.set_visible(visible);
                layer.set_recipe(*recipe);
                layer.set_adjustments(*adjustments);
                Command::LayerAdd(Box::new(LayerAdd {
                    index: index.unwrap_or(self.project().layers.len()),
                    layer,
                }))
            }
            JsonCommand::LayerRemove { layer } => Command::LayerRemove(LayerRemove {
                layer: self.resolve(&layer)?,
            }),
            JsonCommand::LayerReorder { layer, to } => Command::LayerReorder(LayerReorder {
                layer: self.resolve(&layer)?,
                to,
            }),
            JsonCommand::LayerSetVisible { layer, visible } => {
                Command::LayerSetVisible(LayerSetVisible {
                    layer: self.resolve(&layer)?,
                    visible,
                })
            }
            JsonCommand::LayerSetOpacity { layer, opacity } => {
                Command::LayerSetOpacity(LayerSetOpacity {
                    layer: self.resolve(&layer)?,
                    opacity,
                })
            }
            JsonCommand::LayerSetBlend { layer, blend } => Command::LayerSetBlend(LayerSetBlend {
                layer: self.resolve(&layer)?,
                blend,
            }),
            JsonCommand::LayerSetDownscale { layer, downscale } => {
                Command::LayerSetDownscale(LayerSetDownscale {
                    layer: self.resolve(&layer)?,
                    downscale,
                })
            }
            JsonCommand::LayerSetTransform {
                layer,
                transform,
                snap,
            } => Command::LayerSetTransform(LayerSetTransform {
                layer: self.resolve(&layer)?,
                transform,
                snap,
            }),
            JsonCommand::LayerRotate {
                layer,
                degrees,
                relative,
                anchor,
                snap,
            } => Command::LayerRotate(LayerRotate {
                layer: self.resolve(&layer)?,
                degrees,
                relative,
                anchor,
                snap,
            }),
            JsonCommand::LayerNudge { layer, by } => Command::LayerNudge(LayerNudge {
                layer: self.resolve(&layer)?,
                by,
            }),
            JsonCommand::LayerRename { layer, name } => Command::LayerRename(LayerRename {
                layer: self.resolve(&layer)?,
                name,
            }),
            JsonCommand::LayerDuplicate { layer } => Command::LayerDuplicate(LayerDuplicate {
                layer: self.resolve(&layer)?,
            }),
            JsonCommand::AdjustmentsSet { layer, adjustments } => {
                Command::AdjustmentsSet(Box::new(AdjustmentsSet {
                    layer: self.resolve(&layer)?,
                    adjustments,
                }))
            }
            JsonCommand::RecipeSet { layer, recipe } => Command::RecipeSet(Box::new(RecipeSet {
                layer: self.resolve(&layer)?,
                recipe,
            })),
            JsonCommand::ModeSet(c) => Command::ModeSet(c),
            JsonCommand::PaletteSet { palette } => {
                let spec = match palette {
                    PaletteSpec::Vpl(p) => PaletteSpec::Vpl(paths::absolute(&base.join(p))),
                    p => p,
                };
                let resolved = spec.resolve(base, self.store.as_ref())?;
                self.palette_specs.push((resolved.clone(), spec));
                Command::PaletteSet(Box::new(PaletteSet { palette: resolved }))
            }
            JsonCommand::BackgroundPin(c) => Command::BackgroundPin(c),
            JsonCommand::SolveSet(c) => Command::SolveSet(c),
            JsonCommand::SnapSet(c) => Command::SnapSet(c),
            JsonCommand::MaskSourceRemove { layer, index } => {
                Command::MaskSourceRemove(MaskSourceRemove {
                    layer: self.resolve(&layer)?,
                    index,
                })
            }
            JsonCommand::MaskSourceSetCombine {
                layer,
                index,
                combine,
            } => Command::MaskSourceSetCombine(MaskSourceSetCombine {
                layer: self.resolve(&layer)?,
                index,
                combine,
            }),
            JsonCommand::RegionAdd {
                name,
                recipe,
                index,
            } => Command::RegionAdd(Box::new(RegionAdd {
                index: index.unwrap_or(self.project().regions.len()),
                name,
                recipe: *recipe,
            })),
            JsonCommand::RegionRemove { region } => Command::RegionRemove(RegionRemove {
                region: self.resolve_region(&region)?,
            }),
            JsonCommand::RegionRename { region, name } => Command::RegionRename(RegionRename {
                region: self.resolve_region(&region)?,
                name,
            }),
            JsonCommand::RegionReorder { region, to } => Command::RegionReorder(RegionReorder {
                region: self.resolve_region(&region)?,
                to,
            }),
            JsonCommand::RegionSetRecipe { region, recipe } => {
                Command::RegionSetRecipe(Box::new(RegionSetRecipe {
                    region: self.resolve_region(&region)?,
                    recipe,
                }))
            }
            JsonCommand::RegionStroke {
                region,
                points,
                radius,
                hardness,
                flow,
                mode,
            } => Command::RegionStroke(RegionStroke {
                region: self.resolve_region(&region)?,
                points,
                radius,
                hardness,
                flow,
                mode,
            }),
            JsonCommand::RegionFill {
                region,
                polygon,
                value,
                mode,
            } => Command::RegionFill(RegionFill {
                region: self.resolve_region(&region)?,
                polygon,
                value,
                mode,
            }),
            JsonCommand::RegionGradient {
                region,
                gradient,
                combine,
            } => Command::RegionGradient(RegionGradient {
                region: self.resolve_region(&region)?,
                gradient,
                combine,
            }),
            JsonCommand::RegionClear { region, value } => Command::RegionClear(RegionClear {
                region: self.resolve_region(&region)?,
                value,
            }),
        })
    }

    /// Decodes a greyscale PNG (any bit depth; colour is converted to luma) from the
    /// store into a mask raster.
    fn load_mask(&self, path: &Path) -> Result<MaskRaster, ProjectError> {
        let bytes = self.store.read(path)?;
        let img = image::load_from_memory(&bytes)
            .map_err(|e| ProjectError::Io(path.to_owned(), e.to_string()))?
            .to_luma16();
        let (w, h) = img.dimensions();
        MaskRaster::from_u16(w, h, img.as_raw()).map_err(invalid)
    }

    /// Runs one JSON command.
    pub fn run(&mut self, cmd: JsonCommand) -> Result<Dirty, ProjectError> {
        let c = self.to_command(cmd)?;
        self.editor.execute(c).map_err(invalid)
    }

    /// Runs one JSON command whose file paths are relative to `base` (a commands file's
    /// directory, or the working directory for a command given on the command line).
    pub fn run_at(&mut self, cmd: JsonCommand, base: &Path) -> Result<Dirty, ProjectError> {
        let c = self.to_command_at(cmd, base)?;
        self.editor.execute(c).map_err(invalid)
    }

    fn composite(&mut self) {
        let p = self.editor.project();
        if self.compositor.grid().target() != p.target {
            // `mode_set` (or its undo): a new grid, and nothing of the old solve carries.
            let n = self.compositor.grid().n();
            let grid = SuperGrid::new(p.target, n).expect("mode_set checks the mode");
            self.compositor = Compositor::new(grid);
            self.compositor.set_clock(self.clock);
            self.last = None;
            self.dirty = convert64_core::CellSet::all();
        }
        let changed = self.compositor.update(&p.layers, &p.regions);
        self.dirty = self.dirty.union(changed);
    }

    /// Solves the picture: `full` searches backgrounds and refines; otherwise the
    /// interactive fast path re-solves only cells changed since the last solve.
    pub fn solve(&mut self, full: bool) -> &Solution {
        self.composite();
        let p = self.editor.project();
        if self.solver_palette != p.palette {
            // `palette_set` (or its undo): the solver's tables are the palette's.
            self.solver = Solver::new(&p.palette);
            self.solver_palette = p.palette.clone();
            self.last = None;
        }
        let settings = (p.target, p.solve.clone());
        let mode = if full || self.last.is_none() || self.last_settings.as_ref() != Some(&settings)
        {
            SolveMode::Full
        } else {
            SolveMode::Interactive { dirty: self.dirty }
        };
        let opts = p.solve.options(
            mode,
            convert64_core::solve::dither::layer_origins(self.compositor.grid(), &p.layers),
        );
        let sol = self.solver.solve(
            self.compositor.frame(),
            &p.steering,
            self.last.as_ref(),
            &opts,
        );
        self.dirty = convert64_core::CellSet::EMPTY;
        self.last_settings = Some(settings);
        self.last.insert(sol)
    }

    /// The last solution, if any.
    pub fn solution(&self) -> Option<&Solution> {
        self.last.as_ref()
    }

    /// SSIMULACRA2 of the last solve against the frame it was solved from, the prepared
    /// composite on the target grid (ADR-013 §4, [`convert64_core::quality::score_bitmap`]
    /// gives the pairing). `None` before the first solve. Call it right after
    /// [`Self::solve`]: anything that composites again in between (an edit, a preview)
    /// moves the frame on. About 2 ms native and 8 ms in wasm for 320 × 200.
    pub fn ssimulacra2(&self) -> Option<f64> {
        let sol = self.last.as_ref()?;
        Some(convert64_core::quality::score_bitmap(
            self.compositor.frame(),
            &sol.image,
            &self.solver_palette,
        ))
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
    pub fn evaluated_mask(&mut self, layer: usize) -> Result<(u32, u32, Vec<u16>), ProjectError> {
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

    /// The super grid the session composites on: the target mode and the supersample
    /// factor.
    pub fn grid(&self) -> convert64_core::SuperGrid {
        self.compositor.grid()
    }

    /// The supersample factor the Source view is drawn at, at least: with the solve on
    /// the 1× grid (ADR-012), the view composites on its own grid so a zoomed-in layer
    /// still shows its detail and its sub-pixel placement.
    pub const PREVIEW_N: u8 = 4;

    /// Times the mip levels the session's compositors build (#235) with `clock`
    /// (milliseconds from a fixed origin; the core has no clock of its own).
    pub fn set_clock(&mut self, clock: Option<convert64_core::composite::Clock>) {
        self.clock = clock;
        self.compositor.set_clock(clock);
        if let Some(p) = &mut self.preview {
            p.set_clock(clock);
        }
    }

    /// The mip reductions (#235) the session's compositors have built, and the
    /// milliseconds they took by the clock of [`Session::set_clock`].
    pub fn mip_stats(&self) -> (u64, f64) {
        let mut stats = (self.compositor.mip_builds(), self.compositor.mip_build_ms());
        if let Some(p) = &self.preview {
            stats.0 += p.mip_builds();
            stats.1 += p.mip_build_ms();
        }
        stats
    }

    /// The composite at [`Session::PREVIEW_N`] or the project's factor, whichever is
    /// larger, as RGBA8: straight sRGB, alpha = coverage. The app's Source view draws it.
    /// Width and height first; a multicolor super pixel is two hi-res pixels wide.
    pub fn canvas_rgba(&mut self) -> (u32, u32, Vec<u8>) {
        self.composite();
        let canvas = if self.compositor.grid().n() >= Self::PREVIEW_N {
            self.preview = None;
            self.compositor.canvas()
        } else {
            let p = self.editor.project();
            let grid = SuperGrid::new(p.target, Self::PREVIEW_N).expect("a bitmap grid");
            // Only the canvas is drawn, so the reference's front end (a frame) is skipped.
            let clock = self.clock;
            let fresh = || {
                let mut c = Compositor::new(grid);
                c.set_reference_front_end(false);
                c.set_clock(clock);
                c
            };
            let preview = self.preview.get_or_insert_with(fresh);
            if preview.grid() != grid {
                *preview = fresh();
            }
            preview.update(&p.layers, &p.regions);
            preview.canvas()
        };
        let (w, h) = canvas.grid().size();
        let rgba = canvas
            .pixels()
            .iter()
            .flat_map(|&[r, g, b, a]| {
                if a <= 0.0 {
                    return [0; 4];
                }
                let [r, g, b] = [r, g, b].map(|c| linear_to_srgb8(c / a));
                [r, g, b, (a.min(1.0) * 255.0).round() as u8]
            })
            .collect();
        (w, h, rgba)
    }

    /// The attribute map's debug render on the target grid as RGBA8 (hue = dither
    /// method, lightness = strength).
    pub fn attributes_rgba(&mut self) -> (u32, u32, Vec<u8>) {
        self.composite();
        let frame = self.compositor.frame();
        let (w, h) = frame.grid().target_size();
        let rgba: Vec<u8> = frame
            .attributes()
            .debug_rgba()
            .into_iter()
            .flatten()
            .collect();
        (w, h, rgba)
    }
}

/// Paths on disk, for the native front-ends.
#[cfg(feature = "fs")]
impl Session {
    /// Loads a project file, or a version-1 recipe file, and runs its commands.
    pub fn open(path: &Path) -> Result<Self, ProjectError> {
        Self::open_in(Box::new(crate::FsStore), path)
    }

    /// Builds a session from a parsed recipe, resolving paths against `base`.
    pub fn from_file(file: crate::RecipeFile, base: &Path) -> Result<Self, ProjectError> {
        if file.version != crate::recipe::VERSION {
            return Err(invalid(format!(
                "recipe version {} is not supported (this build reads {})",
                file.version,
                crate::recipe::VERSION
            )));
        }
        Self::from_project(ProjectFile::from_recipe(file), base)
    }

    /// Builds a session from a parsed project, resolving paths against `base` on disk,
    /// and runs its commands.
    pub fn from_project(file: ProjectFile, base: &Path) -> Result<Self, ProjectError> {
        Self::load(file, base, Box::new(crate::FsStore))
    }

    /// Saves the project to `path` (see [`Self::to_project_file`]).
    pub fn save(&self, path: &Path, embed: Embed) -> Result<(), ProjectError> {
        let dir = path.parent().map(Path::to_owned).unwrap_or_default();
        let text = self.to_project_file(&dir, embed)?.to_json();
        std::fs::write(path, text).map_err(|e| ProjectError::Io(path.to_owned(), e.to_string()))
    }

    /// Writes [`Self::evaluated_mask`] as a 16-bit greyscale PNG.
    pub fn write_mask(&mut self, layer: usize, path: &Path) -> Result<(), ProjectError> {
        let (w, h, v) = self.evaluated_mask(layer)?;
        let img =
            image::ImageBuffer::<image::Luma<u16>, _>::from_raw(w, h, v).expect("sized above");
        save(path, &image::DynamicImage::ImageLuma16(img))
    }

    /// Writes [`Self::composite_rgba`] as a PNG.
    pub fn write_composite(&mut self, path: &Path) -> Result<(), ProjectError> {
        let (w, h, rgba) = self.composite_rgba();
        let img = image::RgbaImage::from_raw(w, h, rgba).expect("sized above");
        save(path, &image::DynamicImage::ImageRgba8(img))
    }

    /// Writes the attribute map's debug render as a PNG.
    pub fn write_attributes(&mut self, path: &Path) -> Result<(), ProjectError> {
        let (w, h, rgba) = self.attributes_rgba();
        let img = image::RgbaImage::from_raw(w, h, rgba).expect("sized above");
        save(path, &image::DynamicImage::ImageRgba8(img))
    }
}

#[cfg(feature = "fs")]
fn save(path: &Path, img: &image::DynamicImage) -> Result<(), ProjectError> {
    img.save(path)
        .map_err(|e| ProjectError::Io(path.to_owned(), e.to_string()))
}

/// The pixels a layer's source names: a file (read once per path, through `store`),
/// embedded bytes, or an earlier layer's pixels, shared. New files and embedded bytes are
/// recorded in `origins` so a saved project can link or embed them again.
fn resolve_source(
    spec: SourceSpec,
    base: &Path,
    store: &dyn Store,
    origins: &mut Vec<(Arc<SourceImage>, Origin)>,
    layers: &[Layer],
    what: &str,
) -> Result<Arc<SourceImage>, ProjectError> {
    Ok(match spec {
        SourceSpec::Path(p) | SourceSpec::Linked(file::Linked { path: p }) => {
            let path = paths::absolute(&base.join(p));
            let known = origins.iter().find_map(|(s, o)| match o {
                Origin::Linked(q, _) if *q == path => Some(Arc::clone(s)),
                _ => None,
            });
            if let Some(s) = known {
                s
            } else {
                let bytes = store.read(&path)?;
                let s = Arc::new(codec::decode_source(&bytes, &path.display().to_string())?);
                origins.push((Arc::clone(&s), Origin::Linked(path, Arc::new(bytes))));
                s
            }
        }
        SourceSpec::Embedded(e) => {
            let bytes = crate::base64::decode(&e.png_base64)
                .map_err(|e| invalid(format!("{what}: {e}")))?;
            let s = Arc::new(codec::decode_source(&bytes, what)?);
            origins.push((Arc::clone(&s), Origin::Embedded(Arc::new(bytes))));
            s
        }
        SourceSpec::Shared(file::Shared { layer }) => layers
            .get(layer)
            .map(|l| Arc::clone(l.source()))
            .ok_or_else(|| {
                invalid(format!(
                    "{what}: shares the pixels of layer {layer}, which is not below it"
                ))
            })?,
    })
}
