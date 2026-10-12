//! Palette-aware dithers (`docs/reference/dither-methods.md` §4, #115): methods that
//! search the cell's colours for the best *mix* rather than thresholding between the two
//! nearest. All of them are positional (placed by the Bayer 8 matrix, honouring phase,
//! orientation and strength like the ordered methods), mix in linear RGB, where the eye
//! averages a pattern, and score in the pixel's metric.
//!
//! Each returns the colour the pixel shows and the colour (linear RGB) its pattern
//! averages to, which the pass ranks finalists by ([`super::dither::PassResult::seen`]).

use super::input::Px;
use crate::colour::ColourIndex;
use crate::layer::DitherKind;
use crate::palette::luma;

/// How many entries the candidate-list methods build: a quarter of Bayer 8's levels,
/// enough for 6 % steps.
pub const CANDIDATES: usize = 16;

/// Yliluoma's psychovisual penalty for mixing far-apart colours, per unit of their
/// distance (algorithm 1).
pub const YLILUOMA_PENALTY: f32 = 0.1;

/// How much `pair` prefers a same-luma pair, in squared metric distance (about a
/// just-noticeable OKLab difference, 0.045²), on top of the solve's signed pair bias.
pub const PAIR_PREFERENCE: f32 = 0.002;

/// Knoll's error multiplier: how much of the accumulated error each new candidate
/// tries to cancel.
pub const KNOLL_ERROR: f32 = 0.75;

/// What a palette-aware pixel needs besides itself.
pub(crate) struct Ctx<'a> {
    /// The cell's colours the pixel may use (after `max_luma_diff`).
    pub set: &'a [ColourIndex],
    /// The 16 colours in the pixel's metric.
    pub colours: &'a [[f32; 3]; 16],
    /// The 16 colours in linear RGB.
    pub linear: &'a [[f32; 3]; 16],
    /// The 16 colours' OKLab coordinates.
    pub oklab: &'a [[f32; 3]; 16],
    /// Bayer 8 threshold at the pixel, pulled towards ½ by strength.
    pub thr: f32,
    /// Target pixel position (for `pair`'s weave).
    pub x: usize,
    /// Target pixel row.
    pub y: usize,
    /// Multicolor (2:1 pixels): `pair` weaves by scanline.
    pub wide: bool,
    /// The solve's signed same-luma pair bias.
    pub pair_bias: f32,
}

