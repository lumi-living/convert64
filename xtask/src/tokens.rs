//! `cargo xtask tokens [--check]`: generate the app theme from the Figma variables.
//!
//! Reads `design/tokens.json` (the Figma export, `design/README.md`) and writes
//! `crates/convert64-app/src/theme/generated.rs`. Every variable carries `rust`, the
//! path it maps to (`docs/figma.md`); that path decides where the value goes:
//!
//! | `rust` prefix | Generated |
//! |---|---|
//! | `visuals.` | a statement in `apply_light` / `apply_dark` (a single-mode variable goes in both) |
//! | `spacing.` | a statement in `apply_spacing` |
//! | `TextStyle::` | an entry of `text_styles()` |
//! | `FontFamily::` | the font name and its bundled file in `fonts` |
//! | `theme.` | a field of `Theme::LIGHT` / `Theme::DARK` |
//! | `layout::` | a `pub const` in `layout` |
//! | `C64Color::` | nothing: a test checks it against the core's Colodore palette |
//!
//! Each egui path is checked against the fields egui 0.36 has, with their types; an
//! unknown path, a wrong value type or a missing mode is an error, so a renamed or
//! misspelt Figma variable fails here and not as a silently ignored value.
//! `--check` writes nothing and fails if the committed file differs from what it would
//! write (CI runs it).

use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};

/// The token file, relative to the repository root.
pub(crate) const TOKENS: &str = "design/tokens.json";
/// The generated module, relative to the repository root.
const GENERATED: &str = "crates/convert64-app/src/theme/generated.rs";
/// Bundled fonts, relative to the repository root.
const FONTS_DIR: &str = "crates/convert64-app/assets/fonts";
/// `FONTS_DIR` as `include_bytes!` sees it from `GENERATED`.
const FONTS_FROM_GENERATED: &str = "../../assets/fonts";

/// The fields of the app's `Theme` struct (`crates/convert64-app/src/theme/mod.rs`), in
/// declaration order. Every one must have a `theme.<field>` variable.
const THEME_FIELDS: [&str; 14] = [
    "clash",
    "pin",
    "lock",
    "region",
    "nudge",
    "cell_grid",
    "char_grid",
    "canvas_bg",
    "checker_a",
    "checker_b",
    "ok",
    "warn",
    "error",
    "accent",
];

/// egui's text styles; `text_styles()` must define every one.
const TEXT_STYLES: [&str; 5] = ["Small", "Body", "Button", "Monospace", "Heading"];

/// `cargo xtask tokens`: regenerate the theme module, or with `check` only compare.
pub(crate) fn run(root: &Path, check: bool) -> Result<(), String> {
    let json = std::fs::read_to_string(root.join(TOKENS))
        .map_err(|e| format!("cannot read {TOKENS}: {e}"))?;
    let tokens = parse(&json)?;
    let code = rustfmt(
        root,
        &generate(&tokens, &|file| root.join(FONTS_DIR).join(file).is_file())?,
    )?;
    let path = root.join(GENERATED);
    let current = std::fs::read_to_string(&path).unwrap_or_default();
    if check {
        if current == code {
            println!("{GENERATED} is up to date with {TOKENS}");
            Ok(())
        } else {
            Err(format!(
                "{GENERATED} is stale: run `cargo xtask tokens` and commit the result"
            ))
        }
    } else {
        if current == code {
            println!("{GENERATED} unchanged");
        } else {
            std::fs::write(&path, code).map_err(|e| format!("cannot write {GENERATED}: {e}"))?;
            println!("wrote {GENERATED}");
        }
        Ok(())
    }
}

/// One exported Figma variable.
#[derive(Debug, Clone)]
pub(crate) struct Variable {
    /// Figma name, `group/name`.
    pub name: String,
    /// `COLOR`, `FLOAT` or `STRING`.
    pub ty: String,
    /// The Rust path it maps to.
    pub rust: String,
    /// Value per mode name; an alias is `{"alias", "aliasRust"}`.
    pub values: BTreeMap<String, Value>,
}

