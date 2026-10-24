//! Export (UX §6b, #287): the project's export set as named files, made from the result
//! on screen by `convert64_formats::export`, the same calls `convert64 export` and the
//! MCP server's `export_run` make. The app then writes them into a folder (natively) or
//! through the browser's direct save path (#241).

use convert64_core::Palette;
use convert64_core::export::{ExportEntry, ExportFormat};
use convert64_core::model::BitmapImage;

/// The files the whole set makes, each once, named after `stem` where a row has no
/// name of its own.
///
/// # Errors
/// A row fails, or two rows write different bytes to one name (the message says which).
pub fn files(
    exports: &[ExportEntry],
    image: &BitmapImage,
    palette: &Palette,
    stem: &str,
) -> Result<Vec<(String, Vec<u8>)>, String> {
    convert64_formats::export::set_files(exports, image, palette, stem)
}

/// What one row of the Export panel writes: its files' names and sizes, or why it can't.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowFiles {
    /// The files, the first being the row's main one.
    Files(Vec<(String, usize)>),
    /// The row can't be written; the message says why.
    Error(String),
}

/// What each row writes, for the panel's file name previews, and the clashes between
/// rows: a row that would write different bytes to a file an earlier row writes says so.
pub fn preview(
    exports: &[ExportEntry],
    image: &BitmapImage,
    palette: &Palette,
    stem: &str,
) -> Vec<RowFiles> {
    let mut seen: Vec<(String, Vec<u8>, usize)> = Vec::new();
    exports
        .iter()
        .enumerate()
        .map(
            |(i, entry)| match convert64_formats::export::files(entry, image, palette, stem) {
                Ok(files) => {
                    for (name, bytes) in &files {
                        match seen.iter().find(|(n, _, _)| n == name) {
                            Some((_, b, _)) if b == bytes => {}
                            Some((_, _, row)) => {
                                return RowFiles::Error(format!(
                                    "Same file as row {}: give one another name",
                                    row + 1
                                ));
                            }
                            None => seen.push((name.clone(), bytes.clone(), i)),
                        }
                    }
                    RowFiles::Files(files.into_iter().map(|(n, b)| (n, b.len())).collect())
                }
                Err(e) => RowFiles::Error(e),
            },
        )
        .collect()
}

/// The format **+ Add format** adds next: the first in this order the set lacks (UX §6b).
pub fn next_format(exports: &[ExportEntry]) -> ExportFormat {
    use ExportFormat as F;
    [
        F::Picture,
        F::Prg,
        F::Png,
        F::KickAssembler,
        F::Acme,
        F::Tass64,
        F::Ca65,
        F::CHeader,
        F::Raw,
    ]
    .into_iter()
    .find(|f| !exports.iter().any(|e| e.format == *f))
    .unwrap_or(F::Png)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use convert64_core::solve::{Solution, SolveMode};

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
    fn rows_preview_their_files_and_clashes() {
        let (doc, sol) = solved();
        let png = |zoom| ExportEntry {
            zoom,
            ..ExportEntry::new(ExportFormat::Png)
        };
        let set = [
            ExportEntry::new(ExportFormat::Picture),
            ExportEntry::new(ExportFormat::Prg),
            png(1),
            png(4),
        ];
        let rows = preview(&set, &sol.image, doc.palette(), "ramp");
        assert_eq!(
            rows[0],
            RowFiles::Files(vec![("ramp.kla".to_owned(), 10_003)])
        );
        assert!(matches!(&rows[1], RowFiles::Files(f) if f[0].0 == "ramp.prg"));
        assert!(matches!(&rows[2], RowFiles::Files(f) if f[0].0 == "ramp.png"));
        assert_eq!(
            rows[3],
            RowFiles::Error("Same file as row 3: give one another name".to_owned())
        );
        assert!(files(&set, &sol.image, doc.palette(), "ramp").is_err());
        let out = files(&set[..3], &sol.image, doc.palette(), "ramp").unwrap();
        let back = convert64_formats::read_koala(&out[0].1).unwrap();
        assert_eq!(back.image, sol.image);
    }

    #[test]
    fn add_format_picks_the_next_one_missing() {
        let mut set = Vec::new();
        let mut order = Vec::new();
        for _ in 0..4 {
            let f = next_format(&set);
            order.push(f);
            set.push(ExportEntry::new(f));
        }
        use ExportFormat as F;
        assert_eq!(order, [F::Picture, F::Prg, F::Png, F::KickAssembler]);
    }
}
