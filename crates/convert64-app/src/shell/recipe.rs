//! The Recipe tab (UX §6): the selected layer's adjustments, dither recipe and colour
//! fitting, then **Advanced**: the solver (png2amiga's reference, png2c64's or the
//! native one, #261), png2amiga's Refine, ADR-010's divergences of the png2c64 solver
//! and the solver's own settings, collapsed and off by default. Every change is a registry command
//! (`adjustments_set`, `recipe_set`, `solve_set`); a drag is one undo step.
//!
//! Conventions (UX §6): label, then slider with its value field, on one row; a dot
//! before a label whose value differs from the default; a double click on a label
//! resets that value; each group has a Reset.

use super::{Action, Shell};
use crate::theme::{Theme, layout};
use crate::workspace::Workspace;
use convert64_core::ColourIndex;
use convert64_core::SolveSettings;
use convert64_core::command::{AdjustmentsChange, RecipeChange, SolveSet};
use convert64_core::layer::{
    Adjustments, BrightnessContrast, Curve, Curves, DitherFamily, DitherKind, FitMode, Gamma, Hsl,
    Layer, Levels, Method, Metric, Orientation, PatternPhase, Radius, RangeMatch, RangeMethod,
    Recipe, Sharpen,
};
use convert64_core::solve::{Divergences, SolverKind};
use convert64_project::{JsonCommand, LayerRef};
use egui::{Response, RichText, Ui};

/// The width of a row's label column, in points.
const LABEL_WIDTH: f32 = 90.0;

