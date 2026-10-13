//! The open document: a [`Session`] (the project, its history, its own compositor for the
//! Source view) and what the app derives from it for the scheduler and the views.

use crate::scheduler::Snapshot;
use convert64_core::mode::Mode;
use convert64_core::solve::{Solution, SolveMode};
use convert64_core::{Palette, SuperGrid};
use convert64_project::{JsonCommand, MemoryStore, ProjectError, ProjectFile, Session};
use std::path::Path;
use std::sync::Arc;

/// An open project.
#[derive(Debug)]
pub struct Document {
    /// What the title and the status bar call it: the file name.
    pub name: String,
    session: Session,
    /// Bumped by every edit, so views know when to rebuild what they derive.
    revision: u64,
}

impl Document {
    /// A new project of one PNG, scaled to fill the screen, in multicolor with the
    /// default recipe and palette (what `convert64 convert picture.png` makes).
    pub fn from_png(name: &str, png: &[u8]) -> Result<Self, ProjectError> {
        let source = convert64_project::codec::decode_source(png, name)?;
        let store = Arc::new(MemoryStore::new());
        store.insert(name, png.to_vec());
        let file =
            ProjectFile::from_png(Mode::MulticolorBitmap, name, Path::new(name), source.size());
        let project = format!("{name}.convert64");
        store.insert(&project, file.to_json());
        let session = Session::open_in(Box::new(store), Path::new(&project))?;
        Ok(Self {
            name: name.to_owned(),
            session,
            revision: 0,
        })
    }

    /// The session, for reading.
    pub fn session(&self) -> &Session {
        &self.session
    }

    /// Runs a registry command (ADR-005): the app edits only through these.
    pub fn run(&mut self, command: JsonCommand) -> Result<(), ProjectError> {
        self.session.run(command)?;
        self.revision += 1;
        Ok(())
    }

    /// Changes with every edit.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// The target mode.
    pub fn mode(&self) -> Mode {
        self.session.target()
    }

    /// The super grid: target mode and supersample factor.
    pub fn grid(&self) -> SuperGrid {
        self.session.grid()
    }

    /// The palette solved with and previewed in (TAD §3.4: the same object).
    pub fn palette(&self) -> &Palette {
        self.session.palette()
    }

    /// Everything a solve reads, for the scheduler.
    pub fn snapshot(&self, previous: Option<Arc<Solution>>) -> Snapshot {
        let p = self.session.project();
        Snapshot {
            layers: p.layers.clone(),
            regions: p.regions.clone(),
            steering: p.steering.clone(),
            grid: self.grid(),
            palette: p.palette.clone(),
            options: p.solve.options(SolveMode::Full, Vec::new()),
            previous,
        }
    }

    /// The composite on the super grid as RGBA8, for the Source view.
    pub fn source_rgba(&mut self) -> (u32, u32, Vec<u8>) {
        self.session.canvas_rgba()
    }
}

/// `solution` as RGBA8 on the target grid (160 × 200 multicolor, 320 × 200 hi-res), every
/// pixel a colour of `palette`. Drawn with nearest filtering at 2:1 for multicolor.
pub fn result_rgba(solution: &Solution, palette: &Palette, grid: SuperGrid) -> (u32, u32, Vec<u8>) {
    let (w, h) = grid.target_size();
    let mut out = Vec::with_capacity(w as usize * h as usize * 4);
    for y in 0..h {
        for x in 0..w {
            let c = palette.rgb(solution.image.pixel(x as u16, y as u16));
            out.extend([c.r, c.g, c.b, 255]);
        }
    }
    (w, h, out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use convert64_project::codec::encode_rgba8;

    fn png() -> Vec<u8> {
        let rgba = (0..64 * 40)
            .flat_map(|i| [(i % 64 * 4) as u8, (i / 64 * 6) as u8, 128, 255])
            .collect();
        encode_rgba8(64, 40, rgba)
    }

    #[test]
    fn a_png_opens_as_a_multicolor_project_and_solves() {
        let doc = Document::from_png("ramp.png", &png()).unwrap();
        assert_eq!(doc.mode(), Mode::MulticolorBitmap);
        assert_eq!(doc.session().project().layers.len(), 1);
        let mut session_doc = doc;
        let (w, h, rgba) = session_doc.source_rgba();
        let (sw, sh) = session_doc.grid().size();
        assert_eq!((w, h), (sw, sh));
        assert_eq!(rgba.len(), (w * h * 4) as usize);
        // Solve the snapshot the scheduler would get, synchronously.
        let mut scheduler = crate::scheduler::Scheduler::stepping(Default::default());
        scheduler.submit(session_doc.snapshot(None), SolveMode::Full);
        let result = loop {
            scheduler.tick();
            if let Some(r) = scheduler.poll().filter(|r| !r.provisional) {
                break r;
            }
        };
        let (rw, rh, px) = result_rgba(&result.solution, session_doc.palette(), session_doc.grid());
        assert_eq!((rw, rh), (160, 200));
        assert!(px.chunks(4).all(|p| p[3] == 255));
    }

    #[test]
    fn edits_bump_the_revision() {
        let mut doc = Document::from_png("ramp.png", &png()).unwrap();
        let before = doc.revision();
        doc.run(JsonCommand::ModeSet(convert64_core::command::ModeSet {
            mode: Mode::HiresBitmap,
        }))
        .unwrap();
        assert_eq!(doc.revision(), before + 1);
        assert_eq!(doc.mode(), Mode::HiresBitmap);
    }
}
