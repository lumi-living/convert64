//! The Recipe tab (UX §6): the layer header (thumbnail and name), then the selected
//! layer's groups ordered by use (#288): dither recipe, colour fitting, adjustments,
//! sharpen, transform, mask, then **Advanced**: the solver (the png2c64 reference, png2amiga's for
//! comparison or the native one, ADR-014), ADR-010's divergences from png2c64 and the
//! solver's own settings, collapsed and off by default. Every change is a registry command
//! (`adjustments_set`, `recipe_set`, `solve_set`); a drag is one undo step.
//!
//! Conventions (UX §6): label, then slider with its value field, on one row; a dot
//! before a label whose value differs from the default; a double click on a label
//! resets that value; each group has a Reset.

use super::method_picker::RecipeTarget;
use super::{Action, Shell};
use crate::theme::{Theme, layout};
use crate::workspace::Workspace;
use convert64_core::ColourIndex;
use convert64_core::SolveSettings;
use convert64_core::command::{AdjustmentsChange, RecipeChange, SolveSet};
use convert64_core::layer::{
    Adjustments, BlackPoint, BrightnessContrast, Curve, Curves, DitherFamily, FitMode, Gamma, Hsl,
    Layer, Levels, Method, Metric, Orientation, PatternPhase, Radius, RangeMatch, RangeMethod,
    Recipe, Sharpen,
};
use convert64_core::solve::{BackgroundRule, Divergences, SolverKind};
use convert64_project::{JsonCommand, LayerRef};
use egui::{Response, RichText, Ui};

/// The width of a row's label column, in points.
const LABEL_WIDTH: f32 = 90.0;

/// The Recipe tab's groups, top to bottom (UX §6, usability review M5, #288): the
/// dither method first, Sharpen right after Adjustments (#237), Advanced last.
pub const GROUP_ORDER: [&str; 7] = [
    "Dither recipe",
    "Colour fitting",
    "Adjustments",
    "Sharpen",
    "Transform",
    "Mask",
    "Advanced",
];

/// Which of the Recipe tab's groups are open (UX §6): the user's own choice, kept in the
/// app's storage (eframe's, not the project) and the same for every layer and region.
/// Transform, Mask and Advanced start collapsed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecipeGroups {
    /// The groups the user opened (true) or closed (false); the others are at their
    /// default.
    chosen: std::collections::BTreeMap<String, bool>,
}

impl RecipeGroups {
    /// Whether `title` is open before the user opens or closes it.
    pub fn default_open(title: &str) -> bool {
        !matches!(title, "Transform" | "Mask" | "Advanced")
    }

    /// Whether the group `title` is open.
    pub fn is_open(&self, title: &str) -> bool {
        self.chosen
            .get(title)
            .copied()
            .unwrap_or_else(|| Self::default_open(title))
    }

    /// Opens or closes the group `title`.
    pub fn set_open(&mut self, title: &str, open: bool) {
        self.chosen.insert(title.to_owned(), open);
    }

    /// The stored form: a JSON object of the groups the user opened or closed.
    pub fn to_storage(&self) -> String {
        serde_json::to_string(&self.chosen).unwrap_or_default()
    }

    /// The groups as [`Self::to_storage`] stored them; anything unreadable is the default.
    pub fn from_storage(s: &str) -> Self {
        Self {
            chosen: serde_json::from_str(s).unwrap_or_default(),
        }
    }
}

/// Emits commands for one layer, or for one region's recipe (the Region tool's
/// pseudo-layer, UX §7).
struct Form<'a> {
    actions: &'a mut Vec<Action>,
    layer: usize,
    /// Recipe changes go to this region instead of the layer.
    region: Option<usize>,
    /// Which groups are open; a click on a heading changes it.
    groups: RecipeGroups,
}

impl Form<'_> {
    fn emit(&mut self, gesture: Option<&str>, command: JsonCommand) {
        self.actions.push(match gesture {
            Some(name) => Action::Gesture(egui::Id::new(name).value(), command),
            None => Action::Run(command),
        });
    }

    fn recipe(&mut self, gesture: Option<&str>, recipe: RecipeChange) {
        if let Some(r) = self.region {
            let region = convert64_project::command::RegionRef::Index(r);
            self.emit(gesture, JsonCommand::RegionSetRecipe { region, recipe });
            return;
        }
        let layer = LayerRef::Index(self.layer);
        self.emit(gesture, JsonCommand::RecipeSet { layer, recipe });
    }

    fn adjust(&mut self, gesture: Option<&str>, adjustments: AdjustmentsChange) {
        let layer = LayerRef::Index(self.layer);
        self.emit(gesture, JsonCommand::AdjustmentsSet { layer, adjustments });
    }

    fn solve(&mut self, gesture: Option<&str>, set: SolveSet) {
        self.emit(gesture, JsonCommand::SolveSet(set));
    }

    fn end(&mut self) {
        self.actions.push(Action::EndGesture);
    }
}

/// A row's label: a dot when `changed`, the text; a double click on it resets.
fn label(ui: &mut Ui, text: &str, changed: bool) -> Response {
    label_tip(ui, text, changed, None)
}

/// [`label`] with `tip` (what the value does, its unit and range) above the reset hint.
fn label_tip(ui: &mut Ui, text: &str, changed: bool, tip: Option<&str>) -> Response {
    let accent = Theme::current(ui.ctx()).accent;
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(6.0, layout::SWATCH_SIZE), egui::Sense::hover());
    if changed {
        ui.painter().circle_filled(rect.center(), 2.5, accent);
    }
    let width = LABEL_WIDTH - 6.0 - ui.spacing().item_spacing.x;
    let size = egui::vec2(width, ui.spacing().interact_size.y);
    ui.allocate_ui_with_layout(
        size,
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.set_min_width(width);
            ui.add(
                egui::Label::new(text)
                    .truncate()
                    .sense(egui::Sense::click()),
            )
        },
    )
    .inner
    .on_hover_text(match tip {
        Some(tip) => format!("{tip}\nDouble-click to reset"),
        None => "Double-click to reset".to_owned(),
    })
}

/// The tooltip line every slider and number field shares (UX §6, m9).
pub(super) const FINE_TIP: &str = "Shift-drag for fine steps";

/// What a slider row did this frame.
enum Edit {
    /// Nothing.
    None,
    /// The value changed while dragging.
    Drag(f32),
    /// The value was set in one go (typed, clicked) or the label reset it.
    Set(f32),
}

/// A labelled slider row for `value` (in display units) with `default`.
fn slider(
    ui: &mut Ui,
    name: &str,
    value: f32,
    default: f32,
    range: std::ops::RangeInclusive<f32>,
    suffix: &str,
    decimals: usize,
) -> (Edit, bool) {
    slider_tip(ui, name, None, value, default, range, suffix, decimals)
}

/// [`slider`] with `tip` on its label and on the slider.
#[allow(clippy::too_many_arguments)]
fn slider_tip(
    ui: &mut Ui,
    name: &str,
    tip: Option<&str>,
    value: f32,
    default: f32,
    range: std::ops::RangeInclusive<f32>,
    suffix: &str,
    decimals: usize,
) -> (Edit, bool) {
    let mut v = value;
    let mut edit = Edit::None;
    let mut stopped = false;
    ui.horizontal(|ui| {
        let lab = label_tip(ui, name, (value - default).abs() > 1e-6, tip);
        let value_box = 3.0 * ui.spacing().interact_size.x / 2.0;
        ui.spacing_mut().slider_width =
            (ui.available_width() - value_box).max(ui.spacing().interact_size.x);
        let mut r = ui.add(
            egui::Slider::new(&mut v, range.clone())
                .suffix(suffix)
                .fixed_decimals(decimals),
        );
        let fine = super::fine_slider(ui, &r, value, range, decimals);
        if let Some(fine) = fine {
            v = fine;
        }
        r = r.on_hover_text(match tip {
            Some(tip) => format!("{tip}\n{FINE_TIP}"),
            None => FINE_TIP.to_owned(),
        });
        // The slider rounds to its decimals: a value it merely snapped (0.6 × 100 is not
        // exactly 60) is not an edit, or it would be re-sent every frame.
        let step = 0.5 * 10f32.powi(-(decimals as i32));
        if lab.double_clicked() {
            edit = Edit::Set(default);
        } else if (r.changed() || fine.is_some()) && (v - value).abs() > step {
            edit = if r.dragged() {
                Edit::Drag(v)
            } else {
                Edit::Set(v)
            };
        }
        stopped = r.drag_stopped();
    });
    (edit, stopped)
}

/// Sends a slider row's edit through `make` as a command of gesture `name`.
fn commit(
    f: &mut Form,
    name: &str,
    (edit, stopped): (Edit, bool),
    mut make: impl FnMut(&mut Form, Option<&str>, f32),
) {
    match edit {
        Edit::None => {}
        Edit::Drag(v) => make(f, Some(name), v),
        Edit::Set(v) => {
            make(f, None, v);
        }
    }
    if stopped {
        f.end();
    }
}

/// A collapsible group with a dot when anything in it changed and a Reset button; open
/// as `f.groups` says, and a click on the heading opens or closes it there (#288).
fn group(
    f: &mut Form,
    ui: &mut Ui,
    title: &str,
    changed: bool,
    add: impl FnOnce(&mut Form, &mut Ui) -> bool,
) -> bool {
    let mut reset = false;
    let mut text = RichText::new(title).family(egui::FontFamily::Name(crate::theme::MEDIUM.into()));
    if changed {
        text = text.color(Theme::current(ui.ctx()).accent);
    }
    let open = f.groups.is_open(title);
    let r = egui::CollapsingHeader::new(text)
        .id_salt(title)
        .open(Some(open))
        .show(ui, |ui| {
            reset = add(f, ui);
        });
    if r.header_response.clicked() {
        f.groups.set_open(title, !open);
        ui.ctx().request_repaint();
    }
    reset
}

/// A Reset button at the end of a group, enabled when something changed.
fn reset_button(ui: &mut Ui, changed: bool) -> bool {
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_enabled(changed, egui::Button::new("Reset group").small())
                .clicked()
        })
        .inner
    })
    .inner
}

pub(super) fn show(shell: &mut Shell, ws: &Workspace, ui: &mut Ui, actions: &mut Vec<Action>) {
    let project = ws.doc.session().project();
    if project.layers.is_empty() {
        ui.label(RichText::new("No layers: add one to set its recipe.").weak());
        return;
    }
    let i = shell.layer.min(project.layers.len() - 1);
    let layer = &project.layers[i];
    let mut f = Form {
        actions,
        layer: i,
        region: None,
        groups: std::mem::take(&mut shell.groups),
    };
    if shell.tool == super::Tool::Region && shell.mask.tool.is_none() {
        regions(&mut f, shell, ws, ui);
    } else {
        layer_header(ws, i, ui);
        // GROUP_ORDER (#288).
        dither(&mut f, shell, ws, layer.recipe(), ui);
        fitting(&mut f, layer.recipe(), ws, ui);
        adjustments(&mut f, shell, ws, layer, ui);
        sharpen(&mut f, layer.adjustments().sharpen, ui);
        transform_group(&mut f, shell, ws, layer, ui);
        mask_group(&mut f, shell, ws, layer, ui);
        advanced(&mut f, layer, &project.solve, ui);
    }
    shell.groups = f.groups;
}

/// The layer header (UX §6, usability review M5): the layer's thumbnail, its name and
/// which layer of how many it is, with the source's size.
fn layer_header(ws: &Workspace, i: usize, ui: &mut Ui) {
    let layers = &ws.doc.session().project().layers;
    let layer = &layers[i];
    ui.horizontal(|ui| {
        let side = 32.0;
        let (rect, _) = ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::hover());
        ui.painter()
            .rect_filled(rect, 2.0, ui.visuals().extreme_bg_color);
        if let Some(tex) = ws.thumbnail(i) {
            let [w, h] = tex.size().map(|v| v as f32);
            let s = side / w.max(h);
            let image = egui::Rect::from_center_size(rect.center(), egui::vec2(w * s, h * s));
            let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            ui.painter()
                .image(tex.id(), image, uv, egui::Color32::WHITE);
        }
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let heading = RichText::new(layer.name())
                .size(14.0)
                .color(ui.visuals().strong_text_color())
                .family(egui::FontFamily::Name(crate::theme::MEDIUM.into()));
            ui.add(egui::Label::new(heading).truncate());
            let (w, h) = layer.source().size();
            ui.label(
                RichText::new(format!(
                    "Layer {} of {} · {w} × {h} px",
                    layers.len() - i,
                    layers.len()
                ))
                .small()
                .weak(),
            );
        });
    });
    ui.separator();
}

