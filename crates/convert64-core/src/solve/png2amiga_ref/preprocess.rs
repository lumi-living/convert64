//! Preprocessing, ported from png2amiga's `preprocess.cpp`.
//!
//! What differs from png2c64's (`png2c64_ref::preprocess`):
//!
//! - **Gamma defaults to 1** (png2c64: 1.5). Convert64's own default stays 1.5; this
//!   port's [`Settings::default`] is png2amiga's.
//! - **The OKLab conversions are png2amiga's** (its fast cube root and fused
//!   multiply-adds, [`super::colour`]), so the same steps give slightly different bits.
//!   The steps themselves (gamma, the 3 × 3 sharpen/blur on L, levels, brightness,
//!   contrast, saturation, hue) are png2c64's operation for operation; the L-plane
//!   arithmetic of the sharpen is shared ([`p2c::unsharp_box`]).
//! - **`match_palette_range` is new**: a per-(lightness, hue) chroma stretch onto the
//!   palette's reachable hull ([`match_palette_range`]), where png2c64 stretches each
//!   OKLab axis to the palette's bounding box.

use super::super::png2c64_ref::preprocess as p2c;
use super::Image;
use super::colour::{Lab, atan2f, clamped, cosf, linear_to_oklab, oklab_to_linear, powf, sinf};

/// png2amiga's `preprocess::Settings`, with its defaults (gamma 1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    /// Added to OKLab L, `-1..=1`.
    pub brightness: f32,
    /// Multiplies L around 0.5, `0..=3`.
    pub contrast: f32,
    /// Multiplies chroma, `0..=3`.
    pub saturation: f32,
    /// Power applied to each linear channel first, `0.1..=8` (png2amiga's default 1).
    pub gamma: f32,
    /// Hue rotation in degrees, `-180..=180`.
    pub hue_shift: f32,
    /// Positive: 3 × 3 unsharp mask on L; negative: blend toward a 3-pass box blur.
    pub sharpen: f32,
    /// Fraction of L clipped at the dark end, `0..=0.5`.
    pub black_point: f32,
    /// Fraction of L clipped at the light end, `0..=0.5`.
    pub white_point: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            brightness: 0.0,
            contrast: 1.0,
            saturation: 1.0,
            gamma: 1.0,
            hue_shift: 0.0,
            sharpen: 0.0,
            black_point: 0.0,
            white_point: 0.0,
        }
    }
}

/// The gamma step: every channel to the power `gamma` (skipped at 1), a negative value
/// first raised to 0. png2c64's, literally.
pub fn apply_gamma(image: &mut Image, gamma: f32) {
    if gamma != 1.0 {
        for p in &mut image.px {
            *p = p.map(|v| powf(if v < 0.0 { 0.0 } else { v }, gamma));
        }
    }
}

/// `apply_sharpen`, with png2amiga's OKLab conversions.
pub(crate) fn apply_sharpen(image: &mut Image, strength: f32) {
    if strength.abs() < 0.01 {
        return;
    }
    let (w, h) = (image.w, image.h);
    let mut l: Vec<f32> = image.px.iter().map(|&p| linear_to_oklab(p)[0]).collect();
    p2c::unsharp_box(&mut l, None, (w, h), 1, strength, 0.0);
    for (p, &lv) in image.px.iter_mut().zip(&l) {
        let mut lab = linear_to_oklab(*p);
        lab[0] = lv.clamp(0.0, 1.0);
        *p = clamped(oklab_to_linear(lab));
    }
}

/// `apply_levels`.
fn apply_levels(image: &mut Image, black_point: f32, white_point: f32) {
    if black_point <= 0.0 && white_point <= 0.0 {
        return;
    }
    let lo = black_point;
    let mut hi = 1.0 - white_point;
    if hi <= lo {
        hi = lo + 0.01;
    }
    for p in &mut image.px {
        let mut lab = linear_to_oklab(*p);
        lab[0] = ((lab[0] - lo) / (hi - lo)).clamp(0.0, 1.0);
        *p = clamped(oklab_to_linear(lab));
    }
}

/// `preprocess::apply`: gamma, sharpen, levels, then brightness, contrast, saturation and
/// hue in OKLab. Every pixel goes through the OKLab round trip and the clamp even at the
/// default settings, as in png2c64.
pub fn apply(image: &mut Image, s: &Settings) {
    apply_gamma(image, s.gamma);
    apply_sharpen(image, s.sharpen);
    apply_levels(image, s.black_point, s.white_point);
    let hue = s.hue_shift * (std::f32::consts::PI / 180.0);
    let (cos_h, sin_h) = (cosf(hue), sinf(hue));
    let do_hue = s.hue_shift.abs() > 0.1;
    for p in &mut image.px {
        let mut lab = linear_to_oklab(*p);
        lab[0] += s.brightness;
        lab[0] = (lab[0] - 0.5) * s.contrast + 0.5;
        lab[1] *= s.saturation;
        lab[2] *= s.saturation;
        if do_hue {
            let a2 = lab[1] * cos_h - lab[2] * sin_h;
            let b2 = lab[1] * sin_h + lab[2] * cos_h;
            lab[1] = a2;
            lab[2] = b2;
        }
        *p = clamped(oklab_to_linear(lab));
    }
}

