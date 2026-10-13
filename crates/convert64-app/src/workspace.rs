//! An open document with what the views draw from it: the newest solution and the
//! textures of the source composite and of the result, rebuilt only when they change.
//! The working view and the result panel share the result texture (UX §4.3: no second
//! render).

use crate::document::{Document, result_rgba};
use convert64_core::model::CellPos;
use convert64_core::solve::Solution;
use egui::{Color32, ColorImage, TextureHandle, TextureOptions};
use std::sync::Arc;

/// A cell, column and row.
pub type Cell = (u32, u32);

/// A document and its preview.
pub struct Workspace {
    /// The open document.
    pub doc: Document,
    /// The newest solution, once the first solve is in.
    pub solution: Option<Arc<Solution>>,
    /// The newest solve's composite + solve time, in milliseconds.
    pub solve_ms: Option<f64>,
    /// A full search is running and the solution on screen is provisional or stale.
    pub solving: bool,
    source: Option<(u64, TextureHandle)>,
    result: Option<(*const Solution, TextureHandle)>,
    /// One thumbnail per layer, bottom first, keyed by the source it shows.
    thumbs: Vec<(*const convert64_core::SourceImage, TextureHandle)>,
}

impl std::fmt::Debug for Workspace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Workspace")
            .field("doc", &self.doc)
            .field("solved", &self.solution.is_some())
            .field("solving", &self.solving)
            .finish_non_exhaustive()
    }
}

/// What the status bar says about one cell of the result.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CellInfo {
    /// The cell's colours, background first in multicolor.
    pub colours: Vec<Color32>,
    /// The cell's error (Σ weight × distance), if solved.
    pub error: Option<f32>,
    /// The cell's colour set is pinned.
    pub pinned: bool,
    /// The cell is locked.
    pub locked: bool,
    /// The cell clashes (colours wanted but dropped).
    pub clash: bool,
}

impl Workspace {
    /// A workspace for `doc`, with nothing solved yet.
    pub fn new(doc: Document) -> Self {
        Self {
            doc,
            solution: None,
            solve_ms: None,
            solving: true,
            source: None,
            result: None,
            thumbs: Vec::new(),
        }
    }

