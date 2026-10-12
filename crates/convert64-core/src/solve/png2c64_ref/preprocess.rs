//! Preprocessing, ported from png2c64's `preprocess.cpp`: gamma, sharpen/blur, levels,
//! brightness, contrast, saturation and hue in OKLab, and `match_palette_range`.

use super::Image;
use super::colour::{clamped, linear_to_oklab, oklab_to_linear, powf};

/// png2c64's `preprocess::Settings`, with its defaults.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    /// Added to OKLab L, `-1..=1`.
    pub brightness: f32,
    /// Multiplies L around 0.5, `0..=3`.
    pub contrast: f32,
    /// Multiplies chroma, `0..=3`.
    pub saturation: f32,
    /// Power applied to each linear channel first, `0.1..=8` (png2c64's default 1.5).
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
            gamma: 1.5,
            hue_shift: 0.0,
            sharpen: 0.0,
            black_point: 0.0,
            white_point: 0.0,
        }
    }
}

/// `apply_sharpen`.
fn apply_sharpen(image: &mut Image, strength: f32) {
    if strength.abs() < 0.01 {
        return;
    }
    let (w, h) = (image.w, image.h);
    let mut l: Vec<f32> = image.px.iter().map(|&p| linear_to_oklab(p)[0]).collect();
    if strength > 0.0 {
        let orig = l.clone();
        for y in 1..h.saturating_sub(1) {
            for x in 1..w.saturating_sub(1) {
                let mut blur = 0.0f32;
                for dy in 0..3 {
                    for dx in 0..3 {
                        blur += orig[(y + dy - 1) * w + (x + dx - 1)];
                    }
                }
                blur /= 9.0;
                l[y * w + x] += (orig[y * w + x] - blur) * strength;
            }
        }
    } else {
        let amount = (-strength).clamp(0.0, 1.0);
        let orig = l.clone();
        let mut tmp = vec![0.0f32; w * h];
        for _ in 0..3 {
            for y in 0..h {
                for x in 0..w {
                    let (mut sum, mut count) = (0.0f32, 0.0f32);
                    for dy in -1i64..=1 {
                        for dx in -1i64..=1 {
                            let (ny, nx) = (y as i64 + dy, x as i64 + dx);
                            if ny >= 0 && (ny as usize) < h && nx >= 0 && (nx as usize) < w {
                                sum += l[ny as usize * w + nx as usize];
                                count += 1.0;
                            }
                        }
                    }
                    tmp[y * w + x] = sum / count;
                }
            }
            std::mem::swap(&mut l, &mut tmp);
        }
        for (v, &o) in l.iter_mut().zip(&orig) {
            *v = o + (*v - o) * amount;
        }
    }
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
/// hue in OKLab. As png2c64, every pixel goes through the OKLab round trip and the clamp
/// even at the default settings.
pub fn apply(image: &mut Image, s: &Settings) {
    if s.gamma != 1.0 {
        for p in &mut image.px {
            // `std::max(v, 0.0f)`: a negative value becomes 0.
            *p = p.map(|v| powf(if v < 0.0 { 0.0 } else { v }, s.gamma));
        }
    }
    apply_sharpen(image, s.sharpen);
    apply_levels(image, s.black_point, s.white_point);
    let hue = s.hue_shift * (std::f32::consts::PI / 180.0);
    let (cos_h, sin_h) = (hue.cos(), hue.sin());
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

/// `percentile_range`: the values at `percentile` and `1 − percentile` of `vals` sorted.
fn percentile_range(vals: &mut [f32], percentile: f32) -> (f32, f32) {
    vals.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = vals.len();
    let lo = ((n as f32 * percentile) as usize).min(n - 1);
    let hi = ((n as f32 * (1.0 - percentile)) as usize).min(n - 1);
    (vals[lo], vals[hi])
}

/// `match_palette_range`: remaps L from the image's percentile range onto the palette's
/// (shrunk by `margin`), and scales a and b around zero to the palette's extent.
/// png2c64's defaults are `percentile` 0.01 and `margin` 0.05.
pub fn match_palette_range(image: &mut Image, palette: &[[f32; 3]], percentile: f32, margin: f32) {
    if image.px.is_empty() || palette.is_empty() {
        return;
    }
    let mut lo = [1e9f32; 3];
    let mut hi = [-1e9f32; 3];
    for &c in palette {
        let lab = linear_to_oklab(c);
        for k in 0..3 {
            lo[k] = lo[k].min(lab[k]);
            hi[k] = hi[k].max(lab[k]);
        }
    }
    let target = |k: usize| {
        let span = hi[k] - lo[k];
        (lo[k] + span * margin, hi[k] - span * margin)
    };
    let (tl, ta, tb) = (target(0), target(1), target(2));
    let labs: Vec<[f32; 3]> = image.px.iter().map(|&p| linear_to_oklab(p)).collect();
    let chan = |k: usize| {
        let mut v: Vec<f32> = labs.iter().map(|l| l[k]).collect();
        percentile_range(&mut v, percentile)
    };
    let (sl, sa, sb) = (chan(0), chan(1), chan(2));
    let remap = |v: f32, (slo, shi): (f32, f32), (dlo, dhi): (f32, f32)| {
        let span = shi - slo;
        if span < 1e-6 {
            return (dlo + dhi) * 0.5;
        }
        let t = (v - slo) / span;
        dlo + t * (dhi - dlo)
    };
    let around_zero = |v: f32, (slo, shi): (f32, f32), (dlo, dhi): (f32, f32)| {
        if v >= 0.0 {
            v * if shi > 1e-6 { dhi / shi } else { 1.0 }
        } else {
            v * if slo < -1e-6 { dlo / slo } else { 1.0 }
        }
    };
    for (p, lab) in image.px.iter_mut().zip(labs) {
        let out = [
            remap(lab[0], sl, tl),
            around_zero(lab[1], sa, ta),
            around_zero(lab[2], sb, tb),
        ];
        *p = clamped(oklab_to_linear(out));
    }
}

#[cfg(test)]
mod tests {
    use super::super::colour::{PEPTO_HEX, palette_linear, srgb8_to_linear};
    use super::super::testdata as t;
    use super::*;

    #[test]
    fn gamma_and_palette_range_match_png2c64_bit_for_bit() {
        let px = t::PRE_IN
            .iter()
            .map(|&[r, g, b]| srgb8_to_linear(r, g, b))
            .collect();
        let mut im = Image::new(4, 2, px);
        apply(&mut im, &Settings::default());
        for (i, p) in im.px.iter().enumerate() {
            assert_eq!(p.map(f32::to_bits), t::PRE_GAMMA[i], "gamma, pixel {i}");
        }
        match_palette_range(&mut im, &palette_linear(&PEPTO_HEX), 0.01, 0.05);
        for (i, p) in im.px.iter().enumerate() {
            assert_eq!(
                p.map(f32::to_bits),
                t::PRE_MATCH[i],
                "match range, pixel {i}"
            );
        }
    }

    #[test]
    fn the_default_settings_still_round_trip_through_oklab() {
        // As png2c64: gamma 1 skips the power, but every pixel is converted to OKLab and
        // back and clamped, so an out-of-range value comes back in range.
        let mut im = Image::new(1, 1, vec![[1.2, -0.1, 0.5]]);
        apply(
            &mut im,
            &Settings {
                gamma: 1.0,
                ..Settings::default()
            },
        );
        assert!(im.px[0].iter().all(|v| (0.0..=1.0).contains(v)));
    }
}
