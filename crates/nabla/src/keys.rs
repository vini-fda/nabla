//! Keyboard and mouse hints drawn as keycaps, as in a Mac menu: a key is a small grey cap
//! with its symbol (an arrow, Shift) or its name ("space", "F") on it. Keys differ between
//! platforms: Shift is its symbol on macOS and its name elsewhere, and a Mac laptop has no
//! Home or End keys (fn+Left and fn+Right send them).

use egui::{FontFamily, FontId, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Vec2};

use crate::theme::icons::outline;
use crate::theme::palette;

/// Height of a cap, its least width, and the space between its edges and its face.
const HEIGHT: f32 = 20.0;
const MIN_WIDTH: f32 = 20.0;
const PADDING_X: f32 = 5.0;
/// Depth of the darker lip along a cap's bottom edge, which makes it read as a key.
const LIP: f32 = 2.0;
const WORD_SIZE: f32 = 11.5;
const GLYPH_SIZE: f32 = 13.0;
/// Size of a mouse glyph, and of the action word next to it.
const MOUSE_GLYPH_SIZE: f32 = 15.0;
const MOUSE_WORD_SIZE: f32 = 12.0;

/// The face of a keycap: a key's name, or its symbol from the outline icon font.
#[derive(Clone, Copy)]
pub enum Key {
    Word(&'static str),
    Glyph(&'static str),
}

pub const SPACE: Key = Key::Word("space");
pub const LEFT: Key = Key::Glyph(outline::ARROW_LEFT);
pub const RIGHT: Key = Key::Glyph(outline::ARROW_RIGHT);
pub const F: Key = Key::Word("F");
const FN: Key = Key::Word("fn");

/// The keys a hint shows, for the platform it runs on (in the browser too: egui reads the
/// platform from the user agent).
#[derive(Clone, Copy)]
pub struct Keys {
    mac: bool,
}

impl Keys {
    pub fn new(ctx: &egui::Context) -> Self {
        Self {
            mac: ctx.os() == egui::os::OperatingSystem::Mac,
        }
    }

    /// Shift: its symbol on macOS (U+21E7 in the system's menus), its name elsewhere.
    pub fn shift(self) -> Key {
        if self.mac {
            Key::Glyph(outline::ARROW_FAT_UP)
        } else {
            Key::Word("Shift")
        }
    }

    /// The keys that send Home: fn+Left on a Mac, which has no Home key on a laptop.
    pub fn home(self) -> &'static [Key] {
        if self.mac {
            &[FN, LEFT]
        } else {
            &[Key::Word("Home")]
        }
    }

    /// The keys that send End: fn+Right on a Mac.
    pub fn end(self) -> &'static [Key] {
        if self.mac {
            &[FN, RIGHT]
        } else {
            &[Key::Word("End")]
        }
    }
}

/// Draw `keys` as caps, side by side.
pub fn caps(ui: &mut egui::Ui, keys: &[Key]) {
    let spacing = std::mem::replace(&mut ui.spacing_mut().item_spacing.x, 3.0);
    for &key in keys {
        cap(ui, key);
    }
    ui.spacing_mut().item_spacing.x = spacing;
}

fn cap(ui: &mut egui::Ui, key: Key) {
    let (text, font) = match key {
        Key::Word(word) => (word, FontId::new(WORD_SIZE, FontFamily::Proportional)),
        Key::Glyph(glyph) => (glyph, outline::font(GLYPH_SIZE)),
    };
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), font, palette::WHITE);
    let width = (galley.size().x + 2.0 * PADDING_X).max(MIN_WIDTH);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, HEIGHT), Sense::hover());
    let painter = ui.painter();
    let radius = 5.0;
    // The lip: the border's color, showing below the face.
    painter.rect_filled(rect, radius, palette::BORDER);
    let face = Rect::from_min_max(rect.min, Pos2::new(rect.max.x, rect.max.y - LIP));
    painter.rect(
        face,
        radius,
        palette::SURFACE1,
        Stroke::new(1.0, palette::BORDER),
        StrokeKind::Inside,
    );
    painter.galley(face.center() - galley.size() / 2.0, galley, palette::WHITE);
}

/// A dim separator between caps: "+" for keys held together, "/" between alternatives.
pub fn joiner(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(WORD_SIZE).color(palette::OVERLAY0));
}

/// A mouse action: the mouse `glyph` and the action's name, without a cap, so it doesn't
/// look like a key.
pub fn mouse(ui: &mut egui::Ui, glyph: &str, action: &str) {
    let spacing = std::mem::replace(&mut ui.spacing_mut().item_spacing.x, 3.0);
    ui.label(
        RichText::new(glyph)
            .font(outline::font(MOUSE_GLYPH_SIZE))
            .color(palette::WHITE),
    );
    ui.label(RichText::new(action).size(MOUSE_WORD_SIZE));
    ui.spacing_mut().item_spacing.x = spacing;
}

/// A piece of a [`label_with_icons`]: text, or an outline icon inline with it.
pub enum Run {
    Text(&'static str),
    Icon(&'static str),
}

/// A label with icons inline, such as the canvas buttons' to point at them.
pub fn label_with_icons(ui: &mut egui::Ui, runs: &[Run]) {
    let mut job = egui::text::LayoutJob::default();
    for run in runs {
        let text = match *run {
            Run::Text(text) => RichText::new(text),
            Run::Icon(glyph) => RichText::new(glyph).font(outline::font(GLYPH_SIZE)),
        };
        text.append_to(
            &mut job,
            ui.style(),
            egui::FontSelection::Default,
            egui::Align::Center,
        );
    }
    ui.label(job);
}

/// A tooltip naming an action and its keys: "Start [fn][Left]".
pub fn tooltip(ui: &mut egui::Ui, action: &str, keys: &[Key]) {
    ui.horizontal(|ui| {
        ui.label(action);
        caps(ui, keys);
    });
}