    /// Rebuilds whichever texture is out of date. Call once per frame before drawing.
    pub fn prepare(&mut self, ctx: &egui::Context) {
        let rev = self.doc.revision();
        if self.source.as_ref().is_none_or(|(r, _)| *r != rev) {
            let (w, h, rgba) = self.doc.source_rgba();
            let image = ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba);
            // A photograph: smooth when scaled.
            let options = TextureOptions {
                mipmap_mode: Some(egui::TextureFilter::Linear),
                ..TextureOptions::LINEAR
            };
            let tex = ctx.load_texture("source", image, options);
            self.source = Some((rev, tex));
        }
        let layers = &self.doc.session().project().layers;
        let mut thumbs = Vec::with_capacity(layers.len());
        for layer in layers {
            let ptr = Arc::as_ptr(layer.source());
            let tex = match self.thumbs.iter().find(|(p, _)| *p == ptr) {
                Some((_, tex)) => tex.clone(),
                None => {
                    let (w, h, rgba) = thumbnail(layer.source(), THUMB);
                    let image = ColorImage::from_rgba_unmultiplied([w, h], &rgba);
                    ctx.load_texture("layer", image, TextureOptions::LINEAR)
                }
            };
            thumbs.push((ptr, tex));
        }
        self.thumbs = thumbs;
        if let Some(sol) = &self.solution {
            let ptr = Arc::as_ptr(sol);
            if self.result.as_ref().is_none_or(|(p, _)| *p != ptr) {
                let (w, h, rgba) = result_rgba(sol, self.doc.palette(), self.doc.grid());
                let image = ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba);
                // C64 pixels: never smoothed (CLAUDE.md conventions).
                let tex = ctx.load_texture("result", image, TextureOptions::NEAREST);
                self.result = Some((ptr, tex));
            }
        }
    }

    /// Layer `i`'s thumbnail (stack index, bottom first).
    pub fn thumbnail(&self, i: usize) -> Option<&TextureHandle> {
        self.thumbs.get(i).map(|(_, t)| t)
    }

    /// The source composite's texture.
    pub fn source_texture(&self) -> Option<&TextureHandle> {
        self.source.as_ref().map(|(_, t)| t)
    }

    /// The result's texture.
    pub fn result_texture(&self) -> Option<&TextureHandle> {
        self.result.as_ref().map(|(_, t)| t)
    }

    /// The border's colour: the solution's background, black in hi-res (the project has
    /// no border colour of its own yet).
    pub fn border_colour(&self) -> Color32 {
        self.solution
            .as_ref()
            .and_then(|s| s.background)
            .map_or(Color32::BLACK, |bg| rgb(self.doc.palette().rgb(bg)))
    }

    /// Cells that clash in the newest solution, and their count.
    pub fn clashes(&self) -> Option<usize> {
        self.solution.as_ref().map(|s| s.report.summary().clashes)
    }

    /// The worst cell, if any clashes.
    pub fn worst(&self) -> Option<(u32, u32)> {
        let worst = self.solution.as_ref()?.report.summary().worst?;
        Some((u32::from(worst.x), u32::from(worst.y)))
    }

    /// What the status bar shows for `cell`.
    pub fn cell_info(&self, cell: (u32, u32)) -> CellInfo {
        let pos = CellPos {
            x: cell.0 as u16,
            y: cell.1 as u16,
        };
        let steering = &self.doc.session().project().steering;
        let mut info = CellInfo {
            pinned: steering.pin(pos).is_some(),
            locked: steering.locks().contains(pos),
            ..CellInfo::default()
        };
        if let Some(sol) = &self.solution {
            let palette = self.doc.palette();
            let cols = self.doc.grid().cells().0;
            info.colours
                .extend(sol.background.map(|bg| rgb(palette.rgb(bg))));
            if let Some(set) = sol.sets.get((cell.1 * cols + cell.0) as usize) {
                info.colours.extend(set.iter().map(|c| rgb(palette.rgb(c))));
            }
            if let Some(report) = sol.report.cell(pos) {
                info.error = Some(report.error);
                info.clash = report.is_clash();
            }
        }
        info
    }

    /// Every cell's error, row-major, with the largest, for the clash heatmap.
    pub fn cell_errors(&self) -> Option<(Vec<f32>, f32)> {
        let sol = self.solution.as_ref()?;
        let (cols, rows) = self.doc.grid().cells();
        let mut errors = vec![0.0; (cols * rows) as usize];
        for r in &sol.report.cells {
            errors[(u32::from(r.cell.y) * cols + u32::from(r.cell.x)) as usize] = r.error;
        }
        let max = errors.iter().copied().fold(0.0, f32::max);
        Some((errors, max))
    }

    /// Cells that clash (colours wanted but dropped).
    pub fn clash_cells(&self) -> Vec<(u32, u32)> {
        self.solution.as_ref().map_or_else(Vec::new, |s| {
            s.report
                .cells
                .iter()
                .filter(|r| r.is_clash())
                .map(|r| (u32::from(r.cell.x), u32::from(r.cell.y)))
                .collect()
        })
    }

    /// Pinned and locked cells.
    pub fn steered_cells(&self) -> (Vec<Cell>, Vec<Cell>) {
        let steering = &self.doc.session().project().steering;
        let xy = |c: CellPos| (u32::from(c.x), u32::from(c.y));
        (
            steering.pins().map(|(c, _)| xy(c)).collect(),
            steering.locks().iter().map(xy).collect(),
        )
    }
}

fn rgb(c: convert64_core::palette::Rgb8) -> Color32 {
    Color32::from_rgb(c.r, c.g, c.b)
}

/// The longest side of a layer thumbnail's texture, in pixels (drawn smaller, sharp on
/// high-density screens).
const THUMB: u32 = 64;

/// `src` box-filtered to fit `side` × `side`, as sRGB8 RGBA. Width and height first.
fn thumbnail(src: &convert64_core::SourceImage, side: u32) -> (usize, usize, Vec<u8>) {
    use convert64_core::palette::linear_to_srgb8;
    let (w, h) = src.size();
    let s = (side as f32 / w.max(h) as f32).min(1.0);
    let (tw, th) = (
        ((w as f32 * s).round() as u32).max(1),
        ((h as f32 * s).round() as u32).max(1),
    );
    let px = src.pixels();
    let mut out = Vec::with_capacity((tw * th * 4) as usize);
    for ty in 0..th {
        let (y0, y1) = (ty * h / th, ((ty + 1) * h / th).max(ty * h / th + 1));
        for tx in 0..tw {
            let (x0, x1) = (tx * w / tw, ((tx + 1) * w / tw).max(tx * w / tw + 1));
            let mut sum = [0.0f32; 4];
            for y in y0..y1 {
                for x in x0..x1 {
                    let p = px[(y * w + x) as usize];
                    // Straight alpha: weight colour by coverage.
                    for c in 0..3 {
                        sum[c] += p[c] * p[3];
                    }
                    sum[3] += p[3];
                }
            }
            let n = ((y1 - y0) * (x1 - x0)) as f32;
            let a = sum[3] / n;
            let rgb = if sum[3] > 0.0 {
                [0, 1, 2].map(|c| linear_to_srgb8(sum[c] / sum[3]))
            } else {
                [0; 3]
            };
            out.extend([rgb[0], rgb[1], rgb[2], (a * 255.0).round() as u8]);
        }
    }
    (tw as usize, th as usize, out)
}