/// The Adjustments group (UX §6 item 2): tone and colour, Levels, Curves and Range.
/// Sharpen has its own group after this one, with its own Reset.
fn adjustments(f: &mut Form, shell: &mut Shell, ws: &Workspace, layer: &Layer, ui: &mut Ui) {
    let a = layer.adjustments();
    let d = Adjustments::default();
    let changed = !adjustments_default(a);
    let reset = group(f, ui, "Adjustments", changed, |f, ui| {
        let bc = a.brightness_contrast;
        let hsl = a.hsl;
        let lv = a.levels;
        let pct = |v: f32| v * 100.0;
        commit(
            f,
            "brightness",
            slider(
                ui,
                "Brightness",
                pct(bc.brightness()),
                0.0,
                -100.0..=100.0,
                "",
                0,
            ),
            |f, g, v| {
                let x = BrightnessContrast::new(v / 100.0, bc.contrast()).unwrap_or(bc);
                f.adjust(
                    g,
                    AdjustmentsChange {
                        brightness_contrast: Some(x),
                        ..Default::default()
                    },
                );
            },
        );
        commit(
            f,
            "contrast",
            slider(
                ui,
                "Contrast",
                pct(bc.contrast()),
                0.0,
                -100.0..=100.0,
                "",
                0,
            ),
            |f, g, v| {
                let x = BrightnessContrast::new(bc.brightness(), v / 100.0).unwrap_or(bc);
                f.adjust(
                    g,
                    AdjustmentsChange {
                        brightness_contrast: Some(x),
                        ..Default::default()
                    },
                );
            },
        );
        commit(
            f,
            "saturation",
            slider(
                ui,
                "Saturation",
                pct(hsl.saturation()),
                0.0,
                -100.0..=100.0,
                "",
                0,
            ),
            |f, g, v| {
                let x = Hsl::new(hsl.hue(), v / 100.0, hsl.lightness()).unwrap_or(hsl);
                f.adjust(
                    g,
                    AdjustmentsChange {
                        hsl: Some(x),
                        ..Default::default()
                    },
                );
            },
        );
        commit(
            f,
            "hue",
            slider(ui, "Hue", hsl.hue(), 0.0, -180.0..=180.0, "°", 0),
            |f, g, v| {
                let x = Hsl::new(v, hsl.saturation(), hsl.lightness()).unwrap_or(hsl);
                f.adjust(
                    g,
                    AdjustmentsChange {
                        hsl: Some(x),
                        ..Default::default()
                    },
                );
            },
        );
        commit(
            f,
            "lightness",
            slider(
                ui,
                "Lightness",
                pct(hsl.lightness()),
                0.0,
                -100.0..=100.0,
                "",
                0,
            ),
            |f, g, v| {
                let x = Hsl::new(hsl.hue(), hsl.saturation(), v / 100.0).unwrap_or(hsl);
                f.adjust(
                    g,
                    AdjustmentsChange {
                        hsl: Some(x),
                        ..Default::default()
                    },
                );
            },
        );
        commit(
            f,
            "gamma",
            slider(ui, "Gamma", a.gamma.get(), 1.0, 0.1..=8.0, "", 2),
            |f, g, v| {
                let x = Gamma::new(v).unwrap_or(a.gamma);
                f.adjust(
                    g,
                    AdjustmentsChange {
                        gamma: Some(x),
                        ..Default::default()
                    },
                );
            },
        );
        commit(
            f,
            "black point",
            slider_tip(
                ui,
                "Black point",
                Some(BLACK_POINT_TIP),
                a.black_point.get() * 100.0,
                0.0,
                0.0..=50.0,
                " %",
                0,
            ),
            |f, g, v| {
                let x = BlackPoint::new((v / 100.0).clamp(0.0, 0.5)).unwrap_or(a.black_point);
                f.adjust(
                    g,
                    AdjustmentsChange {
                        black_point: Some(x),
                        ..Default::default()
                    },
                );
            },
        );
        ui.label(RichText::new("Levels").small().weak());
        commit(
            f,
            "levels black",
            slider(ui, "Black", lv.black() * 255.0, 0.0, 0.0..=254.0, "", 0),
            |f, g, v| {
                let black = (v / 255.0).min(lv.white() - 1.0 / 255.0);
                let x = Levels::new(black, lv.white(), lv.gamma()).unwrap_or(lv);
                f.adjust(
                    g,
                    AdjustmentsChange {
                        levels: Some(x),
                        ..Default::default()
                    },
                );
            },
        );
        commit(
            f,
            "levels gamma",
            slider(ui, "Mid-tones", lv.gamma(), 1.0, 0.1..=10.0, "", 2),
            |f, g, v| {
                let x = Levels::new(lv.black(), lv.white(), v).unwrap_or(lv);
                f.adjust(
                    g,
                    AdjustmentsChange {
                        levels: Some(x),
                        ..Default::default()
                    },
                );
            },
        );
        commit(
            f,
            "levels white",
            slider(ui, "White", lv.white() * 255.0, 255.0, 1.0..=255.0, "", 0),
            |f, g, v| {
                let white = (v / 255.0).max(lv.black() + 1.0 / 255.0);
                let x = Levels::new(lv.black(), white, lv.gamma()).unwrap_or(lv);
                f.adjust(
                    g,
                    AdjustmentsChange {
                        levels: Some(x),
                        ..Default::default()
                    },
                );
            },
        );
        curves(f, shell, &a.curves, ui);
        // Range is applied last, after the curves (composite::adjust_planes), so its
        // row comes last, in this group and not under Sharpen (#237).
        ui.horizontal(|ui| {
            let on = a.match_range.is_some();
            let lab = label_tip(ui, "Range", on, Some(RANGE_TIP));
            let mut v = on;
            let r = ui
                .checkbox(&mut v, "Match the palette's range")
                .on_hover_text(RANGE_TIP);
            if r.changed() || (lab.double_clicked() && on) {
                // png2c64's box, the default.
                let m = (v && !lab.double_clicked()).then(|| {
                    RangeMatch::for_palette(
                        ws.doc.palette(),
                        RangeMatch::DEFAULT_PERCENTILE,
                        RangeMatch::DEFAULT_MARGIN,
                    )
                });
                f.adjust(
                    None,
                    AdjustmentsChange {
                        match_range: Some(m.and_then(Result::ok)),
                        ..Default::default()
                    },
                );
            }
        });
        if let Some(m) = a.match_range {
            ui.horizontal(|ui| {
                let lab = label_tip(
                    ui,
                    "Method",
                    m.method() != RangeMethod::Box,
                    Some(RANGE_METHOD_TIP),
                );
                let mut want = m.method();
                for (k, n, tip) in RANGE_METHODS {
                    if ui
                        .selectable_label(want == k, n)
                        .on_hover_text(tip)
                        .clicked()
                    {
                        want = k;
                    }
                }
                if lab.double_clicked() {
                    want = RangeMethod::Box;
                }
                if want != m.method() {
                    let pal = ws.doc.palette();
                    let x = match want {
                        RangeMethod::Box => RangeMatch::for_palette(
                            pal,
                            RangeMatch::DEFAULT_PERCENTILE,
                            RangeMatch::DEFAULT_MARGIN,
                        ),
                        RangeMethod::Hull => RangeMatch::hull(pal, 1.0),
                    };
                    if let Ok(x) = x {
                        f.adjust(
                            None,
                            AdjustmentsChange {
                                match_range: Some(Some(x)),
                                ..Default::default()
                            },
                        );
                    }
                }
            });
        }
        match a.match_range {
            Some(m) if m.method() == RangeMethod::Hull => commit(
                f,
                "range amount",
                slider_tip(
                    ui,
                    "Amount",
                    Some(RANGE_AMOUNT_TIP),
                    m.amount() * 100.0,
                    100.0,
                    0.0..=100.0,
                    " %",
                    0,
                ),
                |f, g, v| {
                    if let Ok(x) = m.with_amount((v / 100.0).clamp(0.0, 1.0)) {
                        f.adjust(
                            g,
                            AdjustmentsChange {
                                match_range: Some(Some(x)),
                                ..Default::default()
                            },
                        );
                    }
                },
            ),
            _ => {}
        }
        reset_button(ui, changed)
    });
    if reset {
        f.adjust(
            None,
            AdjustmentsChange {
                gamma: Some(d.gamma),
                sharpen: None,
                black_point: Some(d.black_point),
                levels: Some(d.levels),
                brightness_contrast: Some(d.brightness_contrast),
                hsl: Some(d.hsl),
                curves: Some(d.curves.clone()),
                match_range: Some(None),
            },
        );
    }
}

/// The Range row's tooltip.
const RANGE_TIP: &str = "png2c64's --match-range: the picture's OKLab range stretched to the \
                         palette's, after the curves";

/// The Range Method row's tooltip.
const RANGE_METHOD_TIP: &str = "How the range is matched: png2c64's box (the default) or \
                                png2amiga's hull (experimental)";

/// The range matching methods, as the Range row offers them.
const RANGE_METHODS: [(RangeMethod, &str, &str); 2] = [
    (
        RangeMethod::Box,
        "Box",
        "png2c64's: each OKLab axis stretched onto the palette's extent (the default)",
    ),
    (
        RangeMethod::Hull,
        "Hull (experimental)",
        "png2amiga's: each colour's chroma scaled toward what the palette can reach, \
         lightness and hue kept",
    ),
];

/// The Black point slider's tooltip.
const BLACK_POINT_TIP: &str = "png2c64's --black-point: lightness below this becomes black and \
                               the rest is stretched; after the gamma and the sharpen. 50 % \
                               darkens thin lines in the line-art recipe (ADR-014)";

/// The Range Amount slider's tooltip.
const RANGE_AMOUNT_TIP: &str = "How much of the range matching: 100 % is png2amiga's, 0 % \
                                keeps the picture's own colourfulness";

/// Whether the Adjustments group's values (everything but Sharpen, which has its own
/// group) are at their defaults.
fn adjustments_default(a: &Adjustments) -> bool {
    let d = Adjustments::default();
    Adjustments {
        sharpen: d.sharpen,
        ..a.clone()
    } == d
}

/// The Sharpen group (UX §6, #228, #237): after Adjustments, with its own changed dot
/// and Reset. Amount −100…300 %, Radius (png2c64's 3 × 3 or a Gaussian σ of 0.3–3 C64
/// px, the σ beside the segment) and Threshold 0–10 % of lightness. Each label resets
/// its value on a double click.
fn sharpen(f: &mut Form, s: Sharpen, ui: &mut Ui) {
    let d = Sharpen::default();
    let changed = s != d;
    let set = |f: &mut Form, g: Option<&str>, amount: f32, radius: Radius, threshold: f32| {
        if let Ok(x) = Sharpen::new(amount, radius, threshold) {
            f.adjust(
                g,
                AdjustmentsChange {
                    sharpen: Some(x),
                    ..Default::default()
                },
            );
        }
    };
    let reset = group(f, ui, "Sharpen", changed, |f, ui| {
        commit(
            f,
            "sharpen amount",
            slider_tip(
                ui,
                "Amount",
                Some(AMOUNT_TIP),
                s.amount() * 100.0,
                d.amount() * 100.0,
                -100.0..=300.0,
                " %",
                0,
            ),
            |f, g, v| set(f, g, v / 100.0, s.radius(), s.threshold()),
        );
        let (radius, stopped) = radius_row(ui, s.radius(), d.radius());
        if let Some((radius, gesture)) = radius {
            set(f, gesture, s.amount(), radius, s.threshold());
        }
        if stopped {
            f.end();
        }
        commit(
            f,
            "sharpen threshold",
            slider_tip(
                ui,
                "Threshold",
                Some(THRESHOLD_TIP),
                s.threshold() * 100.0,
                d.threshold() * 100.0,
                0.0..=10.0,
                " %",
                1,
            ),
            |f, g, v| set(f, g, s.amount(), s.radius(), (v / 100.0).clamp(0.0, 0.1)),
        );
        reset_button(ui, changed)
    });
    if reset {
        f.adjust(
            None,
            AdjustmentsChange {
                sharpen: Some(d),
                ..Default::default()
            },
        );
    }
}

