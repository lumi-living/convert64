//! The command engine (ADR-005): every user-visible action is a named command with a
//! serde-serialisable argument struct, applied to a [`Project`] through an [`Editor`]
//! that keeps undo/redo history as command + memento pairs.
//!
//! This is the first slice of the registry: the mask commands of ADR-008 (#77). The
//! app, `convert64 recipe apply` and the MCP server all execute these same values; the
//! JSON forms and file paths live in `convert64-cli`, since this crate does no I/O.
//!
//! Undo mementos are the smallest state that restores the project: for paint, the
//! [`TILE`](crate::mask::TILE)-sized raster tiles the command touched (taken before it
//! ran), so undo is O(touched tiles); for modifier and source edits, the previous mask.

use crate::geom::{PxRect, Vec2};
use crate::layer::{Layer, Levels};
use crate::mask::paint::{self, PaintMode};
use crate::mask::{
    Combine, Gradient, Key, Mask, MaskError, MaskRaster, MaskSource, Modifiers, SnapToCells,
    SourceEntry, Tile,
};
use crate::region::Region;
use serde::{Deserialize, Serialize};
use std::fmt;

/// The document commands act on. E7 extends it (target mode, palette, pins, locks); for
/// now it is the layer stack (bottom first) and the regions.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Project {
    /// Layers, bottom first.
    pub layers: Vec<Layer>,
    /// Region overrides, applied in order.
    pub regions: Vec<Region>,
}

/// Why a command could not run.
#[derive(Clone, Debug, PartialEq)]
pub enum CommandError {
    /// No layer at this index.
    NoLayer(usize),
    /// An argument is out of range.
    Param {
        /// Which argument.
        what: &'static str,
        /// Why.
        why: String,
    },
    /// A mask value was rejected.
    Mask(MaskError),
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoLayer(i) => write!(f, "there is no layer {i}"),
            Self::Param { what, why } => write!(f, "{what}: {why}"),
            Self::Mask(e) => write!(f, "mask: {e}"),
        }
    }
}

impl std::error::Error for CommandError {}

impl From<MaskError> for CommandError {
    fn from(e: MaskError) -> Self {
        Self::Mask(e)
    }
}

fn param(what: &'static str, why: impl Into<String>) -> CommandError {
    CommandError::Param {
        what,
        why: why.into(),
    }
}

fn check_range(what: &'static str, v: f32, lo: f32, hi: f32) -> Result<(), CommandError> {
    if v.is_finite() && (lo..=hi).contains(&v) {
        Ok(())
    } else {
        Err(param(what, format!("{v} is outside {lo}–{hi}")))
    }
}

fn check_points(what: &'static str, pts: &[Vec2], min: usize) -> Result<(), CommandError> {
    if pts.len() < min {
        return Err(param(what, format!("needs at least {min} points")));
    }
    if pts.iter().any(|p| !p.is_finite()) {
        return Err(param(what, "points must be finite"));
    }
    Ok(())
}

/// Paints a soft round brush along a polyline (ADR-008 `MaskStroke`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MaskStroke {
    /// Layer index.
    pub layer: usize,
    /// The polyline, layer pixels.
    pub points: Vec<Vec2>,
    /// Brush radius, layer pixels, `0.25..=1000`.
    pub radius: f32,
    /// Fraction of the radius at full strength, `0..=1`.
    pub hardness: f32,
    /// Strength per dab, `0..=1`; accumulates where dabs overlap.
    pub flow: f32,
    /// Add or subtract.
    pub mode: PaintMode,
}

/// Fills a polygon (lasso) into the mask (ADR-008 `MaskFill`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MaskFill {
    /// Layer index.
    pub layer: usize,
    /// Polygon, layer pixels, at least 3 points.
    pub polygon: Vec<Vec2>,
    /// Fill value, `0..=1`.
    pub value: f32,
    /// Add or subtract.
    pub mode: PaintMode,
}

/// Adds a gradient source (ADR-008 `MaskGradient`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MaskGradient {
    /// Layer index.
    pub layer: usize,
    /// The gradient.
    pub gradient: Gradient,
    /// How it combines.
    pub combine: Combine,
}

/// Adds a key source (ADR-008 `MaskKey`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MaskKey {
    /// Layer index.
    pub layer: usize,
    /// The key.
    pub key: Key,
    /// How it combines.
    pub combine: Combine,
}

