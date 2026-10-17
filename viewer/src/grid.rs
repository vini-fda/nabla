//! Desmos-like coordinate grid renderer.
//!
//! Adaptive 1/2/5 x 10^n major-step ladder; minor steps are major/5.
//! Algorithm borrowed from `egui_plot` (MIT/Apache).
//!
//! Pure drawing: takes a `Painter` + `Camera` + `Rect`; no widget state. The grid lies on
//! the plane `z = 0`, so it tilts with the camera.

use egui::{Align2, Color32, FontId, Painter, Pos2, Rect, Stroke, Vec2};

use crate::camera::{Camera, Orbit};

pub struct GridStyle {
    pub minor_color: Color32,
    pub major_color: Color32,
    pub axis_color: Color32,
    pub label_color: Color32,
    pub label_font: FontId,
    /// Label the major lines with their coordinates.
    pub labels: bool,
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
            labels: true,
            target_major_spacing_px: 80.0,
        }
    }

    /// A fainter grid without labels, a backdrop that stays out of the way.
    pub fn faint(v: &egui::Visuals) -> Self {
        let text = v.text_color();
        Self {
            minor_color: text.gamma_multiply(0.05),
            major_color: text.gamma_multiply(0.12),
            axis_color: text.gamma_multiply(0.25),
            labels: false,
            ..Self::from_visuals(v)
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

/// How squarely the plane must face the viewer (the cosine of the angle between its normal
/// and the view) for the grid to be drawn at all, and for it to be drawn at full strength.
/// The grid fades out in between, as the view turns edge on to the plane.
const MIN_FACING: f32 = 0.05;
const FULL_FACING: f32 = 0.3;

/// The grid of the plane `z = 0`, seen through `camera` in `rect`.
pub fn draw_grid(painter: &Painter, camera: &Camera, rect: Rect, style: &GridStyle) {
    if !(rect.width() >= 1.0 && rect.height() >= 1.0) {
        return;
    }
    let facing = camera.orbit.toward_viewer().z.abs();
    if facing < MIN_FACING {
        return;
    }
    let fade = ((facing - MIN_FACING) / (FULL_FACING - MIN_FACING)).min(1.0);
    let [minor_color, major_color, axis_color] =
        [style.minor_color, style.major_color, style.axis_color].map(|c| c.gamma_multiply(fade));

    // The part of the plane on screen, a parallelogram, and its bounding box.
    let corners = [
        rect.left_top(),
        rect.right_top(),
        rect.left_bottom(),
        rect.right_bottom(),
    ]
    .map(|c| camera.screen_to_world(c, rect));
    let Some(corners) = corners.into_iter().collect::<Option<Vec<_>>>() else {
        return;
    };
    let min = corners.iter().copied().fold(Vec2::INFINITY, Vec2::min);
    let max = corners.iter().copied().fold(-Vec2::INFINITY, Vec2::max);
    let (world_min_x, world_max_x) = (min.x as f64, max.x as f64);
    let (world_min_y, world_max_y) = (min.y as f64, max.y as f64);

    // A tilted plane is foreshortened on screen by `facing` across the tilt, and not at all
    // along it: the spacing is a compromise between the two.
    let target_world = (style.target_major_spacing_px / (camera.scale * facing.sqrt())) as f64;
    let major_step = nice_step(target_world);
    let minor_step = major_step / 5.0;

    let x_line = |x: f64, width: f32, color: Color32| {
        let [a, b] = [world_min_y, world_max_y]
            .map(|y| camera.world_to_screen(Vec2::new(x as f32, y as f32), rect));
        painter.line_segment([a, b], Stroke::new(width, color));
    };
    let y_line = |y: f64, width: f32, color: Color32| {
        let [a, b] = [world_min_x, world_max_x]
            .map(|x| camera.world_to_screen(Vec2::new(x as f32, y as f32), rect));
        painter.line_segment([a, b], Stroke::new(width, color));
    };

    // Minors drawn first; majors overlay them at the same x/y, so we don't need
    // to skip-overlap.
    for (step, color) in [(minor_step, minor_color), (major_step, major_color)] {
        for &x in &major_ticks(world_min_x, world_max_x, step) {
            x_line(x, 1.0, color);
        }
        for &y in &major_ticks(world_min_y, world_max_y, step) {
            y_line(y, 1.0, color);
        }
    }

    // Axes on top.
    if (world_min_x..=world_max_x).contains(&0.0) {
        x_line(0.0, 1.5, axis_color);
    }
    if (world_min_y..=world_max_y).contains(&0.0) {
        y_line(0.0, 1.5, axis_color);
    }

    if style.labels {
        draw_labels(painter, camera, rect, style, major_step, (min, max));
    }
}

/// Coordinates of the major lines that are `major_step` apart, along the axes. Seen from
/// straight above, labels of an axis off screen are pinned to the screen's edge; in other
/// views only the labels of points on screen are drawn.
fn draw_labels(
    painter: &Painter,
    camera: &Camera,
    rect: Rect,
    style: &GridStyle,
    major_step: f64,
    (min, max): (Vec2, Vec2),
) {
    let (world_min_x, world_max_x) = (min.x as f64, max.x as f64);
    let (world_min_y, world_max_y) = (min.y as f64, max.y as f64);
    let pinned = camera.orbit.is_near(Orbit::TOP, 1e-4);
    // Where the label of the axis point at `p` goes, if anywhere.
    let place = |p: Pos2| {
        let inside = Pos2::new(
            p.x.clamp_or_low(rect.left() + 4.0, rect.right() - 4.0),
            p.y.clamp_or_low(rect.top() + 4.0, rect.bottom() - 14.0),
        );
        (pinned || inside == p).then_some(inside)
    };
    let label = |p: Pos2, text: String| {
        painter.text(
            p + Vec2::new(3.0, 2.0),
            Align2::LEFT_TOP,
            text,
            style.label_font.clone(),
            style.label_color,
        );
    };

    for &x in &major_ticks(world_min_x, world_max_x, major_step) {
        if x.abs() < major_step * 0.5 {
            continue; // skip origin label; the y-axis label slot covers it
        }
        if let Some(p) = place(camera.world_to_screen(Vec2::new(x as f32, 0.0), rect)) {
            label(p, format_tick(x, major_step));
        }
    }
    for &y in &major_ticks(world_min_y, world_max_y, major_step) {
        if y.abs() < major_step * 0.5 {
            continue;
        }
        if let Some(p) = place(camera.world_to_screen(Vec2::new(0.0, y as f32), rect)) {
            label(p, format_tick(y, major_step));
        }
    }

    // Origin label.
    if let Some(p) = place(camera.world_to_screen(Vec2::ZERO, rect)) {
        label(p, "0".to_owned());
    }
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
