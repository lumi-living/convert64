//! Painting into a mask raster (ADR-008): the soft round brush and the polygon fill.
//! Both report the layer-space rectangle they touched, which drives undo tiles and the
//! solver's dirty cells.

use super::MaskRaster;
use crate::geom::{PxRect, Vec2};
use serde::{Deserialize, Serialize};

/// Whether paint adds weight or removes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PaintMode {
    /// Towards 1.
    #[default]
    Add,
    /// Towards 0.
    Subtract,
}

/// The layer pixels within `[x0, x1) × [y0, y1)` (floats, layer px), clipped to the
/// raster.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "clamped to the raster before casting"
)]
fn clip(r: &MaskRaster, x0: f32, y0: f32, x1: f32, y1: f32) -> PxRect {
    let (w, h) = r.size();
    let c = |v: f32, hi: u32| v.clamp(0.0, hi as f32);
    PxRect::new(
        c(x0.floor(), w) as u32,
        c(y0.floor(), h) as u32,
        c(x1.ceil(), w) as u32,
        c(y1.ceil(), h) as u32,
    )
}

/// The brush footprint at distance `d` from a dab's centre: 1 inside `radius × hardness`,
/// a smoothstep down to 0 at `radius`.
fn falloff(d: f32, radius: f32, hardness: f32) -> f32 {
    let inner = radius * hardness;
    if d <= inner {
        1.0
    } else if d >= radius {
        0.0
    } else {
        let t = (radius - d) / (radius - inner);
        t * t * (3.0 - 2.0 * t)
    }
}

/// Dab centres along a polyline: the first point, then every `spacing` along the path
/// (continuing across segments), then the last point.
fn dabs(points: &[Vec2], spacing: f32) -> Vec<Vec2> {
    let mut out = Vec::new();
    let Some(&first) = points.first() else {
        return out;
    };
    out.push(first);
    let mut carry = 0.0f32; // distance travelled since the last dab
    for w in points.windows(2) {
        let (a, b) = (w[0], w[1]);
        let d = b - a;
        let len = d.x.hypot(d.y);
        if len == 0.0 {
            continue;
        }
        let mut t = spacing - carry;
        while t <= len {
            out.push(a + d * (t / len));
            t += spacing;
        }
        carry = len - (t - spacing);
    }
    if let Some(&last) = points.last()
        && out.last() != Some(&last)
    {
        out.push(last);
    }
    out
}

/// The rectangle a stroke can touch: the points' bounding box grown by `radius`.
pub(crate) fn stroke_bounds(r: &MaskRaster, points: &[Vec2], radius: f32) -> PxRect {
    let (x0, y0, x1, y1) = bbox(points);
    clip(r, x0 - radius, y0 - radius, x1 + radius, y1 + radius)
}

/// The rectangle a fill can touch: the polygon's bounding box.
pub(crate) fn fill_bounds(r: &MaskRaster, polygon: &[Vec2]) -> PxRect {
    let (x0, y0, x1, y1) = bbox(polygon);
    clip(r, x0, y0, x1, y1)
}

fn bbox(points: &[Vec2]) -> (f32, f32, f32, f32) {
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for p in points {
        x0 = x0.min(p.x);
        y0 = y0.min(p.y);
        x1 = x1.max(p.x);
        y1 = y1.max(p.y);
    }
    (x0, y0, x1, y1)
}

/// Strokes a soft round brush along `points` (layer pixels), stamping dabs at most
/// `radius / 4` apart. Each dab adds `flow × falloff` of the remaining headroom
/// (`Add`: `v += (1 − v)·k`; `Subtract`: `v −= v·k`), so flow accumulates where dabs
/// overlap. Returns the touched rectangle.
pub(crate) fn stroke(
    r: &mut MaskRaster,
    points: &[Vec2],
    radius: f32,
    hardness: f32,
    flow: f32,
    mode: PaintMode,
) -> PxRect {
    let spacing = (radius / 4.0).max(0.25);
    let mut touched = PxRect::EMPTY;
    let w = r.width() as usize;
    for c in dabs(points, spacing) {
        let rect = clip(r, c.x - radius, c.y - radius, c.x + radius, c.y + radius);
        for y in rect.y0..rect.y1 {
            for x in rect.x0..rect.x1 {
                let d = (x as f32 + 0.5 - c.x).hypot(y as f32 + 0.5 - c.y);
                let k = flow * falloff(d, radius, hardness);
                if k <= 0.0 {
                    continue;
                }
                let v = &mut r.data[y as usize * w + x as usize];
                *v = match mode {
                    PaintMode::Add => *v + (1.0 - *v) * k,
                    PaintMode::Subtract => *v - *v * k,
                }
                .clamp(0.0, 1.0);
            }
        }
        touched = touched.union(rect);
    }
    touched
}

/// Even-odd point-in-polygon test.
fn inside(poly: &[Vec2], p: Vec2) -> bool {
    let mut odd = false;
    let n = poly.len();
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + 1) % n]);
        if (a.y > p.y) != (b.y > p.y) {
            let x = a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x);
            if p.x < x {
                odd = !odd;
            }
        }
    }
    odd
}