/// The parsed token file.
#[derive(Debug)]
pub(crate) struct Tokens {
    /// `exported` (a date).
    pub exported: String,
    /// `source` (the Figma file).
    pub source: String,
    /// Every variable of every collection, in file order.
    pub variables: Vec<Variable>,
}

/// Parses `design/tokens.json`.
pub(crate) fn parse(json: &str) -> Result<Tokens, String> {
    let root: Value = serde_json::from_str(json).map_err(|e| format!("{TOKENS}: {e}"))?;
    let text = |v: &Value, key: &str| -> Result<String, String> {
        v.get(key)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| format!("{TOKENS}: missing string `{key}`"))
    };
    let mut variables = Vec::new();
    let collections = root
        .get("collections")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{TOKENS}: missing `collections`"))?;
    for collection in collections {
        let cname = text(collection, "name")?;
        let vars = collection
            .get("variables")
            .and_then(Value::as_array)
            .ok_or_else(|| format!("{TOKENS}: collection `{cname}` has no `variables`"))?;
        for v in vars {
            let name = text(v, "name")?;
            let values = v
                .get("values")
                .and_then(Value::as_object)
                .ok_or_else(|| format!("{TOKENS}: `{cname}/{name}` has no `values`"))?
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            variables.push(Variable {
                ty: text(v, "type")?,
                rust: text(v, "rust")?,
                name,
                values,
            });
        }
    }
    Ok(Tokens {
        exported: text(&root, "exported")?,
        source: text(&root, "source")?,
        variables,
    })
}

/// How a leaf field of egui's style is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Leaf {
    /// `Color32`.
    Color,
    /// `Option<Color32>`, written as `Some(..)`.
    OptColor,
    /// `f32`.
    F32,
    /// `u8`.
    U8,
    /// `i8`.
    I8,
    /// `CornerRadius`, written as `CornerRadius::same(u8)`.
    Radius,
    /// `Margin`, written as `Margin::same(i8)`.
    Margin,
}

/// The type of `egui::Visuals` field `path` (without `visuals.`), or `None` if egui
/// 0.36 has no such field (or it is not a value a token can set).
fn visuals_leaf(path: &str) -> Option<Leaf> {
    let parts: Vec<&str> = path.split('.').collect();
    let stroke = |rest: &[&str]| match rest {
        ["color"] => Some(Leaf::Color),
        ["width"] => Some(Leaf::F32),
        _ => None,
    };
    let radius = |rest: &[&str]| match rest {
        [] => Some(Leaf::Radius),
        ["nw" | "ne" | "sw" | "se"] => Some(Leaf::U8),
        _ => None,
    };
    match parts.as_slice() {
        [
            "widgets",
            "noninteractive" | "inactive" | "hovered" | "active" | "open",
            rest @ ..,
        ] => match rest {
            ["bg_fill" | "weak_bg_fill"] => Some(Leaf::Color),
            ["expansion"] => Some(Leaf::F32),
            ["corner_radius", rest @ ..] => radius(rest),
            ["bg_stroke" | "fg_stroke", rest @ ..] => stroke(rest),
            _ => None,
        },
        ["selection", "bg_fill"] => Some(Leaf::Color),
        ["selection" | "text_cursor", "stroke", rest @ ..] | ["window_stroke", rest @ ..] => {
            stroke(rest)
        }
        ["text_cursor", "on_duration" | "off_duration"] => Some(Leaf::F32),
        ["window_shadow" | "popup_shadow", "color"] => Some(Leaf::Color),
        ["window_shadow" | "popup_shadow", "blur" | "spread"] => Some(Leaf::U8),
        ["window_corner_radius" | "menu_corner_radius", rest @ ..] => radius(rest),
        [
            "hyperlink_color" | "faint_bg_color" | "extreme_bg_color" | "code_bg_color"
            | "warn_fg_color" | "error_fg_color" | "window_fill" | "panel_fill",
        ] => Some(Leaf::Color),
        ["override_text_color" | "weak_text_color" | "text_edit_bg_color"] => Some(Leaf::OptColor),
        ["weak_text_alpha" | "disabled_alpha" | "resize_corner_size" | "clip_rect_margin"] => {
            Some(Leaf::F32)
        }
        _ => None,
    }
}

