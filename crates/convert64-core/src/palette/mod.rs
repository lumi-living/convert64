//! The 16-colour display palette and its cached perceptual coordinates.
//!
//! A [`Palette`] maps each [`ColourIndex`] to an sRGB colour and caches that colour in
//! linear RGB, OKLab, CIELAB and Y'UV. The same object drives the preview and the
//! quantiser (CLAUDE.md non-negotiable 5), so changing presets changes both.
//!
//! RGB values come from `docs/reference/vic-ii-constraints.md` §1.

mod colodore;
mod convert;
pub mod luma;
mod vpl;

pub use colodore::{DEFAULT_BRIGHTNESS, DEFAULT_CONTRAST, DEFAULT_SATURATION};
pub use vpl::VplError;

use crate::colour::ColourIndex;
use std::fmt;

/// An 8-bit-per-channel sRGB colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rgb8 {
    /// Red, gamma-encoded.
    pub r: u8,
    /// Green, gamma-encoded.
    pub g: u8,
    /// Blue, gamma-encoded.
    pub b: u8,
}

impl Rgb8 {
    /// A colour from its three channels.
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// A colour from `0xRRGGBB`.
    pub const fn from_hex(rgb: u32) -> Self {
        Self {
            r: ((rgb >> 16) & 0xFF) as u8,
            g: ((rgb >> 8) & 0xFF) as u8,
            b: (rgb & 0xFF) as u8,
        }
    }

    /// The colour as `0xRRGGBB`.
    pub const fn to_hex(self) -> u32 {
        ((self.r as u32) << 16) | ((self.g as u32) << 8) | self.b as u32
    }
}

impl fmt::Display for Rgb8 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
    }
}

/// Linear-light sRGB, each channel `0..=1`.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct LinearRgb {
    /// Red.
    pub r: f32,
    /// Green.
    pub g: f32,
    /// Blue.
    pub b: f32,
}

/// OKLab (Björn Ottosson), as defined in CSS Color 4. `l` is `0..=1`.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Oklab {
    /// Lightness, `0..=1`.
    pub l: f32,
    /// Green–red axis.
    pub a: f32,
    /// Blue–yellow axis.
    pub b: f32,
}

/// CIELAB relative to D50, as defined in CSS Color 4. `l` is `0..=100`.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Lab {
    /// Lightness, `0..=100`.
    pub l: f32,
    /// Green–red axis.
    pub a: f32,
    /// Blue–yellow axis.
    pub b: f32,
}

/// PAL Y'UV (BT.470) of the gamma-encoded colour, the space the C64 palettes are
/// derived in. `y` is `0..=1`.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Yuv {
    /// Luma, `0..=1`.
    pub y: f32,
    /// Blue-difference chroma.
    pub u: f32,
    /// Red-difference chroma.
    pub v: f32,
}

/// Pepto's PAL palette, as shipped by VICE (`data/C64/pepto-pal.vpl`); reference §1.
const PEPTO_PAL: [u32; 16] = [
    0x000000, 0xFFFFFF, 0x68372B, 0x70A4B2, 0x6F3D86, 0x588D43, 0x352879, 0xB8C76F, 0x6F4F25,
    0x433900, 0x9A6759, 0x444444, 0x6C6C6C, 0x9AD284, 0x6C5EB5, 0x959595,
];

/// Pepto's NTSC palette, as shipped by VICE (`data/C64/pepto-ntsc.vpl`); reference §1.
const PEPTO_NTSC: [u32; 16] = [
    0x000000, 0xFFFFFF, 0x67372B, 0x70A3B1, 0x6F3D86, 0x588C42, 0x342879, 0xB7C66E, 0x6F4E25,
    0x423800, 0x996659, 0x434343, 0x6B6B6B, 0x9AD183, 0x6B5EB5, 0x959595,
];

/// A 16-entry VIC-II display palette with cached colour-space coordinates.
///
/// Lookups are plain array reads: no allocation, no conversion at query time.
#[derive(Clone, Debug, PartialEq)]
pub struct Palette {
    name: String,
    rgb: [Rgb8; 16],
    linear: [LinearRgb; 16],
    oklab: [Oklab; 16],
    lab: [Lab; 16],
    yuv: [Yuv; 16],
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "caches deliberately store f64 results as f32"
)]
fn f32x3(v: [f64; 3]) -> [f32; 3] {
    [v[0] as f32, v[1] as f32, v[2] as f32]
}

