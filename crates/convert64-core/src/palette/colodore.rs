//! Colodore palette generator (Philip "Pepto" Timmermann, <https://www.colodore.com/>).
//!
//! A direct port of the site's `renderPalette()` for the VIC-II with the newer chip's
//! luma levels (the site's default), PAL source gamma 2.8, sRGB target gamma 2.2,
//! phase 0 and V not inverted. Rounding matches JavaScript's `Math.round`.

/// colodore.com's default slider settings.
pub const DEFAULT_BRIGHTNESS: f64 = 50.0;
/// colodore.com's default slider settings.
pub const DEFAULT_CONTRAST: f64 = 100.0;
/// colodore.com's default slider settings.
pub const DEFAULT_SATURATION: f64 = 50.0;

/// Luma levels used by the generator, on its own 0–32 scale (white is 32 there; the
/// reference table's 0–31 levels group the colours identically).
const LUMAS: [f64; 16] = [
    0.0, 32.0, 10.0, 20.0, 12.0, 16.0, 8.0, 24.0, 12.0, 8.0, 16.0, 10.0, 15.0, 24.0, 15.0, 20.0,
];

/// Hue sector per colour (multiples of 360°/16, offset by half a sector); `None` for greys.
const ANGLES: [Option<f64>; 16] = [
    None,
    None,
    Some(4.0),
    Some(12.0),
    Some(2.0),
    Some(10.0),
    Some(15.0),
    Some(7.0),
    Some(5.0),
    Some(6.0),
    Some(4.0),
    None,
    None,
    Some(10.0),
    Some(15.0),
    None,
];

const GAMMA_SOURCE: f64 = 2.8;
const GAMMA_TARGET: f64 = 2.2;
const CONTRAST_BOOST: f64 = 1.0 / 5.0;

fn gamma_pepto(value: f64) -> f64 {
    let value = value.clamp(0.0, 255.0);
    let factor = 255_f64.powf(1.0 - GAMMA_SOURCE);
    let value = (factor * value.powf(GAMMA_SOURCE)).max(0.0);
    let factor = 255_f64.powf(1.0 - 1.0 / GAMMA_TARGET);
    (factor * value.powf(1.0 / GAMMA_TARGET)).clamp(0.0, 255.0)
}

/// JavaScript `Math.round` for the non-negative, clamped values used here.
fn js_round(v: f64) -> u8 {
    // `v` is already clamped to 0..=255, so the cast cannot truncate.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "value is clamped to 0..=255 before rounding"
    )]
    let byte = (v + 0.5).floor().clamp(0.0, 255.0) as u8;
    byte
}

/// The 16 colours colodore.com shows for the given slider values.
///
/// `brightness` and `saturation` are the site's 0–100 sliders, `contrast` its 0–100+
/// slider. Defaults are [`DEFAULT_BRIGHTNESS`], [`DEFAULT_CONTRAST`] and
/// [`DEFAULT_SATURATION`].
pub(crate) fn colodore_rgb(brightness: f64, contrast: f64, saturation: f64) -> [[u8; 3]; 16] {
    let con = contrast / 100.0 + CONTRAST_BOOST;
    let sat = saturation / 1.25;
    let bri = brightness - 50.0;
    let mut out = [[0_u8; 3]; 16];
    for i in 0..16 {
        let mut y = LUMAS[i] * (256.0 / 32.0);
        let (mut u, mut v) = match ANGLES[i] {
            None => (0.0, 0.0),
            Some(sector) => {
                let sector_deg = 360.0 / 16.0;
                let angle = (sector * sector_deg + sector_deg / 2.0).to_radians();
                (sat * angle.cos(), sat * angle.sin())
            }
        };
        y *= con;
        u *= con;
        v *= con;
        y += bri;
        let r = (y + 1.140 * v).clamp(0.0, 255.0);
        let g = (y - 0.396 * u - 0.581 * v).clamp(0.0, 255.0);
        let b = (y + 2.029 * u).clamp(0.0, 255.0);
        out[i] = [
            js_round(gamma_pepto(r)),
            js_round(gamma_pepto(g)),
            js_round(gamma_pepto(b)),
        ];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_BRIGHTNESS, DEFAULT_CONTRAST, DEFAULT_SATURATION, colodore_rgb};

    fn hex(rgb: [[u8; 3]; 16]) -> Vec<String> {
        rgb.iter()
            .map(|c| format!("{:02X}{:02X}{:02X}", c[0], c[1], c[2]))
            .collect()
    }

    #[test]
    fn defaults_reproduce_the_reference_table() {
        // docs/reference/vic-ii-constraints.md §1, "Colodore (default)" column.
        let want = [
            "000000", "FFFFFF", "813338", "75CEC8", "8E3C97", "56AC4D", "2E2C9B", "EDF171",
            "8E5029", "553800", "C46C71", "4A4A4A", "7B7B7B", "A9FF9F", "706DEB", "B2B2B2",
        ];
        assert_eq!(
            hex(colodore_rgb(
                DEFAULT_BRIGHTNESS,
                DEFAULT_CONTRAST,
                DEFAULT_SATURATION
            )),
            want
        );
    }

    #[test]
    fn brightness_47_saturation_70_reproduces_vice_colodore_vpl() {
        // VICE data/C64/colodore.vpl, an independent second data point for the port.
        let want = [
            "000000", "FFFFFF", "96282E", "5BD6CE", "9F2DAD", "41B936", "2724C4", "EFF347",
            "9F4815", "5E3500", "DA5F66", "474747", "787878", "91FF84", "6864FF", "AEAEAE",
        ];
        assert_eq!(hex(colodore_rgb(47.0, 100.0, 70.0)), want);
    }

    #[test]
    fn zero_saturation_gives_greys() {
        for c in colodore_rgb(50.0, 100.0, 0.0) {
            assert!(c[0] == c[1] && c[1] == c[2], "{c:?} is not grey");
        }
    }
}
