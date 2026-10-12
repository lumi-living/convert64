//! The working view (`docs/UX.md` §4): the source composite or the zoomed result. Until
//! the result panel and working view land (#204) it shows the empty state.

use super::{Shell, icon};
use crate::icons;
use egui::{RichText, Ui};

pub(super) fn show(shell: &Shell, ui: &mut Ui) {
    if shell.has_project {
        return;
    }
    let open = ui.ctx().format_shortcut(&super::shortcuts::OPEN);
    ui.centered_and_justified(|ui| {
        ui.vertical_centered(|ui| {
            let block = 3.0 * crate::theme::layout::TOOLBAR_HEIGHT;
            ui.add_space((ui.available_height() - block).max(0.0) / 2.0);
            ui.label(
                icon(icons::ADD_IMAGE)
                    .size(2.0 * crate::theme::layout::ICON_SIZE)
                    .weak(),
            );
            ui.label(RichText::new("Open an image or a .convert64 project").strong());
            ui.label(RichText::new(format!("File › Open… ({open}), or drop a file here")).weak());
        });
    });
}
