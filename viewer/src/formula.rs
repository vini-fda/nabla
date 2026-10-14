//! Math typeset at build time: `build.rs` renders the formulas in its `FORMULAS` with Typst
//! into white SVGs, embedded here as one [`Formula`] constant each (`formula::THETA`, ...).
//! They are drawn as images, tinted with the text color and lined up with the text's baseline.

use egui::{FontId, Rect, Response, Sense, Ui, vec2};

/// Height of every formula's SVG, in em. Keep in sync with `build.rs`.
const HEIGHT: f32 = 1.4;
/// Distance from the top of every formula's SVG to its baseline, in em.
const ASCENT: f32 = 1.0;
/// Typst's math font (New Computer Modern) looks small next to the UI fonts at the same size,
/// so formulas are drawn at this many times the size of the text they sit in.
const SCALE: f32 = 1.2;

/// A formula rendered to SVG by `build.rs`.
pub struct Formula {
    /// The image's URI, for egui's image cache. Ends in `.svg`, for the SVG loader.
    uri: &'static str,
    svg: &'static [u8],
    /// Width in em.
    width: f32,
}

include!(concat!(env!("OUT_DIR"), "/formulas.rs"));

impl Formula {
    fn image(&self) -> egui::Image<'static> {
        egui::Image::from_bytes(self.uri, self.svg)
    }

    /// Shows the formula in place of text in `font`: as wide as the formula, as tall as a row of
    /// that text, and with its baseline on the text's baseline.
    pub fn ui(&self, ui: &mut Ui, font: &FontId) -> Response {
        let color = ui.visuals().text_color();
        let galley = ui
            .painter()
            .layout_no_wrap("x".to_owned(), font.clone(), color);
        let baseline = galley
            .rows
            .first()
            .and_then(|row| row.glyphs.first())
            .map_or(0.8 * font.size, |glyph| glyph.pos.y);
        let em = SCALE * font.size;
        let (rect, response) =
            ui.allocate_exact_size(vec2(self.width * em, galley.size().y), Sense::hover());
        let image = Rect::from_min_size(
            rect.left_top() + vec2(0.0, baseline - ASCENT * em),
            vec2(self.width * em, HEIGHT * em),
        );
        self.image().tint(color).paint_at(ui, image);
        response
    }
}
