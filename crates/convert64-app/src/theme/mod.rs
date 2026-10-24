//! Convert64's look: egui visuals for both themes, spacing, text styles, fonts and the
//! app's own semantic colours (ADR-007).
//!
//! Every value comes from `generated.rs`, which `cargo xtask tokens` writes from the
//! Figma variables in `design/tokens.json` (`docs/figma.md`). Nothing here hard-codes a
//! colour or a size: change the Figma variable, re-export and regenerate.

mod generated;

pub use generated::{apply_dark, apply_light, apply_spacing, fonts, layout, text_styles};

use egui::{FontData, FontDefinitions, FontFamily};
use std::sync::Arc;

/// Convert64's semantic colours for one theme (`Convert64 / Semantic` in Figma).
///
/// Two instances, [`Theme::LIGHT`] and [`Theme::DARK`], generated from the tokens;
/// [`Theme::current`] picks the one egui is drawing with. They colour the overlays and
/// the chrome around the C64 preview, never the preview itself (ADR-007).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    /// Overlay: cells whose colours clash (`overlay/clash`).
    pub clash: egui::Color32,
    /// Overlay: pinned colour sets (`overlay/pin`).
    pub pin: egui::Color32,
    /// Overlay: locked cells (`overlay/lock`).
    pub lock: egui::Color32,
    /// Overlay: region outlines (`overlay/region`).
    pub region: egui::Color32,
    /// Overlay: sub-pixel nudge handles (`overlay/nudge`).
    pub nudge: egui::Color32,
    /// Overlay: the 8 × 8 cell grid (`overlay/cell_grid`).
    pub cell_grid: egui::Color32,
    /// Overlay: the character grid (`overlay/char_grid`).
    pub char_grid: egui::Color32,
    /// Canvas: the background around the preview (`canvas/bg`).
    pub canvas_bg: egui::Color32,
    /// Canvas: transparency checkerboard, first square (`canvas/checker_a`).
    pub checker_a: egui::Color32,
    /// Canvas: transparency checkerboard, second square (`canvas/checker_b`).
    pub checker_b: egui::Color32,
    /// Status: success (`status/ok`).
    pub ok: egui::Color32,
    /// Status: warning (`status/warn`, an alias of `Visuals::warn_fg_color`).
    pub warn: egui::Color32,
    /// Status: error (`status/error`, an alias of `Visuals::error_fg_color`).
    pub error: egui::Color32,
    /// Accent (`accent`, an alias of `Visuals::hyperlink_color`).
    pub accent: egui::Color32,
}

impl Theme {
    /// The semantic colours for `theme`.
    pub const fn of(theme: egui::Theme) -> &'static Self {
        match theme {
            egui::Theme::Light => &Self::LIGHT,
            egui::Theme::Dark => &Self::DARK,
        }
    }

    /// The semantic colours for the theme `ctx` is drawing with.
    pub fn current(ctx: &egui::Context) -> &'static Self {
        Self::of(ctx.theme())
    }
}

/// The named font family for Inter Medium (headings, emphasis), when bundled:
/// `FontFamily::Name(MEDIUM.into())`.
pub const MEDIUM: &str = "medium";

/// egui's font definitions with the bundled fonts first: the `egui / Text` proportional
/// font for `FontFamily::Proportional`, the monospace font for `FontFamily::Monospace`,
/// and its Medium cut as the [`MEDIUM`] family. egui's own fonts stay behind them as
/// fallbacks (emoji and symbols).
pub fn font_definitions() -> FontDefinitions {
    let mut defs = FontDefinitions::default();
    let mut put_first = |family: FontFamily, name: &str, data: &'static [u8]| {
        defs.font_data
            .insert(name.to_owned(), Arc::new(FontData::from_static(data)));
        defs.families
            .entry(family)
            .or_default()
            .insert(0, name.to_owned());
    };
    put_first(
        FontFamily::Proportional,
        fonts::PROPORTIONAL_NAME,
        fonts::PROPORTIONAL,
    );
    put_first(
        FontFamily::Monospace,
        fonts::MONOSPACE_NAME,
        fonts::MONOSPACE,
    );
    if let Some(medium) = fonts::PROPORTIONAL_MEDIUM {
        let name = format!("{} Medium", fonts::PROPORTIONAL_NAME);
        defs.font_data
            .insert(name.clone(), Arc::new(FontData::from_static(medium)));
        // Medium first, then the proportional chain for anything it lacks.
        let mut chain = vec![name];
        chain.extend(
            defs.families
                .get(&FontFamily::Proportional)
                .cloned()
                .unwrap_or_default(),
        );
        defs.families.insert(FontFamily::Name(MEDIUM.into()), chain);
    }
    defs
}

