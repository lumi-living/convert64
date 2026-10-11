//! Blend modes on premultiplied linear RGBA (W3C Compositing and Blending Level 1).
//!
//! The general formula `co = cs (1 − αb) + cb (1 − αs) + αs αb B(Cb, Cs)` is expanded
//! per mode in premultiplied terms, so no channel is ever divided by alpha. A fully
//! transparent source leaves the backdrop exactly unchanged in every mode (each term
//! with `cs` or `αs` vanishes), so the inner loops need no early-out branch and
//! vectorise.

use crate::layer::Blend;

/// Composites premultiplied `src` onto premultiplied `dst` in place.
#[inline]
#[cfg(test)]
pub(crate) fn blend(mode: Blend, dst: &mut [f32; 4], src: [f32; 4]) {
    match mode {
        Blend::Normal => normal(dst, src),
        Blend::Multiply => multiply(dst, src),
        Blend::Screen => screen(dst, src),
        Blend::Overlay => overlay(dst, src),
    }
}

#[inline(always)]
fn normal(dst: &mut [f32; 4], src: [f32; 4]) {
    let sa = src[3];
    let inv_sa = 1.0 - sa;
    for i in 0..3 {
        dst[i] = src[i] + dst[i] * inv_sa;
    }
    dst[3] = sa + dst[3] * inv_sa;
}

#[inline(always)]
fn multiply(dst: &mut [f32; 4], src: [f32; 4]) {
    let (sa, da) = (src[3], dst[3]);
    let (inv_sa, inv_da) = (1.0 - sa, 1.0 - da);
    for i in 0..3 {
        let (cs, cb) = (src[i], dst[i]);
        dst[i] = cs * inv_da + cb * inv_sa + cs * cb;
    }
    dst[3] = sa + da * inv_sa;
}

#[inline(always)]
fn screen(dst: &mut [f32; 4], src: [f32; 4]) {
    let (sa, da) = (src[3], dst[3]);
    for i in 0..3 {
        // cs(1−αb) + cb(1−αs) + αs·cb + αb·cs − cs·cb  =  cs + cb − cs·cb
        let (cs, cb) = (src[i], dst[i]);
        dst[i] = cs + cb - cs * cb;
    }
    dst[3] = sa + da * (1.0 - sa);
}

#[inline(always)]
fn overlay(dst: &mut [f32; 4], src: [f32; 4]) {
    let (sa, da) = (src[3], dst[3]);
    let (inv_sa, inv_da) = (1.0 - sa, 1.0 - da);
    for i in 0..3 {
        let (cs, cb) = (src[i], dst[i]);
        // B = HardLight(Cs, Cb) with the backdrop deciding: multiply when
        // Cb ≤ ½ (cb ≤ ½ αb), screen otherwise.
        // Both arms computed, then selected: branch-free, so the loop vectorises.
        let multiply = 2.0 * cs * cb;
        let screen = sa * da - 2.0 * (da - cb) * (sa - cs);
        let mixed = if 2.0 * cb <= da { multiply } else { screen };
        dst[i] = cs * inv_da + cb * inv_sa + mixed;
    }
    dst[3] = sa + da * inv_sa;
}

/// What one layer writes into the attribute map where it is visible.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Claim {
    /// The layer's attribute code.
    pub code: u16,
    /// Recipe strength × opacity (#99: a 30 %-opaque layer fades its dither as a
    /// feathered edge does).
    pub scale: f32,
}

/// One super-grid row's attribute planes, for [`span`].
pub(crate) struct AttrRow<'a> {
    pub code: &'a mut [u16],
    pub strength: &'a mut [f32],
}

/// One super-grid row's premultiplied colour, one plane per channel so the blend loops
/// vectorise across pixels.
pub(crate) struct RgbaRow<'a> {
    pub r: &'a mut [f32],
    pub g: &'a mut [f32],
    pub b: &'a mut [f32],
    pub a: &'a mut [f32],
}

impl RgbaRow<'_> {
    /// The pixels `cols` of this row.
    pub fn slice(&mut self, cols: std::ops::Range<usize>) -> RgbaRow<'_> {
        RgbaRow {
            r: &mut self.r[cols.clone()],
            g: &mut self.g[cols.clone()],
            b: &mut self.b[cols.clone()],
            a: &mut self.a[cols],
        }
    }
}

/// `a` if `on`, else `b`, as a bit blend: written as a branch, the compiler turns the
/// store into per-lane conditional stores, which do not vectorise.
#[inline(always)]
fn pick(on: bool, a: u16, b: u16) -> u16 {
    let m = u16::from(on).wrapping_neg();
    (a & m) | (b & !m)
}

