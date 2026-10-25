//! Mask tools (UX §7a, ADR-008): Brush `B`, Lasso `W`, Gradient `G` and Key `K` on the
//! selected layer's mask in mask mode (`M`), with the red quick-mask overlay and the
//! options bar (snap, feather, invert, clear, fill, load, from alpha). With mask mode off
//! the same tools build a **selection** for Layer via mask (`Cmd J`, `Shift Cmd J`;
//! UX §5a). Every edit is a registry command; a stroke, polygon, gradient or key is one.

use super::{Action, Shell};
use crate::icons;
use crate::theme::Theme;
use crate::view::Compare;
use crate::workspace::Workspace;
use convert64_core::Vec2;
use convert64_core::command::{ModifierChange, Selection};
use convert64_core::mask::{
    Combine, Gradient, GradientKind, Key, KeyChannel, MaskSource, PaintMode, SnapToCells,
    SourceEntry,
};
use convert64_project::{JsonCommand, LayerRef};
use egui::{Pos2, Rect, Response, RichText, Ui};

/// The mask tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaskTool {
    /// Soft round brush (mask mode only).
    Brush,
    /// Freehand lasso, or a polygon clicked vertex by vertex.
    Lasso,
    /// Linear or radial gradient.
    Gradient,
    /// Luma, chroma or colour key on the layer's pixels.
    Key,
}

impl MaskTool {
    /// Every tool, in toolbar order.
    pub const ALL: [Self; 4] = [Self::Brush, Self::Lasso, Self::Gradient, Self::Key];

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Brush => "Brush",
            Self::Lasso => "Lasso",
            Self::Gradient => "Gradient",
            Self::Key => "Key",
        }
    }

    pub(super) fn icon(self) -> icons::Icon {
        match self {
            Self::Brush => icons::BRUSH,
            Self::Lasso => icons::LASSO,
            Self::Gradient => icons::GRADIENT,
            Self::Key => icons::KEY,
        }
    }

    /// The tool's key (UX §8).
    pub fn key(self) -> egui::Key {
        match self {
            Self::Brush => egui::Key::B,
            Self::Lasso => egui::Key::W,
            Self::Gradient => egui::Key::G,
            Self::Key => egui::Key::K,
        }
    }
}

/// One brush's options. Every tool keeps its own, so picking a tool never carries
/// another tool's Add/Subtract or size over (usability review M7, #290).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BrushOptions {
    /// Add or subtract (`X` swaps; `Alt` subtracts while held).
    pub mode: PaintMode,
    /// Brush radius in screen units (C64 hi-res pixels; `[` and `]`).
    pub size: f32,
    /// Brush hardness, `0..=1`.
    pub hardness: f32,
    /// Brush flow, `0..=1`.
    pub flow: f32,
}

impl Default for BrushOptions {
    fn default() -> Self {
        Self {
            mode: PaintMode::Add,
            size: 6.0,
            hardness: 0.5,
            flow: 1.0,
        }
    }
}

/// `X`: Add becomes Subtract and back.
pub(super) fn swapped(mode: PaintMode) -> PaintMode {
    match mode {
        PaintMode::Add => PaintMode::Subtract,
        PaintMode::Subtract => PaintMode::Add,
    }
}

/// The mode a stroke paints in: `mode`, or Subtract while `Alt` is held over Add.
fn held(mode: PaintMode, alt: bool) -> PaintMode {
    if alt && mode == PaintMode::Add {
        PaintMode::Subtract
    } else {
        mode
    }
}

/// Mask mode, and the options of the mask tools and the Region brush, each its own.
#[derive(Debug, Clone, PartialEq)]
pub struct MaskState {
    /// Mask mode: the tools edit the selected layer's mask, the overlay shows it.
    pub active: bool,
    /// The mask tool in use; `None` while a steering tool is.
    pub tool: Option<MaskTool>,
    /// The mask Brush's options.
    pub brush: BrushOptions,
    /// The Lasso's Add or Subtract.
    pub lasso: PaintMode,
    /// The Region tool's brush (UX §7), not the mask Brush's.
    pub region: BrushOptions,
    /// Gradient shape and softness.
    pub gradient: (GradientKind, f32),
    /// How a gradient or key combines with what is below it.
    pub combine: Combine,
    /// Key channel, range and softness.
    pub key: (KeyChannel, f32, f32),
    /// The mask alone, in grey, instead of the overlay (`Alt`-click the thumbnail).
    pub alone: bool,
    /// The selection made with mask mode off.
    pub selection: Option<Selection>,
    /// A polygon being clicked vertex by vertex, in screen units.
    pub polygon: Vec<Pos2>,
    /// A drag in progress: its points in screen units.
    pub stroke: Vec<Pos2>,
}

impl Default for MaskState {
    fn default() -> Self {
        Self {
            active: false,
            tool: None,
            brush: BrushOptions::default(),
            lasso: PaintMode::Add,
            region: BrushOptions::default(),
            gradient: (GradientKind::Linear, 1.0),
            combine: Combine::Max,
            key: (KeyChannel::Colour, 0.1, 0.05),
            alone: false,
            selection: None,
            polygon: Vec::new(),
            stroke: Vec::new(),
        }
    }
}

fn layer(shell: &Shell) -> LayerRef {
    LayerRef::Index(shell.layer)
}

fn v2(p: Pos2) -> Vec2 {
    Vec2::new(p.x, p.y)
}

/// A screen-unit point in the selected layer's pixels.
fn to_layer(ws: &Workspace, i: usize, p: Pos2) -> Option<Vec2> {
    let l = ws.doc.session().project().layers.get(i)?;
    Some(l.transform().canvas_to_layer(l.source().size(), v2(p)))
}