/// The (lightness, hue) grid's size: `kL × kH`.
const KL: usize = 64;
const KH: usize = 64;

type Grid = [[f32; KH]; KL];

/// The palette's reachable chroma per (L, hue) bin: for each lightness, the palette
/// colours within one bin of it and every pairwise mix crossing it (plus grey), and per
/// hue the furthest projection onto that hue's direction.
fn palette_hull(pal_lab: &[Lab]) -> Box<Grid> {
    let k_pi = std::f32::consts::PI;
    let mut c_pal = Box::new([[0.0f32; KH]; KL]);
    for (lb, row) in c_pal.iter_mut().enumerate() {
        let lt = lb as f32 / (KL - 1) as f32;
        let mut cand: Vec<(f32, f32)> = Vec::new();
        for p in pal_lab {
            if (p[0] - lt).abs() < 1.0 / KL as f32 {
                cand.push((p[1], p[2]));
            }
        }
        for i in 0..pal_lab.len() {
            for j in i + 1..pal_lab.len() {
                let (li, lj) = (pal_lab[i][0], pal_lab[j][0]);
                if (li - lt) * (lj - lt) > 0.0 {
                    continue;
                }
                let dl = lj - li;
                if dl.abs() < 1e-6 {
                    continue;
                }
                let t = (lt - li) / dl;
                let a = pal_lab[i][1] + t * (pal_lab[j][1] - pal_lab[i][1]);
                let b = pal_lab[i][2] + t * (pal_lab[j][2] - pal_lab[i][2]);
                cand.push((a, b));
            }
        }
        cand.push((0.0, 0.0));
        for (hb, cell) in row.iter_mut().enumerate() {
            let h = (hb as f32 / KH as f32) * 2.0 * k_pi;
            let (ch, sh) = (cosf(h), sinf(h));
            let mut best = 0.0f32;
            for &(a, b) in &cand {
                let p = a * ch + b * sh;
                if p > best {
                    best = p;
                }
            }
            *cell = best;
        }
    }
    c_pal
}

/// A pixel's chroma, its hue in `0..2π`, or `None` below 1e-6 chroma.
fn chroma_hue(lab: Lab) -> Option<(f32, f32)> {
    let c = (lab[1] * lab[1] + lab[2] * lab[2]).sqrt();
    if c < 1e-6 {
        return None;
    }
    let mut h = atan2f(lab[2], lab[1]);
    if h < 0.0 {
        h += 2.0 * std::f32::consts::PI;
    }
    Some((c, h))
}

/// The 95th-percentile source chroma per (L, hue) bin, box-diffused twice so sparse
/// bins borrow from their neighbours.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "bins are clamped to the grid"
)]
fn source_chroma(img_lab: &[Lab]) -> Box<Grid> {
    let k_pi = std::f32::consts::PI;
    let mut bins: Vec<Vec<f32>> = vec![Vec::new(); KL * KH];
    for &lab in img_lab {
        let Some((c, h)) = chroma_hue(lab) else {
            continue;
        };
        let lb = ((lab[0].clamp(0.0, 1.0) * (KL - 1) as f32) as i32).clamp(0, KL as i32 - 1);
        let hb = (((h / (2.0 * k_pi)) * KH as f32) as i32) % KH as i32;
        bins[lb as usize * KH + hb as usize].push(c);
    }
    let mut c_src = Box::new([[0.0f32; KH]; KL]);
    for (i, v) in bins.iter_mut().enumerate() {
        if v.is_empty() {
            continue;
        }
        let mut idx = (v.len() as f32 * 0.95) as usize;
        if idx >= v.len() {
            idx = v.len() - 1;
        }
        // `std::nth_element`: the value is the idx-th smallest, whatever the order.
        let (_, nth, _) = v.select_nth_unstable_by(idx, f32::total_cmp);
        c_src[i / KH][i % KH] = *nth;
    }
    for _ in 0..2 {
        let mut tmp = Box::new([[0.0f32; KH]; KL]);
        for lb in 0..KL {
            for hb in 0..KH {
                let (mut s, mut n) = (0.0f32, 0);
                for dh in [-1i64, 0, 1] {
                    let hn = (hb as i64 + dh + KH as i64) as usize % KH;
                    let v = c_src[lb][hn];
                    if v > 0.0 {
                        s += v;
                        n += 1;
                    }
                }
                tmp[lb][hb] = if n > 0 { s / n as f32 } else { c_src[lb][hb] };
            }
        }
        c_src = tmp;
        let mut tmp2 = Box::new([[0.0f32; KH]; KL]);
        for lb in 0..KL {
            for hb in 0..KH {
                let (mut s, mut n) = (0.0f32, 0);
                for dl in [-1i64, 0, 1] {
                    let ln = (lb as i64 + dl).clamp(0, KL as i64 - 1) as usize;
                    let v = c_src[ln][hb];
                    if v > 0.0 {
                        s += v;
                        n += 1;
                    }
                }
                tmp2[lb][hb] = if n > 0 { s / n as f32 } else { c_src[lb][hb] };
            }
        }
        c_src = tmp2;
    }
    c_src
}

