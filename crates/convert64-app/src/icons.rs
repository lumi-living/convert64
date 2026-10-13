//! Every icon the app draws, by what it means (ADR-007's glyph map). The rest of the app
//! names these constants, never `egui_material_icons::icons` directly, so a glyph change
//! is one line here and the Figma `Icon` set (`docs/figma.md`) stays the only other place
//! to update.
//!
//! Each constant is a Material Symbols glyph from `egui_material_icons`; draw it with
//! [`Icon::rich_text`] or pass it wherever egui takes text.

pub use egui_material_icons::MaterialIcon as Icon;
use egui_material_icons::icons as m;

/// Tool: pin a cell's colour set (`P`).
pub const PIN: Icon = m::ICON_KEEP;
/// Tool: lock a cell (`L`).
pub const LOCK: Icon = m::ICON_LOCK;
/// Tool: paint a region (`R`): a recipe painted over the picture.
pub const REGION: Icon = m::ICON_FORMAT_PAINT;
/// Mask tool: brush (`B`), ADR-007's "paint mask".
pub const BRUSH: Icon = m::ICON_BRUSH;
/// Tool: nudge a layer (`V`).
pub const NUDGE: Icon = m::ICON_OPEN_WITH;
/// The 8 × 8 cell grid overlay.
pub const GRID: Icon = m::ICON_GRID_ON;
/// Zoom in.
pub const ZOOM_IN: Icon = m::ICON_ZOOM_IN;
/// Zoom out.
pub const ZOOM_OUT: Icon = m::ICON_ZOOM_OUT;
/// Zoom to fit.
pub const FIT: Icon = m::ICON_FIT_SCREEN;
/// Working view: switch between Source and Result (`Q`).
pub const SOURCE_RESULT: Icon = m::ICON_SWAP_HORIZ;
/// A/B compare (hold `\`).
pub const COMPARE: Icon = m::ICON_COMPARE;
/// Layers.
pub const LAYERS: Icon = m::ICON_LAYERS;
/// A visible layer.
pub const VISIBLE: Icon = m::ICON_VISIBILITY;
/// A hidden layer.
pub const HIDDEN: Icon = m::ICON_VISIBILITY_OFF;
/// The dither recipe.
pub const RECIPE: Icon = m::ICON_TUNE;
/// Open an image or project.
pub const OPEN: Icon = m::ICON_FOLDER_OPEN;
/// Save the project.
pub const SAVE: Icon = m::ICON_SAVE;
/// Export.
pub const EXPORT: Icon = m::ICON_DOWNLOAD;
/// Undo.
pub const UNDO: Icon = m::ICON_UNDO;
/// Redo.
pub const REDO: Icon = m::ICON_REDO;
/// The empty working view: add an image.
pub const ADD_IMAGE: Icon = m::ICON_ADD_PHOTO_ALTERNATE;
/// Clash warning.
pub const WARNING: Icon = m::ICON_WARNING;
/// Light theme.
pub const LIGHT_MODE: Icon = m::ICON_LIGHT_MODE;
/// Dark theme.
pub const DARK_MODE: Icon = m::ICON_DARK_MODE;
/// Drag handle (reorder).
pub const DRAG: Icon = m::ICON_DRAG_INDICATOR;
/// New layer from file.
pub const ADD: Icon = m::ICON_ADD;
/// Duplicate layer.
pub const DUPLICATE: Icon = m::ICON_CONTENT_COPY;
/// Delete layer.
pub const DELETE: Icon = m::ICON_DELETE;
/// Add mask / layer via mask.
pub const MASK: Icon = m::ICON_MASKS;
/// Mask linked to its layer (the chain beside a mask thumbnail).
pub const LINK: Icon = m::ICON_LINK;
/// Mask tool: lasso / polygon (`W`).
pub const LASSO: Icon = m::ICON_LASSO_SELECT;
/// Mask tool: gradient (`G`); feather.
pub const GRADIENT: Icon = m::ICON_GRADIENT;
/// Mask tool: key (`K`), sampling a colour.
pub const KEY: Icon = m::ICON_COLORIZE;
/// Invert a mask.
pub const INVERT: Icon = m::ICON_INVERT_COLORS;
/// Fill a mask.
pub const FILL: Icon = m::ICON_FORMAT_COLOR_FILL;
/// Close / dismiss.
pub const CLOSE: Icon = m::ICON_CLOSE;