/// Fills `polygon` (layer pixels, even-odd, anti-aliased with 4 × 4 samples per pixel).
/// `Add` raises each pixel to at least `value × coverage`; `Subtract` lowers it to at
/// most `1 − value × coverage`. Repeating a fill changes nothing. Returns the touched
/// rectangle.
pub(crate) fn fill(r: &mut MaskRaster, polygon: &[Vec2], value: f32, mode: PaintMode) -> PxRect {
    if polygon.len() < 3 {
        return PxRect::EMPTY;
    }
    let rect = fill_bounds(r, polygon);
    let w = r.width() as usize;
    for y in rect.y0..rect.y1 {
        for x in rect.x0..rect.x1 {
            let mut hits = 0u32;
            for sy in 0..4 {
                for sx in 0..4 {
                    let p = Vec2::new(
                        x as f32 + (sx as f32 + 0.5) / 4.0,
                        y as f32 + (sy as f32 + 0.5) / 4.0,
                    );
                    hits += u32::from(inside(polygon, p));
                }
            }
            if hits == 0 {
                continue;
            }
            let k = value * hits as f32 / 16.0;
            let v = &mut r.data[y as usize * w + x as usize];
            *v = match mode {
                PaintMode::Add => v.max(k),
                PaintMode::Subtract => v.min(1.0 - k),
            };
        }
    }
    rect
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dabs_are_evenly_spaced_and_end_on_the_last_point() {
        let d = dabs(&[Vec2::new(0.0, 0.0), Vec2::new(10.0, 0.0)], 2.5);
        let xs: Vec<f32> = d.iter().map(|p| p.x).collect();
        assert_eq!(xs, vec![0.0, 2.5, 5.0, 7.5, 10.0]);
        let one = dabs(&[Vec2::new(3.0, 4.0)], 1.0);
        assert_eq!(one, vec![Vec2::new(3.0, 4.0)]);
        // Spacing continues across a corner.
        let c = dabs(&[Vec2::ZERO, Vec2::new(3.0, 0.0), Vec2::new(3.0, 3.0)], 2.0);
        assert_eq!(c[1], Vec2::new(2.0, 0.0));
        assert_eq!(c[2], Vec2::new(3.0, 1.0));
    }

    #[test]
    fn hard_brush_fills_its_disc_and_flow_accumulates() {
        let mut r = MaskRaster::filled(20, 20, 0.0).unwrap();
        let rect = stroke(
            &mut r,
            &[Vec2::new(10.0, 10.0)],
            4.0,
            1.0,
            1.0,
            PaintMode::Add,
        );
        assert_eq!(rect, PxRect::new(6, 6, 14, 14));
        assert_eq!(r.get(10, 10), 1.0);
        assert_eq!(r.get(15, 10), 0.0);
        let mut s = MaskRaster::filled(20, 20, 0.0).unwrap();
        stroke(
            &mut s,
            &[Vec2::new(10.0, 10.0)],
            4.0,
            1.0,
            0.5,
            PaintMode::Add,
        );
        assert_eq!(s.get(10, 10), 0.5);
        stroke(
            &mut s,
            &[Vec2::new(10.0, 10.0)],
            4.0,
            1.0,
            0.5,
            PaintMode::Add,
        );
        assert_eq!(s.get(10, 10), 0.75);
        stroke(
            &mut s,
            &[Vec2::new(10.0, 10.0)],
            4.0,
            1.0,
            1.0,
            PaintMode::Subtract,
        );
        assert_eq!(s.get(10, 10), 0.0);
    }

    #[test]
    fn soft_brush_falls_off() {
        let mut r = MaskRaster::filled(40, 1, 0.0).unwrap();
        stroke(
            &mut r,
            &[Vec2::new(20.0, 0.5)],
            10.0,
            0.0,
            1.0,
            PaintMode::Add,
        );
        let v: Vec<f32> = (20..31).map(|x| r.get(x, 0)).collect();
        assert!(v.windows(2).all(|w| w[1] <= w[0]), "{v:?}");
        assert!(v[0] > 0.9 && v[10] == 0.0, "{v:?}");
    }

    #[test]
    fn polygon_fill_is_anti_aliased_and_idempotent() {
        let mut r = MaskRaster::filled(10, 10, 0.0).unwrap();
        let sq = [
            Vec2::new(2.0, 2.0),
            Vec2::new(6.5, 2.0),
            Vec2::new(6.5, 6.0),
            Vec2::new(2.0, 6.0),
        ];
        let rect = fill(&mut r, &sq, 1.0, PaintMode::Add);
        assert_eq!(rect, PxRect::new(2, 2, 7, 6));
        assert_eq!(r.get(3, 3), 1.0);
        assert_eq!(r.get(6, 3), 0.5);
        assert_eq!(r.get(1, 3), 0.0);
        let snapshot = r.clone();
        fill(&mut r, &sq, 1.0, PaintMode::Add);
        assert_eq!(r, snapshot);
        fill(&mut r, &sq, 1.0, PaintMode::Subtract);
        assert_eq!(r.get(3, 3), 0.0);
        assert_eq!(r.get(6, 3), 0.5);
    }
}
