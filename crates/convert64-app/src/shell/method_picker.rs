//! The dither method picker (UX §6a; Figma `Method Field`, `Method Picker` and
//! `Method Card`, to come): the Method row of a layer's or a region's recipe, and the
//! popover it opens.
//!
//! - **Field**: a 32 px thumbnail of the current method, its name and ▾, with ▲▼ step
//!   buttons. A click or `Enter` opens the picker; with the field focused, `↑`/`↓` step
//!   through the methods in picker order (each step an edit, so the result follows).
//! - **Picker**: a popover anchored to the dock's left edge, over the working view, 4
//!   columns: the search field (always focused, so typing anywhere searches), *Preview
//!   on: Gradient · Picture*, Recent, then the families in UX order. Cards show real
//!   solves ([`crate::thumbs`]), a neutral placeholder until theirs arrives.
//! - **Live preview**: resting on a card, or arrowing to it, for 150 ms previews that
//!   method on the whole result ([`Action::PreviewMethod`]); moving off reverts; a click
//!   or `Enter` commits as one undo step and closes; `Esc` or a click outside reverts
//!   and closes.

use super::{Action, Shell, icon};
use crate::icons;
use crate::theme::{Theme, layout};
use crate::thumbs::{PreviewSource, ThumbJob};
use crate::workspace::Workspace;
use convert64_core::layer::{DitherKind, Method, Recipe};
use convert64_core::preview::catalogue::{self, FAMILIES, MethodInfo};
use egui::{Rect, RichText, Ui, pos2, vec2};

/// Whose recipe the picker edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecipeTarget {
    /// Layer `i` (stack index, bottom first).
    Layer(usize),
    /// Region `i`.
    Region(usize),
}

/// How long a card must be rested on (or arrowed to) before it previews (UX §6a).
pub const PREVIEW_DELAY: f64 = 0.15;

/// Cards per row.
pub const COLUMNS: usize = 4;

/// A card's size: a 64 px thumbnail and the name beside it.
const CARD: egui::Vec2 = vec2(156.0, 72.0);
/// The thumbnail on a card: 32 × 32 C64 pixels at 2×.
const CARD_THUMB: f32 = 64.0;
/// The thumbnail in the field.
const FIELD_THUMB: f32 = 32.0;
/// Space between cards.
const GAP: f32 = 8.0;
/// The popover's width: four cards, their gaps and the frame's margins.
pub const WIDTH: f32 = COLUMNS as f32 * CARD.x + (COLUMNS as f32 - 1.0) * GAP + 2.0 * 12.0;

/// The picker's state (in [`Shell`]).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PickerState {
    /// Open, on this recipe.
    pub open: Option<RecipeTarget>,
    /// The search text.
    pub query: String,
    /// The keyboard's card: row and column of the grid as drawn.
    focus: Option<(usize, usize)>,
    /// The last move was a key (the focused card previews) rather than the pointer
    /// (the hovered card does).
    keyboard: bool,
    /// The card the pointer or keyboard is on, and since when (egui seconds).
    resting: Option<(Option<DitherKind>, f64)>,
    /// The method previewing on the result now.
    previewing: Option<DitherKind>,
    /// What the thumbnails show (remembered across sessions by the app).
    pub source: PreviewSource,
    /// Opened this frame: focus the search, put the keyboard on the current method.
    just_opened: bool,
    /// The field's rectangle, where the popover hangs from.
    field: Option<Rect>,
    /// The picker was drawn this frame (the Recipe tab is showing its recipe).
    pub(super) drawn: bool,
    /// Methods recently committed in this project (newest first), shown as Recent.
    pub recent: Vec<DitherKind>,
}

impl PickerState {
    /// Opens the picker on `target`.
    pub fn open(&mut self, target: RecipeTarget) {
        self.open = Some(target);
        self.query.clear();
        self.focus = None;
        self.keyboard = false;
        self.resting = None;
        self.previewing = None;
        self.just_opened = true;
    }