/// The type of `egui::style::Spacing` field `path` (without `spacing.`), or `None`.
fn spacing_leaf(path: &str) -> Option<Leaf> {
    let parts: Vec<&str> = path.split('.').collect();
    let margin = |rest: &[&str]| match rest {
        [] => Some(Leaf::Margin),
        ["left" | "right" | "top" | "bottom"] => Some(Leaf::I8),
        _ => None,
    };
    match parts.as_slice() {
        [
            "item_spacing" | "button_padding" | "interact_size" | "default_area_size",
            "x" | "y",
        ] => Some(Leaf::F32),
        ["window_margin" | "menu_margin", rest @ ..] | ["scroll", "content_margin", rest @ ..] => {
            margin(rest)
        }
        [
            "indent"
            | "slider_width"
            | "slider_rail_height"
            | "combo_width"
            | "text_edit_width"
            | "extra_text_line_spacing"
            | "icon_width"
            | "icon_width_inner"
            | "icon_spacing"
            | "tooltip_width"
            | "menu_width"
            | "menu_spacing"
            | "combo_height",
        ] => Some(Leaf::F32),
        [
            "scroll",
            "bar_width"
            | "handle_min_length"
            | "bar_inner_margin"
            | "bar_outer_margin"
            | "floating_width"
            | "floating_allocated_width"
            | "dormant_background_opacity"
            | "active_background_opacity"
            | "interact_background_opacity"
            | "dormant_handle_opacity"
            | "active_handle_opacity",
        ] => Some(Leaf::F32),
        _ => None,
    }
}

/// A Rust expression for a colour token: `rgb(0xrrggbb)` or `rgba(0xrrggbbaa)`
/// (unmultiplied alpha, as Figma stores it).
fn color_expr(hex: &str) -> Option<String> {
    let digits = hex.strip_prefix('#')?;
    if !digits.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let digits = digits.to_ascii_lowercase();
    match digits.len() {
        6 => Some(format!("rgb(0x{digits})")),
        8 if digits.ends_with("ff") => Some(format!("rgb(0x{})", &digits[..6])),
        8 => Some(format!("rgba(0x{digits})")),
        _ => None,
    }
}

/// An `f32` literal for `x` (always with a fractional part).
fn f32_lit(x: f64) -> String {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "egui's fields are f32; the tokens are written for them"
    )]
    let x = x as f32;
    format!("{x:?}")
}

/// An integer token in `lo..=hi`.
fn int_value(x: f64, lo: i64, hi: i64) -> Option<i64> {
    #[expect(clippy::cast_possible_truncation, reason = "checked to be integral")]
    let i = x as i64;
    (x.fract() == 0.0 && (lo..=hi).contains(&i)).then_some(i)
}

/// Generation context: all variables, for resolving aliases.
struct Ctx<'a> {
    vars: &'a [Variable],
}

