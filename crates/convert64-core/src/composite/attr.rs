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

/// One super pixel of the map.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct AttrPx {
    pub code: u16,
    pub strength: f32,
}

impl AttrPx {
    pub const EMPTY: Self = Self {
        code: NONE,
        strength: 0.0,
    };
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
    /// Box-filters a super-grid map to the target grid. Per target pixel the source
    /// covering the most super pixels wins (ties go to the higher layer, regions above
    /// layers); its strength is summed over its super pixels and divided by `n²`.
    pub(crate) fn reduce(
        grid: SuperGrid,
        sup: &[AttrPx],
        layers: Vec<Recipe>,
        regions: Vec<Recipe>,
    ) -> Self {
        let (tw, th) = grid.target_size();
        let (w, _) = grid.size();
        let n = u32::from(grid.n());
        let inv = 1.0 / (n * n) as f32;
        let mut px = vec![
            TargetAttr {
                source: AttrSource::None,
                strength: 0.0
            };
            tw as usize * th as usize
        ];
        let mut rows: Vec<(usize, &mut [TargetAttr])> =
            px.chunks_mut(tw as usize).enumerate().collect();
        crate::par::for_each(&mut rows, |(ty, row)| {
            // Up to n² distinct codes per pixel; a small linear table beats a hash map.
            let mut seen: Vec<(u16, u32, f32)> = Vec::with_capacity(4);
            for (tx, out) in row.iter_mut().enumerate() {
                seen.clear();
                for sy in 0..n {
                    let base = (*ty as u32 * n + sy) as usize * w as usize + tx * n as usize;
                    for a in &sup[base..base + n as usize] {
                        if a.code == NONE {
                            continue;
                        }
                        match seen.iter_mut().find(|e| e.0 == a.code) {
                            Some(e) => {
                                e.1 += 1;
                                e.2 += a.strength;
                            }
                            None => seen.push((a.code, 1, a.strength)),
                        }
                    }
                }
                if let Some(&(code, _, sum)) = seen.iter().max_by_key(|e| (e.1, e.0)) {
                    *out = TargetAttr {
                        source: AttrSource::from_code(code),
                        strength: (sum * inv).clamp(0.0, 1.0),
                    };
                }
            }
        });
        Self {
            grid,
            layers,
            regions,
            px,
        }
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