/// Emits commands for one layer, or for one region's recipe (the Region tool's
/// pseudo-layer, UX §7).
struct Form<'a> {
    actions: &'a mut Vec<Action>,
    layer: usize,
    /// Recipe changes go to this region instead of the layer.
    region: Option<usize>,
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
            egui::Slider::new(&mut v, range)
                .suffix(suffix)
                .fixed_decimals(decimals),
        );
        if let Some(tip) = tip {
            r = r.on_hover_text(tip);
        }
        // The slider rounds to its decimals: a value it merely snapped (0.6 × 100 is not
        // exactly 60) is not an edit, or it would be re-sent every frame.
        let step = 0.5 * 10f32.powi(-(decimals as i32));
        if lab.double_clicked() {
            edit = Edit::Set(default);
        } else if r.changed() && (v - value).abs() > step {
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

/// A collapsible group with a dot when anything in it changed and a Reset button.
fn group(
    ui: &mut Ui,
    title: &str,
    changed: bool,
    open: bool,
    add: impl FnOnce(&mut Ui) -> bool,
) -> bool {
    let mut reset = false;
    let mut text = RichText::new(title).family(egui::FontFamily::Name(crate::theme::MEDIUM.into()));
    if changed {
        text = text.color(Theme::current(ui.ctx()).accent);
    }
    egui::CollapsingHeader::new(text)
        .id_salt(title)
        .default_open(open)
        .show(ui, |ui| {
            reset = add(ui);
        });
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
    if shell.tool == super::Tool::Region && shell.mask.tool.is_none() {
        let mut f = Form {
            actions,
            layer: i,
            region: None,
        };
        return regions(&mut f, shell, ws, ui);
    }
    let mut f = Form {
        actions,
        layer: i,
        region: None,
    };
    picture(&mut f, ws, ui);
    ui.label(
        RichText::new(format!("Layer: {}", layer.name()))
            .small()
            .weak(),
    );

    transform_group(&mut f, shell, ws, layer, ui);
    adjustments(&mut f, shell, ws, layer, ui);
    sharpen(&mut f, layer.adjustments().sharpen, ui);
    mask_group(&mut f, shell, ws, layer, ui);
    dither(&mut f, layer.recipe(), ui);
    fitting(&mut f, layer.recipe(), ws, ui);
    advanced(&mut f, layer, &project.solve, ui);
}

/// The Adjustments group (UX §6 item 2): tone and colour, Levels, Curves and Range.
/// Sharpen has its own group after this one, with its own Reset.
fn adjustments(f: &mut Form, shell: &mut Shell, ws: &Workspace, layer: &Layer, ui: &mut Ui) {
    let a = layer.adjustments();
    let d = Adjustments::default();
    let changed = !adjustments_default(a);
    let reset = group(ui, "Adjustments", changed, true, |ui| {
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
                // png2amiga's hull at its full amount (#261).
                let m =
                    (v && !lab.double_clicked()).then(|| RangeMatch::hull(ws.doc.palette(), 1.0));
                f.adjust(
                    None,
                    AdjustmentsChange {
                        match_range: Some(m.and_then(Result::ok)),
                        ..Default::default()
                    },
                );
            }
        });
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
            Some(_) => {
                ui.label(
                    RichText::new(
                        "png2c64's box (an older project): turn Range off and on for png2amiga's",
                    )
                    .small()
                    .weak(),
                );
            }
            None => {}
        }
        reset_button(ui, changed)
    });
    if reset {
        f.adjust(
            None,
            AdjustmentsChange {
                gamma: Some(d.gamma),
                sharpen: None,
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
const RANGE_TIP: &str = "png2amiga's --match-range: each colour's chroma scaled toward what \
                         the palette can reach, lightness and hue kept; after the curves";

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
    let reset = group(ui, "Sharpen", changed, true, |ui| {
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

/// The picture-wide settings at the top (Figma `Recipe tab`): mode, palette (Pepto PAL
/// and NTSC, Colodore, a `.vpl` file) and the background (auto or one colour).
fn picture(f: &mut Form, ws: &Workspace, ui: &mut Ui) {
    use convert64_core::Palette;
    use convert64_core::command::BackgroundPin;
    use convert64_core::mode::Mode;
    use convert64_core::solve::Background;
    use convert64_project::PaletteSpec;
    let mode = ws.doc.mode();
    ui.horizontal(|ui| {
        label(ui, "Mode", false);
        egui::ComboBox::from_id_salt("recipe_mode")
            .selected_text(super::status::mode_name(mode))
            .show_ui(ui, |ui| {
                for m in [Mode::MulticolorBitmap, Mode::HiresBitmap] {
                    if ui
                        .selectable_label(m == mode, super::status::mode_name(m))
                        .clicked()
                        && m != mode
                    {
                        f.actions.push(Action::SetMode(m));
                    }
                }
            });
    });
    let palette = ws.doc.palette();
    let presets = [
        (PaletteSpec::PeptoPal, Palette::pepto_pal(), "Pepto PAL"),
        (PaletteSpec::PeptoNtsc, Palette::pepto_ntsc(), "Pepto NTSC"),
        (
            PaletteSpec::Colodore,
            Palette::colodore(50.0, 100.0, 50.0),
            "Colodore",
        ),
    ];
    let current = presets
        .iter()
        .find(|(_, p, _)| p == palette)
        .map_or_else(|| palette.name().to_owned(), |(_, _, n)| (*n).to_owned());
    ui.horizontal(|ui| {
        label(ui, "Palette", current != "Pepto PAL");
        egui::ComboBox::from_id_salt("recipe_palette")
            .selected_text(&current)
            .show_ui(ui, |ui| {
                for (spec, p, name) in presets {
                    if ui.selectable_label(&p == palette, name).clicked() && &p != palette {
                        f.emit(None, JsonCommand::PaletteSet { palette: spec });
                    }
                }
                ui.separator();
                if ui.button("VICE palette file (.vpl)…").clicked() {
                    f.actions.push(Action::LoadPalette);
                }
            });
    });
    let bg = ws.doc.session().project().solve.background;
    ui.horizontal(|ui| {
        label(ui, "Background", bg != Background::Auto);
        ui.vertical(|ui| {
            let mut auto = bg == Background::Auto;
            if ui.checkbox(&mut auto, "Auto (the solver picks)").changed() {
                let background = if auto {
                    Background::Auto
                } else {
                    let chosen = ws
                        .solution
                        .as_ref()
                        .and_then(|s| s.background)
                        .unwrap_or(ColourIndex::ALL[0]);
                    Background::Fixed(chosen)
                };
                f.emit(
                    None,
                    JsonCommand::BackgroundPin(BackgroundPin { background }),
                );
            }
            let side = layout::SWATCH_SIZE;
            ui.spacing_mut().item_spacing = egui::vec2(2.0, 2.0);
            for row in ColourIndex::ALL.chunks(8) {
                ui.horizontal(|ui| {
                    for &c in row {
                        let (rect, resp) =
                            ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::click());
                        let rgb = palette.rgb(c);
                        ui.painter().rect_filled(
                            rect,
                            1.0,
                            egui::Color32::from_rgb(rgb.r, rgb.g, rgb.b),
                        );
                        let solved = ws.solution.as_ref().and_then(|s| s.background) == Some(c);
                        let pinned = bg == Background::Fixed(c);
                        // The chosen background is ringed: accent when pinned, text colour when
                        // the solver picked it (not colour alone: the ring's width differs too).
                        if pinned || solved {
                            let (w, colour) = if pinned {
                                (2.5, Theme::current(ui.ctx()).accent)
                            } else {
                                (1.5, ui.visuals().text_color())
                            };
                            ui.painter().rect_stroke(
                                rect,
                                1.0,
                                (w, colour),
                                egui::StrokeKind::Outside,
                            );
                        }
                        let tip = format!(
                            "Background {}{}",
                            c.get(),
                            if pinned {
                                " (pinned)"
                            } else if solved {
                                " (solver's pick)"
                            } else {
                                ""
                            }
                        );
                        if resp.on_hover_text(tip).clicked() && !pinned {
                            f.emit(
                                None,
                                JsonCommand::BackgroundPin(BackgroundPin {
                                    background: Background::Fixed(c),
                                }),
                            );
                        }
                    }
                });
            }
        });
    });
    ui.separator();
}

/// The Region tool's panel: the regions (select, add, rename, remove) and the selected
/// region's dither recipe and colour fitting, edited like a layer's (`region_set_recipe`).
fn regions(f: &mut Form, shell: &mut Shell, ws: &Workspace, ui: &mut Ui) {
    use convert64_project::command::RegionRef;
    let regions = &ws.doc.session().project().regions;
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
    ui.label(
        RichText::new(format!("Region: {}", region.name()))
            .small()
            .weak(),
    );
    f.region = Some(shell.region);
    dither(f, region.recipe(), ui);
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

/// The Transform group (UX §6 item 1, ADR-011): X, Y, W %, H % (chained by default),
/// rotation, the anchor, Snap, flips, Downscale (#235: Auto or png2c64 exact) and
/// presets, in C64 (canvas) pixels, sub-pixel.
fn transform_group(f: &mut Form, shell: &mut Shell, ws: &Workspace, layer: &Layer, ui: &mut Ui) {
    use convert64_core::Vec2;
    let t = layer.transform();
    let size = layer.source().size();
    let (w, h) = (size.0 as f32, size.1 as f32);
    let changed = t.rotate() != 0.0
        || t.flip_h()
        || t.flip_v()
        || layer.downscale() != convert64_core::Downscale::Auto;
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
    let _ = group(ui, "Transform", changed, true, |ui| {
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
        ui.horizontal(|ui| {
            label(ui, "W / H", false);
            let mut wp = sc.x * 100.0;
            let mut hp = sc.y * 100.0;
            let rw = ui.add(
                egui::DragValue::new(&mut wp)
                    .speed(0.1)
                    .range(1.0..=1000.0)
                    .fixed_decimals(2)
                    .suffix(" %"),
            );
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
            let rh = ui.add(
                egui::DragValue::new(&mut hp)
                    .speed(0.1)
                    .range(1.0..=1000.0)
                    .fixed_decimals(2)
                    .suffix(" %"),
            );
            let linked = shell.scale_linked;
            let target = if rw.changed() {
                Some((
                    wp / 100.0,
                    if linked {
                        sc.y * wp / 100.0 / sc.x
                    } else {
                        sc.y
                    },
                    rw.dragged(),
                ))
            } else if rh.changed() {
                Some((
                    if linked {
                        sc.x * hp / 100.0 / sc.y
                    } else {
                        sc.x
                    },
                    hp / 100.0,
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
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(2.0, 2.0);
                for ay in [0.0, 0.5, 1.0] {
                    ui.horizontal(|ui| {
                        for ax in [0.0, 0.5, 1.0] {
                            let on = t.anchor() == Vec2::new(ax, ay);
                            let (rect, resp) = ui
                                .allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::click());
                            let v = ui.visuals();
                            let stroke = if resp.hovered() {
                                v.widgets.hovered.fg_stroke
                            } else {
                                v.widgets.inactive.fg_stroke
                            };
                            if on {
                                ui.painter().rect_filled(
                                    rect,
                                    2.0,
                                    Theme::current(ui.ctx()).accent,
                                );
                            } else {
                                ui.painter().rect_stroke(
                                    rect.shrink(1.0),
                                    2.0,
                                    stroke,
                                    egui::StrokeKind::Inside,
                                );
                            }
                            if resp.on_hover_text("Pivot for scale and rotate").clicked()
                                && !on
                                && let Ok(n) = t.reanchored(Vec2::new(ax, ay), size)
                            {
                                set(f, None, n);
                            }
                        }
                    });
                }
            });
        });
        ui.horizontal(|ui| {
            label(ui, "Snap", false);
            super::toolbar::snap_menu(Some(ws), ui, f.actions);
        });
        ui.horizontal(|ui| {
            label(ui, "Flip", flips.0 || flips.1);
            for (name, fl) in [
                ("Horizontal", (!flips.0, flips.1)),
                ("Vertical", (flips.0, !flips.1)),
            ] {
                let on = if name == "Horizontal" {
                    flips.0
                } else {
                    flips.1
                };
                if ui.selectable_label(on, name).clicked()
                    && let Some(n) = rebuilt(&t, p, sc, fl)
                {
                    set(f, None, n);
                }
            }
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
                    "png2c64 exact",
                    "Always sample the original with png2c64's four taps, as png2c64 \
                     does: large shrinks alias.",
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
            label(ui, "Presets", false);
            ui.menu_button("Fit…", |ui| {
                let (sw, sh) = (320.0, 200.0);
                let centred = |s: f32| Vec2::new((sw - w * s) / 2.0, (sh - h * s) / 2.0);
                let presets: [(&str, f32); 7] = [
                    ("Fit width", sw / w),
                    ("Fit height", sh / h),
                    ("Fit screen", (sw / w).min(sh / h)),
                    ("Fill screen", (sw / w).max(sh / h)),
                    ("1 source px = 1 C64 px", 1.0),
                    ("2 source px = 1 C64 px", 0.5),
                    ("100 %", 1.0),
                ];
                for (name, s) in presets {
                    if ui.button(name).clicked() {
                        if let Some(n) = rebuilt(&t, centred(s), Vec2::new(s, s), flips) {
                            set(f, None, n);
                        }
                        ui.close();
                    }
                }
            });
        });
        false
    });
}

/// The Mask group (UX §6 item 3): modifiers, the sources list and Edit mask; a layer
/// without a mask shows one button, Add mask.
fn mask_group(f: &mut Form, shell: &mut Shell, ws: &Workspace, layer: &Layer, ui: &mut Ui) {
    use convert64_core::command::ModifierChange;
    use convert64_core::mask::{MaskSource, Modifiers, SnapToCells};
    let Some(mask) = layer.mask() else {
        let _ = group(ui, "Mask", false, true, |ui| {
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
    let reset = group(ui, "Mask", changed, true, |ui| {
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

/// A dither method's name for menus, e.g. `Floyd–Steinberg`, `Bayer 4×4`.
fn method_name(kind: DitherKind) -> String {
    let id = serde_json::to_value(kind)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default();
    match id.as_str() {
        "floyd-steinberg" => "Floyd–Steinberg".to_owned(),
        "jarvis-judice-ninke" => "Jarvis–Judice–Ninke".to_owned(),
        _ => {
            let mut c = id.chars();
            c.next()
                .map_or_else(String::new, |f| {
                    f.to_uppercase().chain(c).collect::<String>()
                })
                .replace('-', " ")
        }
    }
}

const FAMILIES: [(DitherFamily, &str); 6] = [
    (DitherFamily::None, "None"),
    (DitherFamily::Ordered, "Ordered"),
    (DitherFamily::Diffusion, "Diffusion"),
    (DitherFamily::Noise, "Noise"),
    (DitherFamily::Effects, "Effects"),
    (DitherFamily::PaletteAware, "Palette-aware"),
];

fn dither(f: &mut Form, r: &Recipe, ui: &mut Ui) {
    let d = Recipe::default();
    let changed = r.method() != d.method()
        || r.strength() != d.strength()
        || r.phase() != d.phase()
        || r.error_clamp() != d.error_clamp();
    let reset = group(ui, "Dither recipe", changed, true, |ui| {
        let kind = r.method().kind;
        let family = kind.family();
        // Family first (Ditherista's lesson), then the method within it.
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for (fam, name) in FAMILIES {
                if ui.selectable_label(family == fam, name).clicked() && family != fam {
                    let first = DitherKind::ALL
                        .into_iter()
                        .find(|k| k.family() == fam)
                        .unwrap_or(DitherKind::None);
                    f.recipe(
                        None,
                        RecipeChange {
                            method: Some(Method {
                                kind: first,
                                ..r.method()
                            }),
                            ..Default::default()
                        },
                    );
                }
            }
        });
        ui.horizontal(|ui| {
            label(ui, "Method", r.method() != d.method());
            egui::ComboBox::from_id_salt("dither_method")
                .selected_text(method_name(kind))
                .width(ui.available_width())
                .show_ui(ui, |ui| {
                    for k in DitherKind::ALL.into_iter().filter(|k| k.family() == family) {
                        if ui.selectable_label(k == kind, method_name(k)).clicked() && k != kind {
                            f.recipe(
                                None,
                                RecipeChange {
                                    method: Some(Method {
                                        kind: k,
                                        ..r.method()
                                    }),
                                    ..Default::default()
                                },
                            );
                        }
                    }
                });
        });
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
            for (phase, name) in [
                (PatternPhase::Screen, "Screen-locked"),
                (PatternPhase::Layer, "Layer-locked"),
            ] {
                if ui.selectable_label(r.phase() == phase, name).clicked() && r.phase() != phase {
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
    let reset = group(ui, "Colour fitting", changed, true, |ui| {
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
        "png2amiga's C64 path (ADR-013), the default",
    ),
    (
        SolverKind::Png2c64,
        "png2c64 (old reference)",
        "png2c64's pipeline, the reference before ADR-013; kept for comparison",
    ),
    (SolverKind::Native, "Native", "Convert64's own solver"),
];

/// The Refine checkbox's tooltip.
const REFINE_TIP: &str = "png2amiga's cell refine: keeps the result only if SSIMULACRA2 improves";

fn advanced(f: &mut Form, layer: &Layer, s: &SolveSettings, ui: &mut Ui) {
    let d = SolveSettings::default();
    let changed = *s != d;
    let reset = group(ui, "Advanced", changed, false, |ui| {
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
        ui.horizontal(|ui| {
            let lab = label_tip(ui, "Refine", s.refine != d.refine, Some(REFINE_TIP));
            let mut v = s.refine;
            ui.checkbox(&mut v, "").on_hover_text(REFINE_TIP);
            if lab.double_clicked() && s.refine != d.refine {
                v = d.refine;
            }
            if v != s.refine {
                f.solve(
                    None,
                    SolveSet {
                        refine: Some(v),
                        ..Default::default()
                    },
                );
            }
        });
        ui.label(RichText::new("Divergences of the png2c64 solver (ADR-010): all off by default; turning one on solves with png2c64's port.").small().weak());
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
        fn collect(shape: &egui::Shape, out: &mut Vec<(String, egui::Rect)>) {
            match shape {
                egui::Shape::Text(t) => {
                    out.push((t.galley.text().to_owned(), t.visual_bounding_rect()));
                }
                egui::Shape::Vec(v) => v.iter().for_each(|s| collect(s, out)),
                _ => {}
            }
        }
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(WINDOW, 3000.0),
            )),
            ..Default::default()
        };
        let mut shell = Shell {
            tab: super::super::DockTab::Recipe,
            ..Shell::default()
        };
        let mut actions = Vec::new();
        let mut output = ctx.run_ui(input, |ui| actions = shell.show(ui, Some(&mut ws)));
        output.textures_delta.clear();
        let mut texts = Vec::new();
        for clipped in &output.shapes {
            collect(&clipped.shape, &mut texts);
        }
        (texts, actions)
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

    /// A box (png2c64's, from an older project) has no amount: a note instead.
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
        assert!(texts.iter().any(|(t, _)| t.starts_with("png2c64's box")));
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
