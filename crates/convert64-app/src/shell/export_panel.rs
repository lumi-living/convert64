//! The Export tab: the export panel (UX §6b, usability review M4, #287). Rows of format ·
//! options · file name with its size, **+ Add format**, and one primary **Export**
//! button that writes them all. The rows are the project's export set; every change is
//! an `export_set` command (ADR-005), so it is saved with the project, undoes, and is
//! what `Shift Cmd E`, `convert64 export` and `export_run` write.

use super::{Action, Shell, icon};
use crate::export::RowFiles;
use crate::files::human_size;
use crate::icons;
use crate::theme::Theme;
use crate::workspace::Workspace;
use convert64_core::command::ExportSet;
use convert64_core::export::{
    DEFAULT_ZOOM, ExportData, ExportEntry, ExportFormat, ExportGroup, MAX_ZOOM,
};
use convert64_core::mode::Mode;
use convert64_formats::export::{description, label};
use convert64_project::JsonCommand;
use egui::{Button, RichText, Ui};

/// What the panel last computed each row writes, and for what: the solution (by
/// address), the rows and the project's name. Rebuilt when any of them changes.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ExportCache {
    key: Option<(usize, Vec<ExportEntry>, String)>,
    rows: Vec<RowFiles>,
}

/// The format's entry in the dropdown: its name and extension, "Koala (.kla)".
fn format_text(format: ExportFormat, mode: Mode) -> String {
    let ext = match format {
        ExportFormat::Picture => {
            convert64_formats::BitmapFormat::for_mode(mode).map_or("", |f| f.extensions()[0])
        }
        ExportFormat::Prg => "prg",
        ExportFormat::Raw => "bin",
        ExportFormat::Png => "png",
        other => convert64_formats::export::assembler(other).map_or("", |a| a.extension()),
    };
    format!("{} (.{ext})", label(format, mode))
}

fn set(exports: Vec<ExportEntry>) -> Action {
    Action::Run(JsonCommand::ExportSet(ExportSet { exports }))
}

pub(super) fn show(shell: &mut Shell, ws: &Workspace, ui: &mut Ui, actions: &mut Vec<Action>) {
    let mode = ws.doc.mode();
    let stem = ws.doc.stem();
    let exports = ws.doc.session().project().exports.clone();
    let ready = ws.solution.is_some() && !ws.solving;
    // Rebuild the previews once per solution and set.
    if let Some(sol) = ws.solution.as_ref().filter(|_| ready) {
        let key = (
            std::sync::Arc::as_ptr(sol) as usize,
            exports.clone(),
            stem.clone(),
        );
        if shell.export_cache.key.as_ref() != Some(&key) {
            shell.export_cache.rows =
                crate::export::preview(&exports, &sol.image, ws.doc.palette(), &stem);
            shell.export_cache.key = Some(key);
        }
    }
    let previews = shell
        .export_cache
        .key
        .as_ref()
        .filter(|(_, rows, s)| *rows == exports && *s == stem)
        .map(|_| shell.export_cache.rows.clone());

    if exports.is_empty() {
        ui.label(RichText::new("Choose what to export. The set is saved with the project.").weak());
        ui.add_space(4.0);
    }
    let theme = Theme::current(ui.ctx());
    let mut clash = false;
    for (i, entry) in exports.iter().enumerate() {
        let preview = previews.as_ref().and_then(|p| p.get(i));
        clash |= matches!(preview, Some(RowFiles::Error(_)));
        ui.push_id(("export_row", i), |ui| {
            row(
                shell, ui, &exports, i, entry, mode, &stem, preview, theme, actions,
            );
        });
        ui.add_space(6.0);
    }
    let add = Button::new((icon(icons::ADD), "Add format"));
    if ui
        .add(add)
        .on_hover_text("Add a row to the export set")
        .clicked()
    {
        let mut next = exports.clone();
        next.push(ExportEntry::new(crate::export::next_format(&exports)));
        actions.push(set(next));
    }
    ui.add_space(8.0);

    let n = exports.len();
    let text = if n == 0 {
        "Export".to_owned()
    } else if n == 1 {
        "Export 1 format".to_owned()
    } else {
        format!("Export {n} formats")
    };
    // The panel's one primary button: the selection fill, strong text (dark on the
    // light theme's pale accent, light on the dark theme's deep one), label centred.
    let visuals = ui.visuals();
    let primary = Button::new(
        RichText::new(text)
            .strong()
            .color(visuals.strong_text_color()),
    )
    .fill(visuals.selection.bg_fill);
    let enabled = n > 0 && ready && !clash;
    let why = if n == 0 {
        "Add a format first"
    } else if !ready {
        "Waiting for the full solve"
    } else if clash {
        "Two rows write the same file: rename one"
    } else {
        "Write every row's files into a folder you pick"
    };
    let size = egui::vec2(ui.available_width(), 28.0);
    if ui
        .add_enabled_ui(enabled, |ui| ui.add_sized(size, primary))
        .inner
        .on_hover_text(why)
        .on_disabled_hover_text(why)
        .clicked()
    {
        actions.push(Action::Export { again: false });
    }
    let again = ui.ctx().format_shortcut(&super::shortcuts::EXPORT_AGAIN);
    let note = match &shell.last_export_dir {
        Some(dir) => format!("Last: {dir}/ · Export Again {again}"),
        None => format!("To a folder you pick. Export Again {again}"),
    };
    ui.label(RichText::new(note).small().weak());
}

