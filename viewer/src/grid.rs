//! Desmos-like coordinate grid renderer.
//!
//! Adaptive 1/2/5 x 10^n major-step ladder; minor steps are major/5.
//! Algorithm borrowed from `egui_plot` (MIT/Apache).
//!
//! Pure drawing: takes a `Painter` + `Camera` + `Rect`; no widget state.

use egui::{Align2, Color32, FontId, Painter, Pos2, Rect, Stroke, Vec2};

use crate::camera::Camera;

pub struct GridStyle {
    pub minor_color: Color32,
    pub major_color: Color32,
    pub axis_color: Color32,
    pub label_color: Color32,
    pub label_font: FontId,
    /// Target screen-pixel spacing between adjacent major lines.
    pub target_major_spacing_px: f32,
}

impl GridStyle {
    /// Derive a theme-aware style from the active egui visuals.
    pub fn from_visuals(v: &egui::Visuals) -> Self {
        let text = v.text_color();
        Self {
            minor_color: text.gamma_multiply(0.10),
            major_color: text.gamma_multiply(0.25),
            axis_color: text.gamma_multiply(0.65),
            label_color: text,
            label_font: FontId::monospace(11.0),
            target_major_spacing_px: 80.0,
        }
    }
}

/// Like `f32::clamp`, but returns `low` instead of panicking when `low > high`, which happens
/// when the canvas is narrower than the label margins (e.g. on a phone).
trait ClampOrLow {
    fn clamp_or_low(self, low: f32, high: f32) -> f32;
}

impl ClampOrLow for f32 {
    fn clamp_or_low(self, low: f32, high: f32) -> f32 {
        self.min(high).max(low)
    }
}

pub fn draw_grid(painter: &Painter, camera: &Camera, rect: Rect, style: &GridStyle) {
    if !(rect.width() >= 1.0 && rect.height() >= 1.0) {
        return;
    }
    let bottom_left_world = camera.screen_to_world(rect.left_bottom(), rect);
    let top_right_world = camera.screen_to_world(rect.right_top(), rect);
    let world_min_x = bottom_left_world.x;
    let world_max_x = top_right_world.x;
    let world_min_y = bottom_left_world.y;
    let world_max_y = top_right_world.y;

    let target_world = (style.target_major_spacing_px / camera.scale) as f64;
    let major_step = nice_step(target_world);
    let minor_step = major_step / 5.0;

    // Minors drawn first; majors overlay them at the same x/y, so we don't need
    // to skip-overlap.
    for &x in &major_ticks(world_min_x as f64, world_max_x as f64, minor_step) {
        let p = camera.world_to_screen(Vec2::new(x as f32, 0.0), rect);
        painter.line_segment(
            [Pos2::new(p.x, rect.top()), Pos2::new(p.x, rect.bottom())],
            Stroke::new(1.0_f32, style.minor_color),
        );
    }
    for &y in &major_ticks(world_min_y as f64, world_max_y as f64, minor_step) {
        let p = camera.world_to_screen(Vec2::new(0.0, y as f32), rect);
        painter.line_segment(
            [Pos2::new(rect.left(), p.y), Pos2::new(rect.right(), p.y)],
            Stroke::new(1.0_f32, style.minor_color),
        );
    }
    for &x in &major_ticks(world_min_x as f64, world_max_x as f64, major_step) {
        let p = camera.world_to_screen(Vec2::new(x as f32, 0.0), rect);
        painter.line_segment(
            [Pos2::new(p.x, rect.top()), Pos2::new(p.x, rect.bottom())],
            Stroke::new(1.0_f32, style.major_color),
        );
    }
    for &y in &major_ticks(world_min_y as f64, world_max_y as f64, major_step) {
        let p = camera.world_to_screen(Vec2::new(0.0, y as f32), rect);
        painter.line_segment(
            [Pos2::new(rect.left(), p.y), Pos2::new(rect.right(), p.y)],
            Stroke::new(1.0_f32, style.major_color),
        );
    }

    // Axes on top.
    if (world_min_x..=world_max_x).contains(&0.0) {
        let p = camera.world_to_screen(Vec2::ZERO, rect);
        painter.line_segment(
            [Pos2::new(p.x, rect.top()), Pos2::new(p.x, rect.bottom())],
            Stroke::new(1.5_f32, style.axis_color),
        );
    }
    if (world_min_y..=world_max_y).contains(&0.0) {
        let p = camera.world_to_screen(Vec2::ZERO, rect);
        painter.line_segment(
            [Pos2::new(rect.left(), p.y), Pos2::new(rect.right(), p.y)],
            Stroke::new(1.5_f32, style.axis_color),
        );
    }

    // Labels along the visible x-axis (or pinned to bottom edge if y=0 is offscreen)
    // and y-axis (or left edge).
    let x_label_screen_y = camera
        .world_to_screen(Vec2::ZERO, rect)
        .y
        .clamp_or_low(rect.top() + 4.0, rect.bottom() - 14.0);
    let y_label_screen_x = camera
        .world_to_screen(Vec2::ZERO, rect)
        .x
        .clamp_or_low(rect.left() + 4.0, rect.right() - 4.0);

    for &x in &major_ticks(world_min_x as f64, world_max_x as f64, major_step) {
        if x.abs() < major_step * 0.5 {
            continue; // skip origin label; the y-axis label slot covers it
        }
        let p = camera.world_to_screen(Vec2::new(x as f32, 0.0), rect);
        painter.text(
            Pos2::new(p.x + 3.0, x_label_screen_y + 2.0),
            Align2::LEFT_TOP,
            format_tick(x, major_step),
            style.label_font.clone(),
            style.label_color,
        );
    }
    for &y in &major_ticks(world_min_y as f64, world_max_y as f64, major_step) {
        if y.abs() < major_step * 0.5 {
            continue;
        }
        let p = camera.world_to_screen(Vec2::new(0.0, y as f32), rect);
        painter.text(
            Pos2::new(y_label_screen_x + 3.0, p.y + 2.0),
            Align2::LEFT_TOP,
            format_tick(y, major_step),
            style.label_font.clone(),
            style.label_color,
        );
    }

    // Origin label.
    let o = camera.world_to_screen(Vec2::ZERO, rect);
    painter.text(
        Pos2::new(
            o.x.clamp_or_low(rect.left() + 4.0, rect.right() - 4.0) + 3.0,
            o.y.clamp_or_low(rect.top() + 4.0, rect.bottom() - 14.0) + 2.0,
        ),
        Align2::LEFT_TOP,
        "0",
        style.label_font.clone(),
        style.label_color,
    );
}