impl Ctx<'_> {
    /// `var`'s value in `mode`: its own value for that mode, or its only value if it
    /// has just one mode; aliases resolved (by Rust path, in the same mode). Returns the
    /// value and, for an alias, the path it came from.
    fn value(&self, var: &Variable, mode: &str) -> Result<(Value, Option<String>), String> {
        self.value_depth(var, mode, 0)
    }

    fn value_depth(
        &self,
        var: &Variable,
        mode: &str,
        depth: usize,
    ) -> Result<(Value, Option<String>), String> {
        if depth > 8 {
            return Err(format!("`{}`: alias chain too deep", var.name));
        }
        let raw = match var.values.get(mode) {
            Some(v) => v,
            None if var.values.len() == 1 => var.values.values().next().expect("one value"),
            None => {
                return Err(format!(
                    "`{}` ({}) has no `{mode}` value (modes: {})",
                    var.name,
                    var.rust,
                    var.values.keys().cloned().collect::<Vec<_>>().join(", ")
                ));
            }
        };
        let Some(target) = raw.get("aliasRust").and_then(Value::as_str) else {
            return Ok((raw.clone(), None));
        };
        let aliased =
            self.vars.iter().find(|v| v.rust == target).ok_or_else(|| {
                format!("`{}` aliases `{target}`, which no variable has", var.name)
            })?;
        let (value, _) = self.value_depth(aliased, mode, depth + 1)?;
        Ok((value, Some(target.to_owned())))
    }

    /// The Rust expression for `var` in `mode` written to a field of type `leaf`.
    fn expr(&self, var: &Variable, mode: &str, leaf: Leaf) -> Result<String, String> {
        let (value, _) = self.value(var, mode)?;
        let wrong = |what: &str| {
            format!(
                "`{}` ({}): expected {what}, found {} `{value}`",
                var.name, var.rust, var.ty
            )
        };
        match leaf {
            Leaf::Color | Leaf::OptColor => {
                if var.ty != "COLOR" {
                    return Err(wrong("a COLOR"));
                }
                let c = value
                    .as_str()
                    .and_then(color_expr)
                    .ok_or_else(|| wrong("#rrggbb or #rrggbbaa"))?;
                Ok(if leaf == Leaf::OptColor {
                    format!("Some({c})")
                } else {
                    c
                })
            }
            Leaf::F32 | Leaf::U8 | Leaf::I8 | Leaf::Radius | Leaf::Margin => {
                if var.ty != "FLOAT" {
                    return Err(wrong("a FLOAT"));
                }
                let x = value.as_f64().ok_or_else(|| wrong("a number"))?;
                match leaf {
                    Leaf::F32 => Ok(f32_lit(x)),
                    Leaf::U8 => int_value(x, 0, 255)
                        .map(|i| i.to_string())
                        .ok_or_else(|| wrong("an integer 0..=255")),
                    Leaf::Radius => int_value(x, 0, 255)
                        .map(|i| format!("CornerRadius::same({i})"))
                        .ok_or_else(|| wrong("an integer 0..=255")),
                    Leaf::I8 => int_value(x, -128, 127)
                        .map(|i| i.to_string())
                        .ok_or_else(|| wrong("an integer -128..=127")),
                    Leaf::Margin => int_value(x, -128, 127)
                        .map(|i| format!("Margin::same({i})"))
                        .ok_or_else(|| wrong("an integer -128..=127")),
                    Leaf::Color | Leaf::OptColor => unreachable!(),
                }
            }
        }
    }
}

