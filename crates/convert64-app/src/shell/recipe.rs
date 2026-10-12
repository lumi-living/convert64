//! The Recipe tab (UX §6): the selected layer's adjustments, dither recipe and colour
//! fitting, then **Advanced**: ADR-010's divergences from png2c64 and the solver's own
//! settings, collapsed and off by default. Every change is a registry command
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
    Layer, Levels, Method, Metric, Orientation, PatternPhase, RangeMatch, Recipe,
};
use convert64_core::solve::{Divergences, SolverKind};
use convert64_project::{JsonCommand, LayerRef};
use egui::{Response, RichText, Ui};

/// The width of a row's label column, in points.
const LABEL_WIDTH: f32 = 84.0;

/// Emits commands for one layer.
struct Form<'a> {
    actions: &'a mut Vec<Action>,
    layer: usize,
}

impl Form<'_> {
    fn emit(&mut self, gesture: Option<&str>, command: JsonCommand) {
        self.actions.push(match gesture {
            Some(name) => Action::Gesture(egui::Id::new(name).value(), command),
            None => Action::Run(command),
        });
    }

    fn recipe(&mut self, gesture: Option<&str>, recipe: RecipeChange) {
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
    .on_hover_text("Double-click to reset")
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
    let mut v = value;
    let mut edit = Edit::None;
    let mut stopped = false;
    ui.horizontal(|ui| {
        let lab = label(ui, name, (value - default).abs() > 1e-6);
        let value_box = 3.0 * ui.spacing().interact_size.x / 2.0;
        ui.spacing_mut().slider_width =
            (ui.available_width() - value_box).max(ui.spacing().interact_size.x);
        let r = ui.add(
            egui::Slider::new(&mut v, range)
                .suffix(suffix)
                .fixed_decimals(decimals),
        );
        if lab.double_clicked() {
            edit = Edit::Set(default);
        } else if r.changed() {
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
    let mut f = Form { actions, layer: i };
    ui.label(
        RichText::new(format!("Layer: {}", layer.name()))
            .small()
            .weak(),
    );

    adjustments(&mut f, shell, ws, layer, ui);
    dither(&mut f, layer.recipe(), ui);
    fitting(&mut f, layer.recipe(), ws, ui);
    advanced(&mut f, layer, &project.solve, ui);
}

fn adjustments(f: &mut Form, shell: &mut Shell, ws: &Workspace, layer: &Layer, ui: &mut Ui) {
    let a = layer.adjustments();
    let d = Adjustments::default();
    let changed = *a != d;
    let reset =
        group(ui, "Adjustments", changed, true, |ui| {
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
            ui.horizontal(|ui| {
                let on = a.match_range.is_some();
                let lab = label(ui, "Range", on);
                let mut v = on;
                let r = ui.checkbox(&mut v, "Match the palette's range").on_hover_text(
                "png2c64's --match-range: the picture's OKLab range stretched to the palette's",
            );
                if r.changed() || (lab.double_clicked() && on) {
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
            reset_button(ui, changed)
        });
    if reset {
        f.adjust(
            None,
            AdjustmentsChange {
                gamma: Some(d.gamma),
                levels: Some(d.levels),
                brightness_contrast: Some(d.brightness_contrast),
                hsl: Some(d.hsl),
                curves: Some(d.curves.clone()),
                match_range: Some(None),
            },
        );
    }
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
        let hit = response.interact_pointer_pos().and_then(near);
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

fn advanced(f: &mut Form, layer: &Layer, s: &SolveSettings, ui: &mut Ui) {
    let d = SolveSettings::default();
    let changed = *s != d;
    let reset = group(ui, "Advanced", changed, false, |ui| {
        ui.label(RichText::new("Divergences from png2c64 (ADR-010): all off by default, so the result matches png2c64's.").small().weak());
        ui.horizontal(|ui| {
            label(ui, "Solver", s.solver != d.solver);
            for (k, n) in [
                (SolverKind::Reference, "Reference"),
                (SolverKind::Native, "Native"),
            ] {
                if ui.selectable_label(s.solver == k, n).clicked() && s.solver != k {
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
