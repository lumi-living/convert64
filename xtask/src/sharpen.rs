//! `cargo xtask sharpen-sheet` (#228): the Sharpen adjustment on every fixture, for a
//! human to judge. Each fixture at its png2c64 baseline settings (gamma 1.5, its mode,
//! method and strength) gets a row with four settings: amount 0, 0.5 and 1.0 with
//! png2c64's 3 × 3 kernel, and amount 1.0 with a Gaussian of σ 0.8 target pixels. Each
//! setting shows the prepared frame (what the solver gets) above its solve by the
//! reference solver. Defaults stay off: amount 0 is today's output.
//!
//! The project path is the product's: a plain picture with png2c64's kernel takes
//! png2c64's front end (#190), the Gaussian the compositor.

use crate::font;
use crate::frontend::{FIXTURES, render_with};
use crate::repo_root;
use crate::sheet::save;
use convert64_core::layer::{Adjustments, Gamma, Radius, Sharpen};
use image::{Rgb, RgbImage};

const TILE_W: u32 = 320;
const TILE_H: u32 = 200;
const GAP: u32 = 8;
const LABEL: u32 = 20;
const NOTE: u32 = 20;
const BACKGROUND: [u8; 3] = [30, 30, 30];
const TEXT: [u8; 3] = [225, 225, 225];

/// The columns: a caption and the sharpen.
fn settings() -> Result<Vec<(&'static str, Sharpen)>, String> {
    let e = |e: convert64_core::layer::AdjustError| e.to_string();
    Ok(vec![
        ("off", Sharpen::default()),
        ("3x3 0.5", Sharpen::png2c64(0.5).map_err(e)?),
        ("3x3 1.0", Sharpen::png2c64(1.0).map_err(e)?),
        (
            "gauss .8 1.0",
            Sharpen::new(1.0, Radius::gaussian(0.8).map_err(e)?, 0.0).map_err(e)?,
        ),
    ])
}

pub(crate) fn run() -> Result<(), String> {
    let columns = settings()?;
    let cols = columns.len() as u32;
    let rows = FIXTURES.len() as u32;
    let w = cols * TILE_W + (cols + 1) * GAP;
    let row_h = LABEL + 2 * TILE_H + GAP / 2 + NOTE + GAP;
    let h = rows * row_h + GAP;
    let mut sheet = RgbImage::from_pixel(w, h, Rgb(BACKGROUND));
    for (r, (name, mode, kind, strength)) in FIXTURES.iter().enumerate() {
        let y = GAP + r as u32 * row_h;
        font::draw(
            &mut sheet,
            GAP + 2,
            y + 3,
            &format!(
                "{name} ({} {strength}, gamma 1.5): off, png2c64 3x3 0.5 and 1.0, gaussian s 0.8 1.0",
                kind.id()
            ),
            2,
            TEXT,
        );
        let mut off: Option<RgbImage> = None;
        for (c, (caption, sharpen)) in columns.iter().enumerate() {
            let adjustments = Adjustments {
                gamma: Gamma::PNG2C64,
                sharpen: *sharpen,
                ..Adjustments::default()
            };
            let (prepared, solved) =
                render_with(name, *mode, *kind, *strength, &adjustments, true)?;
            let x = GAP + c as u32 * (TILE_W + GAP);
            image::imageops::replace(&mut sheet, &prepared, i64::from(x), i64::from(y + LABEL));
            image::imageops::replace(
                &mut sheet,
                &solved,
                i64::from(x),
                i64::from(y + LABEL + TILE_H + GAP / 2),
            );
            let note = match &off {
                None => caption.to_string(),
                Some(base) => {
                    let same = base
                        .pixels()
                        .zip(solved.pixels())
                        .filter(|(a, b)| a == b)
                        .count();
                    let pct = 100.0 * same as f64 / f64::from(TILE_W * TILE_H);
                    format!("{caption}: {pct:.1}% same")
                }
            };
            font::draw(
                &mut sheet,
                x + 2,
                y + LABEL + 2 * TILE_H + GAP / 2 + 4,
                &note,
                2,
                TEXT,
            );
            println!("{name:<9} {note}");
            if off.is_none() {
                off = Some(solved);
            }
        }
    }
    let dir = repo_root().join("docs/reference/dither-previews/convert64-sheets");
    save(&sheet, &dir.join("sheet-sharpen.png"))
}