    /// Closes it; a preview showing is reverted.
    pub fn close(&mut self, actions: &mut Vec<Action>) {
        if self.previewing.take().is_some() {
            actions.push(Action::PreviewMethod(None));
        }
        self.open = None;
        self.resting = None;
    }

    /// The method previewing on the result, if any.
    pub fn previewing(&self) -> Option<DitherKind> {
        self.previewing
    }

    /// Notes a committed method for the Recent row: the last four, newest first.
    pub fn note_recent(&mut self, kind: DitherKind) {
        self.recent.retain(|k| *k != kind);
        self.recent.insert(0, kind);
        self.recent.truncate(4);
    }
}

/// The picker's grid as drawn: rows of cards, the Recent row first, then each family's
/// cards in rows of [`COLUMNS`]. Sections start new rows.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Grid {
    /// Section headings with the rows they own: `(heading, first row, row count)`.
    pub sections: Vec<(&'static str, usize, usize)>,
    /// The cards, row by row.
    pub rows: Vec<Vec<DitherKind>>,
}

impl Grid {
    /// The grid for `query` with `recent` methods (Recent shows only without a query).
    pub fn new(query: &str, recent: &[DitherKind]) -> Self {
        let mut g = Self::default();
        let found = catalogue::search(query);
        if query.trim().is_empty() && !recent.is_empty() {
            g.section("Recent", recent.to_vec());
        }
        for (family, name) in FAMILIES {
            let kinds: Vec<DitherKind> = found
                .iter()
                .copied()
                .filter(|k| k.family() == family)
                .collect();
            if !kinds.is_empty() {
                g.section(name, kinds);
            }
        }
        g
    }

    fn section(&mut self, name: &'static str, kinds: Vec<DitherKind>) {
        let first = self.rows.len();
        for chunk in kinds.chunks(COLUMNS) {
            self.rows.push(chunk.to_vec());
        }
        self.sections.push((name, first, self.rows.len() - first));
    }

    /// Whether no card is shown.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// The card at `(row, column)`.
    pub fn at(&self, (r, c): (usize, usize)) -> Option<DitherKind> {
        self.rows.get(r)?.get(c).copied()
    }

    /// Where `kind` is, outside Recent if it is in a family too.
    pub fn find(&self, kind: DitherKind) -> Option<(usize, usize)> {
        let recent = match self.sections.first() {
            Some(("Recent", _, n)) => *n,
            _ => 0,
        };
        let pos = |skip: usize| {
            self.rows
                .iter()
                .enumerate()
                .skip(skip)
                .find_map(|(r, row)| row.iter().position(|k| *k == kind).map(|c| (r, c)))
        };
        pos(recent).or_else(|| pos(0))
    }

    /// The card a key moves to from `from` (none: the first card): arrows move across
    /// the grid, keeping the column where the next row is long enough; `Home` and `End`
    /// jump to the first and last card.
    pub fn step(&self, from: Option<(usize, usize)>, key: egui::Key) -> Option<(usize, usize)> {
        use egui::Key;
        if self.rows.is_empty() {
            return None;
        }
        let last = (
            self.rows.len() - 1,
            self.rows[self.rows.len() - 1].len() - 1,
        );
        let Some((r, c)) = from else {
            return Some(if key == Key::End { last } else { (0, 0) });
        };
        Some(match key {
            Key::Home => (0, 0),
            Key::End => last,
            Key::ArrowRight if c + 1 < self.rows[r].len() => (r, c + 1),
            Key::ArrowRight if r + 1 < self.rows.len() => (r + 1, 0),
            Key::ArrowLeft if c > 0 => (r, c - 1),
            Key::ArrowLeft if r > 0 => (r - 1, self.rows[r - 1].len() - 1),
            Key::ArrowDown if r + 1 < self.rows.len() => (r + 1, c.min(self.rows[r + 1].len() - 1)),
            Key::ArrowUp if r > 0 => (r - 1, c.min(self.rows[r - 1].len() - 1)),
            _ => (r, c),
        })
    }
}