pub(crate) fn nice_step(target: f64) -> f64 {
    if !target.is_finite() || target <= 0.0 {
        return 1.0;
    }
    let exp = target.log10().floor();
    let f = target / 10f64.powf(exp);
    let nice = if f < 1.5 {
        1.0
    } else if f < 3.5 {
        2.0
    } else if f < 7.5 {
        5.0
    } else {
        10.0
    };
    nice * 10f64.powf(exp)
}

pub(crate) fn major_ticks(min: f64, max: f64, step: f64) -> Vec<f64> {
    if !step.is_finite() || step <= 0.0 || min > max {
        return Vec::new();
    }
    let first = (min / step).ceil() as i64;
    let last = (max / step).floor() as i64;
    if last < first {
        return Vec::new();
    }
    // Cap to avoid pathological draws on extreme zoom.
    let n = (last - first + 1).min(10_000);
    (0..n).map(|i| (first + i) as f64 * step).collect()
}

pub(crate) fn format_tick(value: f64, step: f64) -> String {
    let decimals = (-(step.log10().floor() as i32)).max(0) as usize;
    format!("{value:.decimals$}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_or_low_never_panics() {
        assert_eq!(5.0_f32.clamp_or_low(0.0, 10.0), 5.0, "inside");
        assert_eq!(15.0_f32.clamp_or_low(0.0, 10.0), 10.0, "above");
        // The phone case: an empty canvas gives low > high.
        assert_eq!(
            304.0_f32.clamp_or_low(308.125, 300.125),
            308.125,
            "empty range"
        );
    }

    #[test]
    fn nice_step_ladder() {
        // Boundaries: f in [1, 1.5) -> 1, [1.5, 3.5) -> 2, [3.5, 7.5) -> 5, [7.5, 10) -> 10
        let cases = [
            (1.0, 1.0),
            (1.4, 1.0),
            (1.6, 2.0),
            (3.4, 2.0),
            (3.6, 5.0),
            (7.4, 5.0),
            (7.6, 10.0),
            (0.7, 0.5), // f=7 -> 5
            (23.0, 20.0),
            (0.23, 0.2),
            (0.07, 0.05),
            (250.0, 200.0),
        ];
        for (input, want) in cases {
            let got = nice_step(input);
            assert!(
                (got - want).abs() / want < 1e-9,
                "nice_step({input}) = {got}, want {want}"
            );
        }
    }

    #[test]
    fn nice_step_handles_degenerates() {
        assert_eq!(nice_step(0.0), 1.0);
        assert_eq!(nice_step(-3.0), 1.0);
        assert_eq!(nice_step(f64::NAN), 1.0);
        assert_eq!(nice_step(f64::INFINITY), 1.0);
    }

    #[test]
    fn major_ticks_inclusive() {
        let ticks = major_ticks(-3.4, 7.1, 1.0);
        assert_eq!(ticks, (-3..=7).map(|i| i as f64).collect::<Vec<_>>());
    }

    #[test]
    fn major_ticks_step_alignment() {
        let ticks = major_ticks(-10.0, 10.0, 5.0);
        assert_eq!(ticks, vec![-10.0, -5.0, 0.0, 5.0, 10.0]);
    }

    #[test]
    fn major_ticks_empty_when_range_smaller_than_step() {
        let ticks = major_ticks(0.1, 0.4, 1.0);
        assert!(ticks.is_empty());
    }

    #[test]
    fn format_tick_decimal_width_follows_step() {
        assert_eq!(format_tick(1.0, 1.0), "1");
        assert_eq!(format_tick(100.0, 100.0), "100");
        assert_eq!(format_tick(0.2, 0.1), "0.2");
        assert_eq!(format_tick(0.05, 0.01), "0.05");
    }
}