impl Palette {
    /// A palette from 16 sRGB colours in index order; computes all caches.
    pub fn from_rgb(name: impl Into<String>, rgb: [Rgb8; 16]) -> Self {
        let mut linear = [LinearRgb::default(); 16];
        let mut oklab = [Oklab::default(); 16];
        let mut lab = [Lab::default(); 16];
        let mut yuv = [Yuv::default(); 16];
        for (i, c) in rgb.iter().enumerate() {
            let srgb = [c.r, c.g, c.b].map(|v| f64::from(v) / 255.0);
            let lin = srgb.map(convert::srgb_to_linear);
            let xyz = convert::linear_srgb_to_xyz(lin);
            let [r, g, b] = f32x3(lin);
            linear[i] = LinearRgb { r, g, b };
            let [l, a, b] = f32x3(convert::xyz_to_oklab(xyz));
            oklab[i] = Oklab { l, a, b };
            let [l, a, b] = f32x3(convert::xyz_to_lab(xyz));
            lab[i] = Lab { l, a, b };
            let [y, u, v] = f32x3(convert::srgb_to_yuv(srgb));
            yuv[i] = Yuv { y, u, v };
        }
        Self {
            name: name.into(),
            rgb,
            linear,
            oklab,
            lab,
            yuv,
        }
    }

    fn from_hex_table(name: &str, table: &[u32; 16]) -> Self {
        Self::from_rgb(name, table.map(Rgb8::from_hex))
    }

    /// Pepto's PAL palette (the default preset).
    pub fn pepto_pal() -> Self {
        Self::from_hex_table("Pepto (PAL)", &PEPTO_PAL)
    }

    /// Pepto's NTSC palette.
    pub fn pepto_ntsc() -> Self {
        Self::from_hex_table("Pepto (NTSC)", &PEPTO_NTSC)
    }

    /// The Colodore palette for colodore.com's brightness, contrast and saturation
    /// sliders. `Palette::colodore(50.0, 100.0, 50.0)` is the site's default and the
    /// reference table's "Colodore (default)" column.
    pub fn colodore(brightness: f64, contrast: f64, saturation: f64) -> Self {
        let rgb = colodore::colodore_rgb(brightness, contrast, saturation)
            .map(|[r, g, b]| Rgb8::new(r, g, b));
        let name = if brightness == DEFAULT_BRIGHTNESS
            && contrast == DEFAULT_CONTRAST
            && saturation == DEFAULT_SATURATION
        {
            "Colodore".to_owned()
        } else {
            format!("Colodore (b{brightness} c{contrast} s{saturation})")
        };
        Self::from_rgb(name, rgb)
    }

    /// Display name, for example `"Pepto (PAL)"`.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// sRGB colour of `c`.
    pub const fn rgb(&self, c: ColourIndex) -> Rgb8 {
        self.rgb[c.as_usize()]
    }

    /// All 16 sRGB colours in index order.
    pub const fn rgb_table(&self) -> &[Rgb8; 16] {
        &self.rgb
    }

    /// Cached linear-light RGB of `c`.
    pub const fn linear(&self, c: ColourIndex) -> LinearRgb {
        self.linear[c.as_usize()]
    }

    /// Cached OKLab of `c`.
    pub const fn oklab(&self, c: ColourIndex) -> Oklab {
        self.oklab[c.as_usize()]
    }

    /// Cached CIELAB (D50) of `c`.
    pub const fn lab(&self, c: ColourIndex) -> Lab {
        self.lab[c.as_usize()]
    }

    /// Cached PAL Y'UV of `c`.
    pub const fn yuv(&self, c: ColourIndex) -> Yuv {
        self.yuv[c.as_usize()]
    }
}

/// OKLab of any sRGB colour, with the same maths the palette caches use. For comparing
/// arbitrary images (golden PNGs, sources) in the quantiser's colour space.
pub fn oklab_of(c: Rgb8) -> Oklab {
    let srgb = [c.r, c.g, c.b].map(|v| f64::from(v) / 255.0);
    let xyz = convert::linear_srgb_to_xyz(srgb.map(convert::srgb_to_linear));
    let [l, a, b] = f32x3(convert::xyz_to_oklab(xyz));
    Oklab { l, a, b }
}

/// OKLab in polar form: lightness, chroma and hue angle (CSS `oklch()`).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Oklch {
    /// Lightness, `0..=1`, the same as [`Oklab::l`].
    pub l: f32,
    /// Chroma, `√(a² + b²)`, `0` for greys.
    pub c: f32,
    /// Hue angle in radians, `-π..=π`, measured from the `+a` axis towards `+b`.
    pub h: f32,
}