/// Relative luminance of a linear colour, for ordering candidate lists.
fn luminance(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

fn lerp(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [0, 1, 2].map(|i| a[i] + t * (b[i] - a[i]))
}

/// The share of `b` whose linear mix with `a` is nearest `x` (projection, clamped).
fn ratio(x: [f32; 3], a: [f32; 3], b: [f32; 3]) -> f32 {
    let seg = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let len2 = seg[0] * seg[0] + seg[1] * seg[1] + seg[2] * seg[2];
    if len2 <= 0.0 {
        return 0.0;
    }
    let d = [x[0] - a[0], x[1] - a[1], x[2] - a[2]];
    ((d[0] * seg[0] + d[1] * seg[1] + d[2] * seg[2]) / len2).clamp(0.0, 1.0)
}

/// The pixel's distance (its metric) to a linear colour.
fn score(q: &Px, lin: [f32; 3]) -> f32 {
    q.metric
        .distance(q.coords, q.metric.coords(lin.map(|v| v.max(0.0))))
}

/// The colour a palette-aware pixel shows, and the linear colour its pattern averages
/// to. `ctx.set` is non-empty.
pub(crate) fn pick(q: &Px, ctx: &Ctx<'_>) -> (ColourIndex, [f32; 3]) {
    let nearest = super::ordered::nearest(q.metric, q.coords, ctx.set, ctx.colours);
    if ctx.set.len() < 2 || q.strength * q.adapt <= 0.0 {
        return (nearest, ctx.linear[nearest.as_usize()]);
    }
    match q.kind {
        DitherKind::Yliluoma1 => yliluoma1(q, ctx, nearest),
        DitherKind::Yliluoma2 => list_pick(ctx, &yliluoma2(q, ctx)),
        DitherKind::Yliluoma3 => list_pick(ctx, &knoll_list(q, ctx, true)),
        DitherKind::Knoll => list_pick(ctx, &knoll_list(q, ctx, false)),
        DitherKind::Luma => luma_pick(q, ctx, nearest),
        DitherKind::Pair => pair_pick(q, ctx, nearest),
        _ => (nearest, ctx.linear[nearest.as_usize()]),
    }
}

/// Algorithm 1: every pair of set colours with its best ratio in 1/64 steps (the
/// projection, then the better of the two neighbouring steps), plus Yliluoma's penalty;
/// the pixel takes `b` where the threshold is below the ratio.
fn yliluoma1(q: &Px, ctx: &Ctx<'_>, nearest: ColourIndex) -> (ColourIndex, [f32; 3]) {
    let mut best = (
        score(q, ctx.linear[nearest.as_usize()]),
        nearest,
        nearest,
        0.0f32,
    );
    for (i, &a) in ctx.set.iter().enumerate() {
        for &b in &ctx.set[i + 1..] {
            let (la, lb) = (ctx.linear[a.as_usize()], ctx.linear[b.as_usize()]);
            let t = ratio(q.lin, la, lb);
            let spread = q
                .metric
                .distance(ctx.colours[a.as_usize()], ctx.colours[b.as_usize()]);
            for step in [(t * 64.0).floor(), (t * 64.0).ceil()] {
                let r = step / 64.0;
                let s = score(q, lerp(la, lb, r))
                    + YLILUOMA_PENALTY
                        * spread
                        * ((r - 0.5).abs() + 0.5)
                        * f32::from(u8::from(r > 0.0 && r < 1.0));
                if s < best.0 {
                    best = (s, a, b, r);
                }
            }
        }
    }
    let (_, a, b, r) = best;
    let shows = if r > ctx.thr { b } else { a };
    (
        shows,
        lerp(ctx.linear[a.as_usize()], ctx.linear[b.as_usize()], r),
    )
}

/// Algorithm 2: each candidate is the set colour that brings the running linear
/// average closest to the pixel.
fn yliluoma2(q: &Px, ctx: &Ctx<'_>) -> Vec<ColourIndex> {
    let mut sum = [0.0f32; 3];
    let mut list = Vec::with_capacity(CANDIDATES);
    for n in 1..=CANDIDATES {
        let n = n as f32;
        let c = *ctx
            .set
            .iter()
            .min_by(|&&a, &&b| {
                let avg = |c: ColourIndex| {
                    let l = ctx.linear[c.as_usize()];
                    [0, 1, 2].map(|i| (sum[i] + l[i]) / n)
                };
                score(q, avg(a)).total_cmp(&score(q, avg(b)))
            })
            .expect("non-empty set");
        let l = ctx.linear[c.as_usize()];
        sum = [0, 1, 2].map(|i| sum[i] + l[i]);
        list.push(c);
    }
    list
}

/// Knoll's list: each candidate is the colour nearest the pixel plus `KNOLL_ERROR` ×
/// the error accumulated so far, in the metric's space (`knoll`) or in linear light
/// (`yliluoma3`, Yliluoma's gamma-aware variant).
fn knoll_list(q: &Px, ctx: &Ctx<'_>, linear_light: bool) -> Vec<ColourIndex> {
    let mut err = [0.0f32; 3];
    let mut list = Vec::with_capacity(CANDIDATES);
    for _ in 0..CANDIDATES {
        let c = if linear_light {
            let attempt = [0, 1, 2].map(|i| (q.lin[i] + err[i] * KNOLL_ERROR).max(0.0));
            let c =
                super::ordered::nearest(q.metric, q.metric.coords(attempt), ctx.set, ctx.colours);
            let l = ctx.linear[c.as_usize()];
            err = [0, 1, 2].map(|i| err[i] + q.lin[i] - l[i]);
            c
        } else {
            let attempt = [0, 1, 2].map(|i| q.coords[i] + err[i] * KNOLL_ERROR);
            let c = super::ordered::nearest(q.metric, attempt, ctx.set, ctx.colours);
            let m = ctx.colours[c.as_usize()];
            err = [0, 1, 2].map(|i| err[i] + q.coords[i] - m[i]);
            c
        };
        list.push(c);
    }
    list
}

/// A candidate list sorted by luminance (then index), indexed by the threshold.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "thr is in 0..=1"
)]
fn list_pick(ctx: &Ctx<'_>, list: &[ColourIndex]) -> (ColourIndex, [f32; 3]) {
    let mut sorted = list.to_vec();
    sorted.sort_by(|a, b| {
        luminance(ctx.linear[a.as_usize()])
            .total_cmp(&luminance(ctx.linear[b.as_usize()]))
            .then(a.cmp(b))
    });
    let n = sorted.len();
    let i = ((ctx.thr * n as f32) as usize).min(n - 1);
    let avg = sorted.iter().fold([0.0f32; 3], |acc, c| {
        let l = ctx.linear[c.as_usize()];
        [0, 1, 2].map(|k| acc[k] + l[k] / n as f32)
    });
    (sorted[i], avg)
}