/// `match_palette_range` (png2amiga's): every pixel's chroma scaled by
/// `c_pal(L, h) / c_src(L, h)` (bilinear in the grid), clipped at `c_pal`, hue and L kept.
/// png2amiga ignores its `percentile` and `margin` arguments; this port drops them.
///
/// `amount` (Convert64's, 0–1) blends each changed pixel's chroma between the source's
/// (0) and the matched (1): `k = 1 + (k_match − 1) · amount`. At 1 the result is
/// png2amiga's bit for bit. Pixels png2amiga leaves alone (no chroma, or no
/// source/palette chroma in their bin) are left alone at any amount.
pub fn match_palette_range(image: &mut Image, palette: &[[f32; 3]], amount: f32) {
    if palette.len() < 2 || image.px.is_empty() {
        return;
    }
    let pal_lab: Vec<Lab> = palette.iter().map(|&c| linear_to_oklab(c)).collect();
    let c_pal = palette_hull(&pal_lab);
    let img_lab: Vec<Lab> = image.px.iter().map(|&p| linear_to_oklab(p)).collect();
    let c_src = source_chroma(&img_lab);
    for (p, &lab) in image.px.iter_mut().zip(&img_lab) {
        if let Some(k) = stretch(lab, &c_pal, &c_src) {
            let k = if amount == 1.0 {
                k
            } else {
                1.0 + (k - 1.0) * amount
            };
            let mut lab = lab;
            lab[1] *= k;
            lab[2] *= k;
            *p = clamped(oklab_to_linear(lab));
        }
    }
}

/// The factor png2amiga multiplies a pixel's a and b by, or `None` where it leaves the
/// pixel alone.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "bins are clamped to the grid"
)]
fn stretch(lab: Lab, c_pal: &Grid, c_src: &Grid) -> Option<f32> {
    let k_pi = std::f32::consts::PI;
    let (c, h) = chroma_hue(lab)?;
    let lf = lab[0].clamp(0.0, 1.0) * (KL - 1) as f32;
    let hf = (h / (2.0 * k_pi)) * KH as f32;
    let l0 = (lf as i32).clamp(0, KL as i32 - 2) as usize;
    let l1 = l0 + 1;
    let lt = lf - l0 as f32;
    let h0 = (hf as i32 % KH as i32) as usize;
    let h1 = (h0 + 1) % KH;
    let ht = hf - hf.floor();
    let interp = |g: &Grid| {
        let (v00, v01) = (g[l0][h0], g[l0][h1]);
        let (v10, v11) = (g[l1][h0], g[l1][h1]);
        (1.0 - lt) * ((1.0 - ht) * v00 + ht * v01) + lt * ((1.0 - ht) * v10 + ht * v11)
    };
    let c_p = interp(c_pal);
    let c_s = interp(c_src);
    if c_s < 1e-4 || c_p < 1e-6 {
        return None;
    }
    let scale = c_p / c_s;
    let mut new_c = c * scale;
    if new_c > c_p {
        new_c = c_p;
    }
    Some(new_c / c)
}

#[cfg(test)]
mod tests {
    use super::super::colour::{PEPTO_HEX, palette_linear};
    use super::*;

    #[test]
    fn match_range_keeps_grey_and_lightness() {
        let mut im = Image::new(
            3,
            1,
            vec![[0.2, 0.2, 0.2], [0.3, 0.1, 0.05], [0.05, 0.1, 0.3]],
        );
        let before: Vec<Lab> = im.px.iter().map(|&p| linear_to_oklab(p)).collect();
        match_palette_range(&mut im, &palette_linear(&PEPTO_HEX), 1.0);
        assert_eq!(im.px[0], [0.2, 0.2, 0.2], "grey is left alone");
        for (p, b) in im.px.iter().zip(&before).skip(1) {
            let a = linear_to_oklab(*p);
            assert!((a[0] - b[0]).abs() < 2e-3, "lightness kept: {a:?} vs {b:?}");
        }
    }

    #[test]
    fn amount_zero_changes_nothing_but_the_round_trip() {
        let px = vec![[0.3, 0.1, 0.05], [0.05, 0.1, 0.3], [0.1, 0.4, 0.1]];
        let mut a = Image::new(3, 1, px.clone());
        match_palette_range(&mut a, &palette_linear(&PEPTO_HEX), 0.0);
        for (p, q) in a.px.iter().zip(&px) {
            for k in 0..3 {
                assert!((p[k] - q[k]).abs() < 1e-4);
            }
        }
    }
}
