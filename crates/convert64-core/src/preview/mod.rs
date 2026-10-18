//! Method thumbnails for the dither method picker (UX §6a): the real solver on a small
//! tile, so a card shows what the layer would get.
//!
//! - [`SourceTile`]: what is dithered, a [`TILE_CELLS`]² block of cells with one ring
//!   of [`CONTEXT_CELLS`] around it so diffusion enters the tile as it would mid-picture.
//!   Either a [`SourceTile::gradient`] (a diagonal ramp through the palette's greys plus
//!   one hue, with ±2/255 seeded jitter, png2amiga's lesson) or a
//!   [`SourceTile::crop`] of the composite at a cell.
//! - [`TileSolver::render`] / [`method_tile`]: places the block at its screen position
//!   in an otherwise empty frame, locks every other cell to a flat solution in the
//!   given background and solves with [`Solver::solve`] in full: the same path, the
//!   same recipe (only the method swapped), the same pattern phase. Returns the inner
//!   tile as RGBA.
//! - [`TileKey`] / [`TileCache`]: thumbnails cached by method, settings and source.
//!
//! A tile is 4 × 4 cells: 32 × 32 hi-res pixels, or 16 × 32 multicolor pixels shown at
//! 2:1, so the card is square in both modes.

pub mod catalogue;

use crate::colour::{ColourIndex, ColourSet};
use crate::composite::{AttrSource, TargetFrame};
use crate::geom::{CellSet, SuperGrid};
use crate::layer::{Method, Recipe};
use crate::mode::Mode;
use crate::model::{BitmapImage, CellPos, IndexedBitmap};
use crate::palette::{Palette, linear_to_srgb8};
use crate::solve::{ClashReport, Solution, SolveMode, SolveOptions, Solver, SolverKind, Steering};
use std::collections::{HashMap, VecDeque};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;

/// The tile's size in cells (both ways).
pub const TILE_CELLS: u16 = 4;

/// The ring of context cells solved around the tile and then dropped.
pub const CONTEXT_CELLS: u16 = 1;

/// The block solved: the tile and its context ring, in cells.
pub const BLOCK_CELLS: u16 = TILE_CELLS + 2 * CONTEXT_CELLS;

/// Screen cells across and down.
const SCREEN_CELLS: (u16, u16) = (40, 25);

/// The seed of the gradient's jitter (fixed, so thumbnails are reproducible).
pub const GRADIENT_SEED: u32 = 0x00C6_4A1E;

/// What a thumbnail dithers: a block of [`BLOCK_CELLS`]² cells of linear RGB on the
/// target grid, and where on the screen it sits (the pattern phase depends on it).
#[derive(Clone, Debug, PartialEq)]
pub struct SourceTile {
    mode: Mode,
    /// The block's top-left cell (the context ring's corner).
    origin: CellPos,
    /// Row-major straight linear RGB, `block_px(mode)` in size.
    colour: Vec<[f32; 3]>,
    /// The colours have been through png2c64's preprocessing (a composite's have).
    prepared: bool,
    /// The background the tile is solved against (multicolor): the current result's.
    background: Option<ColourIndex>,
    hash: u64,
}

/// The block's size in target pixels for `mode`.
fn block_px(mode: Mode) -> (usize, usize) {
    let (cw, ch) = cell_px(mode);
    (cw * usize::from(BLOCK_CELLS), ch * usize::from(BLOCK_CELLS))
}

/// A cell's size in target pixels.
fn cell_px(mode: Mode) -> (usize, usize) {
    match mode {
        Mode::MulticolorBitmap => (4, 8),
        _ => (8, 8),
    }
}

/// A uniform value in `-1..=1` from `(x, y, seed)` (a small integer hash).
fn jitter(x: u32, y: u32, seed: u32) -> f32 {
    let mut h = x.wrapping_mul(0x8DA6_B343) ^ y.wrapping_mul(0xD816_3841) ^ seed;
    h ^= h >> 13;
    h = h.wrapping_mul(0x5BD1_E995);
    h ^= h >> 15;
    (h & 0xFFFF) as f32 / 32767.5 - 1.0
}