/// Draws a thumbnail of `method` (or the placeholder) into `rect`, asking for it to be
/// rendered when missing.
fn thumb(
    ui: &Ui,
    ws: &Workspace,
    method: Method,
    recipe: &Recipe,
    rect: Rect,
    jobs: &mut Vec<ThumbJob>,
) {
    let thumbs = &ws.method_thumbs;
    let tex = thumbs.key(method, recipe).and_then(|k| thumbs.texture(&k));
    match tex {
        Some(tex) => {
            let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
            ui.painter().image(tex.id(), rect, uv, egui::Color32::WHITE);
        }
        None => {
            // Rendering: a neutral placeholder, never a spinner (UX §6a).
            ui.painter()
                .rect_filled(rect, 2.0, ui.visuals().widgets.noninteractive.bg_fill);
            if let Some(job) = thumbs.job(method, recipe) {
                jobs.push(job);
            }
        }
    }
}

/// A method's name laid out in `width`, on at most `rows` rows, elided with `…`.
fn name_galley(
    ui: &Ui,
    name: &str,
    colour: egui::Color32,
    width: f32,
    rows: usize,
) -> std::sync::Arc<egui::Galley> {
    let font = egui::TextStyle::Body.resolve(ui.style());
    // egui breaks lines at spaces only: a long name joined by an en dash
    // (Floyd–Steinberg) breaks after the dash instead of mid-word.
    let fits = || {
        ui.painter()
            .layout_no_wrap(name.to_owned(), font.clone(), colour)
            .size()
            .x
            <= width
    };
    let text = if rows > 1 && name.contains('–') && !fits() {
        name.replacen('–', "–\n", 1)
    } else {
        name.to_owned()
    };
    let mut job = egui::text::LayoutJob::simple(text, font, colour, width);
    job.wrap.max_rows = rows;
    job.wrap.break_anywhere = rows == 1;
    ui.painter().layout_job(job)
}

/// What the field did this frame.
#[derive(Debug, Default)]
pub(super) struct FieldOutcome {
    /// ▲▼ or `↑`/`↓`: this method instead (one edit).
    pub step: Option<Method>,
    /// Clicked or `Enter`: open the picker, or close it if open.
    pub toggle: bool,
}