impl Oklab {
    /// The same colour in polar form.
    pub fn to_oklch(self) -> Oklch {
        Oklch {
            l: self.l,
            c: self.a.hypot(self.b),
            h: self.b.atan2(self.a),
        }
    }
}

impl Oklch {
    /// The same colour in rectangular form.
    pub fn to_oklab(self) -> Oklab {
        let (sin, cos) = self.h.sin_cos();
        Oklab {
            l: self.l,
            a: self.c * cos,
            b: self.c * sin,
        }
    }
}

impl LinearRgb {
    /// OKLab of this linear-light colour, computed in `f32` with the same CSS Color 4
    /// matrices as the palette caches. Per-pixel work (adjustments) uses this.
    pub fn to_oklab(self) -> Oklab {
        let [l, a, b] = convert::linear_to_oklab_f32([self.r, self.g, self.b]);
        Oklab { l, a, b }
    }

    /// The linear-light colour of an OKLab value; the inverse of [`Self::to_oklab`].
    /// Out-of-gamut values are returned unclipped.
    pub fn from_oklab(lab: Oklab) -> Self {
        let [r, g, b] = convert::oklab_to_linear_f32([lab.l, lab.a, lab.b]);
        Self { r, g, b }
    }
}

/// Lookup tables for 8-bit sRGB: the linear value of every byte, and the linear
/// thresholds halfway (in gamma-encoded space) between neighbouring bytes.
struct SrgbTables {
    to_linear: [f32; 256],
    thresholds: [f32; 255],
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "the tables are f32 by design; computed in f64 once"
)]
fn srgb_tables() -> &'static SrgbTables {
    static TABLES: std::sync::OnceLock<SrgbTables> = std::sync::OnceLock::new();
    TABLES.get_or_init(|| {
        let mut to_linear = [0.0; 256];
        for (i, v) in to_linear.iter_mut().enumerate() {
            *v = convert::srgb_to_linear(i as f64 / 255.0) as f32;
        }
        let mut thresholds = [0.0; 255];
        for (i, t) in thresholds.iter_mut().enumerate() {
            *t = convert::srgb_to_linear((i as f64 + 0.5) / 255.0) as f32;
        }
        SrgbTables {
            to_linear,
            thresholds,
        }
    })
}

/// An 8-bit gamma-encoded sRGB channel as linear light `0..=1` (a table lookup).
pub fn srgb8_to_linear(v: u8) -> f32 {
    srgb_tables().to_linear[usize::from(v)]
}

/// Linear light to the nearest 8-bit sRGB value, rounding in gamma-encoded space.
///
/// Exact inverse of [`srgb8_to_linear`] for all 256 values; out-of-range and NaN inputs
/// clamp (NaN to 0). Uses a binary search over precomputed thresholds, so the result
/// does not depend on the platform's `powf`.
#[expect(
    clippy::cast_possible_truncation,
    reason = "the partition point of 255 thresholds is at most 255"
)]
pub fn linear_to_srgb8(v: f32) -> u8 {
    if v.is_nan() {
        return 0;
    }
    srgb_tables().thresholds.partition_point(|&t| t <= v) as u8
}

impl Default for Palette {
    /// Pepto (PAL), the default preset (`docs/PDD.md`).
    fn default() -> Self {
        Self::pepto_pal()
    }
}

#[cfg(test)]
mod tests {
    use super::{ColourIndex, LinearRgb, Oklab, Palette, Rgb8};

    fn hex(p: &Palette) -> Vec<String> {
        p.rgb_table().iter().map(ToString::to_string).collect()
    }

    /// The 16 values in `docs/reference/vic-ii-constraints.md` §1, copied verbatim.
    const REFERENCE_COLODORE: [&str; 16] = [
        "#000000", "#FFFFFF", "#813338", "#75CEC8", "#8E3C97", "#56AC4D", "#2E2C9B", "#EDF171",
        "#8E5029", "#553800", "#C46C71", "#4A4A4A", "#7B7B7B", "#A9FF9F", "#706DEB", "#B2B2B2",
    ];
    const REFERENCE_PEPTO_PAL: [&str; 16] = [
        "#000000", "#FFFFFF", "#68372B", "#70A4B2", "#6F3D86", "#588D43", "#352879", "#B8C76F",
        "#6F4F25", "#433900", "#9A6759", "#444444", "#6C6C6C", "#9AD284", "#6C5EB5", "#959595",
    ];
    const REFERENCE_PEPTO_NTSC: [&str; 16] = [
        "#000000", "#FFFFFF", "#67372B", "#70A3B1", "#6F3D86", "#588C42", "#342879", "#B7C66E",
        "#6F4E25", "#423800", "#996659", "#434343", "#6B6B6B", "#9AD183", "#6B5EB5", "#959595",
    ];

