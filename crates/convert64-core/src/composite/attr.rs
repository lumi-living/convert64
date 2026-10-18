//! The attribute map (ADR-002, `docs/TAD.md` §3.5): which recipe governs each pixel and
//! how strongly.
//!
//! On the super grid every pixel records the top-most layer whose effective alpha
//! (PNG alpha × mask, ADR-008) is above zero, with that layer's strength multiplied by
//! the alpha, so dither fades across a feather. Regions then override with the same rule.
//! [`AttributeMap`] is that map box-filtered to the target grid.

use crate::geom::SuperGrid;
use crate::layer::{DitherKind, Recipe};
use crate::palette::{LinearRgb, Oklch, linear_to_srgb8};

/// Which layer or region governs a pixel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AttrSource {
    /// Nothing covers it.
    None,
    /// Layer `i` of the stack (0 = bottom).
    Layer(u16),
    /// Region `i` (0 = first applied).
    Region(u16),
}

const NONE: u16 = u16::MAX;
const REGION_BIT: u16 = 0x8000;

impl AttrSource {
    pub(crate) const fn code(self) -> u16 {
        match self {
            Self::None => NONE,
            Self::Layer(i) => i,
            Self::Region(i) => REGION_BIT | i,
        }
    }

    pub(crate) const fn from_code(c: u16) -> Self {
        if c == NONE {
            Self::None
        } else if c & REGION_BIT != 0 {
            Self::Region(c & !REGION_BIT)
        } else {
            Self::Layer(c)
        }
    }
}

/// The code of a super pixel nothing covers.
pub(crate) const UNCLAIMED: u16 = NONE;

/// The attribute map on the super grid, one plane per field so the fold's loops
/// vectorise: per super pixel the source code and its strength.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AttrPlanes {
    pub code: Vec<u16>,
    pub strength: Vec<f32>,
}

impl AttrPlanes {
    /// `len` unclaimed super pixels.
    pub fn new(len: usize) -> Self {
        Self {
            code: vec![UNCLAIMED; len],
            strength: vec![0.0; len],
        }
    }
}

/// One target pixel of the map.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TargetAttr {
    /// The layer or region whose recipe applies.
    pub source: AttrSource,
    /// Dither strength `0..=1`: the recipe's strength × alpha, averaged over the pixel.
    pub strength: f32,
}

/// The attribute map on the target grid, with the recipes it refers to.
#[derive(Clone, Debug, PartialEq)]
pub struct AttributeMap {
    grid: SuperGrid,
    layers: Vec<Recipe>,
    regions: Vec<Recipe>,
    px: Vec<TargetAttr>,
}

impl AttributeMap {
    /// An empty map (nothing covered) for `grid`.
    pub(crate) fn empty(grid: SuperGrid) -> Self {
        let (tw, th) = grid.target_size();
        Self {
            grid,
            layers: Vec::new(),
            regions: Vec::new(),
            px: vec![
                TargetAttr {
                    source: AttrSource::None,
                    strength: 0.0
                };
                tw as usize * th as usize
            ],
        }
    }

    /// A map from its parts, as [`Self::pixels`], [`Self::layer_recipes`] and
    /// [`Self::region_recipes`] return them (#265: a solve in a web worker).
    ///
    /// # Panics
    /// If `px` is not one entry per target pixel of `grid`.
    pub fn from_parts(
        grid: SuperGrid,
        layers: Vec<Recipe>,
        regions: Vec<Recipe>,
        px: Vec<TargetAttr>,
    ) -> Self {
        let (tw, th) = grid.target_size();
        assert_eq!(
            px.len(),
            tw as usize * th as usize,
            "an attribute map needs one entry per target pixel"
        );
        Self {
            grid,
            layers,
            regions,
            px,
        }
    }

    /// The recipes of [`AttrSource::Layer`] sources, by layer index.
    pub fn layer_recipes(&self) -> &[Recipe] {
        &self.layers
    }

    /// The recipes of [`AttrSource::Region`] sources, by region index.
    pub fn region_recipes(&self) -> &[Recipe] {
        &self.regions
    }

    /// Replaces the recipe tables the sources refer to.
    pub(crate) fn set_recipes(&mut self, layers: Vec<Recipe>, regions: Vec<Recipe>) {
        self.layers = layers;
        self.regions = regions;
    }

    /// The layer and region recipe tables, copied.
    pub(crate) fn recipes(&self) -> (Vec<Recipe>, Vec<Recipe>) {
        (self.layers.clone(), self.regions.clone())
    }

