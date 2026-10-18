//! Quality scores of a result against what it was made from (#261).
//!
//! [`ssimulacra2`] is reported in the CLI's `--json` and by the MCP `solve` tool, as
//! information only: no score decides quality, and none is a gate or in the app until it
//! agrees with the eye on 90 % of `fixtures/eye-rankings.toml` (ADR-014 §7). [`score_bitmap`] pairs a solve's output with its frame the way png2amiga
//! scores its C64 output.

mod ssimulacra2;

pub use ssimulacra2::ssimulacra2;

use crate::composite::TargetFrame;
use crate::model::BitmapImage;
use crate::palette::Palette;

/// `image` at its logical resolution (160 × 200 multicolor, 320 × 200 hi-res) as
/// row-major linear RGB through `palette`, one value per logical pixel: what png2amiga's
/// C64 encoders return as `rendered`.
pub fn render_linear(image: &BitmapImage, palette: &Palette) -> Vec<[f32; 3]> {
    let w: u16 = if image.background().is_some() {
        160
    } else {
        320
    };
    let mut out = Vec::with_capacity(usize::from(w) * 200);
    for y in 0..200 {
        for x in 0..w {
            let c = palette.linear(image.pixel(x, y));
            out.push([c.r, c.g, c.b]);
        }
    }
    out
}

/// SSIMULACRA2 of a solve's `image` (rendered through `palette`) against `frame`, the
/// target-grid composite it was solved from.
///
/// This is png2amiga's pairing for its C64 modes (`api.cpp`, `finalize_psnr`): the
/// encoder's input against its render, both at the mode's logical raster (160 × 200 for
/// multicolor, so one value per wide pixel; 320 × 200 for hi-res), in linear light. The
/// input is the prepared source: scaled to the target grid and preprocessed, with
/// transparent pixels set to black. Here that is the frame's colour (in a session it has
/// been through the front end's preprocessing, `TargetFrame::prepared`), with
/// pixels under half covered set to black, as png2amiga's transparency mask does.
///
/// # Panics
/// If `frame`'s target size is not `image`'s logical size.
pub fn score_bitmap(frame: &TargetFrame, image: &BitmapImage, palette: &Palette) -> f64 {
    let (w, h) = frame.grid().target_size();
    let rendered = render_linear(image, palette);
    assert_eq!(
        rendered.len(),
        w as usize * h as usize,
        "the frame is {w} × {h}, the picture {}",
        if image.background().is_some() {
            "160 × 200"
        } else {
            "320 × 200"
        }
    );
    let reference: Vec<[f32; 3]> = frame
        .colour()
        .iter()
        .zip(frame.coverage())
        .map(|(&c, &a)| if a < 0.5 { [0.0; 3] } else { c })
        .collect();
    ssimulacra2(&reference, &rendered, w as usize, h as usize)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::colour::ColourIndex;
    use crate::geom::SuperGrid;
    use crate::mode::Mode;
    use crate::model::IndexedBitmap;

    fn frame(mode: Mode, colour: [f32; 3], coverage: f32) -> TargetFrame {
        let mut f = TargetFrame::empty(SuperGrid::new(mode, 1).expect("a bitmap grid"));
        let (c, a, _) = f.parts_mut();
        c.fill(colour);
        a.fill(coverage);
        f
    }

    fn filled(mode: Mode, c: ColourIndex) -> BitmapImage {
        BitmapImage::encode(&IndexedBitmap::filled(mode, c).expect("bitmap")).expect("legal")
    }

    #[test]
    fn scores_the_render_against_the_frame_at_the_logical_raster() {
        let pal = Palette::default();
        for (mode, w) in [(Mode::MulticolorBitmap, 160), (Mode::HiresBitmap, 320)] {
            let white = pal.linear(ColourIndex::WHITE);
            let white = [white.r, white.g, white.b];
            let img = filled(mode, ColourIndex::WHITE);
            assert_eq!(render_linear(&img, &pal), vec![white; w * 200]);
            assert_eq!(score_bitmap(&frame(mode, white, 1.0), &img, &pal), 100.0);
            // Under half covered is black, as png2amiga's transparency mask makes it.
            let s = score_bitmap(&frame(mode, white, 0.4), &img, &pal);
            assert!(s < 100.0, "{s}");
            let black = filled(mode, ColourIndex::BLACK);
            assert_eq!(score_bitmap(&frame(mode, white, 0.4), &black, &pal), 100.0);
        }
    }
}
