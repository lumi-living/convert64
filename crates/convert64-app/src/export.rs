//! Export (UX §3 Export menu, #205): the E6 formats from the result on screen, as named
//! files the app then saves (natively) or downloads (browser). The same
//! `convert64-formats` calls `convert64 convert` and `convert64 export` make.

use convert64_core::Palette;
use convert64_core::mode::Mode;
use convert64_core::solve::Solution;
use convert64_formats::BitmapFormat;
use convert64_formats::asm::{self, Assembler, DataStyle, EmitOptions};

/// What to export.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// The mode's picture file: Koala (multicolor) or Art Studio (hi-res).
    Picture,
    /// A self-displaying PRG.
    Prg,
    /// A palette-exact PNG preview.
    Png,
    /// The raw bitmap set: headerless `.bin` files and a JSON manifest.
    Raw,
    /// Assembler source or a C header.
    Source(Assembler),
}

impl Format {
    /// Every format, in the dialog's order.
    pub const ALL: [Self; 9] = [
        Self::Picture,
        Self::Prg,
        Self::Png,
        Self::Raw,
        Self::Source(Assembler::ALL[0]),
        Self::Source(Assembler::ALL[1]),
        Self::Source(Assembler::ALL[2]),
        Self::Source(Assembler::ALL[3]),
        Self::Source(Assembler::ALL[4]),
    ];

    /// The format's name for `mode`, e.g. "Koala" or "KickAssembler source".
    pub fn label(self, mode: Mode) -> String {
        match self {
            Self::Picture => BitmapFormat::for_mode(mode)
                .map_or("Picture", BitmapFormat::name)
                .to_owned(),
            Self::Prg => "PRG (self-displaying)".to_owned(),
            Self::Png => "PNG preview".to_owned(),
            Self::Raw => "Raw bitmap set".to_owned(),
            Self::Source(a) => a.name().to_owned(),
        }
    }
}

/// Export settings, kept for Export Again (`Shift Cmd E`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    /// The format.
    pub format: Format,
    /// PNG preview zoom, 1–8.
    pub zoom: u8,
    /// Assembler data: inline bytes, or include the raw set or the picture file.
    pub data: DataStyle,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            format: Format::Picture,
            zoom: 2,
            data: DataStyle::Bytes,
        }
    }
}

/// The files `settings` makes from `solution`, named after `name` (a file stem). A
/// picture, PRG or PNG is one file; a raw set or assembler source several.
pub fn files(
    solution: &Solution,
    palette: &Palette,
    mode: Mode,
    name: &str,
    settings: &Settings,
) -> Result<Vec<(String, Vec<u8>)>, String> {
    let image = &solution.image;
    let picture = || {
        BitmapFormat::for_mode(mode).ok_or_else(|| {
            format!(
                "Can't export {}: no picture format for it yet.",
                mode.name()
            )
        })
    };
    let named = |files: Vec<(std::path::PathBuf, Vec<u8>)>| {
        files
            .into_iter()
            .map(|(p, b)| (p.to_string_lossy().into_owned(), b))
            .collect()
    };
    Ok(match settings.format {
        Format::Picture => {
            let f = picture()?;
            let ext = f.extensions()[0];
            vec![(
                format!("{name}.{ext}"),
                f.write(image).map_err(|e| e.to_string())?,
            )]
        }
        Format::Prg => {
            let f = picture()?;
            let file = f.write(image).map_err(|e| e.to_string())?;
            let prg = convert64_formats::prg::display_prg(f, &file).map_err(|e| e.to_string())?;
            vec![(format!("{name}.prg"), prg)]
        }
        Format::Png => {
            let png = convert64_formats::png::preview(image, palette, settings.zoom)
                .map_err(|e| e.to_string())?;
            vec![(format!("{name}.png"), png)]
        }
        Format::Raw => {
            named(convert64_formats::raw::write(image, name, palette).map_err(|e| e.to_string())?)
        }
        Format::Source(a) => {
            let opts = EmitOptions::new(name).with_data(settings.data);
            named(asm::emit(a, image, &opts).map_err(|e| e.to_string())?)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use convert64_core::solve::SolveMode;

    fn solved() -> (Document, Solution) {
        let rgba = (0..64 * 40)
            .flat_map(|i| [(i % 64 * 4) as u8, (i / 64 * 6) as u8, 90, 255])
            .collect();
        let png = convert64_project::codec::encode_rgba8(64, 40, rgba);
        let doc = Document::from_png("ramp.png", &png).unwrap();
        let mut scheduler = crate::scheduler::Scheduler::stepping(Default::default());
        scheduler.submit(doc.snapshot(None), SolveMode::Full);
        let sol = loop {
            scheduler.tick();
            if let Some(r) = scheduler.poll().filter(|r| !r.provisional) {
                break (*r.solution).clone();
            }
        };
        (doc, sol)
    }

    #[test]
    fn every_format_exports_and_the_picture_reads_back() {
        let (doc, sol) = solved();
        for format in Format::ALL {
            let settings = Settings {
                format,
                ..Settings::default()
            };
            let out = files(&sol, doc.palette(), doc.mode(), "ramp", &settings).unwrap();
            assert!(!out.is_empty(), "{format:?}");
            assert!(out.iter().all(|(_, b)| !b.is_empty()), "{format:?}");
        }
        let koala = files(
            &sol,
            doc.palette(),
            doc.mode(),
            "ramp",
            &Settings::default(),
        )
        .unwrap();
        assert_eq!(koala[0].0, "ramp.kla");
        assert_eq!(koala[0].1.len(), 10_003);
        let back = convert64_formats::read_koala(&koala[0].1).unwrap();
        assert_eq!(back.image, sol.image);
    }
}