    /// Mutable target pixels, for the compositor's fused fold-and-filter.
    pub(crate) fn pixels_mut(&mut self) -> &mut [TargetAttr] {
        &mut self.px
    }

    /// The grid this map belongs to.
    pub const fn grid(&self) -> SuperGrid {
        self.grid
    }

    /// Row-major target pixels.
    pub fn pixels(&self) -> &[TargetAttr] {
        &self.px
    }

    /// The attribute of target pixel `(x, y)`; panics outside the target.
    pub fn get(&self, x: u32, y: u32) -> TargetAttr {
        let (w, h) = self.grid.target_size();
        assert!(x < w && y < h, "({x}, {y}) is outside the target");
        self.px[(y * w + x) as usize]
    }

    /// The recipe of a source, or `None` for [`AttrSource::None`].
    pub fn recipe(&self, s: AttrSource) -> Option<&Recipe> {
        match s {
            AttrSource::None => None,
            AttrSource::Layer(i) => self.layers.get(usize::from(i)),
            AttrSource::Region(i) => self.regions.get(usize::from(i)),
        }
    }

    /// The recipe and strength governing target pixel `(x, y)`.
    pub fn recipe_at(&self, x: u32, y: u32) -> Option<(&Recipe, f32)> {
        let a = self.get(x, y);
        self.recipe(a.source).map(|r| (r, a.strength))
    }

    /// The false-colour debug overlay (UX §4.2), RGBA8 row-major at target resolution:
    /// hue from the dither method, lightness from the strength, black where nothing
    /// covers the pixel. `None` (no dither) is drawn grey.
    pub fn debug_rgba(&self) -> Vec<[u8; 4]> {
        self.px
            .iter()
            .map(|a| match self.recipe(a.source) {
                None => [0, 0, 0, 255],
                Some(r) => debug_colour(r.method().kind, a.strength),
            })
            .collect()
    }
}

/// Box-filters target pixel `tx` from `codes` and `strengths`, the `n` super rows (each
/// `w` wide) of its target row. The source covering the most super pixels wins (ties go
/// to the higher layer, regions above layers); its strength is summed in scan order and
/// divided by `n²`. `seen` is scratch space, reused between calls.
pub(crate) fn reduce_px(
    codes: &[u16],
    strengths: &[f32],
    w: usize,
    n: u32,
    tx: usize,
    seen: &mut Vec<(u16, u32, f32)>,
) -> TargetAttr {
    let n = n as usize;
    let rows = || (0..n).map(|sy| sy * w + tx * n..sy * w + tx * n + n);
    // Usually one source covers the whole pixel.
    let first = codes[tx * n];
    if rows().all(|r| codes[r].iter().all(|&c| c == first)) {
        if first == NONE {
            return TargetAttr {
                source: AttrSource::None,
                strength: 0.0,
            };
        }
        let mut all = rows().flat_map(|r| &strengths[r]);
        let mut sum = *all.next().expect("n ≥ 1");
        for s in all {
            sum += s;
        }
        return TargetAttr {
            source: AttrSource::from_code(first),
            strength: (sum * (1.0 / (n * n) as f32)).clamp(0.0, 1.0),
        };
    }
    seen.clear();
    for r in rows() {
        for (&code, &strength) in codes[r.clone()].iter().zip(&strengths[r]) {
            if code == NONE {
                continue;
            }
            match seen.iter_mut().find(|e| e.0 == code) {
                Some(e) => {
                    e.1 += 1;
                    e.2 += strength;
                }
                None => seen.push((code, 1, strength)),
            }
        }
    }
    match seen.iter().max_by_key(|e| (e.1, e.0)) {
        Some(&(code, _, sum)) => TargetAttr {
            source: AttrSource::from_code(code),
            strength: (sum * (1.0 / (n * n) as f32)).clamp(0.0, 1.0),
        },
        None => TargetAttr {
            source: AttrSource::None,
            strength: 0.0,
        },
    }
}

/// Target pixels this many at a time in [`reduce_row`]: their sums are independent, so
/// they overlap instead of waiting on one long chain of additions.
const LANES: usize = 4;

