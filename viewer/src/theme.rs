//! The viewer's look: a single dark theme with a pure black background and white foreground,
//! using the Catppuccin Mocha pastels as accent colors. There is no light mode.

use egui::{Color32, CornerRadius, FontFamily, FontId, Margin, Stroke, TextStyle, Theme};

fn stroke(width: f32, color: Color32) -> Stroke {
    Stroke::new(width, color)
}

/// Catppuccin Mocha accents (<https://catppuccin.com/palette>), used for data and highlights.
pub mod palette {
    use egui::Color32;

    pub const ROSEWATER: Color32 = Color32::from_rgb(0xf5, 0xe0, 0xdc);
    pub const PINK: Color32 = Color32::from_rgb(0xf5, 0xc2, 0xe7);
    pub const MAUVE: Color32 = Color32::from_rgb(0xcb, 0xa6, 0xf7);
    pub const RED: Color32 = Color32::from_rgb(0xf3, 0x8b, 0xa8);
    pub const PEACH: Color32 = Color32::from_rgb(0xfa, 0xb3, 0x87);
    pub const YELLOW: Color32 = Color32::from_rgb(0xf9, 0xe2, 0xaf);
    pub const GREEN: Color32 = Color32::from_rgb(0xa6, 0xe3, 0xa1);
    pub const TEAL: Color32 = Color32::from_rgb(0x94, 0xe2, 0xd5);
    pub const SKY: Color32 = Color32::from_rgb(0x89, 0xdc, 0xeb);
    pub const SAPPHIRE: Color32 = Color32::from_rgb(0x74, 0xc7, 0xec);
    pub const BLUE: Color32 = Color32::from_rgb(0x89, 0xb4, 0xfa);
    pub const LAVENDER: Color32 = Color32::from_rgb(0xb4, 0xbe, 0xfe);

    // Neutrals. Mocha's own surfaces are blue-tinted; on a black background plain grays look
    // cleaner, so only the accents above come from Mocha.
    pub const BLACK: Color32 = Color32::BLACK;
    pub const WHITE: Color32 = Color32::WHITE;
    pub const SURFACE0: Color32 = Color32::from_gray(18);
    pub const SURFACE1: Color32 = Color32::from_gray(30);
    pub const SURFACE2: Color32 = Color32::from_gray(46);
    pub const BORDER: Color32 = Color32::from_gray(40);
    pub const OVERLAY0: Color32 = Color32::from_gray(110);
    pub const OVERLAY1: Color32 = Color32::from_gray(140);
    pub const SUBTEXT: Color32 = Color32::from_gray(185);
}

const PLEX: &[u8] = include_bytes!("../assets/fonts/IBMPlexSans-Regular.ttf");
const LILEX: &[u8] = include_bytes!("../assets/fonts/Lilex-Regular.ttf");
/// Phosphor Icons, fill weight (<https://phosphoricons.com>), cut down to the glyphs in [`icons`]:
/// `pyftsubset Phosphor-Fill.ttf --unicodes=U+E138,... --layout-features='' --no-hinting`, with the
/// font and codepoints from `@phosphor-icons/web@2.1.1` (`src/fill/`). Add a glyph there first.
const PHOSPHOR: &[u8] = include_bytes!("../assets/fonts/Phosphor-Fill-subset.ttf");

/// Phosphor icon glyphs, usable in any text, so they follow the widget's text color.
pub mod icons {
    pub const CARET_LEFT: &str = "\u{E138}";
    pub const CARET_RIGHT: &str = "\u{E13A}";
    pub const PAUSE: &str = "\u{E39E}";
    pub const PLAY: &str = "\u{E3D0}";
    pub const SKIP_BACK: &str = "\u{E5A4}";
    pub const SKIP_FORWARD: &str = "\u{E5A6}";
}

/// IBM Plex Sans for the UI (as in Zed) and Lilex for code (as in Zed). The fonts are embedded, so
/// the look is the same on every platform. Phosphor comes next, for the [`icons`], and egui's
/// default fonts stay as glyph fallbacks.
fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    for (name, bytes, family) in [
        ("IBM Plex Sans", PLEX, FontFamily::Proportional),
        ("Lilex", LILEX, FontFamily::Monospace),
    ] {
        fonts.font_data.insert(
            name.to_owned(),
            std::sync::Arc::new(egui::FontData::from_static(bytes)),
        );
        fonts
            .families
            .entry(family)
            .or_default()
            .insert(0, name.to_owned());
    }
    fonts.font_data.insert(
        "Phosphor".to_owned(),
        std::sync::Arc::new(egui::FontData::from_static(PHOSPHOR)),
    );
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .insert(1, "Phosphor".to_owned());
    }
    ctx.set_fonts(fonts);
}