/// The Amount row's tooltip.
const AMOUNT_TIP: &str = "An unsharp mask on lightness at the C64 grid, after gamma. \
                          −100…300 %: 0 is off, negative softens (for noisy photos)";
/// The Radius row's tooltip.
const RADIUS_TIP: &str = "The blur the sharpen compares against: png2c64's 3×3 kernel, \
                          or a Gaussian of σ 0.30–3.00 C64 pixels";
/// The Threshold row's tooltip (#237).
const THRESHOLD_TIP: &str = "Skip changes smaller than this, so noise isn't sharpened. \
                             0–10 % of lightness; 0 sharpens every change";
/// The σ field's tooltip.
const SIGMA_TIP: &str = "σ of the Gaussian in C64 pixels, 0.30–3.00; drag or type";
/// The σ a click on Gaussian starts from, in C64 pixels.
const GAUSSIAN_START: f32 = 1.0;

/// The Radius row: its label, the segment *png2c64 · Gaussian* (always both, the
/// chosen one selected) and, with Gaussian, the σ field beside it (#237). Returns the
/// new radius with the drag gesture's name when it is a drag, and whether a drag
/// stopped.
fn radius_row(
    ui: &mut Ui,
    radius: Radius,
    default: Radius,
) -> (Option<(Radius, Option<&'static str>)>, bool) {
    let gaussian = |sigma: f32| Radius::gaussian(sigma.clamp(0.3, 3.0)).unwrap_or(Radius::Png2c64);
    let mut out = None;
    let mut stopped = false;
    // The segment's words are short ("3×3" and "σ" live in the tooltips) so the σ field
    // fits beside it in the dock at its default width; in a narrower dock it wraps
    // under the segment rather than past the edge.
    ui.horizontal_wrapped(|ui| {
        let lab = label_tip(ui, "Radius", radius != default, Some(RADIUS_TIP));
        if lab.double_clicked() && radius != default {
            out = Some((default, None));
        }
        let png2c64 = radius == Radius::Png2c64;
        ui.spacing_mut().item_spacing.x = 2.0;
        if ui
            .selectable_label(png2c64, "png2c64")
            .on_hover_text("png2c64's own 3×3 kernel, bit for bit")
            .clicked()
            && !png2c64
        {
            out = Some((Radius::Png2c64, None));
        }
        if ui
            .selectable_label(!png2c64, "Gaussian")
            .on_hover_text("A Gaussian blur of σ C64 pixels, 0.30–3.00")
            .clicked()
            && png2c64
        {
            out = Some((gaussian(GAUSSIAN_START), None));
        }
        if let Radius::Gaussian(sigma) = radius {
            ui.add_space(2.0);
            let mut x = sigma;
            let r = ui
                .add(
                    egui::DragValue::new(&mut x)
                        .speed(0.01)
                        .range(0.3..=3.0)
                        .fixed_decimals(2)
                        .suffix(" px"),
                )
                .on_hover_text(SIGMA_TIP);
            // As the sliders: a value merely rounded for display is not an edit.
            if r.changed() && (x - sigma).abs() > 0.005 {
                out = Some((gaussian(x), r.dragged().then_some("sharpen radius")));
            }
            stopped = r.drag_stopped();
        }
    });
    (out, stopped)
}

/// The curves editor: a channel picker and a square with the curve over a diagonal;
/// drag a point, click empty space to add one, right-click a point to remove it.
fn curves(f: &mut Form, shell: &mut Shell, c: &Curves, ui: &mut Ui) {
    let channels: [(&str, &Curve); 4] = [
        ("Master", &c.master),
        ("Red", &c.red),
        ("Green", &c.green),
        ("Blue", &c.blue),
    ];
    ui.horizontal(|ui| {
        let changed = *c != Curves::default();
        let lab = label(ui, "Curves", changed);
        for (k, (name, _)) in channels.iter().enumerate() {
            if ui
                .selectable_label(shell.curve_channel == k, *name)
                .clicked()
            {
                shell.curve_channel = k;
            }
        }
        if lab.double_clicked() && changed {
            f.adjust(
                None,
                AdjustmentsChange {
                    curves: Some(Curves::default()),
                    ..Default::default()
                },
            );
        }
    });
    let k = shell.curve_channel.min(3);
    let curve = channels[k].1;
    let side = ui.available_width().min(120.0);
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), side),
        egui::Sense::click_and_drag(),
    );
    let area = egui::Rect::from_center_size(rect.center(), egui::vec2(side, side));
    let v = ui.visuals();
    let p = ui.painter_at(rect);
    p.rect_filled(area, 0.0, v.extreme_bg_color);
    p.rect_stroke(
        area,
        0.0,
        v.widgets.noninteractive.bg_stroke,
        egui::StrokeKind::Inside,
    );
    let to_screen =
        |q: [f32; 2]| egui::pos2(area.left() + q[0] * side, area.bottom() - q[1] * side);
    let to_curve = |s: egui::Pos2| {
        [
            ((s.x - area.left()) / side).clamp(0.0, 1.0),
            ((area.bottom() - s.y) / side).clamp(0.0, 1.0),
        ]
    };
    p.line_segment(
        [area.left_bottom(), area.right_top()],
        (1.0, v.weak_text_color().gamma_multiply(0.4)),
    );
    let line: Vec<egui::Pos2> = (0..=64)
        .map(|i| {
            let x = i as f32 / 64.0;
            to_screen([x, curve.eval(x).clamp(0.0, 1.0)])
        })
        .collect();
    let colour = [
        v.text_color(),
        egui::Color32::from_rgb(220, 60, 60),
        egui::Color32::from_rgb(60, 170, 80),
        egui::Color32::from_rgb(70, 110, 230),
    ][k];
    p.add(egui::Shape::line(line, (1.5, colour)));
    let points = curve.points().to_vec();
    for q in &points {
        p.circle_filled(to_screen(*q), 3.5, colour);
    }
    let near = |pos: egui::Pos2| {
        points
            .iter()
            .position(|q| to_screen(*q).distance(pos) < 8.0)
    };
    let set = |f: &mut Form, g: Option<&str>, pts: Vec<[f32; 2]>| {
        let Ok(new) = Curve::new(pts) else { return };
        let mut all = c.clone();
        match k {
            0 => all.master = new,
            1 => all.red = new,
            2 => all.green = new,
            _ => all.blue = new,
        }
        f.adjust(
            g,
            AdjustmentsChange {
                curves: Some(all),
                ..Default::default()
            },
        );
    };
    let drag_key = ui.id().with("curve_drag");
    if response.drag_started() {
        // The point under the press, not under the pointer once the drag threshold is
        // passed (#263).
        let hit = super::drag_origin(ui, &response).and_then(near);
        ui.data_mut(|d| d.insert_temp(drag_key, hit));
    }
    if response.dragged()
        && let Some(Some(idx)) = ui.data(|d| d.get_temp::<Option<usize>>(drag_key))
        && let Some(pos) = response.interact_pointer_pos()
    {
        let mut pts = points.clone();
        let mut q = to_curve(pos);
        // Keep x between the neighbours; the end points stay at x = 0 and 1.
        let lo = if idx == 0 {
            0.0
        } else {
            pts[idx - 1][0] + 0.01
        };
        let hi = if idx + 1 == pts.len() {
            1.0
        } else {
            pts[idx + 1][0] - 0.01
        };
        q[0] = if idx == 0 || idx + 1 == pts.len() {
            pts[idx][0]
        } else {
            q[0].clamp(lo, hi)
        };
        pts[idx] = q;
        set(f, Some("curve"), pts);
    }
    if response.drag_stopped() {
        f.end();
    }
    if response.clicked()
        && let Some(pos) = response.interact_pointer_pos()
        && near(pos).is_none()
        && area.contains(pos)
    {
        let mut pts = points.clone();
        let q = to_curve(pos);
        let at = pts.iter().position(|p| p[0] > q[0]).unwrap_or(pts.len());
        pts.insert(at, q);
        set(f, None, pts);
    }
    if response.secondary_clicked()
        && let Some(idx) = response.interact_pointer_pos().and_then(near)
        && idx != 0
        && idx + 1 != points.len()
    {
        let mut pts = points.clone();
        pts.remove(idx);
        set(f, None, pts);
    }
}

/// The Region tool's breadcrumb (UX §7, usability review M7): `Region 1 ‹ source`, the
/// region in the heading style, then the selected layer's name as a link back to its
/// recipe (and to the Nudge tool, as `V`).
fn breadcrumb(shell: &mut Shell, ws: &Workspace, ui: &mut Ui) {
    let project = ws.doc.session().project();
    let here = project
        .regions
        .get(shell.region)
        .map_or("Regions", |r| r.name());
    let Some(layer) = project
        .layers
        .get(shell.layer.min(project.layers.len().saturating_sub(1)))
    else {
        return;
    };
    ui.horizontal(|ui| {
        let heading = RichText::new(here)
            .size(14.0)
            .color(ui.visuals().strong_text_color())
            .family(egui::FontFamily::Name(crate::theme::MEDIUM.into()));
        ui.label(heading);
        ui.label(RichText::new("‹").weak());
        let key = super::Tool::Nudge.key().name();
        let back = ui
            .link(layer.name())
            .on_hover_text(format!("Back to {}'s recipe ({key})", layer.name()));
        if back.clicked() {
            shell.tool = super::Tool::Nudge;
            shell.mask.stroke.clear();
        }
    });
    ui.separator();
}

/// The Region tool's panel: the regions (select, add, rename, remove) and the selected
/// region's dither recipe and colour fitting, edited like a layer's (`region_set_recipe`).
fn regions(f: &mut Form, shell: &mut Shell, ws: &Workspace, ui: &mut Ui) {
    use convert64_project::command::RegionRef;
    let regions = &ws.doc.session().project().regions;
    breadcrumb(shell, ws, ui);
    ui.label(RichText::new("Regions: a recipe painted over the picture. Paint with R; the first stroke makes a region.").small().weak());
    for (k, r) in regions.iter().enumerate() {
        ui.horizontal(|ui| {
            if ui.selectable_label(shell.region == k, r.name()).clicked() {
                shell.region = k;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let delete = egui::Button::new(super::icon(crate::icons::DELETE)).frame(false);
                if ui.add(delete).on_hover_text("Remove the region").clicked() {
                    f.emit(
                        None,
                        JsonCommand::RegionRemove {
                            region: RegionRef::Index(k),
                        },
                    );
                }
            });
        });
    }
    if ui.button("Add region").clicked() {
        let name = format!("Region {}", regions.len() + 1);
        f.emit(
            None,
            JsonCommand::RegionAdd {
                name,
                recipe: Box::default(),
                index: None,
            },
        );
        shell.region = regions.len();
    }
    let Some(region) = regions.get(shell.region) else {
        return;
    };
    ui.separator();
    f.region = Some(shell.region);
    dither(f, shell, ws, region.recipe(), ui);
    fitting(f, region.recipe(), ws, ui);
}

/// `t` with translate, scale and flips replaced; rotation and anchor kept.
fn rebuilt(
    t: &convert64_core::Transform,
    translate: convert64_core::Vec2,
    scale: convert64_core::Vec2,
    flips: (bool, bool),
) -> Option<convert64_core::Transform> {
    convert64_core::Transform::new(translate, scale, flips.0, flips.1)
        .and_then(|n| n.with_rotation(t.rotate()))
        .and_then(|n| n.with_anchor(t.anchor()))
        .ok()
}