    #[test]
    fn colodore_default_matches_the_reference_hex_values() {
        assert_eq!(
            hex(&Palette::colodore(50.0, 100.0, 50.0)),
            REFERENCE_COLODORE
        );
    }

    #[test]
    fn pepto_presets_match_the_reference_hex_values() {
        assert_eq!(hex(&Palette::pepto_pal()), REFERENCE_PEPTO_PAL);
        assert_eq!(hex(&Palette::pepto_ntsc()), REFERENCE_PEPTO_NTSC);
        assert_eq!(Palette::default(), Palette::pepto_pal());
    }

    #[test]
    fn caches_hold_the_converted_values() {
        // Spot-check against the CSS Color 4 oracle values used in `convert`.
        let p = Palette::colodore(50.0, 100.0, 50.0);
        let red = p.oklab(ColourIndex::RED);
        assert!((red.l - 0.431_521_7).abs() < 1e-6, "{red:?}");
        assert!((red.a - 0.102_093).abs() < 1e-6, "{red:?}");
        let grey = p.lab(ColourIndex::MEDIUM_GREY);
        assert!(
            (grey.l - 51.618_22).abs() < 1e-4 && grey.a.abs() < 1e-4,
            "{grey:?}"
        );
        let white = p.yuv(ColourIndex::WHITE);
        assert!(
            (white.y - 1.0).abs() < 1e-6 && white.u.abs() < 1e-4,
            "{white:?}"
        );
        let blue = p.linear(ColourIndex::BLUE);
        assert!((blue.b - 0.327_778_1).abs() < 1e-6, "{blue:?}");
    }

    #[test]
    fn srgb8_round_trips_all_256_values() {
        for v in 0..=255u8 {
            assert_eq!(super::linear_to_srgb8(super::srgb8_to_linear(v)), v, "{v}");
        }
        assert_eq!(super::linear_to_srgb8(-1.0), 0);
        assert_eq!(super::linear_to_srgb8(2.0), 255);
        assert_eq!(super::linear_to_srgb8(f32::NAN), 0);
        assert_eq!(super::srgb8_to_linear(255), 1.0);
    }

    #[test]
    fn f32_oklab_agrees_with_the_palette_cache() {
        let p = Palette::pepto_pal();
        for c in ColourIndex::ALL {
            let lin = p.linear(c);
            let got = lin.to_oklab();
            let want = p.oklab(c);
            for (g, w) in [(got.l, want.l), (got.a, want.a), (got.b, want.b)] {
                assert!((g - w).abs() < 2e-5, "{c}: {got:?} vs {want:?}");
            }
            let back = LinearRgb::from_oklab(got);
            for (g, w) in [(back.r, lin.r), (back.g, lin.g), (back.b, lin.b)] {
                assert!((g - w).abs() < 2e-5, "{c}: {back:?} vs {lin:?}");
            }
        }
    }

    #[test]
    fn oklch_round_trips() {
        for lab in [
            Oklab {
                l: 0.5,
                a: 0.1,
                b: -0.05,
            },
            Oklab {
                l: 0.9,
                a: -0.2,
                b: 0.15,
            },
            Oklab {
                l: 0.3,
                a: 0.0,
                b: 0.0,
            },
        ] {
            let lch = lab.to_oklch();
            assert!(lch.c >= 0.0);
            let back = lch.to_oklab();
            assert!((back.a - lab.a).abs() < 1e-6 && (back.b - lab.b).abs() < 1e-6);
            assert_eq!(back.l, lab.l);
        }
    }

    #[test]
    fn oklab_of_matches_the_palette_cache() {
        let p = Palette::colodore(50.0, 100.0, 50.0);
        for c in ColourIndex::ALL {
            assert_eq!(super::oklab_of(p.rgb(c)), p.oklab(c), "{c}");
        }
    }

    #[test]
    fn named_and_custom_palettes() {
        assert_eq!(Palette::pepto_pal().name(), "Pepto (PAL)");
        assert_eq!(Palette::colodore(50.0, 100.0, 50.0).name(), "Colodore");
        assert_eq!(
            Palette::colodore(47.0, 100.0, 70.0).name(),
            "Colodore (b47 c100 s70)"
        );
        let mut rgb = [Rgb8::new(0, 0, 0); 16];
        rgb[1] = Rgb8::from_hex(0x123456);
        let p = Palette::from_rgb("custom", rgb);
        assert_eq!(p.rgb(ColourIndex::WHITE).to_hex(), 0x123456);
    }
}