/// The Method field and its ▲▼ (UX §6a), without its label.
pub(super) fn field(
    shell: &mut Shell,
    ws: &Workspace,
    recipe: &Recipe,
    ui: &mut Ui,
    jobs: &mut Vec<ThumbJob>,
) -> FieldOutcome {
    let mut out = FieldOutcome::default();
    let kind = recipe.method().kind;
    let info = catalogue::info(kind);
    let open_here = shell.picker.open.is_some();
    let mut step = None;
    let height = FIELD_THUMB + 8.0;
    ui.horizontal(|ui| {
        let steppers = 2.0 * (layout::ICON_SIZE + 6.0) + ui.spacing().item_spacing.x;
        let width = (ui.available_width() - steppers).max(80.0);
        let (rect, response) = ui.allocate_exact_size(vec2(width, height), egui::Sense::click());
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::ComboBox,
                true,
                format!("Dither method: {}", info.name),
            )
        });
        let visuals = ui.style().interact_selectable(&response, open_here);
        ui.painter().rect(
            rect,
            visuals.corner_radius,
            visuals.weak_bg_fill,
            visuals.bg_stroke,
            egui::StrokeKind::Inside,
        );
        let t = Rect::from_min_size(rect.min + vec2(4.0, 4.0), vec2(FIELD_THUMB, FIELD_THUMB));
        thumb(ui, ws, recipe.method(), recipe, t, jobs);
        let text_x = t.right() + 8.0;
        let arrow = ui.painter().text(
            pos2(rect.right() - 4.0, rect.center().y),
            egui::Align2::RIGHT_CENTER,
            icons::DROP_DOWN.codepoint,
            egui::FontId::new(layout::ICON_SIZE, icons::DROP_DOWN.font_family()),
            visuals.text_color(),
        );
        let galley = name_galley(
            ui,
            info.name,
            visuals.text_color(),
            (arrow.left() - text_x - 4.0).max(10.0),
            1,
        );
        let y = rect.center().y - galley.size().y / 2.0;
        ui.painter()
            .galley(pos2(text_x, y), galley, visuals.text_color());
        let response = response.on_hover_text(format!(
            "{}\nCLI: --dither {}\nClick or Enter: all methods · ↑↓: step",
            info.description,
            info.id()
        ));
        if response.clicked()
            || (response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
        {
            out.toggle = true;
        }
        if response.has_focus() {
            ui.input_mut(|i| {
                if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {
                    step = Some(-1);
                }
                if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) {
                    step = Some(1);
                }
            });
        }
        shell.picker.field = Some(rect);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let up = ui.add(egui::Button::new(icon(icons::STEP_UP)).frame(false).small());
            if up.on_hover_text("Previous method (↑)").clicked() {
                step = Some(-1);
            }
            let down = ui.add(
                egui::Button::new(icon(icons::STEP_DOWN))
                    .frame(false)
                    .small(),
            );
            if down.on_hover_text("Next method (↓)").clicked() {
                step = Some(1);
            }
        });
    });
    if let Some(s) = step {
        let next = catalogue::step(kind, s);
        if next != kind {
            shell.picker.note_recent(next);
            out.step = Some(Method {
                kind: next,
                ..recipe.method()
            });
        }
    }
    out
}