/// Relative luminance of linear RGB.
fn luminance(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

impl SourceTile {
    /// The gradient: a diagonal ramp from the darkest to the lightest of the palette's
    /// greys (black, the three greys and white, in the palette's own colours), tinted
    /// towards the palette's light blue at matching lightness along the other diagonal,
    /// with ±2/255 seeded jitter in sRGB so error diffusion does not lock into a
    /// regular pattern (png2amiga's lesson). Centred on the screen.
    pub fn gradient(mode: Mode, palette: &Palette, background: Option<ColourIndex>) -> Self {
        let lin = |c: ColourIndex| {
            let l = palette.linear(c);
            [l.r, l.g, l.b]
        };
        let mut greys: Vec<[f32; 3]> = [
            ColourIndex::BLACK,
            ColourIndex::DARK_GREY,
            ColourIndex::MEDIUM_GREY,
            ColourIndex::LIGHT_GREY,
            ColourIndex::WHITE,
        ]
        .map(lin)
        .to_vec();
        greys.sort_by(|a, b| luminance(*a).total_cmp(&luminance(*b)));
        let hue = lin(ColourIndex::LIGHT_BLUE);
        let ramp = |t: f32| {
            let s = t.clamp(0.0, 1.0) * (greys.len() - 1) as f32;
            let i = (s.floor() as usize).min(greys.len() - 2);
            let f = s - i as f32;
            let (a, b) = (greys[i], greys[i + 1]);
            [0, 1, 2].map(|k| a[k] + (b[k] - a[k]) * f)
        };
        let (w, h) = block_px(mode);
        let xs = match mode {
            Mode::MulticolorBitmap => 2.0,
            _ => 1.0,
        };
        // Screen (hi-res) units, so the ramp is 45° in both modes.
        let (sw, sh) = (w as f32 * xs, h as f32);
        let mut colour = Vec::with_capacity(w * h);
        for y in 0..h {
            for x in 0..w {
                let sx = (x as f32 + 0.5) * xs;
                let sy = y as f32 + 0.5;
                let t = (sx + sy) / (sw + sh);
                let grey = ramp(t);
                // Tinted towards the hue at the grey's lightness above the diagonal.
                let wgt = ((sx / sw - sy / sh) * 1.2).clamp(0.0, 0.8);
                let scale = luminance(grey) / luminance(hue).max(1e-6);
                let tint = hue.map(|c| (c * scale).min(1.0));
                let c = [0, 1, 2].map(|k| grey[k] + (tint[k] - grey[k]) * wgt);
                // ±2/255 in sRGB, seeded per pixel.
                let j = jitter(x as u32, y as u32, GRADIENT_SEED) * 2.0 / 255.0;
                let c = c.map(|v| {
                    let s = f32::from(linear_to_srgb8(v)) / 255.0 + j;
                    srgb_to_linear(s.clamp(0.0, 1.0))
                });
                colour.push(c);
            }
        }
        let origin = CellPos::new(
            (SCREEN_CELLS.0 - BLOCK_CELLS) / 2,
            (SCREEN_CELLS.1 - BLOCK_CELLS) / 2,
        );
        Self::new(mode, origin, colour, false, background)
    }

    /// A crop of `frame` (the composite on the target grid) around `cell`: the tile is
    /// the 4 × 4 cells starting one cell up and left of it, moved inwards so its context
    /// ring stays on the screen. Uncovered pixels are black, as the solver sees them.
    pub fn crop(frame: &TargetFrame, cell: CellPos, background: Option<ColourIndex>) -> Self {
        let mode = frame.grid().target();
        let max = (SCREEN_CELLS.0 - BLOCK_CELLS, SCREEN_CELLS.1 - BLOCK_CELLS);
        // The block's corner: the cell sits in the tile's second row and column.
        let origin = CellPos::new(
            cell.x.saturating_sub(2).min(max.0),
            cell.y.saturating_sub(2).min(max.1),
        );
        let (cw, ch) = cell_px(mode);
        let (w, h) = block_px(mode);
        let (fw, _) = frame.grid().target_size();
        let (x0, y0) = (usize::from(origin.x) * cw, usize::from(origin.y) * ch);
        let mut colour = Vec::with_capacity(w * h);
        for y in 0..h {
            for x in 0..w {
                let i = (y0 + y) * fw as usize + x0 + x;
                colour.push(if frame.coverage()[i] > 0.0 {
                    frame.colour()[i]
                } else {
                    [0.0; 3]
                });
            }
        }
        Self::new(mode, origin, colour, frame.png2c64_prepared(), background)
    }

    fn new(
        mode: Mode,
        origin: CellPos,
        colour: Vec<[f32; 3]>,
        prepared: bool,
        background: Option<ColourIndex>,
    ) -> Self {
        // Multicolor always has one (black when there is no result yet); hi-res none.
        let background =
            (mode == Mode::MulticolorBitmap).then(|| background.unwrap_or(ColourIndex::BLACK));
        let mut h = DefaultHasher::new();
        mode.hash(&mut h);
        origin.x.hash(&mut h);
        origin.y.hash(&mut h);
        prepared.hash(&mut h);
        background.map(ColourIndex::get).hash(&mut h);
        for c in &colour {
            for v in c {
                v.to_bits().hash(&mut h);
            }
        }
        Self {
            mode,
            origin,
            colour,
            prepared,
            background,
            hash: h.finish(),
        }
    }

    /// The mode it was made for.
    pub const fn mode(&self) -> Mode {
        self.mode
    }

    /// The block's top-left cell on the screen.
    pub const fn origin(&self) -> CellPos {
        self.origin
    }

    /// The background it is solved against (multicolor).
    pub const fn background(&self) -> Option<ColourIndex> {
        self.background
    }

    /// A hash of everything in it, for [`TileKey`].
    pub const fn hash(&self) -> u64 {
        self.hash
    }

    /// The block's cells on the screen.
    fn block_cells(&self) -> CellSet {
        let (x, y) = (self.origin.x, self.origin.y);
        let mut s = CellSet::EMPTY;
        s.insert_rect(x, y, x + BLOCK_CELLS, y + BLOCK_CELLS);
        s
    }
}

/// sRGB (0–1) to linear light.
fn srgb_to_linear(s: f32) -> f32 {
    if s <= 0.04045 {
        s / 12.92
    } else {
        ((s + 0.055) / 1.055).powf(2.4)
    }
}

/// A solved thumbnail: the inner tile as straight RGBA8 on the target grid
/// (32 × 32 hi-res, 16 × 32 multicolor; draw multicolor pixels 2:1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tile {
    /// Width in target pixels.
    pub width: u32,
    /// Height in target pixels.
    pub height: u32,
    /// Row-major RGBA8.
    pub rgba: Vec<u8>,
}