/// Generates the (unformatted) theme module. `font_exists(file)` says whether
/// `FONTS_DIR/file` exists.
pub(crate) fn generate(
    tokens: &Tokens,
    font_exists: &dyn Fn(&str) -> bool,
) -> Result<String, String> {
    let ctx = Ctx {
        vars: &tokens.variables,
    };
    let mut light = String::new();
    let mut dark = String::new();
    let mut spacing = String::new();
    let mut styles: BTreeMap<&str, String> = BTreeMap::new();
    let mut families: BTreeMap<&str, String> = BTreeMap::new();
    let mut theme_light: BTreeMap<&str, String> = BTreeMap::new();
    let mut theme_dark: BTreeMap<&str, String> = BTreeMap::new();
    let mut layout = String::new();
    let mut errors = Vec::new();

    for var in &tokens.variables {
        let r = &var.rust;
        let result: Result<(), String> = (|| {
            if let Some(path) = r.strip_prefix("visuals.") {
                let leaf = visuals_leaf(path).ok_or_else(|| {
                    format!("`{}`: egui 0.36 `Visuals` has no field `{path}`", var.name)
                })?;
                writeln!(light, "v.{path} = {};", ctx.expr(var, "Light", leaf)?).expect("string");
                writeln!(dark, "v.{path} = {};", ctx.expr(var, "Dark", leaf)?).expect("string");
            } else if let Some(path) = r.strip_prefix("spacing.") {
                let leaf = spacing_leaf(path).ok_or_else(|| {
                    format!("`{}`: egui 0.36 `Spacing` has no field `{path}`", var.name)
                })?;
                single_mode(var)?;
                writeln!(spacing, "s.{path} = {};", ctx.expr(var, "Default", leaf)?)
                    .expect("string");
            } else if let Some(style) = r.strip_prefix("TextStyle::") {
                let style = TEXT_STYLES
                    .iter()
                    .find(|s| **s == style)
                    .ok_or_else(|| format!("`{}`: egui has no `TextStyle::{style}`", var.name))?;
                single_mode(var)?;
                let size = ctx.expr(var, "Default", Leaf::F32)?;
                let family = if *style == "Monospace" {
                    "Monospace"
                } else {
                    "Proportional"
                };
                if styles
                    .insert(
                        style,
                        format!("(TextStyle::{style}, FontId::new({size}, FontFamily::{family})),"),
                    )
                    .is_some()
                {
                    return Err(format!("`TextStyle::{style}` is defined twice"));
                }
            } else if let Some(family) = r.strip_prefix("FontFamily::") {
                let family = ["Proportional", "Monospace"]
                    .into_iter()
                    .find(|f| *f == family)
                    .ok_or_else(|| format!("`{}`: unknown `FontFamily::{family}`", var.name))?;
                single_mode(var)?;
                let (value, _) = ctx.value(var, "Default")?;
                let name = value
                    .as_str()
                    .filter(|_| var.ty == "STRING")
                    .ok_or_else(|| format!("`{}` must be a STRING font name", var.name))?;
                families.insert(family, name.to_owned());
            } else if let Some(field) = r.strip_prefix("theme.") {
                if !THEME_FIELDS.contains(&field) {
                    return Err(format!(
                        "`{}`: `Theme` has no field `{field}` (add it to THEME_FIELDS in xtask/src/tokens.rs and to the struct)",
                        var.name
                    ));
                }
                let field = THEME_FIELDS.iter().find(|f| **f == field).expect("checked");
                for (mode, out) in [("Light", &mut theme_light), ("Dark", &mut theme_dark)] {
                    let (_, alias) = ctx.value(var, mode)?;
                    let expr = ctx.expr(var, mode, Leaf::Color)?;
                    let line = match alias {
                        Some(a) => format!("// `{}` = `{a}`\n{field}: {expr},", var.name),
                        None => format!("// `{}`\n{field}: {expr},", var.name),
                    };
                    if out.insert(field, line).is_some() {
                        return Err(format!("`theme.{field}` is defined twice"));
                    }
                }
            } else if let Some(name) = r.strip_prefix("layout::") {
                if name.is_empty()
                    || !name
                        .chars()
                        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
                {
                    return Err(format!(
                        "`{}`: `layout::{name}` is not a SCREAMING_SNAKE_CASE constant",
                        var.name
                    ));
                }
                single_mode(var)?;
                let value = ctx.expr(var, "Default", Leaf::F32)?;
                writeln!(
                    layout,
                    "/// `{}`\npub const {name}: f32 = {value};",
                    var.name
                )
                .expect("string");
            } else if r.starts_with("C64Color::") {
                // Not generated: `palette_matches_core_colodore` checks it.
            } else {
                return Err(format!("`{}`: no generator for Rust path `{r}`", var.name));
            }
            Ok(())
        })();
        if let Err(e) = result {
            errors.push(e);
        }
    }

    for style in TEXT_STYLES {
        if !styles.contains_key(style) {
            errors.push(format!("no variable for `TextStyle::{style}`"));
        }
    }
    for field in THEME_FIELDS {
        if !theme_light.contains_key(field) {
            errors.push(format!("no variable for `theme.{field}`"));
        }
    }
    let mut font_consts = String::new();
    for (family, konst) in [("Proportional", "PROPORTIONAL"), ("Monospace", "MONOSPACE")] {
        let Some(name) = families.get(family) else {
            errors.push(format!("no variable for `FontFamily::{family}`"));
            continue;
        };
        let stem: String = name.chars().filter(|c| !c.is_whitespace()).collect();
        let regular = format!("{stem}-Regular.ttf");
        if !font_exists(&regular) {
            errors.push(format!(
                "`FontFamily::{family}` is \"{name}\", but {FONTS_DIR}/{regular} does not exist"
            ));
        }
        writeln!(
            font_consts,
            "/// `FontFamily::{family}`'s font name (`egui / Text`).\n\
             pub const {konst}_NAME: &str = {name:?};\n\
             /// \"{name}\" Regular, the `FontFamily::{family}` font.\n\
             pub const {konst}: &[u8] = include_bytes!(\"{FONTS_FROM_GENERATED}/{regular}\");"
        )
        .expect("string");
        if family == "Proportional" {
            let medium = format!("{stem}-Medium.ttf");
            let value = if font_exists(&medium) {
                format!("Some(include_bytes!(\"{FONTS_FROM_GENERATED}/{medium}\"))")
            } else {
                "None".to_owned()
            };
            writeln!(
                font_consts,
                "/// \"{name}\" Medium, if bundled (`{FONTS_DIR}/{medium}`).\n\
                 pub const {konst}_MEDIUM: Option<&[u8]> = {value};"
            )
            .expect("string");
        }
    }

    if !errors.is_empty() {
        return Err(format!("{TOKENS}:\n  {}", errors.join("\n  ")));
    }

    let theme_body = |m: &BTreeMap<&str, String>| -> String {
        THEME_FIELDS
            .iter()
            .map(|f| m[f].as_str())
            .collect::<Vec<_>>()
            .join("\n")
    };
    let styles_body = TEXT_STYLES
        .iter()
        .map(|s| styles[s].as_str())
        .collect::<Vec<_>>()
        .join("\n");

    // Imports and helpers only as far as the generated code uses them (unused ones
    // would warn).
    let body = format!("{light}{dark}{spacing}");
    let mut egui_items = vec!["Color32"];
    for item in ["CornerRadius", "Margin"] {
        if body.contains(&format!("{item}::")) {
            egui_items.push(item);
        }
    }
    egui_items.extend(["FontFamily", "FontId", "TextStyle"]);
    let all = format!(
        "{body}{}{}",
        theme_body(&theme_light),
        theme_body(&theme_dark)
    );
    let mut helpers = String::new();
    if all.contains("rgb(") {
        helpers.push_str(
            "/// An opaque colour from `0xrrggbb`.
const fn rgb(hex: u32) -> Color32 {
    let [_, r, g, b] = hex.to_be_bytes();
    Color32::from_rgb(r, g, b)
}

",
        );
    }
    if all.contains("rgba(") {
        helpers.push_str(
            "/// A colour from `0xrrggbbaa` with unmultiplied alpha, as Figma stores it.
const fn rgba(hex: u32) -> Color32 {
    let [r, g, b, a] = hex.to_be_bytes();
    Color32::from_rgba_unmultiplied_const(r, g, b, a)
}

",
        );
    }

    Ok(format!(
        "//! Generated by `cargo xtask tokens` from `{TOKENS}`. Do not edit.
//!
//! Source: {source},
//! exported {exported}. Change the Figma variables, re-export them
//! (`design/README.md`) and run `cargo xtask tokens`; CI runs
//! `cargo xtask tokens --check` and fails when this file is stale.

use super::Theme;
use egui::{{{egui_items}}};
use std::collections::BTreeMap;

{helpers}/// `egui / Visuals`, Light mode, applied over `egui::Visuals::light()`.
pub fn apply_light(v: &mut egui::Visuals) {{
{light}}}

/// `egui / Visuals`, Dark mode, applied over `egui::Visuals::dark()`.
pub fn apply_dark(v: &mut egui::Visuals) {{
{dark}}}

/// `egui / Spacing`, applied over `egui::style::Spacing::default()`.
pub fn apply_spacing(s: &mut egui::style::Spacing) {{
{spacing}}}

/// `egui / Text`: the size and family of every text style.
pub fn text_styles() -> BTreeMap<TextStyle, FontId> {{
    BTreeMap::from([
{styles_body}
    ])
}}

/// The bundled fonts named by `egui / Text` (`{FONTS_DIR}`).
pub mod fonts {{
{font_consts}}}

impl Theme {{
    /// `Convert64 / Semantic`, Light mode.
    pub const LIGHT: Theme = Theme {{
{theme_light}
    }};

    /// `Convert64 / Semantic`, Dark mode.
    pub const DARK: Theme = Theme {{
{theme_dark}
    }};
}}

/// Convert64's layout constants (`egui / Spacing`, `convert64/*`), in points.
pub mod layout {{
{layout}}}
",
        source = tokens.source,
        exported = tokens.exported,
        egui_items = egui_items.join(", "),
        theme_light = theme_body(&theme_light),
        theme_dark = theme_body(&theme_dark),
    ))
}

