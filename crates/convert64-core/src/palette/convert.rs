//! Colour-space conversions used to cache perceptual coordinates per palette entry.
//!
//! sRGB, CIE XYZ, OKLab and CIELAB follow the CSS Color Module Level 4 sample code
//! (`css-color-4/conversions.js` in the W3C `csswg-drafts` repository) step for step:
//! the same transfer function, matrices and white points. CIELAB is therefore relative to
//! D50, reached from sRGB's D65 with the CSS chromatic-adaptation matrix, exactly as CSS
//! `lab()` is defined. Y'UV is the PAL (BT.470) matrix applied to gamma-encoded sRGB,
//! the space Pepto's and Colodore's palettes are derived in.
//!
//! All maths is `f64`; palettes store the results as `f32`.

/// sRGB transfer function, gamma-encoded `0..=1` to linear light (CSS `lin_sRGB`).
pub(crate) fn srgb_to_linear(c: f64) -> f64 {
    let abs = c.abs();
    if abs <= 0.04045 {
        c / 12.92
    } else {
        c.signum() * ((abs + 0.055) / 1.055).powf(2.4)
    }
}

fn mul(m: &[[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

/// Linear sRGB to CIE XYZ relative to D65 (CSS `lin_sRGB_to_XYZ`).
pub(crate) fn linear_srgb_to_xyz(rgb: [f64; 3]) -> [f64; 3] {
    const M: [[f64; 3]; 3] = [
        [
            506_752.0 / 1_228_815.0,
            87_881.0 / 245_763.0,
            12_673.0 / 70_218.0,
        ],
        [
            87_098.0 / 409_605.0,
            175_762.0 / 245_763.0,
            12_673.0 / 175_545.0,
        ],
        [
            7_918.0 / 409_605.0,
            87_881.0 / 737_289.0,
            1_001_167.0 / 1_053_270.0,
        ],
    ];
    mul(&M, rgb)
}

/// CIE XYZ (D65) to OKLab (CSS `XYZ_to_OKLab`). `L` is `0..=1`.
pub(crate) fn xyz_to_oklab(xyz: [f64; 3]) -> [f64; 3] {
    const XYZ_TO_LMS: [[f64; 3]; 3] = [
        [
            0.819_022_437_996_703,
            0.361_906_260_052_890_4,
            -0.128_873_781_520_987_9,
        ],
        [
            0.032_983_653_932_388_5,
            0.929_286_861_586_343_4,
            0.036_144_666_350_642_4,
        ],
        [
            0.048_177_189_359_624_2,
            0.264_239_531_752_730_8,
            0.633_547_828_469_430_9,
        ],
    ];
    const LMS_TO_OKLAB: [[f64; 3]; 3] = [
        [
            0.210_454_268_309_314,
            0.793_617_774_702_305_4,
            -0.004_072_043_011_619_3,
        ],
        [
            1.977_998_532_431_168_4,
            -2.428_592_242_048_58,
            0.450_593_709_617_411,
        ],
        [
            0.025_904_042_465_547_8,
            0.782_771_712_457_529_6,
            -0.808_675_754_923_077_4,
        ],
    ];
    let lms = mul(&XYZ_TO_LMS, xyz);
    mul(&LMS_TO_OKLAB, [lms[0].cbrt(), lms[1].cbrt(), lms[2].cbrt()])
}

/// CIE XYZ D65 to D50 (CSS `D65_to_D50`, Bradford-derived).
fn xyz_d65_to_d50(xyz: [f64; 3]) -> [f64; 3] {
    const M: [[f64; 3]; 3] = [
        [
            1.047_929_792_544_997,
            0.022_946_870_601_609_652,
            -0.050_192_266_289_205_24,
        ],
        [
            0.029_627_808_770_055_99,
            0.990_434_426_753_879_9,
            -0.017_073_799_063_418_826,
        ],
        [
            -0.009_243_040_646_204_504,
            0.015_055_191_490_298_152,
            0.751_874_281_428_137_1,
        ],
    ];
    mul(&M, xyz)
}

/// CIE XYZ (D65) to CIELAB relative to D50 (CSS `D65_to_D50` then `XYZ_to_Lab`).
/// `L` is `0..=100`.
pub(crate) fn xyz_to_lab(xyz_d65: [f64; 3]) -> [f64; 3] {
    const EPSILON: f64 = 216.0 / 24_389.0;
    const KAPPA: f64 = 24_389.0 / 27.0;
    const D50: [f64; 3] = [0.3457 / 0.3585, 1.0, (1.0 - 0.3457 - 0.3585) / 0.3585];
    let xyz = xyz_d65_to_d50(xyz_d65);
    let f = |i: usize| {
        let v = xyz[i] / D50[i];
        if v > EPSILON {
            v.cbrt()
        } else {
            (KAPPA * v + 16.0) / 116.0
        }
    };
    let (fx, fy, fz) = (f(0), f(1), f(2));
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

/// Gamma-encoded sRGB `0..=1` to PAL Y'UV (BT.470 matrix): `Y` in `0..=1`,
/// `U` in about `±0.436`, `V` in about `±0.615`.
pub(crate) fn srgb_to_yuv(rgb: [f64; 3]) -> [f64; 3] {
    const M: [[f64; 3]; 3] = [
        [0.299, 0.587, 0.114],
        [-0.147_13, -0.288_86, 0.436],
        [0.615, -0.514_99, -0.100_01],
    ];
    mul(&M, rgb)
}

#[cfg(test)]
mod tests {
    use super::{linear_srgb_to_xyz, srgb_to_linear, srgb_to_yuv, xyz_to_lab, xyz_to_oklab};

    /// Reference values produced by the CSS Color 4 sample code
    /// (`conversions.js`, `lin_sRGB` → `lin_sRGB_to_XYZ` → `XYZ_to_OKLab` /
    /// `D65_to_D50` + `XYZ_to_Lab`), printed to 9 decimals.
    /// Columns: sRGB bytes, linear sRGB, OKLab, CIELAB (D50).
    #[expect(clippy::type_complexity, reason = "test table")]
    const CSS_ORACLE: [([u8; 3], [f64; 3], [f64; 3], [f64; 3]); 9] = [
        (
            [255, 0, 0],
            [1.0, 0.0, 0.0],
            [0.627_955_364, 0.224_863_068, 0.125_846_277],
            [54.290_541_405, 80.804_928_170, 69.890_964_769],
        ),
        (
            [0, 255, 0],
            [0.0, 1.0, 0.0],
            [0.866_439_618, -0.233_887_581, 0.179_498_445],
            [87.818_534_365, -79.271_060_776, 80.994_581_484],
        ),
        (
            [0, 0, 255],
            [0.0, 0.0, 1.0],
            [0.452_013_718, -0.032_456_975, -0.311_528_166],
            [29.568_301_979, 68.287_364_603, -112.029_709_525],
        ),
        (
            [255, 255, 255],
            [1.0, 1.0, 1.0],
            [1.0, 0.0, 0.0],
            [100.0, 0.0, 0.0],
        ),
        ([0, 0, 0], [0.0, 0.0, 0.0], [0.0, 0.0, 0.0], [0.0, 0.0, 0.0]),
        (
            [0x81, 0x33, 0x38],
            [0.219_526_200, 0.033_104_767, 0.039_546_235],
            [0.431_521_735, 0.102_093_009, 0.035_066_985],
            [32.913_167_946, 34.661_642_000, 14.908_896_655],
        ),
        (
            [0x2E, 0x2C, 0x9B],
            [0.027_320_892, 0.025_186_860, 0.327_778_098],
            [0.377_938_277, 0.015_094_756, -0.172_989_703],
            [24.953_495_834, 30.565_009_022, -61.330_667_532],
        ),
        (
            [0x7B, 0x7B, 0x7B],
            [0.198_069_320, 0.198_069_320, 0.198_069_320],
            [0.582_915_678, 0.0, 0.0],
            [51.618_218_703, 0.0, 0.0],
        ),
        (
            [0xA9, 0xFF, 0x9F],
            [0.396_755_231, 1.0, 0.346_704_056],
            [0.922_466_257, -0.119_588_703, 0.093_905_585],
            [92.846_914_798, -40.475_072_161, 37.366_647_185],
        ),
    ];

    fn assert_close(got: [f64; 3], want: [f64; 3], tol: f64, what: &str) {
        for i in 0..3 {
            assert!(
                (got[i] - want[i]).abs() <= tol,
                "{what}: component {i} got {got:?} want {want:?}"
            );
        }
    }

    #[test]
    fn matches_css_color_4_reference_code() {
        for (srgb, lin, oklab, lab) in CSS_ORACLE {
            let s = srgb.map(|v| f64::from(v) / 255.0);
            let l = s.map(srgb_to_linear);
            assert_close(l, lin, 1e-9, "linear");
            let xyz = linear_srgb_to_xyz(l);
            assert_close(xyz_to_oklab(xyz), oklab, 1e-8, "oklab");
            assert_close(xyz_to_lab(xyz), lab, 1e-7, "lab");
        }
    }

    #[test]
    fn yuv_matches_bt470_reference_points() {
        // Y'UV of the sRGB primaries and white per the BT.470 / PAL matrix.
        assert_close(srgb_to_yuv([1.0, 1.0, 1.0]), [1.0, 0.0, 0.0], 1e-4, "white");
        assert_close(srgb_to_yuv([0.0, 0.0, 0.0]), [0.0, 0.0, 0.0], 0.0, "black");
        assert_close(
            srgb_to_yuv([1.0, 0.0, 0.0]),
            [0.299, -0.147_13, 0.615],
            1e-9,
            "red",
        );
        assert_close(
            srgb_to_yuv([0.0, 0.0, 1.0]),
            [0.114, 0.436, -0.100_01],
            1e-9,
            "blue",
        );
    }

    #[test]
    fn yuv_inverts_with_the_pal_decoder_matrix() {
        // R = Y + 1.140 V, G = Y - 0.395 U - 0.581 V, B = Y + 2.032 U (BT.470 inverse).
        for rgb in [[0.2, 0.5, 0.9], [0.9, 0.1, 0.3], [0.5, 0.5, 0.5]] {
            let [y, u, v] = srgb_to_yuv(rgb);
            let back = [y + 1.140 * v, y - 0.395 * u - 0.581 * v, y + 2.032 * u];
            assert_close(back, rgb, 2e-3, "round trip");
        }
    }
}