/// Layer pixels per screen unit for the selected layer (the brush radius' scale).
fn layer_per_unit(ws: &Workspace, i: usize) -> f32 {
    ws.doc
        .session()
        .project()
        .layers
        .get(i)
        .map_or(1.0, |l| 1.0 / l.transform().scale().x.abs().max(1e-6))
}

/// The options bar's sliders: narrow, so a whole bar fits beside the dock.
pub(super) fn narrow_sliders(ui: &mut Ui) {
    ui.spacing_mut().slider_width = 56.0;
}

/// A label, then its slider: labels sit before controls, as in the Recipe tab
/// (usability review M8).
fn labelled(ui: &mut Ui, label: &str, slider: egui::Slider<'_>) -> Response {
    ui.label(label);
    ui.add(slider)
}

/// The mask tools' options in the options bar (UX §7a).
pub(super) fn options_bar(
    shell: &mut Shell,
    ws: Option<&Workspace>,
    ui: &mut Ui,
    actions: &mut Vec<Action>,
) {
    let Some(tool) = shell.mask.tool else { return };
    let what = if shell.mask.active {
        "Mask"
    } else {
        "Selection"
    };
    ui.label(
        RichText::new(format!("{what} · {}", tool.label()))
            .small()
            .strong(),
    );
    ui.separator();
    let m = &mut shell.mask;
    match tool {
        MaskTool::Brush => mode_toggle(&mut m.brush.mode, ui),
        MaskTool::Lasso => mode_toggle(&mut m.lasso, ui),
        MaskTool::Gradient | MaskTool::Key => {}
    }
    match tool {
        MaskTool::Brush => brush_sliders(&mut m.brush, ui),
        MaskTool::Lasso => {
            ui.label(
                RichText::new("Drag freehand, or click vertices and double-click to close")
                    .small()
                    .weak(),
            );
        }
        MaskTool::Gradient => {
            for (k, n) in [
                (GradientKind::Linear, "Linear"),
                (GradientKind::Radial, "Radial"),
            ] {
                if ui.selectable_label(m.gradient.0 == k, n).clicked() {
                    m.gradient.0 = k;
                }
            }
            labelled(
                ui,
                "Softness",
                egui::Slider::new(&mut m.gradient.1, 0.0..=1.0),
            );
        }
        MaskTool::Key => {
            for (k, n) in [
                (KeyChannel::Luma, "Luma"),
                (KeyChannel::Chroma, "Chroma"),
                (KeyChannel::Colour, "Colour"),
            ] {
                if ui.selectable_label(m.key.0 == k, n).clicked() {
                    m.key.0 = k;
                }
            }
            labelled(ui, "Range", egui::Slider::new(&mut m.key.1, 0.0..=1.0));
            labelled(ui, "Softness", egui::Slider::new(&mut m.key.2, 0.0..=1.0));
        }
    }
    if matches!(tool, MaskTool::Gradient | MaskTool::Key) {
        ui.label("Combine");
        egui::ComboBox::from_id_salt("mask_combine")
            .selected_text(format!("{:?}", m.combine))
            .width(70.0)
            .show_ui(ui, |ui| {
                for c in [Combine::Max, Combine::Min, Combine::Multiply] {
                    ui.selectable_value(&mut m.combine, c, format!("{c:?}"));
                }
            });
    }
    ui.separator();
    if shell.mask.active {
        mask_buttons(shell, ws, ui, actions);
    } else {
        selection_buttons(shell, ui, actions);
    }
}

/// Add · Subtract for the tool whose own mode is `own`.
fn mode_toggle(own: &mut PaintMode, ui: &mut Ui) {
    for (mode, name) in [(PaintMode::Add, "Add"), (PaintMode::Subtract, "Subtract")] {
        if ui
            .selectable_label(*own == mode, name)
            .on_hover_text("X swaps; Alt subtracts while held")
            .clicked()
        {
            *own = mode;
        }
    }
}

/// A brush's Size, Hardness and Flow.
fn brush_sliders(b: &mut BrushOptions, ui: &mut Ui) {
    labelled(
        ui,
        "Size",
        egui::Slider::new(&mut b.size, 0.5..=64.0).logarithmic(true),
    )
    .on_hover_text("[ and ] change it");
    labelled(
        ui,
        "Hardness",
        egui::Slider::new(&mut b.hardness, 0.0..=1.0),
    );
    labelled(ui, "Flow", egui::Slider::new(&mut b.flow, 0.0..=1.0));
}

/// What *Reveal all* and *Hide all* say they do.
const REVEAL_ALL_TIP: &str = "Fill the mask white: the whole layer shows, and paint hides it";
const HIDE_ALL_TIP: &str = "Fill the mask black: the layer is hidden, and paint shows it";
/// What *From transparency* does (it read "From alpha").
pub(super) const FROM_TRANSPARENCY_TIP: &str =
    "Mask from the picture's transparency: transparent pixels hide the layer, opaque ones show it";

/// Starts the selected layer's mask, or refills it, as Photoshop's *Reveal all*
/// (white) or *Hide all* (black), and sets the mask Brush and the Lasso to the mode
/// whose stroke shows on it: Subtract after Reveal all, Add after Hide all (usability
/// review M8).
pub(super) fn reveal_or_hide_all(shell: &mut Shell, reveal: bool) -> Action {
    let mode = if reveal {
        PaintMode::Subtract
    } else {
        PaintMode::Add
    };
    shell.mask.brush.mode = mode;
    shell.mask.lasso = mode;
    Action::Run(JsonCommand::MaskClear {
        layer: layer(shell),
        value: if reveal { 1.0 } else { 0.0 },
    })
}