/// A solver for thumbnails in one palette, with the project's solve options.
#[derive(Clone, Debug)]
pub struct TileSolver {
    solver: Solver,
    options: SolveOptions,
}

impl TileSolver {
    /// A thumbnail solver for `palette` with `options` (the project's; their mode is
    /// replaced by a full solve). The tile is layer 0 of its frame, so
    /// `options.layer_origins[0]` anchors a layer-locked pattern: pass the selected
    /// layer's origin there.
    pub fn new(palette: &Palette, options: SolveOptions) -> Self {
        Self {
            solver: Solver::new(palette),
            options,
        }
    }

    /// The palette.
    pub const fn palette(&self) -> &Palette {
        self.solver.palette()
    }

    /// The solve options.
    pub const fn options(&self) -> &SolveOptions {
        &self.options
    }

    /// `recipe` with `method` on `source`, solved: see the module docs.
    pub fn render(&self, method: Method, recipe: &Recipe, source: &SourceTile) -> Tile {
        let mode = source.mode;
        let grid = SuperGrid::new(mode, 1).expect("a bitmap mode");
        let mut frame = TargetFrame::empty(grid);
        let recipe = recipe.with_dither_method(method);
        frame.attributes_mut().set_recipes(vec![recipe], Vec::new());
        let (fw, _) = grid.target_size();
        let (cw, ch) = cell_px(mode);
        let (bw, bh) = block_px(mode);
        let (x0, y0) = (
            usize::from(source.origin.x) * cw,
            usize::from(source.origin.y) * ch,
        );
        {
            let (colour, coverage, attrs) = frame.parts_mut();
            for y in 0..bh {
                for x in 0..bw {
                    let i = (y0 + y) * fw as usize + x0 + x;
                    colour[i] = source.colour[y * bw + x];
                    coverage[i] = 1.0;
                    attrs[i].source = AttrSource::Layer(0);
                    attrs[i].strength = recipe.strength();
                }
            }
        }
        if source.prepared {
            frame.set_prepared(None);
        }
        // Every other cell locked to a flat picture in the background.
        let bg = source.background.unwrap_or(ColourIndex::BLACK);
        let base = flat_solution(mode, bg);
        let block = source.block_cells();
        let mut steering = Steering::new();
        for cell in CellSet::all().iter() {
            if !block.contains(cell) {
                steering.set_lock(cell, true);
            }
        }
        let opts = SolveOptions {
            mode: SolveMode::Full,
            ..self.options.clone()
        };
        let sol = self.solver.solve(&frame, &steering, Some(&base), &opts);
        // The inner tile.
        let c = usize::from(CONTEXT_CELLS);
        let (tw, th) = (cw * usize::from(TILE_CELLS), ch * usize::from(TILE_CELLS));
        let (tx, ty) = (x0 + c * cw, y0 + c * ch);
        let palette = self.solver.palette();
        let mut rgba = Vec::with_capacity(tw * th * 4);
        for y in 0..th {
            for x in 0..tw {
                let p = palette.rgb(sol.image.pixel((tx + x) as u16, (ty + y) as u16));
                rgba.extend([p.r, p.g, p.b, 255]);
            }
        }
        Tile {
            width: tw as u32,
            height: th as u32,
            rgba,
        }
    }
}