/// One modifier slider (ADR-008 `MaskSetModifier`).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ModifierChange {
    /// Invert on or off.
    Invert(bool),
    /// Levels.
    Levels(Levels),
    /// Snap to cells.
    Snap(SnapToCells),
    /// Feather σ in target pixels, `0..=32`.
    Feather(f32),
}

/// Sets one modifier of a layer's mask.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MaskSetModifier {
    /// Layer index.
    pub layer: usize,
    /// The change.
    pub modifier: ModifierChange,
}

/// Replaces the mask raster with an imported greyscale image of the layer's size
/// (ADR-008 `MaskLoad`; the CLI and MCP server read the PNG).
#[derive(Clone, Debug, PartialEq)]
pub struct MaskLoad {
    /// Layer index.
    pub layer: usize,
    /// The raster.
    pub raster: MaskRaster,
}

/// Sets the whole raster to one value (ADR-008 `MaskClear`): 0 is the options bar's
/// **Clear**, 1 its **Fill**. Sources and modifiers stay.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MaskClear {
    /// Layer index.
    pub layer: usize,
    /// The value, `0..=1`.
    #[serde(default)]
    pub value: f32,
}

/// Copies the layer's PNG alpha into its mask raster (ADR-008 `MaskFromAlpha`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MaskFromAlpha {
    /// Layer index.
    pub layer: usize,
}

/// A selection made with the mask tools while no mask is being edited (UX §5a): lasso
/// polygons filled at 1, then procedural sources, then modifiers, all in the source
/// layer's space.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Selection {
    /// Lasso polygons, layer pixels.
    #[serde(default)]
    pub polygons: Vec<Vec<Vec2>>,
    /// Gradients and keys.
    #[serde(default)]
    pub sources: Vec<SourceEntry>,
    /// Modifiers for the new layer's mask.
    #[serde(default)]
    pub modifiers: Modifiers,
}

/// A new layer above `source_layer` sharing its pixels (`Arc`) with the selection as its
/// mask; with `cut`, the original keeps only the inverse (ADR-008 `LayerViaMask`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LayerViaMask {
    /// The layer to split.
    pub source_layer: usize,
    /// The selection.
    pub selection: Selection,
    /// Also remove the selection from the original.
    #[serde(default)]
    pub cut: bool,
}

/// A command (ADR-005). See [`REGISTRY`] for the names.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    /// `mask_stroke`.
    MaskStroke(MaskStroke),
    /// `mask_fill`.
    MaskFill(MaskFill),
    /// `mask_gradient`.
    MaskGradient(MaskGradient),
    /// `mask_key`.
    MaskKey(MaskKey),
    /// `mask_set_modifier`.
    MaskSetModifier(MaskSetModifier),
    /// `mask_load`.
    MaskLoad(MaskLoad),
    /// `mask_clear`.
    MaskClear(MaskClear),
    /// `mask_from_alpha`.
    MaskFromAlpha(MaskFromAlpha),
    /// `layer_via_mask`.
    LayerViaMask(LayerViaMask),
}

/// A command's name and a one-line description, for front-ends (CLI help, MCP tool list).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommandInfo {
    /// The command's name in JSON and MCP.
    pub name: &'static str,
    /// What it does.
    pub summary: &'static str,
}

/// Every command, by name.
pub const REGISTRY: &[CommandInfo] = &[
    CommandInfo {
        name: "mask_stroke",
        summary: "Paint the mask with a soft round brush along a polyline (add or subtract).",
    },
    CommandInfo {
        name: "mask_fill",
        summary: "Fill a polygon (lasso) into the mask (add or subtract).",
    },
    CommandInfo {
        name: "mask_gradient",
        summary: "Add a linear or radial gradient source to the mask.",
    },
    CommandInfo {
        name: "mask_key",
        summary: "Add a luma, chroma or colour key on the layer's own pixels to the mask.",
    },
    CommandInfo {
        name: "mask_set_modifier",
        summary: "Set one mask modifier: invert, levels, snap to cells or feather.",
    },
    CommandInfo {
        name: "mask_load",
        summary: "Replace the mask raster with a greyscale PNG of the layer's size.",
    },
    CommandInfo {
        name: "mask_clear",
        summary: "Set the whole mask raster to one value (0 clears, 1 fills).",
    },
    CommandInfo {
        name: "mask_from_alpha",
        summary: "Copy the layer's PNG alpha into its mask raster.",
    },
    CommandInfo {
        name: "layer_via_mask",
        summary: "New layer sharing the source pixels, with a selection as its mask; cut \
                  removes the selection from the original.",
    },
];