/// `luma`: the nearest colour and, on the other side of the pixel's OKLab L, the set
/// colour closest to it in hue (OKLab a, b); the ratio is taken on luminance.
fn luma_pick(q: &Px, ctx: &Ctx<'_>, nearest: ColourIndex) -> (ColourIndex, [f32; 3]) {
    let l = super::metric::Space::OkLab.coords(q.lin)[0];
    let n = ctx.oklab[nearest.as_usize()];
    let above = n[0] < l;
    let partner = ctx
        .set
        .iter()
        .copied()
        .filter(|&c| {
            let lc = ctx.oklab[c.as_usize()][0];
            if above { lc > l } else { lc < l }
        })
        .min_by(|&a, &b| {
            let hue = |c: ColourIndex| {
                let o = ctx.oklab[c.as_usize()];
                (o[1] - n[1]) * (o[1] - n[1]) + (o[2] - n[2]) * (o[2] - n[2])
            };
            hue(a).total_cmp(&hue(b)).then(
                (ctx.oklab[a.as_usize()][0] - l)
                    .abs()
                    .total_cmp(&(ctx.oklab[b.as_usize()][0] - l).abs()),
            )
        });
    let Some(p) = partner else {
        return (nearest, ctx.linear[nearest.as_usize()]);
    };
    let (yn, yp) = (
        luminance(ctx.linear[nearest.as_usize()]),
        luminance(ctx.linear[p.as_usize()]),
    );
    let t = if (yp - yn).abs() > 0.0 {
        ((luminance(q.lin) - yn) / (yp - yn)).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let shows = if t > ctx.thr { p } else { nearest };
    (
        shows,
        lerp(ctx.linear[nearest.as_usize()], ctx.linear[p.as_usize()], t),
    )
}

/// `pair`: the two set colours whose even mix is nearest, a same-luma pair scored
/// `pair_bias − PAIR_PREFERENCE` lower; used only if it beats the nearest colour alone.
/// The weave alternates by scanline on multicolor (PAL's delay line blends successive
/// lines) and as a checker on hi-res. Strength below ½ keeps the nearest colour.
fn pair_pick(q: &Px, ctx: &Ctx<'_>, nearest: ColourIndex) -> (ColourIndex, [f32; 3]) {
    let alone = (score(q, ctx.linear[nearest.as_usize()]), nearest, nearest);
    let mut best = alone;
    for (i, &a) in ctx.set.iter().enumerate() {
        for &b in &ctx.set[i + 1..] {
            let mix = lerp(ctx.linear[a.as_usize()], ctx.linear[b.as_usize()], 0.5);
            let same = luma::level(a) == luma::level(b);
            let s = score(q, mix)
                + if same {
                    ctx.pair_bias - PAIR_PREFERENCE
                } else {
                    0.0
                };
            if s < best.0 {
                best = (s, a, b);
            }
        }
    }
    let (_, a, b) = best;
    if a == b || q.strength * q.adapt < 0.5 {
        return (nearest, ctx.linear[nearest.as_usize()]);
    }
    let odd = if ctx.wide {
        ctx.y % 2 == 1
    } else {
        (ctx.x + ctx.y) % 2 == 1
    };
    let mix = lerp(ctx.linear[a.as_usize()], ctx.linear[b.as_usize()], 0.5);
    (if odd { b } else { a }, mix)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::{Recipe, SourceImage};
    use crate::palette::Palette;
    use crate::solve::input::{Input, InputOptions};
    use crate::solve::metric::{MetricTables, Space};
    use crate::{Compositor, SuperGrid};
    use std::sync::Arc;

    /// A flat linear colour `v` as a hi-res frame dithered with `kind` at strength 1.
    fn flat(kind: DitherKind, v: [f32; 3]) -> (Input, MetricTables) {
        let px = vec![[v[0], v[1], v[2], 1.0]; 320 * 200];
        let mut l = crate::layer::Layer::new(
            "grey",
            Arc::new(SourceImage::from_linear(320, 200, px).unwrap()),
        );
        l.set_recipe(Recipe::with_method(kind, 1.0).unwrap());
        let mut c = Compositor::new(SuperGrid::new(crate::mode::Mode::HiresBitmap, 1).unwrap());
        c.update(&[l], &[]);
        let t = MetricTables::new(&Palette::pepto_pal());
        (Input::new(c.frame(), &t, InputOptions::default()), t)
    }

    fn picks(kind: DitherKind, v: [f32; 3], set: &[ColourIndex]) -> Vec<ColourIndex> {
        let (input, t) = flat(kind, v);
        let linear = ColourIndex::ALL.map(|c| {
            let l = t.palette().linear(c);
            [l.r, l.g, l.b]
        });
        let q = &input.px[0];
        (0..64)
            .map(|i| {
                let thr = crate::solve::ordered::threshold(
                    DitherKind::Bayer8,
                    crate::layer::Orientation::Auto,
                    i % 8,
                    i / 8,
                )
                .unwrap();
                let ctx = Ctx {
                    set,
                    colours: t.colours(q.metric.space()),
                    linear: &linear,
                    oklab: t.colours(Space::OkLab),
                    thr,
                    x: (i % 8) as usize,
                    y: (i / 8) as usize,
                    wide: false,
                    pair_bias: 0.0,
                };
                pick(q, &ctx).0
            })
            .collect()
    }

    #[test]
    fn luma_never_mixes_in_a_hue_the_grey_does_not_have() {
        // Grey between dark and mid grey, with red (same luma as dark grey) and light
        // blue (same luma as mid grey) also in the set: luma dithers greys only.
        use ColourIndex as C;
        let set = [C::DARK_GREY, C::RED, C::MEDIUM_GREY, C::LIGHT_BLUE];
        let out = picks(DitherKind::Luma, [0.06; 3], &set);
        assert!(
            out.iter()
                .all(|c| matches!(*c, C::DARK_GREY | C::MEDIUM_GREY)),
            "{out:?}"
        );
        assert!(
            out.contains(&C::DARK_GREY) && out.contains(&C::MEDIUM_GREY),
            "{out:?}"
        );
    }

    #[test]
    fn candidate_lists_average_to_the_pixel() {
        // Linear 0.25 from black and white. Mixing in linear light gives a quarter of
        // the positions white; Knoll averages in OKLab (L 0.63), so about 63 %.
        use ColourIndex as C;
        for (kind, want) in [
            (DitherKind::Yliluoma1, 16),
            (DitherKind::Yliluoma2, 16),
            (DitherKind::Yliluoma3, 16),
            (DitherKind::Knoll, 40),
        ] {
            let out = picks(kind, [0.25; 3], &[C::BLACK, C::WHITE]);
            let whites = out.iter().filter(|&&c| c == C::WHITE).count();
            assert!(whites.abs_diff(want) <= 6, "{kind:?}: {whites} of 64");
        }
    }

    #[test]
    fn pair_weaves_a_same_luma_pair() {
        // The even mix of grey and light blue (one luma level) is the pixel itself.
        use ColourIndex as C;
        let t = MetricTables::new(&Palette::pepto_pal());
        let (g, b) = (
            t.palette().linear(C::MEDIUM_GREY),
            t.palette().linear(C::LIGHT_BLUE),
        );
        let mid = [(g.r + b.r) / 2.0, (g.g + b.g) / 2.0, (g.b + b.b) / 2.0];
        let out = picks(
            DitherKind::Pair,
            mid,
            &[C::MEDIUM_GREY, C::LIGHT_BLUE, C::BLACK],
        );
        assert!(
            out.iter()
                .all(|c| matches!(*c, C::MEDIUM_GREY | C::LIGHT_BLUE)),
            "{out:?}"
        );
        // A checker on hi-res.
        assert_ne!(out[0], out[1]);
        assert_eq!(out[0], out[9]);
    }
}