/// The Transform group (UX §6 item 1, ADR-011, usability review M6, #289): X, Y and
/// W × H in screen pixels (the 320 × 200 grid every mode is composited on; a multicolor
/// pixel is 2 wide), the scale as a % of the source under them, rotation, the 3 × 3
/// anchor, the presets Fit · Fill · 1:1 · 2:1, the flip toggles, Snap, Downscale (#235:
/// Auto or Exact) and Resample (ADR-014 §3: Mitchell or Lanczos); all sub-pixel.
fn transform_group(f: &mut Form, shell: &mut Shell, ws: &Workspace, layer: &Layer, ui: &mut Ui) {
    use crate::presets::{self, Preset};
    use convert64_core::Vec2;
    let t = layer.transform();
    let size = layer.source().size();
    let (w, h) = (size.0 as f32, size.1 as f32);
    let mode = ws.doc.mode();
    let changed = t.rotate() != 0.0
        || t.flip_h()
        || t.flip_v()
        || layer.downscale() != convert64_core::Downscale::Auto
        || layer.resample() != convert64_core::Resample::Mitchell;
    let set = |f: &mut Form, g: Option<&str>, transform: convert64_core::Transform| {
        let layer = LayerRef::Index(f.layer);
        f.emit(
            g,
            JsonCommand::LayerSetTransform {
                layer,
                transform,
                snap: None,
            },
        );
    };
    /// A labelled drag value; returns the new value and whether it was a drag.
    fn field(
        ui: &mut Ui,
        name: &str,
        v: f32,
        speed: f64,
        range: std::ops::RangeInclusive<f32>,
        suffix: &str,
    ) -> (Option<(f32, bool)>, bool) {
        let mut x = v;
        let mut out = None;
        let mut stopped = false;
        ui.horizontal(|ui| {
            label(ui, name, false);
            let r = ui.add(
                egui::DragValue::new(&mut x)
                    .speed(speed)
                    .range(range)
                    .fixed_decimals(2)
                    .suffix(suffix),
            );
            if r.changed() {
                out = Some((x, r.dragged()));
            }
            stopped = r.drag_stopped();
        });
        (out, stopped)
    }
    let _ = group(f, ui, "Transform", changed, |f, ui| {
        let p = t.translate();
        let sc = t.scale();
        let flips = (t.flip_h(), t.flip_v());
        for (name, axis) in [("X", 0), ("Y", 1)] {
            let v = if axis == 0 { p.x } else { p.y };
            let (edit, stopped) = field(ui, name, v, 0.05, -640.0..=640.0, " px");
            if let Some((x, drag)) = edit {
                let np = if axis == 0 {
                    Vec2::new(x, p.y)
                } else {
                    Vec2::new(p.x, x)
                };
                if let Some(n) = rebuilt(&t, np, sc, flips) {
                    set(f, drag.then_some("transform position"), n);
                }
            }
            if stopped {
                f.end();
            }
        }
        // W × H in screen pixels (#289), the chain between them.
        ui.horizontal(|ui| {
            label_tip(ui, "W × H", false, Some(SIZE_TIP));
            let (pw, ph) = presets::screen_size(&t, size);
            let (mut wv, mut hv) = (pw, ph);
            let rw = ui
                .add(
                    egui::DragValue::new(&mut wv)
                        .speed(0.1)
                        .range(w * 0.01..=w * 10.0)
                        .fixed_decimals(2)
                        .suffix(" px"),
                )
                .on_hover_text(SIZE_TIP);
            let glyph = if shell.scale_linked {
                crate::icons::CHAIN
            } else {
                crate::icons::CHAIN_OFF
            };
            let tip = if shell.scale_linked {
                "Uniform: W and H together"
            } else {
                "Free aspect"
            };
            if ui
                .add(egui::Button::new(super::icon(glyph)).frame(false))
                .on_hover_text(tip)
                .clicked()
            {
                shell.scale_linked = !shell.scale_linked;
            }
            let rh = ui
                .add(
                    egui::DragValue::new(&mut hv)
                        .speed(0.1)
                        .range(h * 0.01..=h * 10.0)
                        .fixed_decimals(2)
                        .suffix(" px"),
                )
                .on_hover_text(SIZE_TIP);
            let linked = shell.scale_linked;
            // The chain keeps H / W as it is (UX §6 item 1).
            let target = if rw.changed() {
                let sx = presets::scale_for(wv, size.0);
                Some((
                    sx,
                    if linked { sc.y * sx / sc.x } else { sc.y },
                    rw.dragged(),
                ))
            } else if rh.changed() {
                let sy = presets::scale_for(hv, size.1);
                Some((
                    if linked { sc.x * sy / sc.y } else { sc.x },
                    sy,
                    rh.dragged(),
                ))
            } else {
                None
            };
            if let Some((sx, sy, drag)) = target
                && let Ok(n) = t.scaled_about_anchor(Vec2::new(sx, sy), size)
            {
                set(f, drag.then_some("transform scale"), n);
            }
            if rw.drag_stopped() || rh.drag_stopped() {
                f.end();
            }
        });
        // The secondary readout: % of the source, and multicolor pixels.
        ui.horizontal(|ui| {
            // Under the fields: the label column and its spacing.
            ui.add_space(LABEL_WIDTH + ui.spacing().item_spacing.x);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for line in presets::secondary(&t, size, mode) {
                    ui.label(RichText::new(line).small().weak());
                }
            });
        });
        let (edit, stopped) = field(ui, "Rotate", t.rotate(), 0.05, -180.0..=180.0, "°");
        if let Some((deg, drag)) = edit
            && let Ok(n) = t.with_rotation(deg)
        {
            set(f, drag.then_some("transform rotate"), n);
        }
        if stopped {
            f.end();
        }
        ui.horizontal(|ui| {
            label(ui, "Anchor", t.anchor() != Vec2::new(0.5, 0.5));
            if let Some(a) = anchor_control(ui, t.anchor())
                && let Ok(n) = t.reanchored(a, size)
            {
                set(f, None, n);
            }
        });
        ui.horizontal(|ui| {
            let now = Preset::matching(&t, size, mode);
            label_tip(ui, "Presets", false, Some(PRESETS_TIP));
            for preset in Preset::ALL {
                if ui
                    .selectable_label(now == Some(preset), preset.label())
                    .on_hover_text(preset.tip())
                    .clicked()
                    && let Some(n) = preset.apply(&t, size, mode)
                {
                    set(f, None, n);
                }
            }
        });
        ui.horizontal(|ui| {
            label(ui, "Flip", flips.0 || flips.1);
            for (vertical, on, fl) in [
                (false, flips.0, (!flips.0, flips.1)),
                (true, flips.1, (flips.0, !flips.1)),
            ] {
                if flip_toggle(ui, on, vertical).clicked()
                    && let Some(n) = rebuilt(&t, p, sc, fl)
                {
                    set(f, None, n);
                }
            }
        });
        ui.horizontal(|ui| {
            label(ui, "Snap", false);
            super::toolbar::snap_menu(Some(ws), ui, f.actions);
        });
        ui.horizontal(|ui| {
            use convert64_core::Downscale;
            let now = layer.downscale();
            label(ui, "Downscale", now != Downscale::Auto);
            for (d, name, tip) in [
                (
                    Downscale::Auto,
                    "Auto",
                    "Past a 4× shrink, sample a cached, box-reduced copy of the source, \
                     so large pictures do not alias (#235). Up to 4× it is png2c64's.",
                ),
                (
                    Downscale::Exact,
                    "Exact",
                    "Always sample the original, as png2c64 does: with Mitchell's four \
                     taps large shrinks alias.",
                ),
            ] {
                if ui
                    .selectable_label(now == d, name)
                    .on_hover_text(tip)
                    .clicked()
                    && now != d
                {
                    f.emit(
                        None,
                        JsonCommand::LayerSetDownscale {
                            layer: LayerRef::Index(f.layer),
                            downscale: d,
                        },
                    );
                }
            }
        });
        ui.horizontal(|ui| {
            use convert64_core::Resample;
            let now = layer.resample();
            label(ui, "Resample", now != Resample::Mitchell);
            for (r, name, tip) in [
                (
                    Resample::Mitchell,
                    "Mitchell",
                    "png2c64's four-tap bicubic, the default (ADR-012).",
                ),
                (
                    Resample::Lanczos,
                    "Lanczos",
                    "png2amiga's Lanczos-3, widened by the shrink: thin lines come out \
                     more even, and greyer; for line art (ADR-014).",
                ),
            ] {
                if ui
                    .selectable_label(now == r, name)
                    .on_hover_text(tip)
                    .clicked()
                    && now != r
                {
                    f.emit(
                        None,
                        JsonCommand::LayerSetResample {
                            layer: LayerRef::Index(f.layer),
                            resample: r,
                        },
                    );
                }
            }
        });
        false
    });
}

/// The W × H row's tooltip: what the unit is (#289).
const SIZE_TIP: &str = "The layer's size on the screen, in screen pixels: the 320 × 200 grid \
                        every mode is placed on. A multicolor pixel is 2 px wide";

/// The Presets row's tooltip.
const PRESETS_TIP: &str = "Scale and place the layer; rotation, anchor and flips stay. The one \
                           the layer is at is selected";

/// The anchor control's cells, row by row: the anchor and its name.
const ANCHORS: [((f32, f32), &str); 9] = [
    ((0.0, 0.0), "top left"),
    ((0.5, 0.0), "top"),
    ((1.0, 0.0), "top right"),
    ((0.0, 0.5), "left"),
    ((0.5, 0.5), "centre"),
    ((1.0, 0.5), "right"),
    ((0.0, 1.0), "bottom left"),
    ((0.5, 1.0), "bottom"),
    ((1.0, 1.0), "bottom right"),
];

/// The side of one of the anchor control's cells, in points (UX §6 item 1: about 16).
const ANCHOR_CELL: f32 = 16.0;

/// The anchor control (UX §6 item 1, #289): a 3 × 3 grid of 16 px cells in one frame,
/// the anchor's cell filled in the accent colour, the others a dot, a hovered one
/// lighter. Returns the anchor clicked, if it is a new one.
fn anchor_control(ui: &mut Ui, anchor: convert64_core::Vec2) -> Option<convert64_core::Vec2> {
    let (gap, pad) = (2.0, 2.0);
    let side = 3.0 * ANCHOR_CELL + 2.0 * gap + 2.0 * pad;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::hover());
    let accent = Theme::current(ui.ctx()).accent;
    let v = ui.visuals().clone();
    ui.painter().rect(
        rect,
        v.widgets.inactive.corner_radius,
        v.extreme_bg_color,
        v.widgets.inactive.bg_stroke,
        egui::StrokeKind::Inside,
    );
    let mut picked = None;
    for (k, ((ax, ay), name)) in ANCHORS.into_iter().enumerate() {
        let (col, row) = ((k % 3) as f32, (k / 3) as f32);
        let cell = egui::Rect::from_min_size(
            rect.min
                + egui::vec2(
                    pad + col * (ANCHOR_CELL + gap),
                    pad + row * (ANCHOR_CELL + gap),
                ),
            egui::vec2(ANCHOR_CELL, ANCHOR_CELL),
        );
        let id = ui.id().with(("anchor", k));
        let resp = ui.interact(cell, id, egui::Sense::click());
        let on = anchor == convert64_core::Vec2::new(ax, ay);
        let label = format!("Pivot: {name}");
        resp.widget_info(|| {
            egui::WidgetInfo::selected(egui::WidgetType::RadioButton, true, on, &label)
        });
        let painter = ui.painter();
        if resp.hovered() {
            painter.rect_filled(cell, 2.0, v.widgets.hovered.weak_bg_fill);
        }
        if on {
            painter.rect_filled(cell.shrink(3.0), 2.0, accent);
        } else {
            let dot = if resp.hovered() {
                v.widgets.hovered.fg_stroke.color
            } else {
                v.widgets.inactive.fg_stroke.color
            };
            painter.circle_filled(cell.center(), 2.0, dot);
        }
        let clicked = resp
            .on_hover_text(format!("{label}: scale and rotate about it"))
            .clicked();
        if clicked && !on {
            picked = Some(convert64_core::Vec2::new(ax, ay));
        }
    }
    picked
}

/// A flip toggle (UX §6 item 1, #289): the flip glyph, turned a quarter for `vertical`,
/// on a selectable button's background when `on`.
fn flip_toggle(ui: &mut Ui, on: bool, vertical: bool) -> Response {
    let name = if vertical {
        "Flip vertical"
    } else {
        "Flip horizontal"
    };
    let side = ui.spacing().interact_size.y + 4.0;
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::click());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, on, name));
    let visuals = ui.style().interact_selectable(&resp, on);
    if on || resp.hovered() || resp.has_focus() {
        ui.painter().rect(
            rect,
            visuals.corner_radius,
            visuals.weak_bg_fill,
            visuals.bg_stroke,
            egui::StrokeKind::Inside,
        );
    }
    let galley = egui::WidgetText::from(super::icon(crate::icons::FLIP)).into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        f32::INFINITY,
        egui::TextStyle::Button,
    );
    let angle = if vertical {
        std::f32::consts::FRAC_PI_2
    } else {
        0.0
    };
    // Turned about its own centre: the text's origin is its top left corner.
    let half = galley.size() / 2.0;
    let pos = rect.center() - egui::emath::Rot2::from_angle(angle) * half;
    ui.painter()
        .add(egui::epaint::TextShape::new(pos, galley, visuals.text_color()).with_angle(angle));
    resp.on_hover_text(name)
}