/// Fails unless `var` has exactly one mode.
fn single_mode(var: &Variable) -> Result<(), String> {
    if var.values.len() == 1 {
        Ok(())
    } else {
        Err(format!(
            "`{}` ({}) must have one mode, has {}",
            var.name,
            var.rust,
            var.values.len()
        ))
    }
}

/// Formats `code` with the pinned toolchain's rustfmt, so the file is fmt-clean.
fn rustfmt(root: &Path, code: &str) -> Result<String, String> {
    let mut child = Command::new("rustfmt")
        .args(["--edition", "2024", "--emit", "stdout"])
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot run rustfmt: {e}"))?;
    child
        .stdin
        .take()
        .expect("piped")
        .write_all(code.as_bytes())
        .map_err(|e| format!("rustfmt: {e}"))?;
    let out = child
        .wait_with_output()
        .map_err(|e| format!("rustfmt: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "rustfmt failed on the generated code:\n{}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    String::from_utf8(out.stdout).map_err(|e| format!("rustfmt: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use convert64_core::colour::ColourIndex;
    use convert64_core::palette::{
        DEFAULT_BRIGHTNESS, DEFAULT_CONTRAST, DEFAULT_SATURATION, Palette,
    };

    fn committed() -> Tokens {
        let root = crate::repo_root();
        parse(&std::fs::read_to_string(root.join(TOKENS)).expect("design/tokens.json"))
            .expect("parses")
    }

    /// `C64 / Palette` is not imported; it must equal the core's Colodore palette at
    /// colodore.com's default sliders (the Figma mode is called just "Colodore", which
    /// is the name `Palette::colodore` gives exactly those settings).
    #[test]
    fn palette_matches_core_colodore() {
        let core = Palette::colodore(DEFAULT_BRIGHTNESS, DEFAULT_CONTRAST, DEFAULT_SATURATION);
        assert_eq!(core.name(), "Colodore");
        let tokens = committed();
        let figma: Vec<&Variable> = tokens
            .variables
            .iter()
            .filter(|v| v.rust.starts_with("C64Color::"))
            .collect();
        assert_eq!(figma.len(), 16, "C64 / Palette must have 16 colours");
        let mut seen = [false; 16];
        for var in figma {
            // `colodore/NN Name`: NN is the colour index.
            let index: u8 = var
                .name
                .strip_prefix("colodore/")
                .and_then(|rest| rest.get(..2))
                .and_then(|nn| nn.parse().ok())
                .unwrap_or_else(|| panic!("`{}` is not `colodore/NN Name`", var.name));
            let c = ColourIndex::new(index).expect("index 0..=15");
            assert!(!seen[c.as_usize()], "index {index} twice");
            seen[c.as_usize()] = true;
            assert_eq!(var.values.len(), 1, "`{}` has one mode", var.name);
            let hex = var
                .values
                .values()
                .next()
                .and_then(Value::as_str)
                .expect("a colour");
            let want = format!("#{:06x}", core.rgb(c).to_hex());
            assert_eq!(
                hex.to_ascii_lowercase(),
                want,
                "{} ({}): Figma has {hex}, convert64_core's Colodore has {want}",
                var.name,
                var.rust
            );
        }
        assert!(seen.iter().all(|s| *s), "every index 0..=15 once");
    }

    #[test]
    fn committed_file_is_up_to_date() {
        let root = crate::repo_root();
        let fonts = |f: &str| root.join(FONTS_DIR).join(f).is_file();
        let code =
            rustfmt(&root, &generate(&committed(), &fonts).expect("generates")).expect("rustfmt");
        let current = std::fs::read_to_string(root.join(GENERATED)).expect("generated.rs");
        assert!(
            current == code,
            "{GENERATED} is stale: run `cargo xtask tokens`"
        );
    }

    fn var(name: &str, ty: &str, rust: &str, values: &[(&str, Value)]) -> Variable {
        Variable {
            name: name.into(),
            ty: ty.into(),
            rust: rust.into(),
            values: values
                .iter()
                .map(|(k, v)| ((*k).to_owned(), v.clone()))
                .collect(),
        }
    }

    /// The committed tokens with `extra` added.
    fn with(extra: Variable) -> Result<String, String> {
        let mut tokens = committed();
        tokens.variables.push(extra);
        generate(&tokens, &|_| true)
    }

    #[test]
    fn unknown_paths_fail_loudly() {
        let light_dark = [
            ("Light", Value::from("#ffffff")),
            ("Dark", Value::from("#000000")),
        ];
        for rust in [
            "visuals.widgets.inactive.bg_colour",
            "visuals.widgets.pressed.bg_fill",
            "visuals.window_stroke",
            "spacing.item_spacing.z",
            "theme.sparkle",
            "layout::dock_width",
            "Style::animation_time",
        ] {
            let err = with(var("x/y", "COLOR", rust, &light_dark)).expect_err(rust);
            assert!(err.contains("x/y"), "{rust}: {err}");
        }
    }

    #[test]
    fn wrong_types_and_modes_fail() {
        let one = |v: Value| [("Default", v)];
        // A colour where egui wants a number, a fraction or overflow where it wants u8.
        assert!(with(var("a", "COLOR", "spacing.indent", &one("#ffffff".into()))).is_err());
        let radius = one(2.5.into());
        assert!(with(var("b", "FLOAT", "visuals.window_corner_radius", &radius)).is_err());
        let blur = one(300.into());
        assert!(with(var("c", "FLOAT", "visuals.window_shadow.blur", &blur)).is_err());
        assert!(with(var("d", "COLOR", "visuals.panel_fill", &one("#fff".into()))).is_err());
        // Spacing has no modes.
        let two = [("Light", Value::from(1)), ("Dark", Value::from(2))];
        assert!(with(var("e", "FLOAT", "spacing.indent", &two)).is_err());
        // A two-mode visual needs both modes.
        let other = [
            ("Light", Value::from("#ffffff")),
            ("Sepia", Value::from("#000000")),
        ];
        assert!(with(var("f", "COLOR", "visuals.panel_fill", &other)).is_err());
    }

    #[test]
    fn missing_theme_field_or_font_fails() {
        let mut tokens = committed();
        tokens.variables.retain(|v| v.rust != "theme.nudge");
        let err = generate(&tokens, &|_| true).expect_err("missing nudge");
        assert!(err.contains("theme.nudge"), "{err}");
        let no_inter = |f: &str| !f.starts_with("Inter-Regular");
        let err = generate(&committed(), &no_inter).expect_err("no font");
        assert!(err.contains("Inter-Regular.ttf"), "{err}");
    }

    #[test]
    fn colours_keep_figma_alpha_unmultiplied() {
        assert_eq!(color_expr("#F8F8F8").as_deref(), Some("rgb(0xf8f8f8)"));
        assert_eq!(color_expr("#ffffff26").as_deref(), Some("rgba(0xffffff26)"));
        assert_eq!(color_expr("#123456ff").as_deref(), Some("rgb(0x123456)"));
        assert_eq!(color_expr("#12345"), None);
        assert_eq!(color_expr("123456"), None);
        assert_eq!(f32_lit(12.5), "12.5");
        assert_eq!(f32_lit(8.0), "8.0");
        assert_eq!(f32_lit(0.6), "0.6");
    }
}
