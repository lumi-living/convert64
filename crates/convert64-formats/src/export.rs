//! The export set's writers (#287): the files each row of a project's `exports`
//! ([`convert64_core::export`]) makes from a solved picture, named and in bytes. The app's
//! Export panel, `convert64 export` and the MCP server's `export_run` all call
//! [`set_files`], so the three write the same files.

use crate::BitmapFormat;
use crate::asm::{self, Assembler, DataStyle, EmitOptions};
use convert64_core::Palette;
use convert64_core::export::{ExportData, ExportEntry, ExportFormat};
use convert64_core::mode::Mode;
use convert64_core::model::BitmapImage;

/// The assembler a source format emits for.
pub const fn assembler(format: ExportFormat) -> Option<Assembler> {
    Some(match format {
        ExportFormat::KickAssembler => Assembler::KickAssembler,
        ExportFormat::Acme => Assembler::Acme,
        ExportFormat::Tass64 => Assembler::Tass64,
        ExportFormat::Ca65 => Assembler::Ca65,
        ExportFormat::CHeader => Assembler::CHeader,
        _ => return None,
    })
}

const fn data_style(d: ExportData) -> DataStyle {
    match d {
        ExportData::Bytes => DataStyle::Bytes,
        ExportData::IncludeRaw => DataStyle::IncludeRaw,
        ExportData::IncludePicture => DataStyle::IncludePicture,
    }
}

/// The format's name for `mode`: "Koala" (multicolor) or "Art Studio" (hi-res) for the
/// picture file, "PRG", "Raw bitmap set", "KickAssembler", …, "PNG".
pub fn label(format: ExportFormat, mode: Mode) -> String {
    match format {
        ExportFormat::Picture => BitmapFormat::for_mode(mode)
            .map_or("Picture", BitmapFormat::name)
            .to_owned(),
        ExportFormat::Prg => "PRG".to_owned(),
        ExportFormat::Raw => "Raw bitmap set".to_owned(),
        ExportFormat::Png => "PNG".to_owned(),
        other => assembler(other).map_or("", Assembler::name).to_owned(),
    }
}

/// What a row makes, in one line, for its tooltip.
pub fn description(format: ExportFormat, mode: Mode) -> String {
    match format {
        ExportFormat::Picture => format!(
            "The {} picture file, for C64 paint programs and your own loader",
            label(format, mode)
        ),
        ExportFormat::Prg => "A self-displaying program: LOAD and RUN it".to_owned(),
        ExportFormat::Raw => {
            "Headerless .bin blocks (bitmap, screen, colour) and a JSON manifest".to_owned()
        }
        ExportFormat::Png => "A palette-exact PNG of the result, at a whole zoom".to_owned(),
        ExportFormat::CHeader => "A C header with the picture as byte arrays".to_owned(),
        other => format!("{} source with the picture data", label(other, mode)),
    }
}

/// The files `entry` makes from `image`, named after its stem (its own name, else
/// `project`): the source file or picture first, then any file it includes.
///
/// # Errors
/// The row's options are out of range, the mode has no picture format yet, or a writer
/// refused; the message says which.
pub fn files(
    entry: &ExportEntry,
    image: &BitmapImage,
    palette: &Palette,
    project: &str,
) -> Result<Vec<(String, Vec<u8>)>, String> {
    entry.check()?;
    let name = entry.stem(project);
    convert64_core::export::check_stem(name)?;
    let mode = image.mode();
    let picture = || {
        BitmapFormat::for_mode(mode)
            .ok_or_else(|| format!("{} has no picture file format yet", mode.name()))
    };
    let named = |files: Vec<(std::path::PathBuf, Vec<u8>)>| {
        files
            .into_iter()
            .map(|(p, b)| (p.to_string_lossy().into_owned(), b))
            .collect()
    };
    Ok(match entry.format {
        ExportFormat::Picture => {
            let f = picture()?;
            let ext = f.extensions()[0];
            vec![(
                format!("{name}.{ext}"),
                f.write(image).map_err(|e| e.to_string())?,
            )]
        }
        ExportFormat::Prg => {
            let f = picture()?;
            let file = f.write(image).map_err(|e| e.to_string())?;
            let prg = crate::prg::display_prg(f, &file).map_err(|e| e.to_string())?;
            vec![(format!("{name}.prg"), prg)]
        }
        ExportFormat::Png => {
            let png = crate::png::preview(image, palette, entry.zoom).map_err(|e| e.to_string())?;
            vec![(format!("{name}.png"), png)]
        }
        ExportFormat::Raw => {
            named(crate::raw::write(image, name, palette).map_err(|e| e.to_string())?)
        }
        source => {
            let a = assembler(source).expect("the other formats are source");
            let opts = EmitOptions::new(name).with_data(data_style(entry.data));
            named(asm::emit(a, image, &opts).map_err(|e| e.to_string())?)
        }
    })
}