/// A picture of one colour (the background in multicolor), every cell's own set
/// that colour: what the locked cells of a thumbnail's frame keep.
fn flat_solution(mode: Mode, colour: ColourIndex) -> Solution {
    let indexed = IndexedBitmap::filled(mode, colour).expect("a bitmap mode");
    let image = BitmapImage::encode(&indexed).expect("one colour fits every cell");
    let background = (mode == Mode::MulticolorBitmap).then_some(colour);
    let own = if background.is_some() {
        ColourSet::EMPTY
    } else {
        ColourSet::EMPTY.with(colour)
    };
    let n = usize::from(SCREEN_CELLS.0) * usize::from(SCREEN_CELLS.1);
    Solution {
        image,
        background,
        sets: vec![own; n],
        error: 0.0,
        refined: 0,
        rounds: Vec::new(),
        report: ClashReport { cells: Vec::new() },
        solver: SolverKind::Reference,
        fallback: None,
    }
}

/// `recipe` with `method` on `source` in `mode` and `palette`, with default solve
/// options: one thumbnail (UX §6a). For many, keep a [`TileSolver`]: its palette tables
/// are built once.
///
/// # Panics
/// If `source` was made for another mode.
pub fn method_tile(
    method: Method,
    recipe: &Recipe,
    mode: Mode,
    palette: &Palette,
    source: &SourceTile,
) -> Tile {
    assert_eq!(source.mode, mode, "the source tile's mode");
    TileSolver::new(palette, SolveOptions::default()).render(method, recipe, source)
}

/// A hash of everything but the method that changes a thumbnail: the recipe's other
/// settings, the mode, the palette and the solve options.
pub fn settings_hash(
    recipe: &Recipe,
    mode: Mode,
    palette: &Palette,
    options: &SolveOptions,
) -> u64 {
    let mut h = DefaultHasher::new();
    // The method is the key's own field; the rest of the recipe by its debug form
    // (every field, floats exactly).
    let neutral = recipe.with_dither_method(Method::new(crate::layer::DitherKind::None));
    format!("{neutral:?}").hash(&mut h);
    mode.hash(&mut h);
    format!("{palette:?}").hash(&mut h);
    let o = SolveOptions {
        mode: SolveMode::Full,
        ..options.clone()
    };
    format!("{o:?}").hash(&mut h);
    h.finish()
}

/// A thumbnail's cache key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TileKey {
    /// The method.
    pub method: Method,
    /// [`settings_hash`].
    pub settings: u64,
    /// [`SourceTile::hash`].
    pub source: u64,
}

/// Thumbnails by [`TileKey`], keeping the newest `capacity`.
#[derive(Clone, Debug)]
pub struct TileCache {
    map: HashMap<TileKey, Arc<Tile>>,
    order: VecDeque<TileKey>,
    capacity: usize,
}

impl TileCache {
    /// An empty cache holding at most `capacity` thumbnails.
    pub fn new(capacity: usize) -> Self {
        Self {
            map: HashMap::new(),
            order: VecDeque::new(),
            capacity: capacity.max(1),
        }
    }

    /// The thumbnail for `key`, if cached.
    pub fn get(&self, key: &TileKey) -> Option<&Arc<Tile>> {
        self.map.get(key)
    }

    /// Whether `key` is cached.
    pub fn contains(&self, key: &TileKey) -> bool {
        self.map.contains_key(key)
    }

    /// Caches `tile` under `key`, dropping the oldest entry when full.
    pub fn insert(&mut self, key: TileKey, tile: Arc<Tile>) {
        if self.map.insert(key, tile).is_none() {
            self.order.push_back(key);
        }
        while self.map.len() > self.capacity {
            if let Some(old) = self.order.pop_front() {
                self.map.remove(&old);
            }
        }
    }

    /// Cached thumbnails.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Whether nothing is cached.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

impl Default for TileCache {
    /// Room for every method on two sources in two modes, twice over.
    fn default() -> Self {
        Self::new(512)
    }
}

#[cfg(test)]
mod tests;