/// Installs the generated look on `ctx`: fonts, then the Material Symbols icon font
/// (ADR-007: `egui_material_icons::initialize` after `set_fonts`, which would otherwise
/// drop it), then the light and dark styles. The theme follows the system until
/// [`set_theme`] says otherwise. Call once, at startup.
pub fn install(ctx: &egui::Context) {
    ctx.set_fonts(font_definitions());
    egui_material_icons::initialize(ctx);
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        ctx.set_style_of(theme, style(theme));
    }
    set_theme(ctx, egui::ThemePreference::System);
}

/// The full egui style for `theme`: egui's default with every generated value applied.
pub fn style(theme: egui::Theme) -> egui::Style {
    let mut style = theme.default_style();
    match theme {
        egui::Theme::Light => apply_light(&mut style.visuals),
        egui::Theme::Dark => apply_dark(&mut style.visuals),
    }
    apply_spacing(&mut style.spacing);
    style.text_styles = text_styles();
    style
}

/// Switches between the system theme, light and dark (the future View → Theme menu).
pub fn set_theme(ctx: &egui::Context, preference: egui::ThemePreference) {
    ctx.set_theme(preference);
}

/// WCAG 2.x relative luminance of an opaque sRGB colour.
fn luminance(c: egui::Color32) -> f64 {
    let lin = |v: u8| {
        let v = f64::from(v) / 255.0;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(c.r()) + 0.7152 * lin(c.g()) + 0.0722 * lin(c.b())
}

/// The WCAG contrast ratio of text `fg` on `bg` (1 to 21). A translucent `fg` is
/// blended over `bg` as the painter would.
pub fn contrast(fg: egui::Color32, bg: egui::Color32) -> f64 {
    let a = f32::from(fg.a()) / 255.0;
    let fg = if fg.a() == 255 {
        fg
    } else {
        // Color32 is premultiplied: fg + bg × (1 − a).
        let mix = |f: u8, b: u8| (f32::from(f) + f32::from(b) * (1.0 - a)).round() as u8;
        egui::Color32::from_rgb(
            mix(fg.r(), bg.r()),
            mix(fg.g(), bg.g()),
            mix(fg.b(), bg.b()),
        )
    };
    let (l1, l2) = (luminance(fg), luminance(bg));
    (l1.max(l2) + 0.05) / (l1.min(l2) + 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs one frame so pending fonts and styles take effect.
    fn frame(ctx: &egui::Context) {
        let mut output = ctx.run_ui(egui::RawInput::default(), |_| {});
        output.textures_delta.clear(); // no renderer here
    }

    #[test]
    fn install_applies_generated_styles_to_both_themes() {
        let ctx = egui::Context::default();
        install(&ctx);
        frame(&ctx);
        for theme in [egui::Theme::Light, egui::Theme::Dark] {
            // Field by field: egui updates `visuals.text_options` from the frame's input.
            let installed = ctx.style_of(theme);
            let mut want = style(theme);
            want.visuals.text_options = installed.visuals.text_options;
            assert_eq!(installed.visuals, want.visuals, "{theme:?}");
            assert_eq!(installed.spacing, want.spacing, "{theme:?}");
            assert_eq!(installed.text_styles, text_styles());
        }
        assert_eq!(
            ctx.options(|o| o.theme_preference),
            egui::ThemePreference::System
        );
    }

    #[test]
    fn set_theme_switches_semantic_colours() {
        let ctx = egui::Context::default();
        install(&ctx);
        set_theme(&ctx, egui::ThemePreference::Dark);
        frame(&ctx);
        assert_eq!(Theme::current(&ctx), &Theme::DARK);
        assert_eq!(
            ctx.global_style().visuals.panel_fill,
            style(egui::Theme::Dark).visuals.panel_fill
        );
        set_theme(&ctx, egui::ThemePreference::Light);
        frame(&ctx);
        assert_eq!(Theme::current(&ctx), &Theme::LIGHT);
        assert!(!ctx.global_style().visuals.dark_mode);
    }

    #[test]
    fn bundled_fonts_lead_their_families_and_icons_are_registered() {
        let ctx = egui::Context::default();
        install(&ctx);
        frame(&ctx);
        let defs = ctx.fonts(|f| f.definitions().clone());
        let first = |family: &FontFamily| defs.families[family].first().cloned();
        assert_eq!(
            first(&FontFamily::Proportional).as_deref(),
            Some(fonts::PROPORTIONAL_NAME)
        );
        assert_eq!(
            first(&FontFamily::Monospace).as_deref(),
            Some(fonts::MONOSPACE_NAME)
        );
        // The icon font survives `set_fonts` and is a fallback of the proportional family.
        assert!(
            defs.font_data
                .contains_key(egui_material_icons::FONT_FAMILY)
        );
        assert!(
            defs.families[&FontFamily::Proportional]
                .iter()
                .any(|f| f == egui_material_icons::FONT_FAMILY)
        );
        if fonts::PROPORTIONAL_MEDIUM.is_some() {
            assert!(defs.families.contains_key(&FontFamily::Name(MEDIUM.into())));
        }
    }

    /// Usability review M11 (#293): body, hint (weak) and link text reach WCAG AA, 4.5:1,
    /// on every background they sit on (panels, popups, the faint stripe, text fields and
    /// the canvas around the picture), in both themes, as the generated theme sets them.
    #[test]
    fn text_colours_reach_4_5_to_1_on_their_backgrounds() {
        // The reference values from WCAG's own examples.
        assert!((contrast(egui::Color32::BLACK, egui::Color32::WHITE) - 21.0).abs() < 1e-9);
        assert!(
            (contrast(egui::Color32::from_gray(0x76), egui::Color32::WHITE) - 4.54).abs() < 0.01
        );
        for theme in [egui::Theme::Light, egui::Theme::Dark] {
            let v = style(theme).visuals;
            let backgrounds = [
                ("panel_fill", v.panel_fill),
                ("window_fill", v.window_fill),
                ("faint_bg_color", v.faint_bg_color),
                ("extreme_bg_color", v.extreme_bg_color),
                ("canvas/bg", Theme::of(theme).canvas_bg),
            ];
            let texts = [
                ("text", v.text_color()),
                ("weak text", v.weak_text_color()),
                ("link / accent", v.hyperlink_color),
            ];
            for (bg_name, bg) in backgrounds {
                for (fg_name, fg) in texts {
                    let ratio = contrast(fg, bg);
                    assert!(
                        ratio >= 4.5,
                        "{theme:?}: {fg_name} {fg:?} on {bg_name} {bg:?} is {ratio:.2}:1"
                    );
                }
            }
            // Hints stay quieter than body text: the hierarchy survives the fix.
            assert!(
                contrast(v.weak_text_color(), v.panel_fill)
                    < contrast(v.text_color(), v.panel_fill),
                "{theme:?}"
            );
        }
    }

    /// #293: no text style is smaller than 11 px, egui's `Small` as the tokens now set
    /// it (the hint lines were 9 px).
    #[test]
    fn small_text_is_11_px() {
        let styles = text_styles();
        assert_eq!(styles[&egui::TextStyle::Small].size, 11.0);
        assert!(styles.values().all(|f| f.size >= 11.0), "{styles:?}");
    }

    #[test]
    fn theme_aliases_follow_egui_visuals() {
        for (theme, sem) in [
            (egui::Theme::Light, Theme::LIGHT),
            (egui::Theme::Dark, Theme::DARK),
        ] {
            let v = style(theme).visuals;
            assert_eq!(sem.warn, v.warn_fg_color, "{theme:?}");
            assert_eq!(sem.error, v.error_fg_color, "{theme:?}");
            assert_eq!(sem.accent, v.hyperlink_color, "{theme:?}");
        }
    }
}