/// What a command changed, for the compositor and solver: `LayerRect` maps to target
/// cells through `composite::mask_change_cells`; the others re-solve the layer's
/// footprint or the whole picture (`Compositor::update` finds the exact cells).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dirty {
    /// Part of a layer's mask raster, layer pixels.
    LayerRect {
        /// Layer index.
        layer: usize,
        /// Touched rectangle.
        rect: PxRect,
    },
    /// A whole layer.
    Layer(usize),
    /// The stack itself (a layer added or removed).
    Stack,
}

/// What undo needs.
#[derive(Clone, Debug, PartialEq)]
enum Memento {
    /// Raster tiles before a paint command; `created` when the command added the mask.
    Tiles {
        layer: usize,
        created: bool,
        tiles: Vec<Tile>,
    },
    /// The whole mask before the command (`None`: the layer had no mask).
    Mask { layer: usize, before: Option<Mask> },
    /// A layer inserted at `index`; the original's mask before a cut.
    Inserted {
        index: usize,
        original: usize,
        original_mask: Option<Option<Mask>>,
    },
}

/// The layer's mask, creating a fully opaque one if it has none. Returns whether it was
/// created.
fn mask_of(layer: &mut Layer) -> Result<(bool, &mut Mask), CommandError> {
    let created = layer.mask().is_none();
    if created {
        let (w, h) = layer.source().size();
        layer
            .set_mask(Some(Mask::opaque(w, h)?))
            .expect("built at the source's size");
    }
    Ok((created, layer.mask_mut().expect("present")))
}

fn layer_mut(p: &mut Project, i: usize) -> Result<&mut Layer, CommandError> {
    p.layers.get_mut(i).ok_or(CommandError::NoLayer(i))
}