/// The Mask group (UX §6 item 3): modifiers, the sources list and Edit mask; a layer
/// without a mask shows one button, Add mask.
fn mask_group(f: &mut Form, shell: &mut Shell, ws: &Workspace, layer: &Layer, ui: &mut Ui) {
    use convert64_core::command::ModifierChange;
    use convert64_core::mask::{MaskSource, Modifiers, SnapToCells};
    let Some(mask) = layer.mask() else {
        let _ = group(f, ui, "Mask", false, |f, ui| {
            ui.label(
                RichText::new("No mask: the whole layer shows.")
                    .small()
                    .weak(),
            );
            if ui.button("Add mask").clicked() {
                let layers = &ws.doc.session().project().layers;
                f.actions.push(super::layers::add_mask(layers, f.layer));
            }
            false
        });
        return;
    };
    let m = mask.modifiers();
    let d = Modifiers::default();
    let changed = m != d || !mask.sources().is_empty();
    let modifier = |f: &mut Form, g: Option<&str>, modifier: ModifierChange| {
        let layer = LayerRef::Index(f.layer);
        f.emit(g, JsonCommand::MaskSetModifier { layer, modifier });
    };
    let reset = group(f, ui, "Mask", changed, |f, ui| {
        let edit = if shell.mask.active {
            "Done editing"
        } else {
            "Edit mask (M)"
        };
        if ui.button(edit).clicked() {
            shell.toggle_mask_mode();
        }
        commit(
            f,
            "mask feather",
            slider(
                ui,
                "Feather",
                m.feather(),
                d.feather(),
                0.0..=32.0,
                " px",
                1,
            ),
            |f, g, v| {
                modifier(f, g, ModifierChange::Feather(v));
            },
        );
        ui.horizontal(|ui| {
            let lab = label(ui, "Invert", m.invert() != d.invert());
            let mut v = m.invert();
            if ui.checkbox(&mut v, "").changed() || (lab.double_clicked() && m.invert()) {
                modifier(f, None, ModifierChange::Invert(v && !lab.double_clicked()));
            }
        });
        ui.horizontal(|ui| {
            let lab = label(ui, "Snap to cells", m.snap() != d.snap());
            egui::ComboBox::from_id_salt("mask_group_snap")
                .selected_text(format!("{:?}", m.snap()))
                .show_ui(ui, |ui| {
                    for s in [
                        SnapToCells::Off,
                        SnapToCells::Expand,
                        SnapToCells::Contract,
                        SnapToCells::Nearest,
                    ] {
                        if ui
                            .selectable_label(m.snap() == s, format!("{s:?}"))
                            .clicked()
                            && m.snap() != s
                        {
                            modifier(f, None, ModifierChange::Snap(s));
                        }
                    }
                });
            if lab.double_clicked() && m.snap() != d.snap() {
                modifier(f, None, ModifierChange::Snap(d.snap()));
            }
        });
        let lv = m.levels();
        commit(
            f,
            "mask black",
            slider(
                ui,
                "Levels black",
                lv.black() * 255.0,
                0.0,
                0.0..=254.0,
                "",
                0,
            ),
            |f, g, v| {
                let black = (v / 255.0).min(lv.white() - 1.0 / 255.0);
                if let Ok(x) = Levels::new(black, lv.white(), lv.gamma()) {
                    modifier(f, g, ModifierChange::Levels(x));
                }
            },
        );
        commit(
            f,
            "mask white",
            slider(
                ui,
                "Levels white",
                lv.white() * 255.0,
                255.0,
                1.0..=255.0,
                "",
                0,
            ),
            |f, g, v| {
                let white = (v / 255.0).max(lv.black() + 1.0 / 255.0);
                if let Ok(x) = Levels::new(lv.black(), white, lv.gamma()) {
                    modifier(f, g, ModifierChange::Levels(x));
                }
            },
        );
        if !mask.sources().is_empty() {
            use convert64_core::mask::Combine;
            ui.label(RichText::new("Sources").small().weak());
            for (k, e) in mask.sources().iter().enumerate() {
                let what = match e.source {
                    MaskSource::Gradient(_) => "Gradient",
                    MaskSource::Key(_) => "Key",
                };
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{}. {what}", k + 1)).small());
                    egui::ComboBox::from_id_salt(("mask_source_combine", k))
                        .selected_text(RichText::new(format!("{:?}", e.combine)).small())
                        .width(0.0)
                        .show_ui(ui, |ui| {
                            for c in [Combine::Max, Combine::Min, Combine::Multiply] {
                                if ui
                                    .selectable_label(e.combine == c, format!("{c:?}"))
                                    .clicked()
                                    && e.combine != c
                                {
                                    let layer = LayerRef::Index(f.layer);
                                    f.emit(
                                        None,
                                        JsonCommand::MaskSourceSetCombine {
                                            layer,
                                            index: k,
                                            combine: c,
                                        },
                                    );
                                }
                            }
                        });
                    let delete = egui::Button::new(super::icon(crate::icons::DELETE)).frame(false);
                    if ui.add(delete).on_hover_text("Remove this source").clicked() {
                        let layer = LayerRef::Index(f.layer);
                        f.emit(None, JsonCommand::MaskSourceRemove { layer, index: k });
                    }
                });
            }
        }
        reset_button(ui, m != d)
    });
    if reset {
        let layer = LayerRef::Index(f.layer);
        let set = |modifier| JsonCommand::MaskSetModifier {
            layer: layer.clone(),
            modifier,
        };
        let commands = vec![
            set(ModifierChange::Invert(d.invert())),
            set(ModifierChange::Snap(d.snap())),
            set(ModifierChange::Feather(d.feather())),
            set(ModifierChange::Levels(d.levels())),
        ];
        f.actions.push(Action::RunAll(
            commands,
            "Reset the mask's modifiers".to_owned(),
        ));
    }
}

/// The Dither recipe group (UX §6 item 4): the Method field and its picker (§6a),
/// Strength, Phase, Orientation and, for diffusion, the error clamp.
fn dither(f: &mut Form, shell: &mut Shell, ws: &Workspace, r: &Recipe, ui: &mut Ui) {
    let d = Recipe::default();
    let changed = r.method() != d.method()
        || r.strength() != d.strength()
        || r.phase() != d.phase()
        || r.error_clamp() != d.error_clamp();
    let target = match f.region {
        Some(i) => RecipeTarget::Region(i),
        None => RecipeTarget::Layer(f.layer),
    };
    let reset = group(f, ui, "Dither recipe", changed, |f, ui| {
        let family = r.method().kind.family();
        let mut jobs = Vec::new();
        let mut out = super::method_picker::FieldOutcome::default();
        ui.horizontal(|ui| {
            label(ui, "Method", r.method() != d.method());
            out = super::method_picker::field(shell, ws, r, ui, &mut jobs);
        });
        if out.toggle {
            if shell.picker.open.is_some() {
                shell.picker.close(f.actions);
            } else {
                shell.picker.open(target);
            }
        }
        let picked = match out.step {
            Some(m) => Some(m),
            None => {
                super::method_picker::popover(shell, ws, target, r, ui.ctx(), f.actions, &mut jobs)
            }
        };
        ws.method_thumbs.want(jobs);
        if let Some(m) = picked.filter(|m| *m != r.method()) {
            f.recipe(
                None,
                RecipeChange {
                    method: Some(m),
                    ..Default::default()
                },
            );
        }
        commit(
            f,
            "strength",
            slider(
                ui,
                "Strength",
                r.strength() * 100.0,
                d.strength() * 100.0,
                0.0..=100.0,
                "%",
                0,
            ),
            |f, g, v| {
                f.recipe(
                    g,
                    RecipeChange {
                        strength: Some(v / 100.0),
                        ..Default::default()
                    },
                );
            },
        );
        ui.horizontal(|ui| {
            let lab = label(ui, "Phase", r.phase() != d.phase());
            // Short segments, so the row fits the narrowest dock (usability review m6).
            for (phase, name, tip) in [
                (
                    PatternPhase::Screen,
                    "Screen",
                    "Screen-locked: the pattern stays put on the screen as the layer moves",
                ),
                (
                    PatternPhase::Layer,
                    "Layer",
                    "Layer-locked: the pattern moves with the layer",
                ),
            ] {
                let segment = ui
                    .selectable_label(r.phase() == phase, name)
                    .on_hover_text(tip);
                if segment.clicked() && r.phase() != phase {
                    f.recipe(
                        None,
                        RecipeChange {
                            phase: Some(phase),
                            ..Default::default()
                        },
                    );
                }
            }
            if lab.double_clicked() && r.phase() != d.phase() {
                f.recipe(
                    None,
                    RecipeChange {
                        phase: Some(d.phase()),
                        ..Default::default()
                    },
                );
            }
        });
        ui.horizontal(|ui| {
            let o = r.method().orientation;
            label(ui, "Orientation", o != Orientation::Auto);
            for (or, name) in [
                (Orientation::Auto, "Auto"),
                (Orientation::VerticalBiased, "Vertical"),
                (Orientation::TwoToOne, "2:1"),
            ] {
                if ui.selectable_label(o == or, name).clicked() && o != or {
                    f.recipe(
                        None,
                        RecipeChange {
                            method: Some(Method {
                                orientation: or,
                                ..r.method()
                            }),
                            ..Default::default()
                        },
                    );
                }
            }
        });
        if family == DitherFamily::Diffusion {
            commit(
                f,
                "error clamp",
                slider(
                    ui,
                    "Error clamp",
                    r.error_clamp(),
                    d.error_clamp(),
                    0.0..=1.0,
                    "",
                    2,
                ),
                |f, g, v| {
                    f.recipe(
                        g,
                        RecipeChange {
                            error_clamp: Some(v),
                            ..Default::default()
                        },
                    );
                },
            );
        }
        reset_button(ui, changed)
    });
    if reset {
        f.recipe(
            None,
            RecipeChange {
                method: Some(d.method()),
                strength: Some(d.strength()),
                phase: Some(d.phase()),
                error_clamp: Some(d.error_clamp()),
                ..Default::default()
            },
        );
    }
}

fn fitting(f: &mut Form, r: &Recipe, ws: &Workspace, ui: &mut Ui) {
    let d = Recipe::default();
    let changed =
        r.metric() != d.metric() || r.fit_mode() != d.fit_mode() || r.allowed() != d.allowed();
    let reset = group(f, ui, "Colour fitting", changed, |f, ui| {
        ui.horizontal(|ui| {
            let lab = label(ui, "Metric", r.metric() != d.metric());
            let names = [
                (Metric::Oklab, "OKLab"),
                (Metric::Cielab, "CIELAB"),
                (Metric::PeptoYuv, "Pepto YUV"),
                (Metric::Rgb, "RGB"),
            ];
            let current = names
                .iter()
                .find(|(m, _)| *m == r.metric())
                .map_or("OKLab", |(_, n)| n);
            egui::ComboBox::from_id_salt("metric")
                .selected_text(current)
                .show_ui(ui, |ui| {
                    for (m, n) in names {
                        if ui.selectable_label(r.metric() == m, n).clicked() && r.metric() != m {
                            f.recipe(
                                None,
                                RecipeChange {
                                    metric: Some(m),
                                    ..Default::default()
                                },
                            );
                        }
                    }
                });
            if lab.double_clicked() && r.metric() != d.metric() {
                f.recipe(
                    None,
                    RecipeChange {
                        metric: Some(d.metric()),
                        ..Default::default()
                    },
                );
            }
        });
        ui.horizontal(|ui| {
            label(ui, "Fitting", r.fit_mode() != d.fit_mode());
            for (m, n) in [(FitMode::Adaptive, "Adaptive"), (FitMode::Force, "Force")] {
                if ui.selectable_label(r.fit_mode() == m, n).clicked() && r.fit_mode() != m {
                    f.recipe(
                        None,
                        RecipeChange {
                            fit_mode: Some(m),
                            ..Default::default()
                        },
                    );
                }
            }
        });
        let help = match r.fit_mode() {
            FitMode::Adaptive => {
                "Adaptive lets the solver reassign cell colours for the lowest error."
            }
            FitMode::Force => "Force keeps this layer's colours even if neighbours pay.",
        };
        ui.label(RichText::new(help).small().weak());
        ui.horizontal(|ui| {
            let lab = label(ui, "Allowed", r.allowed() != d.allowed());
            if lab.double_clicked() && r.allowed() != d.allowed() {
                f.recipe(
                    None,
                    RecipeChange {
                        allowed: Some(ColourIndex::ALL.to_vec()),
                        ..Default::default()
                    },
                );
            }
            swatches(f, r, ws, ui);
        });
        reset_button(ui, changed)
    });
    if reset {
        f.recipe(
            None,
            RecipeChange {
                metric: Some(d.metric()),
                fit_mode: Some(d.fit_mode()),
                allowed: Some(ColourIndex::ALL.to_vec()),
                ..Default::default()
            },
        );
    }
}

