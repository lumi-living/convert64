//! The autosave's two pieces of chrome (#241): the restore bar under the toolbar (a
//! non-modal offer of the last session) and, natively, the question on quitting with
//! unsaved changes (UX §11: a modal only where data would be lost).

use super::{Action, QuitAnswer, Shell};
use egui::{RichText, Ui};

/// The restore bar: "Restore your last session (title.convert64, 3 layers, 2 min ago)?
/// Restore · Discard". The app keeps it while the offer stands.
pub(super) fn restore_bar(text: &str, ui: &mut Ui, actions: &mut Vec<Action>) {
    ui.horizontal_centered(|ui| {
        ui.label(text);
        if ui
            .button(RichText::new("Restore").strong())
            .on_hover_text("Open it as unsaved work")
            .clicked()
        {
            actions.push(Action::RestoreSession);
        }
        ui.label("·");
        if ui
            .link("Discard")
            .on_hover_text("Delete the autosaved session")
            .clicked()
        {
            actions.push(Action::DiscardSession);
        }
    });
}

/// The quit question, while `shell.quit_prompt` names the document.
pub(super) fn quit_prompt(shell: &mut Shell, ctx: &egui::Context, actions: &mut Vec<Action>) {
    let Some(name) = shell.quit_prompt.clone() else {
        return;
    };
    let mut answer = None;
    let modal = egui::Modal::new(egui::Id::new("quit_prompt")).show(ctx, |ui| {
        ui.set_max_width(360.0);
        ui.label(RichText::new(format!("Save changes to {name} before quitting?")).strong());
        ui.label("Your changes are lost if you don't save them.");
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button(RichText::new("Save").strong()).clicked() {
                answer = Some(QuitAnswer::Save);
            }
            if ui.button("Don't save").clicked() {
                answer = Some(QuitAnswer::DontSave);
            }
            if ui.button("Cancel").clicked() {
                answer = Some(QuitAnswer::Cancel);
            }
        });
    });
    if modal.should_close() && answer.is_none() {
        answer = Some(QuitAnswer::Cancel);
    }
    if let Some(a) = answer {
        shell.quit_prompt = None;
        actions.push(Action::AnswerQuit(a));
    }
}