/// Reveal all · Hide all, and From transparency when the picture has any.
pub(super) fn new_mask_buttons(
    shell: &mut Shell,
    l: &convert64_core::layer::Layer,
    ui: &mut Ui,
    actions: &mut Vec<Action>,
) {
    let small = |t: &str| egui::Button::new(RichText::new(t).small());
    if ui
        .add(small("Reveal all"))
        .on_hover_text(REVEAL_ALL_TIP)
        .clicked()
    {
        actions.push(reveal_or_hide_all(shell, true));
    }
    if ui
        .add(small("Hide all"))
        .on_hover_text(HIDE_ALL_TIP)
        .clicked()
    {
        actions.push(reveal_or_hide_all(shell, false));
    }
    let alpha = l.source().pixels().iter().any(|p| p[3] < 1.0);
    if alpha
        && ui
            .add(small("From transparency"))
            .on_hover_text(FROM_TRANSPARENCY_TIP)
            .clicked()
    {
        actions.push(Action::Run(JsonCommand::MaskFromAlpha {
            layer: layer(shell),
        }));
    }
}

/// Mask mode's modifier and raster buttons; on a layer without a mask, the choice to
/// start one (usability review M8).
fn mask_buttons(shell: &mut Shell, ws: Option<&Workspace>, ui: &mut Ui, actions: &mut Vec<Action>) {
    let Some(ws) = ws else { return };
    let Some(l) = ws.doc.session().project().layers.get(shell.layer) else {
        return;
    };
    let Some(mask) = l.mask() else {
        ui.label(
            RichText::new("This layer has no mask: start from")
                .small()
                .weak(),
        );
        new_mask_buttons(shell, l, ui, actions);
        return;
    };
    let mods = mask.modifiers();
    let target = layer(shell);
    let set = |actions: &mut Vec<Action>, g: Option<u64>, modifier| {
        let cmd = JsonCommand::MaskSetModifier {
            layer: target.clone(),
            modifier,
        };
        actions.push(match g {
            Some(id) => Action::Gesture(id, cmd),
            None => Action::Run(cmd),
        });
    };
    let snap = mods.snap();
    ui.label("Snap");
    egui::ComboBox::from_id_salt("mask_snap")
        .selected_text(format!("{snap:?}"))
        .width(70.0)
        .show_ui(ui, |ui| {
            for s in [
                SnapToCells::Off,
                SnapToCells::Expand,
                SnapToCells::Contract,
                SnapToCells::Nearest,
            ] {
                if ui.selectable_label(s == snap, format!("{s:?}")).clicked() && s != snap {
                    set(actions, None, ModifierChange::Snap(s));
                }
            }
        });
    let before = mods.feather();
    let mut feather = before;
    let r = labelled(
        ui,
        "Feather",
        egui::Slider::new(&mut feather, 0.0..=32.0).suffix(" px"),
    );
    if r.changed() && (feather - before).abs() > 1e-3 {
        set(
            actions,
            r.dragged().then_some(0x6665_6174),
            ModifierChange::Feather(feather),
        );
    }
    if r.drag_stopped() {
        actions.push(Action::EndGesture);
    }
    ui.separator();
    // The raster buttons, each with its name (they were bare icons; usability review M8).
    let small = |t: &str| RichText::new(t).small();
    if ui
        .add(egui::Button::selectable(mods.invert(), small("Invert")))
        .on_hover_text("Invert the mask: what shows is hidden and what is hidden shows")
        .clicked()
    {
        set(actions, None, ModifierChange::Invert(!mods.invert()));
    }
    if ui
        .button(small("Reveal all"))
        .on_hover_text(REVEAL_ALL_TIP)
        .clicked()
    {
        actions.push(reveal_or_hide_all(shell, true));
    }
    if ui
        .button(small("Hide all"))
        .on_hover_text(HIDE_ALL_TIP)
        .clicked()
    {
        actions.push(reveal_or_hide_all(shell, false));
    }
    if ui
        .button(small("Load PNG…"))
        .on_hover_text("Load a greyscale PNG as the mask: white shows, black hides")
        .clicked()
    {
        actions.push(Action::LoadMask);
    }
    if ui
        .button(small("From transparency"))
        .on_hover_text(FROM_TRANSPARENCY_TIP)
        .clicked()
    {
        actions.push(Action::Run(JsonCommand::MaskFromAlpha { layer: target }));
    }
    ui.separator();
    // How the mask is shown (it read "Alone").
    ui.label("View");
    for (grey, name, tip) in [
        (false, "Overlay", "The mask as a red overlay on the layer"),
        (
            true,
            "Grey",
            "The mask alone, in grey: white shows the layer, black hides it (Alt-click the mask thumbnail)",
        ),
    ] {
        if ui
            .selectable_label(shell.mask.alone == grey, small(name))
            .on_hover_text(tip)
            .clicked()
        {
            shell.mask.alone = grey;
        }
    }
}

/// Selection mode's buttons: Layer via mask, via cut, deselect.
fn selection_buttons(shell: &mut Shell, ui: &mut Ui, actions: &mut Vec<Action>) {
    let parts = shell
        .mask
        .selection
        .as_ref()
        .map_or(0, |s| s.polygons.len() + s.sources.len());
    ui.label(
        RichText::new(format!("{parts} part{}", if parts == 1 { "" } else { "s" }))
            .small()
            .weak(),
    );
    let ctx = ui.ctx().clone();
    let has = parts > 0;
    let via = format!(
        "Layer via mask ({})",
        crate::shell::shortcuts::format(&ctx, &super::shortcuts::DUPLICATE_LAYER)
    );
    if ui
        .add_enabled(has, egui::Button::new(RichText::new(via).small()))
        .clicked()
    {
        via_mask(shell, false, actions);
    }
    let cut = format!(
        "Via cut ({})",
        crate::shell::shortcuts::format(&ctx, &super::shortcuts::VIA_CUT)
    );
    if ui
        .add_enabled(has, egui::Button::new(RichText::new(cut).small()))
        .clicked()
    {
        via_mask(shell, true, actions);
    }
    if ui
        .add_enabled(has, egui::Button::new(RichText::new("Deselect").small()))
        .clicked()
    {
        shell.mask.selection = None;
    }
}