impl Command {
    /// The command's registry name.
    pub const fn name(&self) -> &'static str {
        match self {
            Self::MaskStroke(_) => "mask_stroke",
            Self::MaskFill(_) => "mask_fill",
            Self::MaskGradient(_) => "mask_gradient",
            Self::MaskKey(_) => "mask_key",
            Self::MaskSetModifier(_) => "mask_set_modifier",
            Self::MaskLoad(_) => "mask_load",
            Self::MaskClear(_) => "mask_clear",
            Self::MaskFromAlpha(_) => "mask_from_alpha",
            Self::LayerViaMask(_) => "layer_via_mask",
        }
    }

    /// Checks the arguments without touching a project.
    fn validate(&self) -> Result<(), CommandError> {
        match self {
            Self::MaskStroke(c) => {
                check_points("points", &c.points, 1)?;
                check_range("radius", c.radius, 0.25, 1000.0)?;
                check_range("hardness", c.hardness, 0.0, 1.0)?;
                check_range("flow", c.flow, 0.0, 1.0)
            }
            Self::MaskFill(c) => {
                check_points("polygon", &c.polygon, 3)?;
                check_range("value", c.value, 0.0, 1.0)
            }
            Self::MaskClear(c) => check_range("value", c.value, 0.0, 1.0),
            Self::MaskSetModifier(c) => match c.modifier {
                ModifierChange::Feather(f) => {
                    Modifiers::default().with_feather(f)?;
                    Ok(())
                }
                _ => Ok(()),
            },
            Self::LayerViaMask(c) => {
                if c.selection.polygons.is_empty() && c.selection.sources.is_empty() {
                    return Err(param("selection", "is empty"));
                }
                for poly in &c.selection.polygons {
                    check_points("selection polygon", poly, 3)?;
                }
                Ok(())
            }
            Self::MaskGradient(_)
            | Self::MaskKey(_)
            | Self::MaskLoad(_)
            | Self::MaskFromAlpha(_) => Ok(()),
        }
    }

    fn apply(&self, p: &mut Project) -> Result<(Memento, Dirty), CommandError> {
        self.validate()?;
        match self {
            Self::MaskStroke(c) => {
                let layer = layer_mut(p, c.layer)?;
                let (created, mask) = mask_of(layer)?;
                let bounds = paint::stroke_bounds(mask.raster(), &c.points, c.radius);
                let tiles = mask.tiles(bounds);
                let rect = mask
                    .paint(|r| paint::stroke(r, &c.points, c.radius, c.hardness, c.flow, c.mode));
                debug_assert_eq!(rect.union(bounds), bounds);
                let m = Memento::Tiles {
                    layer: c.layer,
                    created,
                    tiles,
                };
                Ok((
                    m,
                    Dirty::LayerRect {
                        layer: c.layer,
                        rect,
                    },
                ))
            }
            Self::MaskFill(c) => {
                let layer = layer_mut(p, c.layer)?;
                let (created, mask) = mask_of(layer)?;
                let bounds = paint::fill_bounds(mask.raster(), &c.polygon);
                let tiles = mask.tiles(bounds);
                let rect = mask.paint(|r| paint::fill(r, &c.polygon, c.value, c.mode));
                let m = Memento::Tiles {
                    layer: c.layer,
                    created,
                    tiles,
                };
                Ok((
                    m,
                    Dirty::LayerRect {
                        layer: c.layer,
                        rect,
                    },
                ))
            }
            Self::MaskGradient(MaskGradient {
                layer,
                gradient,
                combine,
            }) => push_source(
                p,
                *layer,
                SourceEntry {
                    source: MaskSource::Gradient(*gradient),
                    combine: *combine,
                },
            ),
            Self::MaskKey(MaskKey {
                layer,
                key,
                combine,
            }) => push_source(
                p,
                *layer,
                SourceEntry {
                    source: MaskSource::Key(*key),
                    combine: *combine,
                },
            ),
            Self::MaskSetModifier(c) => {
                let layer = layer_mut(p, c.layer)?;
                let before = layer.mask().cloned();
                let (_, mask) = mask_of(layer)?;
                let m = mask.modifiers();
                mask.set_modifiers(match c.modifier {
                    ModifierChange::Invert(v) => m.with_invert(v),
                    ModifierChange::Levels(v) => m.with_levels(v),
                    ModifierChange::Snap(v) => m.with_snap(v),
                    ModifierChange::Feather(v) => m.with_feather(v)?,
                });
                Ok((
                    Memento::Mask {
                        layer: c.layer,
                        before,
                    },
                    Dirty::Layer(c.layer),
                ))
            }
            Self::MaskLoad(c) => replace_raster(p, c.layer, |_| Ok(c.raster.clone())),
            Self::MaskClear(c) => replace_raster(p, c.layer, |l| {
                let (w, h) = l.source().size();
                Ok(MaskRaster::filled(w, h, c.value)?)
            }),
            Self::MaskFromAlpha(c) => replace_raster(p, c.layer, |l| {
                let (w, h) = l.source().size();
                let alpha = l.source().pixels().iter().map(|px| px[3]).collect();
                Ok(MaskRaster::from_values(w, h, alpha)?)
            }),
            Self::LayerViaMask(c) => layer_via_mask(p, c),
        }
    }
}

fn push_source(
    p: &mut Project,
    i: usize,
    s: SourceEntry,
) -> Result<(Memento, Dirty), CommandError> {
    let layer = layer_mut(p, i)?;
    let before = layer.mask().cloned();
    let (_, mask) = mask_of(layer)?;
    mask.push_source(s);
    Ok((Memento::Mask { layer: i, before }, Dirty::Layer(i)))
}

fn replace_raster(
    p: &mut Project,
    i: usize,
    raster: impl FnOnce(&Layer) -> Result<MaskRaster, CommandError>,
) -> Result<(Memento, Dirty), CommandError> {
    let layer = layer_mut(p, i)?;
    let new = raster(layer)?;
    if new.size() != layer.source().size() {
        return Err(MaskError::Size {
            expected: layer.source().size(),
            found: new.size(),
        }
        .into());
    }
    let before = layer.mask().cloned();
    let (_, mask) = mask_of(layer)?;
    mask.set_raster(new)?;
    Ok((Memento::Mask { layer: i, before }, Dirty::Layer(i)))
}