/// [`reduce_px`] for the target pixels `txs` of one target row, passing each result to
/// `out`. Pixels one source covers entirely (the common case) are summed `LANES` at a
/// time, each in the same order as in [`reduce_px`] (or, nothing covering them, given
/// no source); the rest go through it.
pub(crate) fn reduce_row(
    codes: &[u16],
    strengths: &[f32],
    w: usize,
    n: u32,
    txs: std::ops::Range<usize>,
    seen: &mut Vec<(u16, u32, f32)>,
    out: impl FnMut(usize, TargetAttr),
) {
    // Specialised for the common factors, so the inner loops have constant bounds.
    match n {
        2 => reduce_row_n(codes, strengths, w, 2, txs, seen, out),
        4 => reduce_row_n(codes, strengths, w, 4, txs, seen, out),
        8 => reduce_row_n(codes, strengths, w, 8, txs, seen, out),
        n => reduce_row_n(codes, strengths, w, n, txs, seen, out),
    }
}

#[inline(always)]
fn reduce_row_n(
    codes: &[u16],
    strengths: &[f32],
    w: usize,
    n: u32,
    txs: std::ops::Range<usize>,
    seen: &mut Vec<(u16, u32, f32)>,
    mut out: impl FnMut(usize, TargetAttr),
) {
    let nn = n as usize;
    let uniform = |tx: usize| {
        let first = codes[tx * nn];
        (0..nn).fold(true, |all, sy| {
            codes[sy * w + tx * nn..][..nn]
                .iter()
                .fold(all, |all, &c| all & (c == first))
        })
    };
    let mut tx = txs.start;
    while tx + LANES <= txs.end {
        if !(0..LANES).all(|l| uniform(tx + l)) {
            for t in tx..tx + LANES {
                out(t, reduce_px(codes, strengths, w, n, t, seen));
            }
            tx += LANES;
            continue;
        }
        // Each lane's sum starts from its first super pixel, as in `reduce_px`.
        let mut acc = [0.0f32; LANES];
        for (l, a) in acc.iter_mut().enumerate() {
            *a = strengths[(tx + l) * nn];
        }
        for sy in 0..nn {
            let row = &strengths[sy * w + tx * nn..][..LANES * nn];
            for sx in usize::from(sy == 0)..nn {
                for (l, a) in acc.iter_mut().enumerate() {
                    *a += row[l * nn + sx];
                }
            }
        }
        for (l, sum) in acc.into_iter().enumerate() {
            let code = codes[(tx + l) * nn];
            let attr = if code == NONE {
                TargetAttr {
                    source: AttrSource::None,
                    strength: 0.0,
                }
            } else {
                TargetAttr {
                    source: AttrSource::from_code(code),
                    strength: (sum * (1.0 / (nn * nn) as f32)).clamp(0.0, 1.0),
                }
            };
            out(tx + l, attr);
        }
        tx += LANES;
    }
    for t in tx..txs.end {
        out(t, reduce_px(codes, strengths, w, n, t, seen));
    }
}

/// The overlay colour for a method at a strength.
pub fn debug_colour(kind: DitherKind, strength: f32) -> [u8; 4] {
    let idx = DitherKind::ALL
        .iter()
        .position(|k| *k == kind)
        .expect("ALL lists every kind");
    let chroma = if kind == DitherKind::None { 0.0 } else { 0.13 };
    let lch = Oklch {
        l: 0.3 + 0.6 * strength.clamp(0.0, 1.0),
        c: chroma,
        h: (idx as f32 * 40.0).to_radians(),
    };
    let rgb = LinearRgb::from_oklab(lch.to_oklab());
    [
        linear_to_srgb8(rgb.r),
        linear_to_srgb8(rgb.g),
        linear_to_srgb8(rgb.b),
        255,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_round_trip() {
        for s in [
            AttrSource::None,
            AttrSource::Layer(0),
            AttrSource::Layer(77),
            AttrSource::Region(3),
        ] {
            assert_eq!(AttrSource::from_code(s.code()), s);
        }
    }

    #[test]
    fn debug_colours_differ_by_method_and_strength() {
        let a = debug_colour(DitherKind::Bayer4, 1.0);
        let b = debug_colour(DitherKind::FloydSteinberg, 1.0);
        let c = debug_colour(DitherKind::Bayer4, 0.2);
        assert_ne!(a, b);
        assert_ne!(a, c);
        let lum = |p: [u8; 4]| u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2]);
        assert!(lum(a) > lum(c));
        let g = debug_colour(DitherKind::None, 0.5);
        assert!(g[0].abs_diff(g[1]) <= 1 && g[1].abs_diff(g[2]) <= 1);
    }
}