/// Layer via mask (UX §5a): a new layer sharing the selected layer's pixels with the
/// selection as its mask; with `cut` the original keeps the inverse.
pub(super) fn via_mask(shell: &mut Shell, cut: bool, actions: &mut Vec<Action>) {
    if let Some(selection) = shell.mask.selection.take() {
        actions.push(Action::Run(JsonCommand::LayerViaMask {
            source_layer: layer(shell),
            selection,
            cut,
        }));
    }
}

/// Handles the mask tool's clicks and drags on the working view.
pub(super) fn interact(
    shell: &mut Shell,
    ws: &Workspace,
    ui: &Ui,
    response: &Response,
    view: Rect,
    actions: &mut Vec<Action>,
) {
    let Some(tool) = shell.mask.tool else { return };
    if shell.compare == Compare::SideBySide || ui.input(|i| i.key_down(egui::Key::Space)) {
        return;
    }
    let i = shell.layer;
    let to_picture = |p: Pos2| shell.camera.to_picture(view, shell.view, p);
    let alt = ui.input(|inp| inp.modifiers.alt);
    let shift = ui.input(|inp| inp.modifiers.shift);

    // A stroke starts where the button went down, not where egui noticed the drag (#263).
    if response.drag_started_by(egui::PointerButton::Primary)
        && let Some(p) = super::drag_origin(ui, response)
    {
        shell.mask.stroke = vec![to_picture(p)];
    }
    if response.dragged_by(egui::PointerButton::Primary)
        && let Some(p) = response.interact_pointer_pos()
    {
        let mut p = to_picture(p);
        if tool == MaskTool::Gradient
            && shift
            && let Some(&a) = shell.mask.stroke.first()
        {
            p = constrain_45(a, p);
        }
        if matches!(tool, MaskTool::Gradient | MaskTool::Key) {
            shell.mask.stroke.truncate(1);
        }
        shell.mask.stroke.push(p);
    }
    let finished = response
        .drag_stopped()
        .then(|| std::mem::take(&mut shell.mask.stroke));
    let clicked = response
        .clicked()
        .then(|| response.interact_pointer_pos())
        .flatten()
        .map(to_picture);
    let double = response.double_clicked();

    match tool {
        MaskTool::Brush if shell.mask.active => {
            let points = finished.or_else(|| clicked.map(|p| vec![p]));
            if let Some(points) = points {
                let points: Vec<Vec2> = points
                    .into_iter()
                    .filter_map(|p| to_layer(ws, i, p))
                    .collect();
                if !points.is_empty() {
                    let b = shell.mask.brush;
                    let cmd = JsonCommand::MaskStroke {
                        layer: layer(shell),
                        points,
                        radius: b.size * layer_per_unit(ws, i),
                        hardness: b.hardness,
                        flow: b.flow,
                        mode: held(b.mode, alt),
                    };
                    actions.push(paint_on(ws, i, held(b.mode, alt), cmd));
                }
            }
        }
        MaskTool::Brush => {}
        MaskTool::Lasso => {
            let polygon = if let Some(points) = finished.filter(|p| p.len() > 2) {
                Some(points)
            } else {
                if let Some(p) = clicked {
                    shell.mask.polygon.push(p);
                }
                let close = double || ui.input(|inp| inp.key_pressed(egui::Key::Enter));
                (close && shell.mask.polygon.len() > 2)
                    .then(|| std::mem::take(&mut shell.mask.polygon))
            };
            if let Some(polygon) = polygon {
                let polygon: Vec<Vec2> = polygon
                    .into_iter()
                    .filter_map(|p| to_layer(ws, i, p))
                    .collect();
                if shell.mask.active {
                    let mode = held(shell.mask.lasso, alt);
                    let cmd = JsonCommand::MaskFill {
                        layer: layer(shell),
                        polygon,
                        value: 1.0,
                        mode,
                    };
                    actions.push(paint_on(ws, i, mode, cmd));
                } else {
                    shell
                        .mask
                        .selection
                        .get_or_insert_with(Selection::default)
                        .polygons
                        .push(polygon);
                }
            }
        }
        MaskTool::Gradient => {
            if let Some(points) = finished
                && let (Some(&a), Some(&b)) = (points.first(), points.last())
                && let (Some(from), Some(to)) = (to_layer(ws, i, a), to_layer(ws, i, b))
                && let Ok(gradient) =
                    Gradient::new(shell.mask.gradient.0, from, to, shell.mask.gradient.1)
            {
                let combine = shell.mask.combine;
                if shell.mask.active {
                    actions.push(Action::Run(JsonCommand::MaskGradient {
                        layer: layer(shell),
                        gradient,
                        combine,
                    }));
                } else {
                    let entry = SourceEntry {
                        source: MaskSource::Gradient(gradient),
                        combine,
                    };
                    shell
                        .mask
                        .selection
                        .get_or_insert_with(Selection::default)
                        .sources
                        .push(entry);
                }
            }
        }
        MaskTool::Key => {
            // Click samples the pixel; a drag also widens the range with its length.
            let sample = finished
                .as_ref()
                .and_then(|p| Some((*p.first()?, p.last()?.distance(*p.first()?))))
                .or(clicked.map(|p| (p, 0.0)));
            if let Some((p, widen)) = sample
                && let Some(centre) = sample_pixel(ws, i, p)
            {
                let (channel, range, softness) = shell.mask.key;
                let range = (range + widen / 100.0).min(2.0);
                if let Ok(key) = Key::new(channel, centre, range, softness) {
                    let combine = shell.mask.combine;
                    if shell.mask.active {
                        actions.push(Action::Run(JsonCommand::MaskKey {
                            layer: layer(shell),
                            key,
                            combine,
                        }));
                    } else {
                        let entry = SourceEntry {
                            source: MaskSource::Key(key),
                            combine,
                        };
                        shell
                            .mask
                            .selection
                            .get_or_insert_with(Selection::default)
                            .sources
                            .push(entry);
                    }
                }
            }
        }
    }
}

