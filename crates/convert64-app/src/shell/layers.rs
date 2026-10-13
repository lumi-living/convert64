//! The Layers tab (UX §5, Figma `Layer Row`): blend mode and opacity of the selected
//! layer above the list; rows top layer first with a drag handle, eye, thumbnail, mask
//! chain, name and recipe badge; the icon strip below. Every edit is a registry command
//! (ADR-005), returned as an [`Action`].

use super::{Action, Shell, icon};
use crate::icons;
use crate::theme::layout;
use crate::workspace::Workspace;
use convert64_core::Blend;
use convert64_core::layer::Layer;
use convert64_project::{JsonCommand, LayerRef};
use egui::{Button, RichText, Ui};

/// Gesture id of the opacity slider (one undo step per drag).
const OPACITY_GESTURE: u64 = 0x6f70_6163;

/// Blend modes, in the dropdown's order.
const BLENDS: [(Blend, &str); 4] = [
    (Blend::Normal, "Normal"),
    (Blend::Multiply, "Multiply"),
    (Blend::Screen, "Screen"),
    (Blend::Overlay, "Overlay"),
];

fn at(i: usize) -> LayerRef {
    LayerRef::Index(i)
}

/// The recipe in short form for the badge, e.g. `FS 100%`, and in full for its tooltip,
/// e.g. `floyd-steinberg 100% · Oklab · Adaptive` (UX §5).
pub(super) fn recipe_text(layer: &Layer) -> (String, String) {
    let r = layer.recipe();
    let id = serde_json::to_value(r.method().kind)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| "custom".to_owned());
    let short = match id.as_str() {
        "floyd-steinberg" => "FS".to_owned(),
        "none" => "None".to_owned(),
        other => {
            let mut c = other.chars();
            c.next()
                .map_or_else(String::new, |f| {
                    f.to_uppercase().chain(c).collect::<String>()
                })
                .replace('-', " ")
        }
    };
    let pct = (r.strength() * 100.0).round();
    (
        format!("{short} {pct}%"),
        format!("{id} {pct}% · {:?} · {:?}", r.metric(), r.fit_mode()),
    )
}

pub(super) fn show(shell: &mut Shell, ws: &Workspace, ui: &mut Ui, actions: &mut Vec<Action>) {
    let layers = &ws.doc.session().project().layers;
    if layers.is_empty() {
        ui.label(
            "Drop an image here or open one. Convert64 shows the C64 result as soon as it loads.",
        );
        if ui.button("Open image…").clicked() {
            actions.push(Action::Open);
        }
        return;
    }
    shell.layer = shell.layer.min(layers.len() - 1);
    let selected = &layers[shell.layer];

    // Blend mode and opacity of the selected layer, above the list.
    ui.horizontal(|ui| {
        let current = BLENDS
            .iter()
            .find(|(b, _)| *b == selected.blend())
            .map_or("Normal", |(_, n)| n);
        egui::ComboBox::from_id_salt("layer_blend")
            .selected_text(current)
            .show_ui(ui, |ui| {
                for (blend, name) in BLENDS {
                    if ui
                        .selectable_label(selected.blend() == blend, name)
                        .clicked()
                        && selected.blend() != blend
                    {
                        actions.push(Action::Run(JsonCommand::LayerSetBlend {
                            layer: at(shell.layer),
                            blend,
                        }));
                    }
                }
            });
        let before = selected.opacity() * 100.0;
        let mut pct = before;
        // The slider takes what is left beside its value box, so the dock never grows.
        let value_box = 3.0 * ui.spacing().interact_size.x / 2.0;
        ui.spacing_mut().slider_width =
            (ui.available_width() - value_box).max(ui.spacing().interact_size.x);
        let slider = ui
            .add(
                egui::Slider::new(&mut pct, 0.0..=100.0)
                    .suffix("%")
                    .integer(),
            )
            .on_hover_text("Opacity");
        // `integer()` snaps 33.3 % to 33: only a real move is an edit, or the value
        // would be re-sent every frame.
        let moved = slider.changed() && (pct - before).abs() > 0.5;
        if moved {
            let cmd = JsonCommand::LayerSetOpacity {
                layer: at(shell.layer),
                opacity: pct / 100.0,
            };
            actions.push(Action::Gesture(OPACITY_GESTURE, cmd));
        }
        if slider.drag_stopped() || (moved && !slider.dragged()) {
            actions.push(Action::EndGesture);
        }
    });
    ui.add_space(ui.spacing().item_spacing.y);

    // Rows, top layer first.
    let n = layers.len();
    let order: Vec<usize> = (0..n).rev().collect();
    let response = egui_dnd::dnd(ui, "layers").show(
        order.iter().copied().map(LayerItem),
        |ui, item, handle, _state| {
            row(shell, ws, item.0, ui, handle, actions);
        },
    );
    if let Some(update) = response.final_update() {
        // Display positions run top first; the stack bottom first.
        let to_display = if update.to > update.from {
            update.to - 1
        } else {
            update.to
        };
        let (from, to) = (n - 1 - update.from, n - 1 - to_display);
        if from != to {
            actions.push(Action::Run(JsonCommand::LayerReorder {
                layer: at(from),
                to,
            }));
            shell.layer = to;
        }
    }

    ui.separator();
    strip(shell, layers, ui, actions);
}