/// Install the theme and fonts, and force dark mode regardless of the OS setting.
pub fn apply(ctx: &egui::Context) {
    install_fonts(ctx);
    ctx.set_theme(egui::ThemePreference::Dark);
    ctx.style_mut_of(Theme::Dark, style);
}

fn style(style: &mut egui::Style) {
    use palette::{
        BLACK, BORDER, MAUVE, OVERLAY0, RED, SAPPHIRE, SUBTEXT, SURFACE0, SURFACE1, SURFACE2,
        WHITE, YELLOW,
    };

    style.text_styles = [
        (
            TextStyle::Small,
            FontId::new(11.0, FontFamily::Proportional),
        ),
        (TextStyle::Body, FontId::new(13.5, FontFamily::Proportional)),
        (
            TextStyle::Button,
            FontId::new(13.5, FontFamily::Proportional),
        ),
        (
            TextStyle::Heading,
            FontId::new(20.0, FontFamily::Proportional),
        ),
        (
            TextStyle::Monospace,
            FontId::new(12.5, FontFamily::Monospace),
        ),
    ]
    .into();

    let spacing = &mut style.spacing;
    spacing.item_spacing = egui::vec2(8.0, 6.0);
    spacing.button_padding = egui::vec2(10.0, 4.0);
    spacing.window_margin = Margin::same(12);
    spacing.interact_size.y = 24.0;

    let r = CornerRadius::same(6);
    let v = &mut style.visuals;
    v.dark_mode = true;
    v.override_text_color = None;
    v.panel_fill = BLACK;
    v.window_fill = BLACK;
    v.extreme_bg_color = BLACK;
    v.faint_bg_color = SURFACE0;
    v.code_bg_color = SURFACE0;
    v.window_stroke = stroke(1.0, BORDER);
    v.window_corner_radius = CornerRadius::same(10);
    v.menu_corner_radius = r;
    v.window_shadow = egui::Shadow::NONE;
    v.popup_shadow = egui::Shadow {
        offset: [0, 4],
        blur: 16,
        spread: 0,
        color: Color32::from_black_alpha(200),
    };
    v.hyperlink_color = SAPPHIRE;
    v.warn_fg_color = YELLOW;
    v.error_fg_color = RED;
    v.selection.bg_fill = MAUVE.gamma_multiply(0.35);
    v.selection.stroke = stroke(1.0, MAUVE);
    v.slider_trailing_fill = true;
    v.striped = false;

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = BLACK;
    w.noninteractive.weak_bg_fill = BLACK;
    w.noninteractive.bg_stroke = stroke(1.0, BORDER);
    w.noninteractive.fg_stroke = stroke(1.0, SUBTEXT);
    w.noninteractive.corner_radius = r;

    w.inactive.bg_fill = SURFACE1;
    w.inactive.weak_bg_fill = SURFACE0;
    w.inactive.bg_stroke = stroke(1.0, BORDER);
    w.inactive.fg_stroke = stroke(1.0, WHITE);
    w.inactive.corner_radius = r;

    w.hovered.bg_fill = SURFACE2;
    w.hovered.weak_bg_fill = SURFACE1;
    w.hovered.bg_stroke = stroke(1.0, OVERLAY0);
    w.hovered.fg_stroke = stroke(1.5, WHITE);
    w.hovered.corner_radius = r;
    w.hovered.expansion = 0.0;

    w.active.bg_fill = SURFACE2;
    w.active.weak_bg_fill = SURFACE2;
    w.active.bg_stroke = stroke(1.0, MAUVE);
    w.active.fg_stroke = stroke(1.5, WHITE);
    w.active.corner_radius = r;
    w.active.expansion = 0.0;

    w.open.bg_fill = SURFACE1;
    w.open.weak_bg_fill = SURFACE1;
    w.open.bg_stroke = stroke(1.0, OVERLAY0);
    w.open.fg_stroke = stroke(1.0, WHITE);
    w.open.corner_radius = r;
}