/// The 16 palette colours as toggles; a colour the layer may not use is hatched.
fn swatches(f: &mut Form, r: &Recipe, ws: &Workspace, ui: &mut Ui) {
    let palette = ws.doc.palette();
    let side = layout::SWATCH_SIZE;
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(2.0, 2.0);
        for row in ColourIndex::ALL.chunks(8) {
            ui.horizontal(|ui| {
                for &c in row {
                    let (rect, resp) =
                        ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::click());
                    let rgb = palette.rgb(c);
                    let fill = egui::Color32::from_rgb(rgb.r, rgb.g, rgb.b);
                    let p = ui.painter();
                    p.rect_filled(rect, 1.0, fill);
                    p.rect_stroke(
                        rect,
                        1.0,
                        ui.visuals().widgets.noninteractive.bg_stroke,
                        egui::StrokeKind::Inside,
                    );
                    let on = r.allowed().contains(c);
                    if !on {
                        let s = egui::Stroke::new(1.5, ui.visuals().text_color());
                        p.line_segment([rect.left_top(), rect.right_bottom()], s);
                        p.line_segment([rect.left_bottom(), rect.right_top()], s);
                    }
                    let tip = format!("{} {}", c.get(), if on { "allowed" } else { "not allowed" });
                    if resp.on_hover_text(tip).clicked() {
                        let change = if on {
                            RecipeChange {
                                deny: Some(vec![c]),
                                ..Default::default()
                            }
                        } else {
                            RecipeChange {
                                allow: Some(vec![c]),
                                ..Default::default()
                            }
                        };
                        f.recipe(None, change);
                    }
                }
            });
        }
    });
}

/// The solver picker's choices: the solver, its label and its tooltip (#261).
const SOLVERS: [(SolverKind, &str, &str); 3] = [
    (
        SolverKind::Reference,
        "Reference",
        "png2c64's pipeline, ported (ADR-010, ADR-014), the default",
    ),
    (
        SolverKind::Png2amiga,
        "png2amiga (comparison)",
        "png2amiga's C64 path with its own front end (Lanczos for every layer), kept to \
         compare against (ADR-014); not the default",
    ),
    (SolverKind::Native, "Native", "Convert64's own solver"),
];

/// The background rule's tooltip.
const BG_RULE_TIP: &str = "Experimental: how the png2amiga solver picks an unpinned multicolor \
                           background (hi-res has none; Reference and Native always search)";

/// The background rules, as the Advanced group offers them.
const BG_RULES: [(BackgroundRule, &str, &str); 2] = [
    (
        BackgroundRule::Black,
        "Black",
        "png2amiga's: always colour 0 (the default)",
    ),
    (
        BackgroundRule::Search,
        "Search",
        "png2c64's: all 16 tried, the lowest total cell error kept",
    ),
];

fn advanced(f: &mut Form, layer: &Layer, s: &SolveSettings, ui: &mut Ui) {
    let d = SolveSettings::default();
    let changed = *s != d;
    let reset = group(f, ui, "Advanced", changed, |f, ui| {
        ui.horizontal(|ui| {
            label(ui, "Solver", s.solver != d.solver);
            ui.horizontal_wrapped(|ui| {
                for (k, n, tip) in SOLVERS {
                    if ui
                        .selectable_label(s.solver == k, n)
                        .on_hover_text(tip)
                        .clicked()
                        && s.solver != k
                    {
                        f.solve(
                            None,
                            SolveSet {
                                solver: Some(k),
                                ..Default::default()
                            },
                        );
                    }
                }
            });
        });
        // Refine is not offered here (ADR-014 §5: experimental, not in the app's full
        // solve); the background rule only with the png2amiga solver, the one it acts on.
        let p2a = s.solver == SolverKind::Png2amiga;
        if p2a {
            ui.horizontal(|ui| {
                let lab = label_tip(
                    ui,
                    "png2amiga bg",
                    s.background_rule != d.background_rule,
                    Some(BG_RULE_TIP),
                );
                let mut v = s.background_rule;
                ui.horizontal_wrapped(|ui| {
                    for (k, n, tip) in BG_RULES {
                        if ui.selectable_label(v == k, n).on_hover_text(tip).clicked() {
                            v = k;
                        }
                    }
                });
                if lab.double_clicked() {
                    v = d.background_rule;
                }
                if v != s.background_rule {
                    f.solve(
                        None,
                        SolveSet {
                            background_rule: Some(v),
                            ..Default::default()
                        },
                    );
                }
            });
        }
        ui.label(RichText::new("Divergences from png2c64 (ADR-010): all off by default, so the result matches png2c64's.").small().weak());
        let div = s.divergences;
        let set_div = |f: &mut Form, g: Option<&str>, x: Divergences| {
            f.solve(
                g,
                SolveSet {
                    divergences: Some(x),
                    ..Default::default()
                },
            )
        };
        for (name, on, flip) in [
            (
                "Chroma guard",
                div.chroma_guard,
                (|d: &mut Divergences| d.chroma_guard = !d.chroma_guard) as fn(&mut Divergences),
            ),
            ("Cell clamp", div.cell_clamp, |d| {
                d.cell_clamp = !d.cell_clamp
            }),
            ("Bridging", div.bridging, |d| d.bridging = !d.bridging),
        ] {
            ui.horizontal(|ui| {
                label(ui, name, on);
                let mut v = on;
                if ui.checkbox(&mut v, "").changed() {
                    let mut x = div;
                    flip(&mut x);
                    set_div(f, None, x);
                }
            });
        }
        optional(
            f,
            ui,
            "Max luma diff",
            div.max_luma_diff,
            0..=8,
            2,
            |x| Divergences {
                max_luma_diff: x,
                ..div
            },
            &set_div,
        );
        optional(
            f,
            ui,
            "Mix levels",
            div.mix_levels,
            1..=16,
            4,
            |x| Divergences {
                mix_levels: x,
                ..div
            },
            &set_div,
        );
        commit(
            f,
            "pair bias",
            slider(ui, "Pair bias", div.pair_bias, 0.0, -1.0..=1.0, "", 2),
            |f, g, v| {
                set_div(
                    f,
                    g,
                    Divergences {
                        pair_bias: v,
                        ..div
                    },
                );
            },
        );
        ui.separator();
        ui.label(RichText::new("Solver").small().weak());
        commit(
            f,
            "refinements",
            slider(
                ui,
                "Refinements",
                f32::from(s.refinements),
                f32::from(d.refinements),
                0.0..=2.0,
                "",
                0,
            ),
            |f, _, v| {
                f.solve(
                    None,
                    SolveSet {
                        refinements: Some(v as u8),
                        ..Default::default()
                    },
                );
            },
        );
        commit(
            f,
            "finalists",
            slider(
                ui,
                "Finalists",
                f32::from(s.finalists),
                f32::from(d.finalists),
                1.0..=16.0,
                "",
                0,
            ),
            |f, _, v| {
                f.solve(
                    None,
                    SolveSet {
                        finalists: Some(v as u8),
                        ..Default::default()
                    },
                );
            },
        );
        if s.solver == SolverKind::Native {
            commit(
                f,
                "luma pair bias",
                slider(
                    ui,
                    "Luma pair bias",
                    s.luma_pair_bias,
                    d.luma_pair_bias,
                    -1.0..=1.0,
                    "",
                    2,
                ),
                |f, g, v| {
                    f.solve(
                        g,
                        SolveSet {
                            luma_pair_bias: Some(v),
                            ..Default::default()
                        },
                    );
                },
            );
            let r = layer.recipe();
            let rd = Recipe::default();
            ui.horizontal(|ui| {
                label(ui, "Smooth", r.smooth() != rd.smooth());
                let mut v = r.smooth();
                if ui.checkbox(&mut v, "Native solver, this layer").changed() {
                    f.recipe(
                        None,
                        RecipeChange {
                            smooth: Some(v),
                            ..Default::default()
                        },
                    );
                }
            });
            commit(
                f,
                "adaptive",
                slider(
                    ui,
                    "Adaptive",
                    r.adaptive() * 100.0,
                    0.0,
                    0.0..=100.0,
                    "%",
                    0,
                ),
                |f, g, v| {
                    f.recipe(
                        g,
                        RecipeChange {
                            adaptive: Some(v / 100.0),
                            ..Default::default()
                        },
                    );
                },
            );
        }
        reset_button(ui, changed)
    });
    if reset {
        f.solve(
            None,
            SolveSet {
                solver: Some(d.solver),
                refine: Some(d.refine),
                background_rule: Some(d.background_rule),
                divergences: Some(d.divergences),
                hysteresis: Some(d.hysteresis),
                luma_pair_bias: Some(d.luma_pair_bias),
                refinements: Some(d.refinements),
                finalists: Some(d.finalists),
            },
        );
    }
}