/// An item of the drag list: a stack index.
#[derive(Clone, Copy, Hash)]
struct LayerItem(usize);

impl egui_dnd::DragDropItem for LayerItem {
    fn id(&self) -> egui::Id {
        egui::Id::new(("layer_row", self.0))
    }
}

fn row(
    shell: &mut Shell,
    ws: &Workspace,
    i: usize,
    ui: &mut Ui,
    handle: egui_dnd::Handle<'_>,
    actions: &mut Vec<Action>,
) {
    let layer = &ws.doc.session().project().layers[i];
    let is_selected = shell.layer == i;
    let size = egui::vec2(ui.available_width(), layout::LAYER_ROW_HEIGHT);
    ui.allocate_ui_with_layout(
        size,
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            if is_selected {
                let bg = ui.visuals().selection.bg_fill;
                ui.painter().rect_filled(
                    ui.max_rect(),
                    ui.visuals().widgets.inactive.corner_radius,
                    bg,
                );
            }
            ui.spacing_mut().item_spacing.x = 4.0;
            handle.ui(ui, |ui| {
                ui.label(icon(icons::DRAG).weak());
            });
            let eye = if layer.visible() {
                icons::VISIBLE
            } else {
                icons::HIDDEN
            };
            let tip = if layer.visible() {
                "Hide layer"
            } else {
                "Show layer"
            };
            if ui
                .add(Button::new(icon(eye)).frame(false))
                .on_hover_text(tip)
                .clicked()
            {
                actions.push(Action::Run(JsonCommand::LayerSetVisible {
                    layer: at(i),
                    visible: !layer.visible(),
                }));
            }
            if let Some(tex) = ws.thumbnail(i) {
                let side = layout::LAYER_ROW_HEIGHT - 8.0;
                let [w, h] = tex.size().map(|v| v as f32);
                let s = side / w.max(h);
                ui.add(egui::Image::new((tex.id(), egui::vec2(w * s, h * s))));
            }
            if layer.mask().is_some() {
                ui.label(icon(icons::LINK).weak())
                    .on_hover_text("The mask moves with the layer");
                mask_thumb(shell, ws, i, ui);
            }
            // Right-aligned parts first, so the name truncates into what is left.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new(format!("{:.0}%", layer.opacity() * 100.0))
                        .small()
                        .weak(),
                );
                let (short, long) = recipe_text(layer);
                let badge = egui::Frame::NONE
                    .fill(ui.visuals().faint_bg_color)
                    .corner_radius(ui.visuals().widgets.inactive.corner_radius)
                    .inner_margin(egui::Margin::symmetric(4, 1))
                    .show(ui, |ui| ui.label(RichText::new(short).small()));
                badge.response.on_hover_text(long);
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    name(shell, layer, i, ui, actions);
                });
            });
        },
    );
}

