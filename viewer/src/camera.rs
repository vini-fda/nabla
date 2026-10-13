//! 2D view camera: maps between world coordinates (y-up) and screen pixels
//! (y-down). Supports panning and cursor-anchored zoom.
//!
//! World convention: positive y points up (matches the ship simulation).
//! Screen convention: positive y points down (egui).

use egui::{Pos2, Rect, Vec2};

const MIN_SCALE: f32 = 0.01;
const MAX_SCALE: f32 = 1000.0;

#[derive(Debug, Clone, Copy, serde::Deserialize, serde::Serialize)]
pub struct Camera {
    /// World coordinates that map to the screen rect's center.
    pub center: Vec2,
    /// Screen pixels per world unit. Always positive.
    pub scale: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            center: Vec2::ZERO,
            scale: 4.0,
        }
    }
}

impl Camera {
    pub fn world_to_screen(&self, world: Vec2, rect: Rect) -> Pos2 {
        let c = rect.center();
        Pos2::new(
            c.x + (world.x - self.center.x) * self.scale,
            c.y - (world.y - self.center.y) * self.scale,
        )
    }

    pub fn screen_to_world(&self, screen: Pos2, rect: Rect) -> Vec2 {
        let c = rect.center();
        Vec2::new(
            self.center.x + (screen.x - c.x) / self.scale,
            self.center.y - (screen.y - c.y) / self.scale,
        )
    }

    /// Translate the view by a screen-space drag delta (egui convention:
    /// positive y is downward). The scene appears to follow the cursor.
    pub fn pan_by_screen(&mut self, delta: Vec2) {
        self.center.x -= delta.x / self.scale;
        self.center.y += delta.y / self.scale;
    }

    /// Multiply scale by `factor`, keeping the world point under
    /// `screen_anchor` fixed on screen.
    pub fn zoom_at(&mut self, factor: f32, screen_anchor: Pos2, rect: Rect) {
        if !factor.is_finite() || factor <= 0.0 {
            return;
        }
        let world_anchor = self.screen_to_world(screen_anchor, rect);
        self.scale = (self.scale * factor).clamp(MIN_SCALE, MAX_SCALE);
        let c = rect.center();
        self.center.x = world_anchor.x - (screen_anchor.x - c.x) / self.scale;
        self.center.y = world_anchor.y + (screen_anchor.y - c.y) / self.scale;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect() -> Rect {
        Rect::from_min_size(Pos2::new(10.0, 20.0), egui::vec2(800.0, 600.0))
    }

    fn assert_near(a: Vec2, b: Vec2, eps: f32) {
        assert!((a - b).length() < eps, "expected {a:?} ~ {b:?}");
    }

    #[test]
    fn round_trip_world_screen() {
        let cam = Camera {
            center: Vec2::new(3.0, -2.5),
            scale: 25.0,
        };
        let r = rect();
        for &(x, y) in &[(0.0, 0.0), (1.5, 4.2), (-10.0, 7.7), (123.456, -78.9)] {
            let w = Vec2::new(x, y);
            let s = cam.world_to_screen(w, r);
            let w2 = cam.screen_to_world(s, r);
            assert_near(w, w2, 1e-3);
        }
    }

    #[test]
    fn zoom_anchor_is_fixed_point() {
        let mut cam = Camera::default();
        let r = rect();
        let anchor = Pos2::new(200.0, 150.0);
        let world_before = cam.screen_to_world(anchor, r);
        cam.zoom_at(2.5, anchor, r);
        let world_after = cam.screen_to_world(anchor, r);
        assert_near(world_before, world_after, 1e-3);
    }

    #[test]
    fn pan_is_invertible() {
        let mut cam = Camera::default();
        let before = cam.center;
        cam.pan_by_screen(Vec2::new(40.0, -30.0));
        cam.pan_by_screen(Vec2::new(-40.0, 30.0));
        assert_near(cam.center, before, 1e-5);
    }

    #[test]
    fn scale_is_clamped() {
        let r = rect();
        let mut cam = Camera::default();
        cam.zoom_at(1e9, Pos2::new(100.0, 100.0), r);
        assert!(cam.scale <= MAX_SCALE);
        cam.zoom_at(1e-9, Pos2::new(100.0, 100.0), r);
        assert!(cam.scale >= MIN_SCALE);
    }

    #[test]
    fn y_axis_is_world_up() {
        let cam = Camera::default();
        let r = rect();
        let s_origin = cam.world_to_screen(Vec2::ZERO, r);
        let s_up = cam.world_to_screen(Vec2::new(0.0, 1.0), r);
        assert!(s_up.y < s_origin.y, "world +y should map to screen up");
    }
}