/// One layer's contribution to a run of pixels, layers applied **bottom-up**.
///
/// Colour: `src` (premultiplied, coverage applied) scaled by `opacity × mask` and
/// blended onto `dst`.
///
/// Attributes: every pixel whose effective alpha (`alpha × mask`) is above zero takes
/// the layer's code and `scale × alpha × mask`, overwriting what a lower layer wrote, so
/// the top-most visible layer governs. Opacity scales the strength only; whether the
/// layer governs a pixel depends on `alpha × mask` alone.
///
/// Specialised per blend mode and per masked/unmasked layer; the loops are branch-free so
/// they vectorise across pixels. Each pixel gets exactly the arithmetic of the per-pixel
/// blend functions.
pub(crate) fn span(
    mode: Blend,
    dst: RgbaRow<'_>,
    attr: AttrRow<'_>,
    src: [&[f32]; 4],
    mask: Option<&[f32]>,
    opacity: f32,
    claim: Claim,
) {
    #[inline(always)]
    fn run(
        f: impl Fn(&mut [f32; 4], [f32; 4]),
        dst: RgbaRow<'_>,
        attr: AttrRow<'_>,
        src: [&[f32]; 4],
        mask: Option<&[f32]>,
        opacity: f32,
        claim: Claim,
    ) {
        let n = src[3].len();
        let px = dst.r[..n]
            .iter_mut()
            .zip(&mut dst.g[..n])
            .zip(dst.b[..n].iter_mut().zip(&mut dst.a[..n]))
            .zip(attr.code[..n].iter_mut().zip(&mut attr.strength[..n]))
            .zip(src[0][..n].iter().zip(&src[1][..n]))
            .zip(src[2][..n].iter().zip(&src[3][..n]));
        let Claim { code: c, scale } = claim;
        match mask {
            Some(mask) => {
                for ((((((r, g), (b, a)), (cd, st)), (s0, s1)), (s2, s3)), &m) in px.zip(&mask[..n])
                {
                    let k = opacity * m;
                    let mut d = [*r, *g, *b, *a];
                    f(&mut d, [s0 * k, s1 * k, s2 * k, s3 * k]);
                    [*r, *g, *b, *a] = d;
                    let alpha = s3 * m;
                    let on = alpha > 0.0;
                    *cd = pick(on, c, *cd);
                    *st = if on { scale * alpha } else { *st };
                }
            }
            None => {
                for (((((r, g), (b, a)), (cd, st)), (s0, s1)), (s2, s3)) in px {
                    let k = opacity;
                    let mut d = [*r, *g, *b, *a];
                    f(&mut d, [s0 * k, s1 * k, s2 * k, s3 * k]);
                    [*r, *g, *b, *a] = d;
                    let alpha = *s3;
                    let on = alpha > 0.0;
                    *cd = pick(on, c, *cd);
                    *st = if on { scale * alpha } else { *st };
                }
            }
        }
    }
    match mode {
        Blend::Normal => run(normal, dst, attr, src, mask, opacity, claim),
        Blend::Multiply => run(multiply, dst, attr, src, mask, opacity, claim),
        Blend::Screen => run(screen, dst, attr, src, mask, opacity, claim),
        Blend::Overlay => run(overlay, dst, attr, src, mask, opacity, claim),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f32; 4], b: [f32; 4]) -> bool {
        a.iter().zip(&b).all(|(x, y)| (x - y).abs() < 1e-6)
    }

    /// The textbook straight-alpha formula, for opaque pixels.
    fn reference(mode: Blend, cb: f32, cs: f32) -> f32 {
        match mode {
            Blend::Normal => cs,
            Blend::Multiply => cs * cb,
            Blend::Screen => cb + cs - cb * cs,
            Blend::Overlay => {
                if cb <= 0.5 {
                    2.0 * cs * cb
                } else {
                    1.0 - 2.0 * (1.0 - cs) * (1.0 - cb)
                }
            }
        }
    }

    #[test]
    fn opaque_pixels_match_the_textbook_formulas() {
        for mode in [
            Blend::Normal,
            Blend::Multiply,
            Blend::Screen,
            Blend::Overlay,
        ] {
            for cb in [0.0, 0.2, 0.5, 0.7, 1.0] {
                for cs in [0.0, 0.3, 0.6, 1.0] {
                    let mut d = [cb, cb, cb, 1.0];
                    blend(mode, &mut d, [cs, cs, cs, 1.0]);
                    let want = reference(mode, cb, cs);
                    assert!((d[0] - want).abs() < 1e-6, "{mode:?} {cb} {cs}: {}", d[0]);
                    assert_eq!(d[3], 1.0);
                }
            }
        }
    }

    #[test]
    fn hand_computed_partial_alpha() {
        // Backdrop 0.8 grey at α 0.5 (premul 0.4); source 0.2 at α 0.5 (premul 0.1).
        let src = [0.1, 0.1, 0.1, 0.5];
        let bd = [0.4, 0.4, 0.4, 0.5];
        let mut d = bd;
        blend(Blend::Normal, &mut d, src);
        assert!(close(d, [0.3, 0.3, 0.3, 0.75]), "{d:?}");
        // Multiply: 0.1·0.5 + 0.4·0.5 + 0.1·0.4 = 0.29.
        let mut d = bd;
        blend(Blend::Multiply, &mut d, src);
        assert!(close(d, [0.29, 0.29, 0.29, 0.75]), "{d:?}");
        // Screen: 0.1 + 0.4 − 0.04 = 0.46.
        let mut d = bd;
        blend(Blend::Screen, &mut d, src);
        assert!(close(d, [0.46, 0.46, 0.46, 0.75]), "{d:?}");
        // Overlay, Cb = 0.8 > ½: 0.05 + 0.2 + (0.25 − 2·0.1·0.4) = 0.42.
        let mut d = bd;
        blend(Blend::Overlay, &mut d, src);
        assert!(close(d, [0.42, 0.42, 0.42, 0.75]), "{d:?}");
    }

    #[test]
    fn transparent_source_is_a_no_op_and_empty_backdrop_takes_the_source() {
        for mode in [
            Blend::Normal,
            Blend::Multiply,
            Blend::Screen,
            Blend::Overlay,
        ] {
            let mut d = [0.3, 0.2, 0.1, 0.6];
            blend(mode, &mut d, [0.0; 4]);
            assert_eq!(d, [0.3, 0.2, 0.1, 0.6]);
            let mut e = [0.0; 4];
            blend(mode, &mut e, [0.25, 0.5, 0.125, 0.5]);
            assert!(close(e, [0.25, 0.5, 0.125, 0.5]), "{mode:?} {e:?}");
        }
    }
}