/// The layer's name: a click selects the row, a double click renames (Enter or
/// clicking away applies, Escape cancels).
fn name(shell: &mut Shell, layer: &Layer, i: usize, ui: &mut Ui, actions: &mut Vec<Action>) {
    let renaming = shell.renaming.as_ref().is_some_and(|(r, _)| *r == i);
    if renaming {
        let (_, text) = shell.renaming.as_mut().expect("renaming");
        let edit = ui.add(egui::TextEdit::singleline(text).desired_width(ui.available_width()));
        edit.request_focus();
        if edit.lost_focus() {
            let (_, name) = shell.renaming.take().expect("renaming");
            let escaped = ui.input(|inp| inp.key_pressed(egui::Key::Escape));
            if !escaped && !name.trim().is_empty() && name != layer.name() {
                actions.push(Action::Run(JsonCommand::LayerRename {
                    layer: at(i),
                    name: name.trim().to_owned(),
                }));
            }
        }
        return;
    }
    let mut text = RichText::new(layer.name());
    if !layer.visible() {
        text = text.weak();
    }
    let label = ui.add(
        egui::Label::new(text)
            .truncate()
            .sense(egui::Sense::click()),
    );
    if label.clicked() {
        shell.layer = i;
    }
    if label.double_clicked() {
        shell.renaming = Some((i, layer.name().to_owned()));
    }
    // A click on the rest of the row selects it too.
    if ui
        .allocate_response(ui.available_size(), egui::Sense::click())
        .clicked()
    {
        shell.layer = i;
    }
}

/// The bottom icon strip: New from file, Duplicate, Layer via mask, Add mask, Delete.
fn strip(shell: &mut Shell, layers: &[Layer], ui: &mut Ui, actions: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        let ctx = ui.ctx().clone();
        let tip = |label: &str, s: &egui::KeyboardShortcut| {
            format!("{label} ({})", ctx.format_shortcut(s))
        };
        let b = |glyph| Button::new(icon(glyph)).frame(false);
        if ui
            .add(b(icons::ADD))
            .on_hover_text(tip("New layer from file", &super::shortcuts::NEW_LAYER))
            .clicked()
        {
            actions.push(Action::NewLayer);
        }
        if ui
            .add(b(icons::DUPLICATE))
            .on_hover_text(tip("Duplicate", &super::shortcuts::DUPLICATE_LAYER))
            .clicked()
        {
            actions.push(Action::Run(JsonCommand::LayerDuplicate {
                layer: at(shell.layer),
            }));
        }
        let has_selection = shell.mask.selection.is_some();
        let via = ui
            .add_enabled(has_selection, b(icons::MASK))
            .on_hover_text(tip("Layer via mask", &super::shortcuts::DUPLICATE_LAYER))
            .on_disabled_hover_text(
                "Layer via mask: make a selection with the mask tools first (W, G, K)",
            );
        if via.clicked() {
            super::mask::via_mask(shell, false, actions);
        }
        let has_mask = layers.get(shell.layer).is_some_and(|l| l.mask().is_some());
        let add = ui
            .add_enabled(!has_mask, b(icons::FILL))
            .on_hover_text("Add mask (from the picture's alpha if it has any)")
            .on_disabled_hover_text("The layer has a mask: M edits it");
        if add.clicked() {
            actions.push(add_mask(layers, shell.layer));
        }
        let delete = ui.add(b(icons::DELETE)).on_hover_text("Delete (⌫)");
        if delete.clicked() {
            actions.push(Action::Run(JsonCommand::LayerRemove {
                layer: at(shell.layer),
            }));
        }
    });
}

/// The mask thumbnail: a click edits the mask (mask mode), `Alt`-click shows it alone.
fn mask_thumb(shell: &mut Shell, ws: &Workspace, i: usize, ui: &mut Ui) {
    let Some(tex) = ws.mask_thumbnail(i) else {
        return;
    };
    let side = layout::LAYER_ROW_HEIGHT - 8.0;
    let [w, h] = tex.size().map(|v| v as f32);
    let s = side / w.max(h);
    let image = egui::Image::new((tex.id(), egui::vec2(w * s, h * s))).sense(egui::Sense::click());
    let thumb = ui
        .add(image)
        .on_hover_text("Edit mask · Alt-click: the mask alone");
    if thumb.clicked() {
        shell.layer = i;
        shell.mask.alone = ui.input(|inp| inp.modifiers.alt);
        if !shell.mask.active {
            shell.toggle_mask_mode();
        }
    }
}

/// Add mask (UX §6): from the picture's alpha if any pixel is transparent, otherwise
/// fully opaque.
pub(super) fn add_mask(layers: &[Layer], i: usize) -> Action {
    let layer = at(i);
    let alpha = layers
        .get(i)
        .is_some_and(|l| l.source().pixels().iter().any(|p| p[3] < 1.0));
    Action::Run(if alpha {
        JsonCommand::MaskFromAlpha { layer }
    } else {
        JsonCommand::MaskClear { layer, value: 1.0 }
    })
}