fn layer_via_mask(p: &mut Project, c: &LayerViaMask) -> Result<(Memento, Dirty), CommandError> {
    let original = p
        .layers
        .get(c.source_layer)
        .ok_or(CommandError::NoLayer(c.source_layer))?;
    let (w, h) = original.source().size();
    let mut raster = MaskRaster::filled(w, h, 0.0)?;
    for poly in &c.selection.polygons {
        paint::fill(&mut raster, poly, 1.0, PaintMode::Add);
    }
    let mut selection = Mask::new(raster);
    selection.set_sources(c.selection.sources.clone());
    selection.set_modifiers(c.selection.modifiers);

    let mut layer = original.clone();
    layer.set_name(format!("{} (mask)", original.name()));
    layer
        .set_mask(Some(selection.clone()))
        .expect("selection is the source's size");

    let original_mask = if c.cut {
        let before = original.mask().cloned();
        let cut = match &before {
            // No mask yet: the original gets the selection, inverted.
            None => {
                let mut m = selection.clone();
                m.set_modifiers(
                    c.selection
                        .modifiers
                        .with_invert(!c.selection.modifiers.invert()),
                );
                m
            }
            // A mask already: multiply its raster by the inverse of the selection.
            Some(m) => {
                let sel = selection.layer_values(original.source());
                let vals = m
                    .raster()
                    .values()
                    .iter()
                    .zip(sel.iter())
                    .map(|(v, s)| v * (1.0 - s))
                    .collect();
                let mut m = m.clone();
                m.set_raster(MaskRaster::from_values(w, h, vals)?)?;
                m
            }
        };
        p.layers[c.source_layer]
            .set_mask(Some(cut))
            .expect("same size");
        Some(before)
    } else {
        None
    };
    let index = c.source_layer + 1;
    p.layers.insert(index, layer);
    Ok((
        Memento::Inserted {
            index,
            original: c.source_layer,
            original_mask,
        },
        Dirty::Stack,
    ))
}

impl Memento {
    fn undo(self, p: &mut Project) -> Dirty {
        match self {
            Self::Tiles {
                layer,
                created,
                tiles,
            } => {
                let l = &mut p.layers[layer];
                if created {
                    l.set_mask(None).expect("removing always fits");
                } else {
                    l.mask_mut().expect("painted mask").restore(&tiles);
                }
                Dirty::Layer(layer)
            }
            Self::Mask { layer, before } => {
                p.layers[layer].set_mask(before).expect("was on this layer");
                Dirty::Layer(layer)
            }
            Self::Inserted {
                index,
                original,
                original_mask,
            } => {
                p.layers.remove(index);
                if let Some(m) = original_mask {
                    p.layers[original].set_mask(m).expect("was on this layer");
                }
                Dirty::Stack
            }
        }
    }
}

/// A project with its undo/redo history. Every mutation goes through [`Self::execute`].
#[derive(Clone, Debug, Default)]
pub struct Editor {
    project: Project,
    done: Vec<(Command, Memento)>,
    undone: Vec<Command>,
}

impl Editor {
    /// An editor with empty history.
    pub fn new(project: Project) -> Self {
        Self {
            project,
            done: Vec::new(),
            undone: Vec::new(),
        }
    }

    /// The project.
    pub const fn project(&self) -> &Project {
        &self.project
    }

    /// The project, ending the editing session.
    pub fn into_project(self) -> Project {
        self.project
    }

    /// Runs a command; on success it joins the undo history and clears redo. A failed
    /// command changes nothing.
    pub fn execute(&mut self, cmd: Command) -> Result<Dirty, CommandError> {
        let (memento, dirty) = cmd.apply(&mut self.project)?;
        self.done.push((cmd, memento));
        self.undone.clear();
        Ok(dirty)
    }

    /// Undoes the last command, if any.
    pub fn undo(&mut self) -> Option<Dirty> {
        let (cmd, memento) = self.done.pop()?;
        let dirty = memento.undo(&mut self.project);
        self.undone.push(cmd);
        Some(dirty)
    }

    /// Redoes the last undone command, if any.
    pub fn redo(&mut self) -> Option<Result<Dirty, CommandError>> {
        let cmd = self.undone.pop()?;
        let undone = std::mem::take(&mut self.undone);
        let result = self.execute(cmd);
        self.undone = undone;
        Some(result)
    }

    /// Commands that can be undone.
    pub fn undo_len(&self) -> usize {
        self.done.len()
    }

    /// Commands that can be redone.
    pub fn redo_len(&self) -> usize {
        self.undone.len()
    }