/// A brush stroke or lasso fill `cmd` on layer `i`. On a layer with no mask yet it
/// first starts one so that the paint shows (usability review M8): an Add starts from
/// Hide all, a Subtract from Reveal all. The core would start every mask white, where an
/// Add changes nothing.
fn paint_on(ws: &Workspace, i: usize, mode: PaintMode, cmd: JsonCommand) -> Action {
    let has_mask = ws
        .doc
        .session()
        .project()
        .layers
        .get(i)
        .is_none_or(|l| l.mask().is_some());
    if has_mask {
        return Action::Run(cmd);
    }
    let (value, from) = match mode {
        PaintMode::Add => (0.0, "Hide all"),
        PaintMode::Subtract => (1.0, "Reveal all"),
    };
    let start = JsonCommand::MaskClear {
        layer: LayerRef::Index(i),
        value,
    };
    Action::RunAll(vec![start, cmd], format!("New mask from {from}"))
}

/// `b` moved to the nearest 45° direction from `a`.
fn constrain_45(a: Pos2, b: Pos2) -> Pos2 {
    let d = b - a;
    let angle =
        (d.y.atan2(d.x) / std::f32::consts::FRAC_PI_4).round() * std::f32::consts::FRAC_PI_4;
    a + egui::vec2(angle.cos(), angle.sin()) * d.length()
}

/// The selected layer's pixel under screen point `p`, linear RGB.
fn sample_pixel(ws: &Workspace, i: usize, p: Pos2) -> Option<[f32; 3]> {
    let l = ws.doc.session().project().layers.get(i)?;
    let q = to_layer(ws, i, p)?;
    let (w, h) = l.source().size();
    if q.x < 0.0 || q.y < 0.0 || q.x >= w as f32 || q.y >= h as f32 {
        return None;
    }
    let px = l.source().pixel(q.x as u32, q.y as u32);
    Some([
        px[0].clamp(0.0, 1.0),
        px[1].clamp(0.0, 1.0),
        px[2].clamp(0.0, 1.0),
    ])
}

/// Draws the brush cursor, the stroke, lasso or gradient in progress and the selection.
pub(super) fn draw(
    shell: &Shell,
    ws: &Workspace,
    painter: &egui::Painter,
    view: Rect,
    hover: Option<Pos2>,
) {
    let Some(tool) = shell.mask.tool else { return };
    let picture = shell.camera.picture_rect(view, shell.view);
    let s = shell.camera.scale(shell.view);
    let at = |p: Pos2| picture.min + p.to_vec2() * s;
    let theme = Theme::current(painter.ctx());
    let stroke = egui::Stroke::new(1.5, theme.accent);
    if tool == MaskTool::Brush
        && let Some(h) = hover
    {
        painter.circle_stroke(at(h), shell.mask.brush.size * s, stroke);
    }
    let line: Vec<Pos2> = shell
        .mask
        .stroke
        .iter()
        .chain(&shell.mask.polygon)
        .map(|p| at(*p))
        .collect();
    if line.len() > 1 {
        painter.add(egui::Shape::line(line, stroke));
    }
    // The selection's lasso polygons, outlined (sources show as the overlay's count).
    if let Some(sel) = &shell.mask.selection
        && let Some(l) = ws.doc.session().project().layers.get(shell.layer)
    {
        let size = l.source().size();
        for poly in &sel.polygons {
            let pts: Vec<Pos2> = poly
                .iter()
                .map(|q| {
                    let c = l.transform().layer_to_canvas(size, *q);
                    at(egui::pos2(c.x, c.y))
                })
                .collect();
            painter.add(egui::Shape::closed_line(
                pts,
                egui::Stroke::new(1.5, theme.region),
            ));
        }
    }
}

/// The Region tool's options bar: its own brush's Add or Subtract, size, hardness and
/// flow (usability review M7), and a note while the selected region is empty.
pub(super) fn region_bar(shell: &mut Shell, ws: Option<&Workspace>, ui: &mut Ui) {
    ui.label(RichText::new("Region · Brush").small().strong());
    ui.separator();
    let b = &mut shell.mask.region;
    mode_toggle(&mut b.mode, ui);
    brush_sliders(b, ui);
    let regions = ws.map(|ws| &ws.doc.session().project().regions);
    if let Some(r) = regions.and_then(|r| r.get(shell.region))
        && region_is_empty(r)
    {
        ui.separator();
        ui.label(
            RichText::new(format!(
                "{} is empty: paint on the picture to add to it",
                r.name()
            ))
            .small()
            .weak(),
        );
    }
}

/// Nothing painted in the region yet.
pub(super) fn region_is_empty(r: &convert64_core::region::Region) -> bool {
    r.mask().values().iter().all(|v| *v <= 0.0)
}

