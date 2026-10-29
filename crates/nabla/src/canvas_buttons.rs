//! Floating buttons over the canvas, like the ones by Blender's navigation gizmo: white
//! outline icons in a grey rounded column in the canvas's top right corner, right of the
//! gizmo. Toggles turn light blue while on; switches between two modes show the mode's icon.

use egui::{Color32, CornerRadius, Pos2, Rect, Sense, Vec2};

use crate::theme::{icons, palette};

/// Side of a button, the icon's size, and the space around and between the buttons.
const SIZE: f32 = 32.0;
const ICON_SIZE: f32 = 20.0;
const PADDING: f32 = 4.0;

/// Width of the column.
pub const WIDTH: f32 = SIZE + 2.0 * PADDING;
/// Space between the column and the canvas's top and right edges.
pub const MARGIN_TOP: f32 = 8.0;
pub const MARGIN_RIGHT: f32 = 8.0;

/// The column's color: a translucent grey, which stands out on the dark canvas.
const COLUMN: Color32 = Color32::from_rgba_unmultiplied_const(46, 46, 46, 210);

/// Color of an icon while on, and of the button behind it.
const ON: Color32 = palette::BLUE;

/// A button: the setting it flips when clicked, and its icon and tooltip while the setting
/// is off and while it is on.
pub struct Button<'a> {
    on: &'a mut bool,
    icons: [&'static str; 2],
    tooltips: [&'static str; 2],
    /// Show "on" light blue.
    highlight: bool,
}

impl<'a> Button<'a> {
    /// A toggle: one icon, light blue while `on`.
    pub fn toggle(icon: &'static str, on: &'a mut bool, tooltip: &'static str) -> Self {
        Self {
            on,
            icons: [icon; 2],
            tooltips: [tooltip; 2],
            highlight: true,
        }
    }

    /// A switch between two modes, off and `on`, each with its own icon and tooltip.
    pub fn switch(icons: [&'static str; 2], on: &'a mut bool, tooltips: [&'static str; 2]) -> Self {
        Self {
            on,
            icons,
            tooltips,
            highlight: false,
        }
    }
}

/// Shows `buttons` in a column in the top right corner of `canvas`, and flips the setting of
/// the one clicked.
pub fn show(ui: &egui::Ui, painter: &egui::Painter, canvas: Rect, buttons: &mut [Button<'_>]) {
    let count = buttons.len() as f32;
    let column = Rect::from_min_size(
        Pos2::new(
            canvas.right() - MARGIN_RIGHT - WIDTH,
            canvas.top() + MARGIN_TOP,
        ),
        Vec2::new(WIDTH, count * SIZE + (count + 1.0) * PADDING),
    );
    painter.rect_filled(
        column,
        CornerRadius::same((SIZE / 2.0 + PADDING) as u8),
        COLUMN,
    );

    for (i, button) in buttons.iter_mut().enumerate() {
        let rect = Rect::from_min_size(
            column.min + Vec2::new(PADDING, PADDING + i as f32 * (SIZE + PADDING)),
            Vec2::splat(SIZE),
        );
        let id = ui.scope_id().with(("canvas_button", button.icons[0]));
        let state = usize::from(*button.on);
        let response = ui
            .interact(rect, id, Sense::click())
            .on_hover_text(button.tooltips[state]);
        if response.clicked() {
            *button.on = !*button.on;
        }
        let state = usize::from(*button.on);
        let lit = button.highlight && *button.on;
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }

        let background = match (lit, response.hovered()) {
            (true, hovered) => Some(ON.gamma_multiply(if hovered { 0.35 } else { 0.25 })),
            (false, true) => Some(Color32::from_white_alpha(25)),
            (false, false) => None,
        };
        if let Some(background) = background {
            painter.rect_filled(rect, CornerRadius::same((SIZE / 2.0) as u8), background);
        }
        let color = if lit { ON } else { palette::WHITE };
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            button.icons[state],
            icons::outline::font(ICON_SIZE),
            color,
        );
    }
}