/// One row: format · options · ×, then the file name and what it writes.
#[allow(clippy::too_many_arguments)]
fn row(
    shell: &mut Shell,
    ui: &mut Ui,
    exports: &[ExportEntry],
    i: usize,
    entry: &ExportEntry,
    mode: Mode,
    stem: &str,
    preview: Option<&RowFiles>,
    theme: &Theme,
    actions: &mut Vec<Action>,
) {
    let changed = |e: ExportEntry| {
        let mut next = exports.to_vec();
        next[i] = e;
        set(next)
    };
    ui.horizontal(|ui| {
        let mut format = entry.format;
        egui::ComboBox::from_id_salt("format")
            .selected_text(format_text(format, mode))
            .width(150.0)
            .show_ui(ui, |ui| {
                for group in ExportGroup::ALL {
                    ui.label(RichText::new(group.label().to_uppercase()).small().weak());
                    for f in ExportFormat::ALL.into_iter().filter(|f| f.group() == group) {
                        ui.selectable_value(&mut format, f, format_text(f, mode))
                            .on_hover_text(description(f, mode));
                    }
                }
            });
        if format != entry.format {
            actions.push(changed(ExportEntry {
                format,
                ..entry.clone()
            }));
        }
        match entry.format {
            ExportFormat::Png => {
                let mut zoom = entry.zoom;
                egui::ComboBox::from_id_salt("zoom")
                    .selected_text(format!("{zoom}×"))
                    .width(44.0)
                    .show_ui(ui, |ui| {
                        for z in 1..=MAX_ZOOM {
                            ui.selectable_value(&mut zoom, z, format!("{z}×"));
                        }
                    })
                    .response
                    .on_hover_text(format!("Zoom (default {DEFAULT_ZOOM}×)"));
                if zoom != entry.zoom {
                    actions.push(changed(ExportEntry {
                        zoom,
                        ..entry.clone()
                    }));
                }
            }
            f if f.is_source() => {
                let mut data = entry.data;
                egui::ComboBox::from_id_salt("data")
                    .selected_text(data.label())
                    .width(0.0)
                    .show_ui(ui, |ui| {
                        for d in ExportData::ALL {
                            ui.selectable_value(&mut data, d, d.label());
                        }
                    })
                    .response
                    .on_hover_text("How the source carries the picture data");
                if data != entry.data {
                    actions.push(changed(ExportEntry {
                        data,
                        ..entry.clone()
                    }));
                }
            }
            _ => {}
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let remove = ui.add(Button::new(icon(icons::CLOSE)).frame(false));
            if remove.on_hover_text("Remove this row").clicked() {
                let mut next = exports.to_vec();
                next.remove(i);
                actions.push(set(next));
            }
        });
    });
    ui.horizontal(|ui| {
        name_field(shell, ui, exports, i, entry, stem, actions);
        match preview {
            Some(RowFiles::Files(files)) => {
                let total: usize = files.iter().map(|(_, n)| n).sum();
                let first = files.first().map_or("", |(n, _)| n.as_str());
                let text = match files.len() {
                    1 => format!("{first} · {}", human_size(total)),
                    2 => format!("{first} + 1 file · {}", human_size(total)),
                    n => format!("{first} + {} files · {}", n - 1, human_size(total)),
                };
                let tip = files
                    .iter()
                    .map(|(n, b)| format!("{n} ({})", human_size(*b)))
                    .collect::<Vec<_>>()
                    .join("\n");
                ui.add(egui::Label::new(RichText::new(text).small().weak()).truncate())
                    .on_hover_text(tip);
            }
            Some(RowFiles::Error(e)) => {
                ui.add(egui::Label::new(RichText::new(e).small().color(theme.error)).truncate())
                    .on_hover_text(e);
            }
            None => {
                ui.label(RichText::new("…").small().weak())
                    .on_hover_text("Waiting for the full solve");
            }
        }
    });
}

/// The row's file name: its own, or empty for the project's (the placeholder). Typed
/// into a buffer and committed on `Enter` or when the field loses focus (UX §6b).
fn name_field(
    shell: &mut Shell,
    ui: &mut Ui,
    exports: &[ExportEntry],
    i: usize,
    entry: &ExportEntry,
    stem: &str,
    actions: &mut Vec<Action>,
) {
    let editing = shell.export_edit.as_ref().filter(|(row, _)| *row == i);
    let mut text = editing.map_or_else(
        || entry.name.clone().unwrap_or_default(),
        |(_, t)| t.clone(),
    );
    let response = ui.add(
        egui::TextEdit::singleline(&mut text)
            .hint_text(stem)
            .desired_width(110.0),
    );
    let response = response.on_hover_text("The files' name; empty for the project's");
    if response.has_focus() || response.changed() {
        shell.export_edit = Some((i, text.clone()));
    }
    if response.lost_focus() {
        shell.export_edit = None;
        let typed = text.trim();
        let name = (!typed.is_empty() && typed != stem).then(|| typed.to_owned());
        if name != entry.name {
            match name.as_deref().map(convert64_core::export::check_stem) {
                Some(Err(e)) => shell.toast(ui.ctx(), format!("Can't use that name: {e}"), true),
                _ => {
                    let mut next = exports.to_vec();
                    next[i] = ExportEntry {
                        name,
                        ..entry.clone()
                    };
                    actions.push(set(next));
                }
            }
        }
    }
}