/// The Region tool on the working view: a drag (or click) paints the selected region's
/// canvas-space mask with the brush; with no region yet, the stroke makes one first.
pub(super) fn region_interact(
    shell: &mut Shell,
    ws: &Workspace,
    ui: &Ui,
    response: &Response,
    view: Rect,
    actions: &mut Vec<Action>,
) {
    use convert64_project::command::RegionRef;
    if shell.compare == Compare::SideBySide || ui.input(|i| i.key_down(egui::Key::Space)) {
        return;
    }
    let to_picture = |p: Pos2| shell.camera.to_picture(view, shell.view, p);
    // A stroke starts where the button went down, not where egui noticed the drag (#263).
    if response.drag_started_by(egui::PointerButton::Primary)
        && let Some(p) = super::drag_origin(ui, response)
    {
        shell.mask.stroke = vec![to_picture(p)];
    }
    if response.dragged_by(egui::PointerButton::Primary)
        && let Some(p) = response.interact_pointer_pos()
    {
        shell.mask.stroke.push(to_picture(p));
    }
    let finished = response
        .drag_stopped()
        .then(|| std::mem::take(&mut shell.mask.stroke));
    let clicked = response
        .clicked()
        .then(|| response.interact_pointer_pos())
        .flatten()
        .map(|p| vec![to_picture(p)]);
    let Some(points) = finished.or(clicked) else {
        return;
    };
    // A stroke whose brush never reaches the picture paints nothing: no empty region
    // and no "Painted" toast for a click on the border (#242).
    let picture = Rect::from_min_size(Pos2::ZERO, crate::view::SCREEN);
    let b = shell.mask.region;
    if points.iter().all(|p| picture.distance_to_pos(*p) > b.size) {
        return;
    }
    let alt = ui.input(|i| i.modifiers.alt);
    let project_regions = &ws.doc.session().project().regions;
    let regions = project_regions.len();
    let mut commands = Vec::new();
    if regions == 0 {
        commands.push(JsonCommand::RegionAdd {
            name: "Region 1".to_owned(),
            recipe: Box::default(),
            index: None,
        });
        shell.region = 0;
    }
    let index = shell.region.min(regions.max(1) - 1);
    // The first stroke of a new or empty region always adds, so it shows (usability
    // review M7): a Subtract there removes nothing. The bar goes back to Add with it.
    let fresh = project_regions.get(index).is_none_or(region_is_empty);
    let mode = if fresh {
        shell.mask.region.mode = PaintMode::Add;
        PaintMode::Add
    } else {
        held(b.mode, alt)
    };
    commands.push(JsonCommand::RegionStroke {
        region: RegionRef::Index(index),
        points: points.into_iter().map(v2).collect(),
        radius: b.size,
        hardness: b.hardness,
        flow: b.flow,
        mode,
    });
    let message = if regions == 0 {
        "Added Region 1"
    } else {
        "Painted the region"
    };
    actions.push(Action::RunAll(commands, message.to_owned()));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;

    fn workspace() -> Workspace {
        let rgba = (0..64 * 40)
            .flat_map(|i| [(i % 64 * 4) as u8, 90, 90, 255])
            .collect();
        let png = convert64_project::codec::encode_rgba8(64, 40, rgba);
        Workspace::new(Document::from_png("ramp.png", &png).unwrap())
    }

    type Handler = fn(&mut Shell, &Workspace, &Ui, &Response, Rect, &mut Vec<Action>);

    /// Drags from picture point `from` by `by` points in small steps that start inside
    /// egui's drag threshold, with `handle` (the mask tools or the Region tool) on each
    /// frame, and returns every action.
    fn drag_with(
        shell: &mut Shell,
        ws: &Workspace,
        from: Pos2,
        by: egui::Vec2,
        handle: Handler,
    ) -> Vec<Action> {
        use super::super::drag_test::{VIEW, drag, path};
        let picture = shell.camera.picture_rect(VIEW, shell.view);
        let from = picture.min + from.to_vec2() * shell.camera.scale(shell.view);
        let mut actions = Vec::new();
        drag(from, &path(from, from + by), |ui, response| {
            handle(shell, ws, ui, response, VIEW, &mut actions);
        });
        actions
    }

    fn close(a: Vec2, b: Vec2) -> bool {
        (a.x - b.x).abs() < 1e-3 && (a.y - b.y).abs() < 1e-3
    }

    /// #263: a mask brush stroke starts where the button went down, not 6 pt along.
    #[test]
    fn a_mask_stroke_starts_at_the_press_point() {
        let mut ws = workspace();
        ws.doc
            .run(JsonCommand::MaskClear {
                layer: LayerRef::Index(0),
                value: 1.0,
            })
            .unwrap();
        let mut shell = Shell::default();
        shell.mask.active = true;
        shell.mask.tool = Some(MaskTool::Brush);
        let press = egui::pos2(100.0, 60.0);
        let actions = drag_with(&mut shell, &ws, press, egui::vec2(40.0, 0.0), interact);
        let points = actions
            .iter()
            .find_map(|a| match a {
                Action::Run(JsonCommand::MaskStroke { points, .. }) => Some(points.clone()),
                _ => None,
            })
            .expect("a stroke");
        let start = to_layer(&ws, 0, press).unwrap();
        assert!(close(points[0], start), "{:?} != {start:?}", points[0]);
        // It ends where the button came up: 40 points at the default 2× is 20 units on.
        let end = to_layer(&ws, 0, press + egui::vec2(20.0, 0.0)).unwrap();
        let last = *points.last().unwrap();
        assert!(close(last, end), "{last:?} != {end:?}");
    }

    /// #263: a gradient runs from the press point.
    #[test]
    fn a_gradient_runs_from_the_press_point() {
        let ws = workspace();
        let mut shell = Shell::default();
        shell.mask.active = true;
        shell.mask.tool = Some(MaskTool::Gradient);
        let press = egui::pos2(100.0, 60.0);
        let actions = drag_with(&mut shell, &ws, press, egui::vec2(60.0, 0.0), interact);
        let from = actions
            .iter()
            .find_map(|a| match a {
                Action::Run(JsonCommand::MaskGradient { gradient, .. }) => Some(gradient.from()),
                _ => None,
            })
            .expect("a gradient");
        let start = to_layer(&ws, 0, press).unwrap();
        assert!(close(from, start), "{from:?} != {start:?}");
    }

    /// #263: a Region brush stroke starts at the press point too.
    #[test]
    fn a_region_stroke_starts_at_the_press_point() {
        let ws = workspace();
        let mut shell = Shell {
            tool: super::super::Tool::Region,
            ..Shell::default()
        };
        let press = egui::pos2(100.0, 60.0);
        let by = egui::vec2(40.0, 0.0);
        let actions = drag_with(&mut shell, &ws, press, by, region_interact);
        let points = actions
            .iter()
            .find_map(|a| match a {
                Action::RunAll(cmds, _) => cmds.iter().find_map(|c| match c {
                    JsonCommand::RegionStroke { points, .. } => Some(points.clone()),
                    _ => None,
                }),
                _ => None,
            })
            .expect("a region stroke");
        assert!(close(points[0], v2(press)), "{:?}", points[0]);
    }

    /// One frame of the whole shell, 1600 × 900 (the test fonts are wider than the
    /// app's), with `events`: each text drawn with its rectangle, the actions, and the
    /// working view's rectangle.
    fn shell_frame(
        ctx: &egui::Context,
        shell: &mut Shell,
        ws: &mut Workspace,
        events: Vec<egui::Event>,
    ) -> (Vec<(String, Rect)>, Vec<Action>, Rect) {
        fn collect(shape: &egui::Shape, out: &mut Vec<(String, Rect)>) {
            match shape {
                egui::Shape::Text(t) => {
                    out.push((t.galley.text().to_owned(), t.visual_bounding_rect()));
                }
                egui::Shape::Vec(v) => v.iter().for_each(|s| collect(s, out)),
                _ => {}
            }
        }
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1600.0, 900.0))),
            events,
            ..Default::default()
        };
        let mut actions = Vec::new();
        let mut output = ctx.run_ui(input, |ui| actions = shell.show(ui, Some(ws)));
        output.textures_delta.clear();
        let mut texts = Vec::new();
        for clipped in &output.shapes {
            collect(&clipped.shape, &mut texts);
        }
        let view = shell.view_rect.expect("the working view was drawn");
        (texts, actions, view)
    }

    fn text(texts: &[(String, Rect)], t: &str) -> Rect {
        texts
            .iter()
            .find(|(s, _)| s == t)
            .unwrap_or_else(|| panic!("no {t:?} in {texts:?}"))
            .1
    }

    /// A press and release at `p`, as two frames' events.
    fn click(p: Pos2) -> [Vec<egui::Event>; 2] {
        let button = |pressed| egui::Event::PointerButton {
            pos: p,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        [
            vec![egui::Event::PointerMoved(p), button(true)],
            vec![button(false)],
        ]
    }

    /// #291 (usability review M8): the options row is always there, so the working
    /// view and the result panel keep their rectangles in every tool and in mask mode.
    #[test]
    fn the_layout_does_not_shift_between_modes() {
        let mut ws = workspace();
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        type Set = fn(&mut Shell);
        let mut rects = |set: Set| {
            let mut shell = Shell::default();
            set(&mut shell);
            shell_frame(&ctx, &mut shell, &mut ws, Vec::new());
            let (texts, _, view) = shell_frame(&ctx, &mut shell, &mut ws, Vec::new());
            (view, text(&texts, "Result"), text(&texts, "Recipe"))
        };
        let normal = rects(|_| {});
        let states: [(&str, Set); 5] = [
            ("mask mode", |s| s.toggle_mask_mode()),
            ("selection", |s| s.mask.tool = Some(MaskTool::Lasso)),
            ("region", |s| s.tool = super::super::Tool::Region),
            ("pin", |s| s.tool = super::super::Tool::Pin),
            ("transform", |s| s.tool = super::super::Tool::Transform),
        ];
        for (name, set) in states {
            assert_eq!(rects(set), normal, "{name}");
        }
    }

    /// #291: in mask mode the labels come before their controls, the raster buttons
    /// have their names, and "From alpha" and "Alone" are gone.
    #[test]
    fn the_mask_bar_has_labels_before_controls_and_named_buttons() {
        let mut ws = workspace();
        ws.doc
            .run(JsonCommand::MaskClear {
                layer: LayerRef::Index(0),
                value: 1.0,
            })
            .unwrap();
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let mut shell = Shell::default();
        shell.toggle_mask_mode();
        shell_frame(&ctx, &mut shell, &mut ws, Vec::new());
        let (texts, _, view) = shell_frame(&ctx, &mut shell, &mut ws, Vec::new());
        // The bar's texts: the row right above the working view.
        let row = crate::theme::layout::TOOLBAR_HEIGHT;
        let bar: Vec<_> = texts
            .iter()
            .filter(|(_, r)| r.center().y > view.top() - row && r.center().y < view.top())
            .cloned()
            .collect();
        let left = |t: &str| text(&bar, t).left();
        // Label, then its value: Size 6, Hardness 0.50, Flow 1.00, Feather 0.0 px.
        let order = [
            "Size", "6.0", "Hardness", "0.50", "Flow", "1.00", "Snap", "Off", "Feather",
        ];
        for pair in order.windows(2) {
            assert!(left(pair[0]) < left(pair[1]), "{pair:?}: {bar:?}");
        }
        for name in [
            "Invert",
            "Reveal all",
            "Hide all",
            "Load PNG…",
            "From transparency",
            "View",
            "Overlay",
            "Grey",
        ] {
            text(&bar, name);
        }
        assert!(
            !bar.iter().any(|(t, _)| t == "From alpha" || t == "Alone"),
            "{bar:?}"
        );
    }

    /// #291 (usability review M8): a new mask starts as chosen. On a layer without one,
    /// the bar offers Reveal all and Hide all; each starts the mask and sets the brush to
    /// the mode whose stroke shows on it. A stroke made before choosing starts a mask it
    /// shows on.
    #[test]
    fn a_new_mask_starts_as_chosen() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        for (button, value, mode) in [
            ("Reveal all", 1.0, PaintMode::Subtract),
            ("Hide all", 0.0, PaintMode::Add),
        ] {
            let mut ws = workspace();
            let mut shell = Shell::default();
            shell.toggle_mask_mode();
            shell_frame(&ctx, &mut shell, &mut ws, Vec::new());
            let (texts, _, _) = shell_frame(&ctx, &mut shell, &mut ws, Vec::new());
            text(&texts, "This layer has no mask: start from");
            let at = text(&texts, button).center();
            let mut asked = Vec::new();
            for events in click(at) {
                asked.extend(shell_frame(&ctx, &mut shell, &mut ws, events).1);
            }
            let cleared: Vec<f32> = asked
                .iter()
                .filter_map(|a| match a {
                    Action::Run(JsonCommand::MaskClear { value, .. }) => Some(*value),
                    _ => None,
                })
                .collect();
            assert_eq!(cleared, vec![value], "{button}: {asked:?}");
            assert_eq!(shell.mask.brush.mode, mode, "{button}");
            assert_eq!(shell.mask.lasso, mode, "{button}");
        }

        // A stroke before a choice: Add starts from Hide all, Subtract from Reveal all.
        for (mode, value) in [(PaintMode::Add, 0.0), (PaintMode::Subtract, 1.0)] {
            let ws = workspace();
            let mut shell = Shell::default();
            shell.toggle_mask_mode();
            shell.mask.brush.mode = mode;
            let actions = drag_with(
                &mut shell,
                &ws,
                egui::pos2(100.0, 60.0),
                egui::vec2(40.0, 0.0),
                interact,
            );
            let Some(Action::RunAll(cmds, _)) = actions.first() else {
                panic!("{actions:?}");
            };
            assert!(
                matches!(cmds.as_slice(), [
                    JsonCommand::MaskClear { value: v, .. },
                    JsonCommand::MaskStroke { mode: m, .. },
                ] if *v == value && *m == mode),
                "{cmds:?}"
            );
        }
    }

    /// The modes of the region strokes in `actions`.
    fn region_modes(actions: &[Action]) -> Vec<PaintMode> {
        actions
            .iter()
            .flat_map(|a| match a {
                Action::RunAll(cmds, _) => cmds.clone(),
                _ => Vec::new(),
            })
            .filter_map(|c| match c {
                JsonCommand::RegionStroke { mode, .. } => Some(mode),
                _ => None,
            })
            .collect()
    }

    /// #290 (usability review M7): the first stroke of a new region adds even with the
    /// Region brush on Subtract, and the bar goes back to Add; so does the first stroke
    /// on an empty region. A region with something in it gets the brush's own mode.
    #[test]
    fn the_first_stroke_of_a_new_region_adds() {
        let mut ws = workspace();
        let mut shell = Shell {
            tool: super::super::Tool::Region,
            ..Shell::default()
        };
        let (press, by) = (egui::pos2(100.0, 60.0), egui::vec2(40.0, 0.0));
        shell.mask.region.mode = PaintMode::Subtract;
        let actions = drag_with(&mut shell, &ws, press, by, region_interact);
        let Some(Action::RunAll(cmds, message)) = actions.first() else {
            panic!("{actions:?}");
        };
        assert!(matches!(cmds[0], JsonCommand::RegionAdd { .. }), "{cmds:?}");
        assert_eq!(message, "Added Region 1");
        assert_eq!(region_modes(&actions), vec![PaintMode::Add]);
        assert_eq!(shell.mask.region.mode, PaintMode::Add);

        // An empty region (Add region) gets an Add first too.
        ws.doc
            .run(JsonCommand::RegionAdd {
                name: "Sky".to_owned(),
                recipe: Box::default(),
                index: None,
            })
            .unwrap();
        shell.mask.region.mode = PaintMode::Subtract;
        let actions = drag_with(&mut shell, &ws, press, by, region_interact);
        assert_eq!(region_modes(&actions), vec![PaintMode::Add]);

        // Once it has paint, Subtract subtracts.
        for action in actions {
            if let Action::RunAll(cmds, _) = action {
                ws.doc.run_all(cmds).unwrap();
            }
        }
        shell.mask.region.mode = PaintMode::Subtract;
        let actions = drag_with(&mut shell, &ws, press, by, region_interact);
        assert_eq!(region_modes(&actions), vec![PaintMode::Subtract]);
    }

    /// #290: a painted region has an outline from the moment it exists; an empty one has
    /// nothing to outline, and the Region bar says so instead.
    #[test]
    fn a_painted_region_is_outlined() {
        let mut ws = workspace();
        let ctx = egui::Context::default();
        let mut shell = Shell {
            tool: super::super::Tool::Region,
            ..Shell::default()
        };
        let actions = drag_with(
            &mut shell,
            &ws,
            egui::pos2(100.0, 60.0),
            egui::vec2(40.0, 0.0),
            region_interact,
        );
        for action in actions {
            if let Action::RunAll(cmds, _) = action {
                ws.doc.run_all(cmds).unwrap();
            }
        }
        ws.doc
            .run(JsonCommand::RegionAdd {
                name: "Sky".to_owned(),
                recipe: Box::default(),
                index: None,
            })
            .unwrap();
        assert!(ws.region_overlay(&ctx).is_some());
        let outlines = ws.region_outlines();
        assert_eq!(outlines.len(), 2);
        assert!(!outlines[0].is_empty(), "Region 1 is outlined");
        assert!(outlines[1].is_empty(), "Sky is empty");
        // The outline hugs the stroke: 100,60 to 120,60 with a 6-unit brush.
        let bounds = outlines[0]
            .iter()
            .flatten()
            .fold(Rect::NOTHING, |r, p| r.union(Rect::from_pos(*p)));
        assert!(
            bounds.contains(egui::pos2(110.0, 60.0)) && bounds.width() < 40.0,
            "{bounds:?}"
        );
        let regions = &ws.doc.session().project().regions;
        assert!(!region_is_empty(&regions[0]) && region_is_empty(&regions[1]));
    }
}