    /// Raster values saved by the history's tile mementos, for tests and budgeting.
    pub fn saved_tile_values(&self) -> usize {
        self.done
            .iter()
            .map(|(_, m)| match m {
                Memento::Tiles { tiles, .. } => tiles.iter().map(Tile::len).sum(),
                _ => 0,
            })
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::SourceImage;
    use crate::mask::{GradientKind, KeyChannel};
    use std::sync::Arc;

    fn project() -> Project {
        let px: Vec<u8> = (0..300 * 200)
            .flat_map(|i| {
                let x = (i % 300) as u8;
                [x, 255 - x, 40, if i % 300 < 150 { 255 } else { 128 }]
            })
            .collect();
        let src = Arc::new(SourceImage::from_srgba8(300, 200, &px).unwrap());
        Project {
            layers: vec![Layer::new("photo", src)],
            regions: Vec::new(),
        }
    }

    fn stroke(layer: usize) -> Command {
        Command::MaskStroke(MaskStroke {
            layer,
            points: vec![
                Vec2::new(20.0, 20.0),
                Vec2::new(150.0, 90.0),
                Vec2::new(160.0, 160.0),
            ],
            radius: 12.0,
            hardness: 0.3,
            flow: 0.6,
            mode: PaintMode::Subtract,
        })
    }

    #[test]
    fn stroke_then_undo_is_byte_identical() {
        let mut e = Editor::new(project());
        e.execute(Command::MaskClear(MaskClear {
            layer: 0,
            value: 0.5,
        }))
        .unwrap();
        let before = e.project().layers[0].mask().unwrap().raster().clone();
        let dirty = e.execute(stroke(0)).unwrap();
        let Dirty::LayerRect { rect, .. } = dirty else {
            panic!("{dirty:?}")
        };
        assert_eq!(rect, PxRect::new(8, 8, 172, 172));
        assert_ne!(e.project().layers[0].mask().unwrap().raster(), &before);
        // The memento holds only the touched tiles, not the whole raster.
        assert!(e.saved_tile_values() < 300 * 200);
        e.undo().unwrap();
        assert_eq!(e.project().layers[0].mask().unwrap().raster(), &before);
        // Redo repeats it exactly.
        e.redo().unwrap().unwrap();
        let after_redo = e.project().layers[0].mask().unwrap().raster().clone();
        e.undo();
        e.execute(stroke(0)).unwrap();
        assert_eq!(e.project().layers[0].mask().unwrap().raster(), &after_redo);
    }

    #[test]
    fn undo_of_an_auto_created_mask_removes_it() {
        let mut e = Editor::new(project());
        e.execute(stroke(0)).unwrap();
        assert!(e.project().layers[0].mask().is_some());
        e.undo();
        assert!(e.project().layers[0].mask().is_none());
    }

    #[test]
    fn mask_from_alpha_on_an_opaque_png_gives_one() {
        let src = Arc::new(SourceImage::from_srgba8(4, 4, &[200; 64]).unwrap());
        let mut opaque = vec![255u8; 64];
        for i in 0..16 {
            opaque[i * 4] = 10;
        }
        let src2 = Arc::new(SourceImage::from_srgba8(4, 4, &opaque).unwrap());
        let mut e = Editor::new(Project {
            layers: vec![Layer::new("a", src), Layer::new("b", src2)],
            regions: Vec::new(),
        });
        e.execute(Command::MaskFromAlpha(MaskFromAlpha { layer: 1 }))
            .unwrap();
        assert!(
            e.project().layers[1]
                .mask()
                .unwrap()
                .raster()
                .values()
                .iter()
                .all(|&v| v == 1.0)
        );
        // A translucent source gives its alpha.
        e.execute(Command::MaskFromAlpha(MaskFromAlpha { layer: 0 }))
            .unwrap();
        let v = e.project().layers[0].mask().unwrap().raster().get(0, 0);
        assert!((v - 200.0 / 255.0).abs() < 1e-6);
    }

    #[test]
    fn sources_and_modifiers_undo() {
        let mut e = Editor::new(project());
        let g =
            Gradient::new(GradientKind::Linear, Vec2::ZERO, Vec2::new(300.0, 0.0), 1.0).unwrap();
        e.execute(Command::MaskGradient(MaskGradient {
            layer: 0,
            gradient: g,
            combine: Combine::Min,
        }))
        .unwrap();
        let k = Key::new(KeyChannel::Luma, [0.5; 3], 0.1, 0.05).unwrap();
        e.execute(Command::MaskKey(MaskKey {
            layer: 0,
            key: k,
            combine: Combine::Max,
        }))
        .unwrap();
        e.execute(Command::MaskSetModifier(MaskSetModifier {
            layer: 0,
            modifier: ModifierChange::Feather(6.0),
        }))
        .unwrap();
        let m = e.project().layers[0].mask().unwrap();
        assert_eq!(m.sources().len(), 2);
        assert_eq!(m.modifiers().feather(), 6.0);
        e.undo();
        assert_eq!(
            e.project().layers[0].mask().unwrap().modifiers().feather(),
            0.0
        );
        e.undo();
        assert_eq!(e.project().layers[0].mask().unwrap().sources().len(), 1);
        e.undo();
        assert!(e.project().layers[0].mask().is_none());
        assert_eq!(e.undo(), None);
    }

    #[test]
    fn invalid_commands_change_nothing() {
        let mut e = Editor::new(project());
        let p0 = e.project().clone();
        for bad in [
            Command::MaskSetModifier(MaskSetModifier {
                layer: 0,
                modifier: ModifierChange::Feather(40.0),
            }),
            Command::MaskClear(MaskClear {
                layer: 3,
                value: 0.0,
            }),
            Command::MaskClear(MaskClear {
                layer: 0,
                value: 1.5,
            }),
            Command::MaskFill(MaskFill {
                layer: 0,
                polygon: vec![Vec2::ZERO; 2],
                value: 1.0,
                mode: PaintMode::Add,
            }),
            Command::MaskStroke(MaskStroke {
                layer: 0,
                points: vec![Vec2::new(f32::NAN, 0.0)],
                radius: 4.0,
                hardness: 0.5,
                flow: 1.0,
                mode: PaintMode::Add,
            }),
            Command::MaskLoad(MaskLoad {
                layer: 0,
                raster: MaskRaster::filled(3, 3, 1.0).unwrap(),
            }),
            Command::LayerViaMask(LayerViaMask {
                source_layer: 0,
                selection: Selection::default(),
                cut: false,
            }),
        ] {
            assert!(e.execute(bad.clone()).is_err(), "{bad:?}");
            assert_eq!(e.project(), &p0, "{}", bad.name());
        }
        assert_eq!(e.undo_len(), 0);
    }

    #[test]
    fn layer_via_mask_shares_pixels_and_cut_inverts_the_original() {
        let mut e = Editor::new(project());
        let sel = Selection {
            polygons: vec![vec![
                Vec2::new(10.0, 10.0),
                Vec2::new(100.0, 10.0),
                Vec2::new(100.0, 80.0),
                Vec2::new(10.0, 80.0),
            ]],
            sources: Vec::new(),
            modifiers: Modifiers::feathered(2.0).unwrap(),
        };
        let dirty = e
            .execute(Command::LayerViaMask(LayerViaMask {
                source_layer: 0,
                selection: sel,
                cut: true,
            }))
            .unwrap();
        assert_eq!(dirty, Dirty::Stack);
        let p = e.project();
        assert_eq!(p.layers.len(), 2);
        assert_eq!(p.layers[1].name(), "photo (mask)");
        assert!(Arc::ptr_eq(p.layers[0].source(), p.layers[1].source()));
        let new = p.layers[1].mask().unwrap();
        let old = p.layers[0].mask().unwrap();
        assert_eq!(new.raster().get(50, 50), 1.0);
        assert_eq!(new.raster().get(150, 50), 0.0);
        assert!(old.modifiers().invert() && !new.modifiers().invert());
        assert_eq!(new.modifiers().feather(), 2.0);
        e.undo();
        assert_eq!(e.project(), &project_with_names(&e));
        assert!(e.project().layers[0].mask().is_none());
    }

    fn project_with_names(e: &Editor) -> Project {
        let mut p = project();
        // Same source Arc as the editor's, so layers compare equal.
        let src = Arc::clone(e.project().layers[0].source());
        p.layers[0] = Layer::new("photo", src);
        p
    }

    #[test]
    fn registry_names_match_commands() {
        let names: Vec<&str> = REGISTRY.iter().map(|c| c.name).collect();
        for c in [
            stroke(0),
            Command::MaskClear(MaskClear {
                layer: 0,
                value: 0.0,
            }),
            Command::MaskFromAlpha(MaskFromAlpha { layer: 0 }),
        ] {
            assert!(names.contains(&c.name()));
        }
        assert_eq!(names.len(), 9);
    }
}