/// The popover, if open on `target` (call right after [`field`]). Returns the method
/// to commit (one undo step), if one was picked; the picker has closed then.
pub(super) fn popover(
    shell: &mut Shell,
    ws: &Workspace,
    target: RecipeTarget,
    recipe: &Recipe,
    ctx: &egui::Context,
    actions: &mut Vec<Action>,
    jobs: &mut Vec<ThumbJob>,
) -> Option<Method> {
    if shell.picker.open != Some(target) {
        return None;
    }
    shell.picker.drawn = true;
    let p = &mut shell.picker;
    let current = recipe.method().kind;
    let grid = Grid::new(&p.query, &p.recent);
    let now = ctx.input(|i| i.time);
    // On open the keyboard starts on the current method, scrolled into view.
    let opened = std::mem::take(&mut p.just_opened);
    if opened {
        p.focus = grid.find(current);
    }
    // Keys first, so the search field never sees the arrows.
    let mut commit = None;
    let mut cancel = false;
    ctx.input_mut(|i| {
        for key in [
            egui::Key::ArrowUp,
            egui::Key::ArrowDown,
            egui::Key::ArrowLeft,
            egui::Key::ArrowRight,
            egui::Key::Home,
            egui::Key::End,
        ] {
            if i.consume_key(egui::Modifiers::NONE, key) {
                p.focus = grid.step(p.focus, key);
                p.keyboard = true;
            }
        }
        if i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
            && let Some(k) = p.focus.and_then(|f| grid.at(f))
        {
            commit = Some(k);
        }
        if i.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
            cancel = true;
        }
        if i.pointer.delta() != egui::Vec2::ZERO {
            p.keyboard = false;
        }
    });
    let screen = ctx.content_rect();
    let field = p
        .field
        .unwrap_or(Rect::from_min_size(screen.right_top(), vec2(0.0, 0.0)));
    let dock_left = shell.dock_left.unwrap_or(field.left());
    let max_h = (screen.height() - 120.0).max(200.0);
    let top = field
        .top()
        .min(screen.bottom() - max_h - 40.0)
        .max(screen.top() + 40.0);
    let pos = pos2(dock_left - WIDTH - 4.0, top);
    let theme = Theme::current(ctx);
    let mut hovered: Option<DitherKind> = None;
    let mut clicked: Option<DitherKind> = None;
    let mut query_changed = false;
    let p = &mut shell.picker;
    let area = egui::Area::new(egui::Id::new("method_picker"))
        .order(egui::Order::Foreground)
        .fixed_pos(pos)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style())
                .inner_margin(12.0)
                .show(ui, |ui| {
                    ui.set_width(WIDTH - 24.0);
                    ui.horizontal(|ui| {
                        ui.label(icon(icons::SEARCH).weak());
                        let search = ui.add(
                            egui::TextEdit::singleline(&mut p.query)
                                .hint_text("Search: name, id or alias (fs, checker, png2c64 line2)")
                                .desired_width(f32::INFINITY),
                        );
                        // Typing anywhere in the picker types here.
                        if !search.has_focus() {
                            search.request_focus();
                        }
                        query_changed = search.changed();
                    });
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Preview on:").small());
                        for (s, name) in [
                            (PreviewSource::Gradient, "Gradient"),
                            (PreviewSource::Picture, "Picture"),
                        ] {
                            let r = ui.selectable_label(p.source == s, name);
                            let r = r.on_hover_text(match s {
                                PreviewSource::Gradient => {
                                    "A diagonal ramp through the palette's greys and one hue"
                                }
                                PreviewSource::Picture => {
                                    "The picture at the last cell the cursor was over"
                                }
                            });
                            if r.clicked() {
                                p.source = s;
                            }
                        }
                    });
                    ui.add_space(4.0);
                    egui::ScrollArea::vertical()
                        .max_height(max_h - 90.0)
                        .min_scrolled_height(max_h - 90.0)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            if grid.is_empty() {
                                ui.label(
                                    RichText::new(format!(
                                        "No method matches “{}”.",
                                        p.query.trim()
                                    ))
                                    .weak(),
                                );
                            }
                            let mut visible = Vec::new();
                            let mut hidden = Vec::new();
                            for &(heading, first, count) in &grid.sections {
                                ui.label(
                                    RichText::new(heading.to_uppercase())
                                        .small()
                                        .color(ui.visuals().weak_text_color()),
                                );
                                for r in first..first + count {
                                    ui.horizontal(|ui| {
                                        ui.spacing_mut().item_spacing.x = GAP;
                                        for (c, &kind) in grid.rows[r].iter().enumerate() {
                                            let focused = p.focus == Some((r, c));
                                            let resp = card(
                                                ui,
                                                ws,
                                                catalogue::info(kind),
                                                recipe,
                                                kind == current,
                                                focused && p.keyboard,
                                                theme,
                                                (&mut visible, &mut hidden),
                                            );
                                            if focused && (p.keyboard || opened) {
                                                resp.scroll_to_me(
                                                    opened.then_some(egui::Align::Center),
                                                );
                                            }
                                            if resp.hovered() {
                                                hovered = Some(kind);
                                            }
                                            if resp.clicked() {
                                                clicked = Some(kind);
                                            }
                                        }
                                    });
                                }
                                ui.add_space(4.0);
                            }
                            // Visible cards first, the rest after.
                            jobs.extend(visible);
                            jobs.extend(hidden);
                        });
                });
        });
    let p = &mut shell.picker;
    if query_changed {
        p.focus = if p.query.trim().is_empty() {
            Grid::new("", &p.recent).find(current)
        } else {
            Grid::new(&p.query, &p.recent).at((0, 0)).map(|_| (0, 0))
        };
    }
    // A press outside the popover and the field reverts and closes.
    let outside = ctx.input(|i| {
        i.pointer.any_pressed()
            && i.pointer.interact_pos().is_some_and(|pos| {
                !area.response.rect.contains(pos) && !p.field.is_some_and(|f| f.contains(pos))
            })
    });
    if cancel || outside {
        p.close(actions);
        return None;
    }
    if let Some(k) = clicked.or(commit) {
        // The preview (if any) becomes the edit: the app keeps the result on screen
        // and solves in full.
        p.previewing = None;
        p.open = None;
        p.note_recent(k);
        return Some(Method {
            kind: k,
            ..recipe.method()
        });
    }
    // Live preview after resting 150 ms on a card (or arrowing to it).
    let on = if p.keyboard {
        p.focus.and_then(|f| grid.at(f))
    } else {
        hovered
    };
    if p.resting.is_none_or(|(k, _)| k != on) {
        p.resting = Some((on, now));
    }
    if let Some((k, since)) = p.resting {
        let wanted = k.filter(|k| *k != current);
        if wanted != p.previewing {
            if now - since >= PREVIEW_DELAY {
                p.previewing = wanted;
                actions.push(Action::PreviewMethod(wanted.map(|kind| {
                    (
                        target,
                        Method {
                            kind,
                            ..recipe.method()
                        },
                    )
                })));
            } else {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(
                    PREVIEW_DELAY - (now - since),
                ));
            }
        }
    }
    None
}