/// An optional integer setting: a checkbox to turn it on and a value when on.
#[allow(clippy::too_many_arguments)]
fn optional(
    f: &mut Form,
    ui: &mut Ui,
    name: &str,
    value: Option<u8>,
    range: std::ops::RangeInclusive<u8>,
    on_value: u8,
    with: impl Fn(Option<u8>) -> Divergences,
    set: &impl Fn(&mut Form, Option<&str>, Divergences),
) {
    ui.horizontal(|ui| {
        label(ui, name, value.is_some());
        let mut on = value.is_some();
        if ui.checkbox(&mut on, "").changed() {
            set(f, None, with(on.then_some(on_value)));
        }
        if let Some(v) = value {
            let mut x = v;
            if ui.add(egui::DragValue::new(&mut x).range(range)).changed() {
                set(f, None, with(Some(x)));
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;

    /// A workspace with one 64 × 40 ramp layer and `sharpen` on it.
    fn workspace(sharpen: Sharpen) -> Workspace {
        let rgba = (0..64 * 40)
            .flat_map(|i| [(i % 64 * 4) as u8, (i / 64 * 6) as u8, 90, 255])
            .collect();
        let png = convert64_project::codec::encode_rgba8(64, 40, rgba);
        let mut doc = Document::from_png("ramp.png", &png).unwrap();
        doc.run(JsonCommand::AdjustmentsSet {
            layer: LayerRef::Index(0),
            adjustments: AdjustmentsChange {
                sharpen: Some(sharpen),
                ..Default::default()
            },
        })
        .unwrap();
        Workspace::new(doc)
    }

    /// The window's width in [`draw`].
    const WINDOW: f32 = 1280.0;

    /// One frame of the shell with the Recipe tab, in a window tall enough for every
    /// group and the dock at its default width: each text drawn with its rectangle, and
    /// the actions the form asked for.
    fn draw(mut ws: Workspace) -> (Vec<(String, egui::Rect)>, Vec<Action>) {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let mut shell = Shell {
            tab: super::super::DockTab::Recipe,
            ..Shell::default()
        };
        frame(&ctx, &mut shell, &mut ws, Vec::new())
    }

    /// One frame of `shell` on `ctx` (a [`WINDOW`] wide, 3000 point tall window) with
    /// `events`: each text drawn with its rectangle, and the actions asked for.
    fn frame(
        ctx: &egui::Context,
        shell: &mut Shell,
        ws: &mut Workspace,
        events: Vec<egui::Event>,
    ) -> (Vec<(String, egui::Rect)>, Vec<Action>) {
        fn collect(shape: &egui::Shape, out: &mut Vec<(String, egui::Rect)>) {
            match shape {
                egui::Shape::Text(t) => {
                    out.push((t.galley.text().to_owned(), t.visual_bounding_rect()));
                }
                egui::Shape::Vec(v) => v.iter().for_each(|s| collect(s, out)),
                _ => {}
            }
        }
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(WINDOW, 3000.0),
            )),
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
        (texts, actions)
    }

    /// The texts in the dock (right of the working view), top to bottom.
    fn dock(texts: Vec<(String, egui::Rect)>) -> Vec<(String, egui::Rect)> {
        let left = WINDOW - layout::DOCK_WIDTH - 20.0;
        let mut out: Vec<_> = texts.into_iter().filter(|(_, r)| r.left() > left).collect();
        out.sort_by(|a, b| a.1.top().total_cmp(&b.1.top()));
        out
    }

    /// A press and release at `p`, as two frames' events.
    fn click(p: egui::Pos2) -> [Vec<egui::Event>; 2] {
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

    /// #288 (usability review M5): the groups run by use, the dither method first, and
    /// Sharpen right after Adjustments (#237); Advanced last.
    #[test]
    fn the_groups_run_in_the_order_of_use() {
        let (texts, _) = draw(workspace(Sharpen::default()));
        let texts = dock(texts);
        let y = |t: &str| find(&texts, t).center().y;
        for pair in GROUP_ORDER.windows(2) {
            assert!(y(pair[0]) < y(pair[1]), "{pair:?}: {texts:?}");
        }
        // The dither method is the first row of the first group.
        assert!(y("Dither recipe") < y("Method") && y("Method") < y("Colour fitting"));
    }

    /// #288: Transform, Mask and Advanced start collapsed, the others open.
    #[test]
    fn transform_and_mask_start_collapsed() {
        let groups = RecipeGroups::default();
        for title in GROUP_ORDER {
            let collapsed = matches!(title, "Transform" | "Mask" | "Advanced");
            assert_eq!(groups.is_open(title), !collapsed, "{title}");
        }
        let (texts, _) = draw(workspace(Sharpen::default()));
        let texts = dock(texts);
        let has = |t: &str| texts.iter().any(|(s, _)| s == t);
        // The headings are there, their rows are not.
        assert!(has("Transform") && has("Mask") && has("Advanced"));
        for row in ["Rotate", "Anchor", "Presets", "Add mask", "Solver"] {
            assert!(!has(row), "{row} shown: {texts:?}");
        }
        for row in ["Method", "Metric", "Brightness", "Threshold"] {
            assert!(has(row), "{row} missing: {texts:?}");
        }
    }

    /// Usability review m6: the dither family control that wrapped "Palette-aware" is
    /// gone (the #260 method picker replaced it). The segmented rows left, Phase and
    /// Orientation, sit on one line and are narrower than the dock at its narrowest
    /// (Phase's *Screen-locked · Layer-locked* were not: 281 px).
    #[test]
    fn the_segmented_dither_rows_fit_the_narrowest_dock() {
        let (texts, _) = draw(workspace(Sharpen::default()));
        let texts = dock(texts);
        let margin = 2.0 * 6.0;
        let narrowest = *super::super::DOCK_WIDTH_RANGE.start() - margin;
        for (label, last) in [("Phase", "Layer"), ("Orientation", "2:1")] {
            let row = find(&texts, label);
            let end = find(&texts, last);
            assert!(
                (row.center().y - end.center().y).abs() < 2.0,
                "{label}: {last} wrapped"
            );
            // From the changed-value dot (6 px and a gap before the label) to the end.
            let width = end.right() - (row.left() - 6.0 - 8.0);
            assert!(width <= narrowest, "{label} row is {width} px");
        }
        assert!(!texts.iter().any(|(t, _)| t == "Palette-aware"));
    }

    /// Usability review M11 (#293): with every group open, in both themes, no text in
    /// the window is smaller than egui's `Small` (11 px), and the hint lines are painted
    /// in the weak text colour at 4.5:1 or better on the dock.
    #[test]
    fn hints_are_small_style_or_larger_and_readable() {
        fn walk(shape: &egui::Shape, out: &mut Vec<(String, f32, egui::Color32)>) {
            match shape {
                egui::Shape::Text(t) => {
                    for s in &t.galley.job.sections {
                        let text =
                            t.galley.job.text[s.byte_range.start.0..s.byte_range.end.0].to_owned();
                        let colour = if s.format.color == egui::Color32::PLACEHOLDER {
                            t.fallback_color
                        } else {
                            s.format.color
                        };
                        let colour = t.override_text_color.unwrap_or(colour);
                        out.push((text, s.format.font_id.size, colour));
                    }
                }
                egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                _ => {}
            }
        }
        for theme in [egui::Theme::Light, egui::Theme::Dark] {
            let ctx = egui::Context::default();
            crate::theme::install(&ctx);
            ctx.set_theme(theme);
            let mut ws = workspace(Sharpen::default());
            let mut shell = Shell {
                tab: super::super::DockTab::Recipe,
                ..Shell::default()
            };
            for title in GROUP_ORDER {
                shell.groups.set_open(title, true);
            }
            let input = || egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(WINDOW, 3000.0),
                )),
                ..Default::default()
            };
            let mut output = ctx.run_ui(input(), |ui| {
                shell.show(ui, Some(&mut ws));
            });
            output.textures_delta.clear();
            let mut output = ctx.run_ui(input(), |ui| {
                shell.show(ui, Some(&mut ws));
            });
            output.textures_delta.clear();
            let mut texts = Vec::new();
            for clipped in &output.shapes {
                walk(&clipped.shape, &mut texts);
            }
            let small = crate::theme::text_styles()[&egui::TextStyle::Small].size;
            let tiny: Vec<_> = texts
                .iter()
                .filter(|(t, size, _)| !t.trim().is_empty() && *size < small)
                .collect();
            assert!(tiny.is_empty(), "{theme:?}: smaller than Small: {tiny:?}");
            let visuals = crate::theme::style(theme).visuals;
            for hint in [
                "No mask: the whole layer shows.",
                "Adaptive lets the solver reassign cell colours for the lowest error.",
            ] {
                let (_, size, colour) = texts
                    .iter()
                    .find(|(t, _, _)| t == hint)
                    .unwrap_or_else(|| panic!("{theme:?}: no {hint:?}"));
                assert!(*size >= small, "{hint}: {size} px");
                assert_eq!(*colour, visuals.weak_text_color(), "{theme:?}: {hint}");
                let ratio = crate::theme::contrast(*colour, visuals.panel_fill);
                assert!(ratio >= 4.5, "{theme:?}: {hint} at {ratio:.2}:1");
            }
        }
    }

    /// #288: a click on a heading opens the group and the choice is the user's: it
    /// survives the app's storage and holds for another layer's form.
    #[test]
    fn an_opened_group_is_remembered() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let mut ws = workspace(Sharpen::default());
        let mut shell = Shell {
            tab: super::super::DockTab::Recipe,
            ..Shell::default()
        };
        let (texts, _) = frame(&ctx, &mut shell, &mut ws, Vec::new());
        let heading = find(&dock(texts), "Transform").center();
        for events in click(heading) {
            frame(&ctx, &mut shell, &mut ws, events);
        }
        assert!(shell.groups.is_open("Transform"));
        assert!(!shell.groups.is_open("Mask"));
        // A click on an open group's heading closes it.
        let (texts, _) = frame(&ctx, &mut shell, &mut ws, Vec::new());
        let heading = find(&dock(texts), "Dither recipe").center();
        for events in click(heading) {
            frame(&ctx, &mut shell, &mut ws, events);
        }
        assert!(!shell.groups.is_open("Dither recipe"));

        // Stored and read back, as the app does (eframe's storage), in a new session.
        let stored = shell.groups.to_storage();
        let groups = RecipeGroups::from_storage(&stored);
        assert_eq!(groups, shell.groups);
        assert_eq!(
            RecipeGroups::from_storage("not json"),
            RecipeGroups::default()
        );
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let mut shell = Shell {
            tab: super::super::DockTab::Recipe,
            groups,
            ..Shell::default()
        };
        let mut ws = workspace(Sharpen::default());
        let mut texts = Vec::new();
        // Two frames: the first opens the group, egui draws its body once it is open.
        for _ in 0..2 {
            texts = dock(frame(&ctx, &mut shell, &mut ws, Vec::new()).0);
        }
        let has = |t: &str| texts.iter().any(|(s, _)| s == t);
        assert!(has("Rotate"), "{texts:?}");
        assert!(!has("Strength"), "{texts:?}");
    }

    /// #288: the form edits the layer only. Mode, Palette and Background are the
    /// result's picture row, above the result, and only there; the layer header names
    /// the layer.
    #[test]
    fn mode_palette_and_background_are_in_the_result_header_only() {
        let ws = workspace(Sharpen::default());
        let name = ws.doc.session().project().layers[0].name().to_owned();
        let (texts, actions) = draw(ws);
        assert!(actions.is_empty(), "{actions:?}");
        let texts = dock(texts);
        let all = |t: &str| texts.iter().filter(|(s, _)| s == t).count();
        assert_eq!(all("Multicolor"), 1, "{texts:?}");
        assert_eq!(all("Pepto PAL"), 1, "{texts:?}");
        for gone in ["Mode", "Palette", "Background", "Auto (the solver picks)"] {
            assert_eq!(all(gone), 0, "{gone}: {texts:?}");
        }
        // The picture row: under the header, above the tabs, on one line. (The form
        // has an Auto of its own: Orientation's.)
        let result = find(&texts, "Result");
        let tab = find(&texts, "Recipe");
        let mode = find(&texts, "Multicolor");
        for r in [find(&texts, "Pepto PAL"), find(&texts, "Auto")] {
            assert!((r.center().y - mode.center().y).abs() < 2.0, "{texts:?}");
            assert!(mode.right() < r.left());
        }
        assert!(result.bottom() < mode.top() && mode.bottom() < tab.top());
        // The layer header, then the first group.
        let header = find(&texts, &name);
        let subtitle = find(&texts, "Layer 1 of 1 · 64 × 40 px");
        assert!(tab.bottom() < header.top() && header.bottom() <= subtitle.top());
        assert!(subtitle.bottom() < find(&texts, "Dither recipe").top());
    }

    /// #288: a region's recipe has the layer form's order (Dither recipe, then Colour
    /// fitting) and no Mode row.
    #[test]
    fn region_recipes_run_in_the_same_order() {
        let mut ws = workspace(Sharpen::default());
        ws.doc
            .run(JsonCommand::RegionAdd {
                name: "Sky".to_owned(),
                recipe: Box::default(),
                index: None,
            })
            .unwrap();
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let mut shell = Shell {
            tab: super::super::DockTab::Recipe,
            tool: super::super::Tool::Region,
            ..Shell::default()
        };
        let (texts, _) = frame(&ctx, &mut shell, &mut ws, Vec::new());
        let texts = dock(texts);
        let y = |t: &str| find(&texts, t).center().y;
        assert!(y("Sky") < y("Dither recipe"));
        assert!(y("Dither recipe") < y("Method") && y("Method") < y("Colour fitting"));
        assert!(!texts.iter().any(|(s, _)| s == "Mode"), "{texts:?}");
    }

    /// #290 (usability review M7): the Region tool's Recipe tab opens with the
    /// breadcrumb `Sky ‹ ramp.png`, and a click on the layer's name goes back to that
    /// layer's recipe and the Nudge tool.
    #[test]
    fn the_region_breadcrumb_goes_back_to_the_layer() {
        let mut ws = workspace(Sharpen::default());
        ws.doc
            .run(JsonCommand::RegionAdd {
                name: "Sky".to_owned(),
                recipe: Box::default(),
                index: None,
            })
            .unwrap();
        let name = ws.doc.session().project().layers[0].name().to_owned();
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let mut shell = Shell {
            tab: super::super::DockTab::Recipe,
            tool: super::super::Tool::Region,
            ..Shell::default()
        };
        let mut texts = Vec::new();
        for _ in 0..2 {
            texts = dock(frame(&ctx, &mut shell, &mut ws, Vec::new()).0);
        }
        let here = find(&texts, "Sky");
        let sep = find(&texts, "‹");
        let back = find(&texts, &name);
        let same_row = |r: egui::Rect| (r.center().y - here.center().y).abs() < 4.0;
        assert!(same_row(sep) && same_row(back), "{texts:?}");
        assert!(here.right() < sep.left() && sep.right() < back.left());
        // Above the region list and the recipe.
        assert!(here.bottom() < find(&texts, "Add region").top());
        assert!(!texts.iter().any(|(t, _)| t.starts_with("Layer 1 of")));

        for events in click(back.center()) {
            frame(&ctx, &mut shell, &mut ws, events);
        }
        assert_eq!(shell.tool, super::super::Tool::Nudge);
        let texts = dock(frame(&ctx, &mut shell, &mut ws, Vec::new()).0);
        assert!(texts.iter().any(|(t, _)| t == "Layer 1 of 1 · 64 × 40 px"));
        assert!(!texts.iter().any(|(t, _)| t == "Sky" || t == "‹"));
    }

    /// The transforms the frame asked `layer_set_transform` for.
    fn transforms(actions: &[Action]) -> Vec<convert64_core::Transform> {
        actions
            .iter()
            .filter_map(|a| match a {
                Action::Run(JsonCommand::LayerSetTransform { transform, .. }) => Some(*transform),
                _ => None,
            })
            .collect()
    }

    /// #289 (usability review M6): W × H in screen pixels with the % of the source and
    /// the multicolor width under them; Fill selected for an opened picture; the
    /// presets as visible buttons, 1:1 landing on whole multicolor pixels; the flips as
    /// toggles; the dock keeps its width.
    #[test]
    fn the_transform_group_shows_screen_pixels_and_presets() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        // 64 × 40, opened: 1:1, as it fits inside the screen (#306).
        let mut ws = workspace(Sharpen::default());
        let mut groups = RecipeGroups::default();
        groups.set_open("Transform", true);
        let mut shell = Shell {
            tab: super::super::DockTab::Recipe,
            groups,
            ..Shell::default()
        };
        let mut texts = Vec::new();
        for _ in 0..2 {
            texts = dock(frame(&ctx, &mut shell, &mut ws, Vec::new()).0);
        }
        let row = find(&texts, "W × H");
        let on_row = |text: &str, at: egui::Rect| {
            texts
                .iter()
                .find(|(t, r)| t == text && (r.center().y - at.center().y).abs() < 4.0)
                .map(|(_, r)| *r)
                .unwrap_or_else(|| panic!("no {text:?} on the row: {texts:?}"))
        };
        let w = on_row("64.00", row);
        let h = on_row("40.00", row);
        assert!(row.right() < w.left() && w.right() < h.left());
        let pct = find(&texts, "100.00 % × 100.00 % of the source");
        let mc = find(&texts, "32.0 multicolor px wide");
        assert!(row.bottom() < pct.top() && pct.bottom() <= mc.top());
        assert!(mc.bottom() < find(&texts, "Rotate").top());
        let presets = find(&texts, "Presets");
        let buttons = ["Fit", "Fill", "1:1", "2:1"].map(|p| on_row(p, presets));
        assert!(buttons.windows(2).all(|b| b[0].right() < b[1].left()));
        // Nothing pushed the dock wider than its default.
        let tab = find(&texts, "Recipe");
        assert!(tab.left() > WINDOW - layout::DOCK_WIDTH, "{tab:?}");
        assert!(texts.iter().all(|(_, r)| r.right() <= WINDOW), "{texts:?}");

        // Fill on a 16:10 picture is 5 × 5 from the top left, the whole screen.
        let mut asked = Vec::new();
        for events in click(buttons[1].center()) {
            asked.extend(frame(&ctx, &mut shell, &mut ws, events).1);
        }
        let fill = transforms(&asked);
        assert_eq!(fill.len(), 1, "{asked:?}");
        assert_eq!(fill[0].scale(), convert64_core::Vec2::new(5.0, 5.0));
        assert_eq!(fill[0].translate(), convert64_core::Vec2::ZERO);

        // The flips are two icon toggles beside the Flip label; the first is horizontal.
        let flip = find(&texts, "Flip");
        let mut glyphs: Vec<egui::Rect> = texts
            .iter()
            .filter(|(t, r)| {
                t == crate::icons::FLIP.codepoint && (r.center().y - flip.center().y).abs() < 6.0
            })
            .map(|(_, r)| *r)
            .collect();
        glyphs.sort_by(|a, b| a.left().total_cmp(&b.left()));
        assert_eq!(glyphs.len(), 2, "{texts:?}");
        assert!(flip.right() < glyphs[0].left());
        let mut asked = Vec::new();
        for events in click(glyphs[0].center()) {
            asked.extend(frame(&ctx, &mut shell, &mut ws, events).1);
        }
        let flipped = transforms(&asked);
        assert_eq!(flipped.len(), 1, "{asked:?}");
        assert!(flipped[0].flip_h() && !flipped[0].flip_v());
    }

    /// #289: the anchor control's cells are 16 px, in a 3 × 3 grid.
    #[test]
    fn the_anchor_control_is_3_by_3_at_16_px() {
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(400.0, 300.0),
            )),
            ..Default::default()
        };
        let mut size = egui::Vec2::ZERO;
        let mut out = ctx.run_ui(input, |ui| {
            let before = ui.cursor().min;
            ui.horizontal(|ui| {
                anchor_control(ui, convert64_core::Vec2::new(0.5, 0.5));
            });
            size = ui.min_rect().max - before;
        });
        out.textures_delta.clear();
        assert_eq!(ANCHOR_CELL, 16.0);
        assert_eq!(ANCHORS.len(), 9);
        // Three cells and two gaps and the frame's padding each way.
        assert!(
            size.x >= 3.0 * ANCHOR_CELL && size.y >= 3.0 * ANCHOR_CELL,
            "{size:?}"
        );
    }

    /// The rectangle of the first text that reads `text`.
    fn find(texts: &[(String, egui::Rect)], text: &str) -> egui::Rect {
        texts
            .iter()
            .find(|(t, _)| t == text)
            .unwrap_or_else(|| panic!("no {text:?} in {texts:?}"))
            .1
    }

    /// #237: Sharpen is a group of its own after Adjustments (after Levels and Curves,
    /// UX §6), and Range stays in Adjustments, above the Sharpen heading.
    #[test]
    fn sharpen_is_its_own_group_after_levels_curves_and_range() {
        let (texts, actions) = draw(workspace(Sharpen::default()));
        assert!(actions.is_empty(), "{actions:?}");
        let y = |t: &str| find(&texts, t).center().y;
        assert!(y("Adjustments") < y("Mid-tones"));
        assert!(y("Mid-tones") < y("Curves"));
        assert!(y("Curves") < y("Range"));
        assert!(y("Range") < y("Sharpen"));
        assert!(y("Sharpen") < y("Amount"));
        assert!(y("Amount") < y("Radius") && y("Radius") < y("Threshold"));
        assert!(y("Threshold") < y("Mask"));
        // Adjustments' Reset sits between Range and the Sharpen heading; Sharpen's
        // own after Threshold.
        let resets: Vec<f32> = texts
            .iter()
            .filter(|(t, _)| t == "Reset group")
            .map(|(_, r)| r.center().y)
            .collect();
        assert!(resets.iter().any(|&r| y("Range") < r && r < y("Sharpen")));
        assert!(resets.iter().any(|&r| y("Threshold") < r && r < y("Mask")));
    }

    /// #237: with Gaussian chosen both segment labels stay visible and the σ field sits
    /// beside them on the Radius row, to their right, inside the dock.
    #[test]
    fn gaussian_keeps_the_segment_and_puts_sigma_beside_it() {
        let s = Sharpen::new(0.8, Radius::gaussian(1.2).unwrap(), 0.02).unwrap();
        let (texts, actions) = draw(workspace(s));
        assert!(actions.is_empty(), "rounded σ re-sent: {actions:?}");
        let radius = find(&texts, "Radius");
        let png2c64 = find(&texts, "png2c64");
        let gaussian = find(&texts, "Gaussian");
        let sigma = find(&texts, "1.20");
        for r in [png2c64, gaussian, sigma] {
            assert!((r.center().y - radius.center().y).abs() < 4.0, "{texts:?}");
        }
        assert!(radius.right() < png2c64.left());
        assert!(png2c64.right() < gaussian.left());
        assert!(gaussian.right() < sigma.left());
        // Its unit too, and inside the dock (the Reset buttons end at its edge).
        let unit = texts
            .iter()
            .find(|(t, r)| t == " px" && (r.center().y - radius.center().y).abs() < 4.0)
            .expect("σ's unit on the Radius row")
            .1;
        let edge = find(&texts, "Reset group").right();
        assert!(
            sigma.right() < unit.left() && unit.right() <= edge,
            "{unit:?}"
        );
        assert!(edge < WINDOW);
    }

    /// With png2c64's kernel there is no σ to show, and Gaussian is still offered.
    #[test]
    fn png2c64_radius_shows_no_sigma() {
        let (texts, _) = draw(workspace(Sharpen::png2c64(0.5).unwrap()));
        let radius = find(&texts, "Radius");
        find(&texts, "png2c64");
        let gaussian = find(&texts, "Gaussian");
        let after = texts.iter().filter(|(_, r)| {
            (r.center().y - radius.center().y).abs() < 4.0 && r.left() > gaussian.right()
        });
        assert_eq!(after.count(), 0, "{texts:?}");
    }

    /// A [`workspace`] whose layer has range matching `m`.
    fn with_range(m: RangeMatch) -> Workspace {
        let mut ws = workspace(Sharpen::default());
        ws.doc
            .run(JsonCommand::AdjustmentsSet {
                layer: LayerRef::Index(0),
                adjustments: AdjustmentsChange {
                    match_range: Some(Some(m)),
                    ..Default::default()
                },
            })
            .unwrap();
        ws
    }

    /// #261: with png2amiga's hull on, an Amount row (0–100 %) follows Range, still in
    /// Adjustments, showing the hull's amount; drawing it sends nothing.
    #[test]
    fn hull_range_matching_shows_its_amount() {
        let hull = RangeMatch::hull(&convert64_core::Palette::pepto_pal(), 0.4).unwrap();
        let (texts, actions) = draw(with_range(hull));
        assert!(actions.is_empty(), "{actions:?}");
        let range = find(&texts, "Range");
        let sharpen = find(&texts, "Sharpen");
        let amount = texts
            .iter()
            .find(|(t, r)| t == "Amount" && r.top() > range.bottom() && r.bottom() < sharpen.top())
            .unwrap_or_else(|| panic!("no Amount between Range and Sharpen: {texts:?}"))
            .1;
        let on_row = |text: &str| {
            texts
                .iter()
                .any(|(t, r)| t == text && (r.center().y - amount.center().y).abs() < 4.0)
        };
        assert!(
            on_row("40") && on_row(" %"),
            "no 40 % on the Amount row: {texts:?}"
        );
    }

    /// A box (png2c64's, the default) has no amount, and a Method row offers the hull.
    #[test]
    fn box_range_matching_has_no_amount() {
        let pal = convert64_core::Palette::pepto_pal();
        let boxed = RangeMatch::for_palette(&pal, 0.01, 0.05).unwrap();
        let (texts, actions) = draw(with_range(boxed));
        assert!(actions.is_empty(), "{actions:?}");
        let range = find(&texts, "Range");
        let sharpen = find(&texts, "Sharpen");
        assert!(
            !texts.iter().any(|(t, r)| t == "Amount"
                && r.top() > range.bottom()
                && r.bottom() < sharpen.top()),
            "{texts:?}"
        );
        assert!(texts.iter().any(|(t, _)| t == "Method"), "{texts:?}");
        assert!(
            texts.iter().any(|(t, _)| t == "Hull (experimental)"),
            "{texts:?}"
        );
    }

    /// Adjustments' dot and Reset ignore Sharpen, which has its own.
    #[test]
    fn the_adjustments_group_leaves_sharpen_to_its_own_group() {
        let mut a = Adjustments {
            sharpen: Sharpen::png2c64(0.5).unwrap(),
            ..Default::default()
        };
        assert!(adjustments_default(&a));
        a.gamma = Gamma::new(1.5).unwrap();
        assert!(!adjustments_default(&a));
    }
}
