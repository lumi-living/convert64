//! `cargo xtask png2c64-previews --png2c64 <path>` (#181): png2c64's own renders of its
//! 30 dither methods on the dither-preview test card (strength 1.0) and on the portrait
//! (png2c64's defaults), and its three contact sheets, in
//! `docs/reference/dither-previews/`. These renders are the parity test's reference, so
//! use the reference build (`docs/reference/png2c64-reference.md`).

use crate::baselines::{output, png2c64_commit};
use crate::repo_root;
use crate::sheet::{SHARED, save, sheet, zoom};
use std::fs;
use std::path::Path;
use std::process::Command;

pub(crate) fn run(png2c64: &Path) -> Result<(), String> {
    let png2c64 = fs::canonicalize(png2c64)
        .map_err(|e| format!("png2c64 binary {}: {e}", png2c64.display()))?;
    let version = output(Command::new(&png2c64).arg("--version"))?
        .trim()
        .to_owned();
    println!("using {version}, commit {}", png2c64_commit(&png2c64));

    let root = repo_root();
    let previews = root.join("docs/reference/dither-previews");
    let work = root.join("target/xtask/png2c64-previews");
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work).map_err(|e| format!("{}: {e}", work.display()))?;
    fs::copy(previews.join("testcard.png"), work.join("card.png"))
        .map_err(|e| format!("testcard.png: {e}"))?;
    fs::copy(
        root.join("fixtures/portrait/source.png"),
        work.join("portrait.png"),
    )
    .map_err(|e| format!("portrait source: {e}"))?;

    let mut card_tiles = Vec::new();
    let mut portrait_tiles = Vec::new();
    let mut changed = Vec::new();
    for (id, _) in SHARED {
        // The card at strength 1.0; the portrait with png2c64's defaults (its code's
        // strength default is 1.0, whatever its `--help` says).
        for (input, extra, dir, tiles) in [
            (
                "card.png",
                &["--dither-strength", "1.0"][..],
                "png2c64-card",
                &mut card_tiles,
            ),
            (
                "portrait.png",
                &[][..],
                "png2c64-portrait",
                &mut portrait_tiles,
            ),
        ] {
            let out = format!("{dir}-{id}.png");
            output(
                Command::new(&png2c64)
                    .args(["--mode", "multicolor", "--palette", "pepto", "--dither", id])
                    .args(extra)
                    .args([input, &out])
                    .current_dir(&work),
            )?;
            let dest = previews.join(dir).join(format!("{id}.png"));
            let bytes = fs::read(work.join(&out)).map_err(|e| format!("{out}: {e}"))?;
            if fs::read(&dest).ok().as_deref() != Some(&bytes[..]) {
                fs::write(&dest, &bytes).map_err(|e| format!("{}: {e}", dest.display()))?;
                changed.push(format!("{dir}/{id}.png"));
            }
            let img = image::load_from_memory(&bytes)
                .map_err(|e| format!("{out}: {e}"))?
                .to_rgb8();
            tiles.push((id.to_owned(), img));
        }
    }
    let zooms: Vec<_> = card_tiles
        .iter()
        .map(|(id, img)| (id.clone(), zoom(img)))
        .collect();
    save(&sheet(&card_tiles), &previews.join("sheet-card.png"))?;
    save(&sheet(&zooms), &previews.join("sheet-zoom.png"))?;
    save(
        &sheet(&portrait_tiles),
        &previews.join("sheet-portrait.png"),
    )?;
    if changed.is_empty() {
        println!("all 60 renders unchanged; sheets rewritten");
    } else {
        println!("{} renders changed:", changed.len());
        for c in &changed {
            println!("  {c}");
        }
    }
    Ok(())
}
