//! Notifications over the canvas, like a phone's: each tells of an event (the autopilot
//! engaged, the ship landed, ...), shows for [`LIFETIME`] and then goes, the newest below the
//! older ones.

use std::collections::VecDeque;

use crate::theme::palette;

/// Seconds a notification shows, from when it is posted.
pub const LIFETIME: f64 = 5.0;
/// Most notifications shown at once: posting another drops the oldest early.
const MAX_SHOWN: usize = 3;
/// Seconds a notification takes to fade in, and to fade out at the end of its life.
const FADE: f64 = 0.2;
/// Space above the first notification, and between neighboring ones, in points.
const TOP: f32 = 16.0;
const GAP: f32 = 8.0;
const FONT_SIZE: f32 = 18.0;
const PADDING: egui::Vec2 = egui::vec2(12.0, 6.0);

struct Notification {
    text: String,
    color: egui::Color32,
    /// When it was posted, on the clock of [`Notifications::tick`].
    posted: f64,
}

/// The notifications showing, oldest first.
#[derive(Default)]
pub struct Notifications {
    /// The time, as of the last [`Self::tick`].
    now: f64,
    shown: VecDeque<Notification>,
}

impl Notifications {
    /// Set the clock to `now` (in seconds, e.g. egui's input time), dropping the notifications
    /// that have shown their full time.
    pub fn tick(&mut self, now: f64) {
        self.now = now;
        self.shown.retain(|n| now - n.posted < LIFETIME);
    }

    /// Whether any notification shows, and so the canvas must be redrawn to fade it out.
    pub fn any(&self) -> bool {
        !self.shown.is_empty()
    }

    /// Drop every notification, as when leaving the flight.
    pub fn clear(&mut self) {
        self.shown.clear();
    }

    /// Tell of an event, in `text`, in `color`.
    pub fn post(&mut self, text: impl Into<String>, color: egui::Color32) {
        if self.shown.len() == MAX_SHOWN {
            self.shown.pop_front();
        }
        self.shown.push_back(Notification {
            text: text.into(),
            color,
            posted: self.now,
        });
    }

    /// Paint the notifications, stacked down from the top of the `canvas`, centered across it.
    pub fn paint(&self, painter: &egui::Painter, canvas: egui::Rect) {
        let mut top = canvas.top() + TOP;
        for n in &self.shown {
            let age = self.now - n.posted;
            let opacity = (age / FADE).min((LIFETIME - age) / FADE).clamp(0.0, 1.0) as f32;
            let galley = painter.layout_no_wrap(
                n.text.clone(),
                egui::FontId::proportional(FONT_SIZE),
                n.color.gamma_multiply(opacity),
            );
            let size = galley.size() + 2.0 * PADDING;
            let rect =
                egui::Rect::from_min_size(egui::pos2(canvas.center().x - size.x / 2.0, top), size);
            painter.rect_filled(
                rect,
                rect.height() / 2.0,
                egui::Color32::from_rgba_unmultiplied(30, 30, 30, 220).gamma_multiply(opacity),
            );
            painter.galley(rect.min + PADDING, galley, palette::WHITE);
            top = rect.bottom() + GAP;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(n: &Notifications) -> Vec<&str> {
        n.shown.iter().map(|n| n.text.as_str()).collect()
    }

    #[test]
    fn each_shows_for_its_lifetime() {
        let mut n = Notifications::default();
        n.tick(10.0);
        n.post("a", palette::WHITE);
        n.tick(12.0);
        n.post("b", palette::WHITE);
        n.tick(10.0 + LIFETIME - 0.01);
        assert_eq!(texts(&n), ["a", "b"]);
        n.tick(10.0 + LIFETIME);
        assert_eq!(texts(&n), ["b"], "the first goes, the second stays");
        n.tick(12.0 + LIFETIME);
        assert!(!n.any());
    }

    #[test]
    fn a_new_one_drops_the_oldest() {
        let mut n = Notifications::default();
        for text in ["a", "b", "c", "d"] {
            n.post(text, palette::WHITE);
        }
        assert_eq!(texts(&n), ["b", "c", "d"]);
    }
}
