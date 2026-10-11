//! Blend modes on premultiplied linear RGBA (W3C Compositing and Blending Level 1).
//!
//! The general formula `co = cs (1 − αb) + cb (1 − αs) + αs αb B(Cb, Cs)` is expanded
//! per mode in premultiplied terms, so no channel is ever divided by alpha.

use crate::layer::Blend;

/// Composites premultiplied `src` onto premultiplied `dst` in place.
#[inline]
pub(crate) fn blend(mode: Blend, dst: &mut [f32; 4], src: [f32; 4]) {
    let (sa, da) = (src[3], dst[3]);
    if sa <= 0.0 {
        return;
    }
    let inv_sa = 1.0 - sa;
    match mode {
        Blend::Normal => {
            for i in 0..3 {
                dst[i] = src[i] + dst[i] * inv_sa;
            }
        }
        Blend::Multiply => {
            let inv_da = 1.0 - da;
            for i in 0..3 {
                let (cs, cb) = (src[i], dst[i]);
                dst[i] = cs * inv_da + cb * inv_sa + cs * cb;
            }
        }
        Blend::Screen => {
            for i in 0..3 {
                // cs(1−αb) + cb(1−αs) + αs·cb + αb·cs − cs·cb  =  cs + cb − cs·cb
                let (cs, cb) = (src[i], dst[i]);
                dst[i] = cs + cb - cs * cb;
            }
        }
        Blend::Overlay => {
            let inv_da = 1.0 - da;
            for i in 0..3 {
                let (cs, cb) = (src[i], dst[i]);
                // B = HardLight(Cs, Cb) with the backdrop deciding: multiply when
                // Cb ≤ ½ (cb ≤ ½ αb), screen otherwise.
                let mixed = if 2.0 * cb <= da {
                    2.0 * cs * cb
                } else {
                    sa * da - 2.0 * (da - cb) * (sa - cs)
                };
                dst[i] = cs * inv_da + cb * inv_sa + mixed;
            }
        }
    }
    dst[3] = sa + da * inv_sa;
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