/// Every file the export set makes, row by row, each file once: two rows that include
/// the same block (two assemblers including the raw set) share it.
///
/// # Errors
/// A row fails ([`files`]; the message names the row, 1-based as the panel shows it), or
/// two rows would write different bytes to one file name.
pub fn set_files(
    entries: &[ExportEntry],
    image: &BitmapImage,
    palette: &Palette,
    project: &str,
) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut out: Vec<(String, Vec<u8>, usize)> = Vec::new();
    for (i, entry) in entries.iter().enumerate() {
        let row = i + 1;
        for (name, bytes) in files(entry, image, palette, project)
            .map_err(|e| format!("row {row} ({}): {e}", entry.format))?
        {
            match out.iter().find(|(n, _, _)| *n == name) {
                Some((_, b, _)) if *b == bytes => {}
                Some((_, _, other)) => {
                    return Err(format!(
                        "rows {other} and {row} both write {name}; give one of them another file name"
                    ));
                }
                None => out.push((name, bytes, row)),
            }
        }
    }
    Ok(out.into_iter().map(|(n, b, _)| (n, b)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A busy picture: every bitmap byte and cell colour varies.
    fn picture(mode: Mode) -> BitmapImage {
        let bitmap: Vec<u8> = (0..8000u32).map(|i| (i * 37 % 251) as u8).collect();
        let screen: Vec<u8> = (0..1000u32).map(|i| (i * 13 % 256) as u8).collect();
        match mode {
            Mode::HiresBitmap => BitmapImage::hires_from_bytes(&bitmap, &screen).unwrap(),
            _ => {
                let colour: Vec<u8> = (0..1000u32).map(|i| (i % 16) as u8).collect();
                BitmapImage::multicolor_from_bytes(&bitmap, &screen, &colour, 6).unwrap()
            }
        }
    }

    #[test]
    fn every_format_writes_and_the_picture_reads_back() {
        let palette = Palette::pepto_pal();
        for mode in [Mode::MulticolorBitmap, Mode::HiresBitmap] {
            let image = picture(mode);
            for format in ExportFormat::ALL {
                let out = files(&ExportEntry::new(format), &image, &palette, "title").unwrap();
                assert!(!out.is_empty(), "{format:?}");
                assert!(out.iter().all(|(_, b)| !b.is_empty()), "{format:?}");
                assert!(out[0].0.starts_with("title."), "{format:?}: {}", out[0].0);
                assert!(!label(format, mode).is_empty());
                assert!(!description(format, mode).is_empty());
            }
        }
        let image = picture(Mode::MulticolorBitmap);
        let koala = files(
            &ExportEntry::new(ExportFormat::Picture),
            &image,
            &palette,
            "title",
        )
        .unwrap();
        assert_eq!(koala[0].0, "title.kla");
        assert_eq!(koala[0].1.len(), 10_003);
        assert_eq!(crate::read_koala(&koala[0].1).unwrap().image, image);
        assert_eq!(
            label(ExportFormat::Picture, Mode::MulticolorBitmap),
            "Koala"
        );
        assert_eq!(
            label(ExportFormat::Picture, Mode::HiresBitmap),
            "Art Studio"
        );
        assert_eq!(convert64_core::export::MAX_ZOOM, crate::png::MAX_ZOOM);
    }

    #[test]
    fn a_set_writes_each_file_once_and_refuses_clashing_names() {
        let palette = Palette::pepto_pal();
        let image = picture(Mode::MulticolorBitmap);
        let raw = |format| ExportEntry {
            data: ExportData::IncludeRaw,
            ..ExportEntry::new(format)
        };
        let set = [
            ExportEntry::new(ExportFormat::Picture),
            ExportEntry::new(ExportFormat::Prg),
            raw(ExportFormat::KickAssembler),
            raw(ExportFormat::Acme),
        ];
        let out = set_files(&set, &image, &palette, "title").unwrap();
        let names: Vec<&str> = out.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names.iter().filter(|n| n.ends_with(".bitmap.bin")).count(),
            1
        );
        assert!(names.contains(&"title.kla") && names.contains(&"title.prg"));
        assert!(names.contains(&"title.asm") && names.contains(&"title.a"));

        // Two PNGs at different zooms need different names.
        let png = |zoom, name: Option<&str>| ExportEntry {
            zoom,
            name: name.map(str::to_owned),
            ..ExportEntry::new(ExportFormat::Png)
        };
        let err = set_files(&[png(1, None), png(4, None)], &image, &palette, "title").unwrap_err();
        assert!(
            err.contains("rows 1 and 2") && err.contains("title.png"),
            "{err}"
        );
        let out = set_files(
            &[png(1, None), png(4, Some("title@4x"))],
            &image,
            &palette,
            "title",
        )
        .unwrap();
        assert_eq!(out[1].0, "title@4x.png");
        let err = set_files(&[png(9, None)], &image, &palette, "title").unwrap_err();
        assert!(err.starts_with("row 1 (png): zoom 9"), "{err}");
    }
}