/// One card (Figma `Method Card`): thumbnail, name, *slow* badge; hover, selected and
/// focused states; tooltip with the description and CLI id; labelled for screen readers.
#[allow(clippy::too_many_arguments)]
fn card(
    ui: &mut Ui,
    ws: &Workspace,
    info: &MethodInfo,
    recipe: &Recipe,
    selected: bool,
    focused: bool,
    theme: &Theme,
    (visible, hidden): (&mut Vec<ThumbJob>, &mut Vec<ThumbJob>),
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(CARD, egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, info.label(selected))
    });
    if ui.is_rect_visible(rect) {
        let v = ui.visuals();
        let (fill, stroke) = if selected {
            (
                theme.accent.gamma_multiply(0.18),
                egui::Stroke::new(1.5, theme.accent),
            )
        } else if response.hovered() {
            (v.widgets.hovered.weak_bg_fill, egui::Stroke::NONE)
        } else {
            (egui::Color32::TRANSPARENT, egui::Stroke::NONE)
        };
        ui.painter()
            .rect(rect, 4.0, fill, stroke, egui::StrokeKind::Inside);
        if focused {
            ui.painter().rect_stroke(
                rect.expand(1.0),
                5.0,
                v.selection.stroke,
                egui::StrokeKind::Outside,
            );
        }
        let t = Rect::from_min_size(
            rect.min + vec2(4.0, (rect.height() - CARD_THUMB) / 2.0),
            vec2(CARD_THUMB, CARD_THUMB),
        );
        let method = Method {
            kind: info.kind,
            ..recipe.method()
        };
        thumb(ui, ws, method, recipe, t, visible);
        let x = t.right() + 8.0;
        let galley = name_galley(ui, info.name, v.text_color(), rect.right() - x - 4.0, 2);
        let mut y = rect.top() + 10.0;
        ui.painter()
            .galley(pos2(x, y), galley.clone(), v.text_color());
        y += galley.size().y + 2.0;
        if info.slow {
            ui.painter().text(
                pos2(x, y),
                egui::Align2::LEFT_TOP,
                "slow",
                egui::TextStyle::Small.resolve(ui.style()),
                theme.warn,
            );
        }
    } else {
        // Off screen: still rendered, after the visible cards.
        let method = Method {
            kind: info.kind,
            ..recipe.method()
        };
        if let Some(job) = ws.method_thumbs.job(method, recipe) {
            hidden.push(job);
        }
    }
    response.on_hover_text(format!("{}\nCLI: --dither {}", info.description, info.id()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::Key;

    #[test]
    fn the_grid_has_recent_then_families_in_rows_of_four() {
        let g = Grid::new("", &[DitherKind::Atkinson, DitherKind::Checker]);
        assert_eq!(g.sections[0], ("Recent", 0, 1));
        assert_eq!(g.rows[0], vec![DitherKind::Atkinson, DitherKind::Checker]);
        assert_eq!(g.sections[1].0, "None");
        assert_eq!(g.sections[2].0, "Ordered");
        assert!(g.rows.iter().all(|r| !r.is_empty() && r.len() <= COLUMNS));
        let names: Vec<&str> = g.sections.iter().map(|s| s.0).collect();
        assert_eq!(
            names,
            [
                "Recent",
                "None",
                "Ordered",
                "Ordered noise",
                "Error diffusion",
                "Palette-aware",
                "Effects"
            ]
        );
        // A search hides Recent and empty families.
        let s = Grid::new("atkinson", &[DitherKind::Atkinson]);
        assert_eq!(s.sections, vec![("Error diffusion", 0, 1)]);
        assert!(Grid::new("zzz", &[]).is_empty());
    }

    #[test]
    fn find_prefers_the_family_over_recent() {
        let g = Grid::new("", &[DitherKind::Atkinson]);
        let (r, _) = g.find(DitherKind::Atkinson).unwrap();
        assert!(r > 0);
        assert_eq!(
            g.at(g.find(DitherKind::Atkinson).unwrap()),
            Some(DitherKind::Atkinson)
        );
    }

    #[test]
    fn arrows_move_across_the_grid_and_home_end_jump() {
        let g = Grid::new("", &[]);
        // None is alone in its row; Ordered starts the next.
        assert_eq!(g.step(None, Key::ArrowDown), Some((0, 0)));
        assert_eq!(g.step(Some((0, 0)), Key::ArrowRight), Some((1, 0)));
        assert_eq!(g.step(Some((1, 0)), Key::ArrowLeft), Some((0, 0)));
        assert_eq!(
            g.step(Some((1, 2)), Key::ArrowUp),
            Some((0, 0)),
            "clamped to the row"
        );
        assert_eq!(g.step(Some((1, 2)), Key::ArrowDown), Some((2, 2)));
        assert_eq!(g.step(Some((0, 0)), Key::ArrowUp), Some((0, 0)));
        let end = g.step(Some((3, 1)), Key::End).unwrap();
        assert_eq!(g.at(end), Some(DitherKind::ValueNoise));
        assert_eq!(g.step(Some(end), Key::ArrowRight), Some(end));
        assert_eq!(g.step(Some(end), Key::Home), Some((0, 0)));
        // Every card is reachable by →.
        let mut at = Some((0, 0));
        let mut seen = 1;
        loop {
            let next = g.step(at, Key::ArrowRight);
            if next == at {
                break;
            }
            at = next;
            seen += 1;
        }
        assert_eq!(seen, catalogue::CATALOGUE.len());
    }

    /// Draws the picker on layer 0 for one frame at `time` with `keys` pressed; returns
    /// what it did.
    fn frame(
        ctx: &egui::Context,
        shell: &mut Shell,
        ws: &Workspace,
        time: f64,
        keys: &[Key],
    ) -> (Vec<Action>, Option<Method>) {
        let events = keys
            .iter()
            .map(|&key| egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            })
            .collect();
        let input = egui::RawInput {
            time: Some(time),
            events,
            ..Default::default()
        };
        let mut actions = Vec::new();
        let mut committed = None;
        let recipe = *ws.doc.session().project().layers[0].recipe();
        let mut output = ctx.run_ui(input, |ui| {
            let mut jobs = Vec::new();
            if let Some(m) = popover(
                shell,
                ws,
                RecipeTarget::Layer(0),
                &recipe,
                ui.ctx(),
                &mut actions,
                &mut jobs,
            ) {
                committed = Some(m);
            }
        });
        output.textures_delta.clear();
        (actions, committed)
    }

    fn workspace() -> Workspace {
        let rgba = (0..32 * 20)
            .flat_map(|i| [(i % 32 * 8) as u8, (i / 32 * 12) as u8, 90, 255])
            .collect();
        let png = convert64_project::codec::encode_rgba8(32, 20, rgba);
        let doc = crate::document::Document::from_png("g.png", &png).unwrap();
        Workspace::new(doc)
    }

    /// UX §6a: arrowing to a card previews it after 150 ms, `Esc` reverts and closes;
    /// `Enter` commits the focused method and closes, with no preview left to revert.
    #[test]
    fn arrows_preview_after_150_ms_esc_reverts_enter_commits() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let ws = workspace();
        let mut shell = Shell::default();
        shell.picker.open(RecipeTarget::Layer(0));
        // Opens on the current method (Floyd–Steinberg), no preview.
        let (a, _) = frame(&ctx, &mut shell, &ws, 0.0, &[]);
        assert!(a.is_empty());
        // → moves to False Floyd–Steinberg; nothing until it has rested 150 ms.
        let (a, _) = frame(&ctx, &mut shell, &ws, 0.01, &[Key::ArrowRight]);
        assert!(a.is_empty(), "{a:?}");
        let (a, _) = frame(&ctx, &mut shell, &ws, 0.1, &[]);
        assert!(a.is_empty(), "{a:?}");
        let (a, _) = frame(&ctx, &mut shell, &ws, 0.2, &[]);
        let preview = Method::new(DitherKind::FalseFs);
        assert_eq!(
            a,
            vec![Action::PreviewMethod(Some((
                RecipeTarget::Layer(0),
                preview
            )))]
        );
        assert_eq!(shell.picker.previewing(), Some(DitherKind::FalseFs));
        // Esc reverts and closes.
        let (a, c) = frame(&ctx, &mut shell, &ws, 0.3, &[Key::Escape]);
        assert_eq!(a, vec![Action::PreviewMethod(None)]);
        assert!(c.is_none() && shell.picker.open.is_none());

        // Again, and Enter commits what the keyboard is on.
        shell.picker.open(RecipeTarget::Layer(0));
        let _ = frame(&ctx, &mut shell, &ws, 1.0, &[]);
        let _ = frame(&ctx, &mut shell, &ws, 1.01, &[Key::ArrowRight]);
        let (a, _) = frame(&ctx, &mut shell, &ws, 1.2, &[]);
        assert_eq!(a.len(), 1);
        let (a, c) = frame(&ctx, &mut shell, &ws, 1.25, &[Key::Enter]);
        assert!(a.is_empty(), "the commit replaces the preview: {a:?}");
        assert_eq!(c, Some(preview));
        assert!(shell.picker.open.is_none() && shell.picker.previewing().is_none());
        assert_eq!(shell.picker.recent, vec![DitherKind::FalseFs]);
    }

    /// Arrowing back to the current method ends the preview (nothing to preview).
    #[test]
    fn back_on_the_current_method_reverts() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let ws = workspace();
        let mut shell = Shell::default();
        shell.picker.open(RecipeTarget::Layer(0));
        let _ = frame(&ctx, &mut shell, &ws, 0.0, &[]);
        let _ = frame(&ctx, &mut shell, &ws, 0.01, &[Key::ArrowRight]);
        let (a, _) = frame(&ctx, &mut shell, &ws, 0.2, &[]);
        assert_eq!(a.len(), 1);
        let _ = frame(&ctx, &mut shell, &ws, 0.21, &[Key::ArrowLeft]);
        let (a, _) = frame(&ctx, &mut shell, &ws, 0.4, &[]);
        assert_eq!(a, vec![Action::PreviewMethod(None)]);
    }

    #[test]
    fn recent_keeps_the_last_four_newest_first() {
        let mut p = PickerState::default();
        for k in [
            DitherKind::Atkinson,
            DitherKind::Checker,
            DitherKind::Bayer4,
            DitherKind::Atkinson,
            DitherKind::Luma,
            DitherKind::Knoll,
        ] {
            p.note_recent(k);
        }
        assert_eq!(
            p.recent,
            vec![
                DitherKind::Knoll,
                DitherKind::Luma,
                DitherKind::Atkinson,
                DitherKind::Bayer4
            ]
        );
    }
}
