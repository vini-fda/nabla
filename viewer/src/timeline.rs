//! After Effects-style timeline with a playhead and a mark at every controller sample.
//!
//! Time labels on top; a thin yellow mark at every controller sample `t_k = k h`. Click or drag
//! to seek, scroll to zoom around the cursor, scroll horizontally to pan, double-click to show the
//! whole run.

use egui::{Color32, Rect, Sense, Stroke};

use crate::grid::{format_tick, major_ticks, nice_step};

const HEIGHT: f32 = 44.0;
const LABEL_HEIGHT: f32 = 16.0;
const SAMPLE_COLOR: Color32 = Color32::from_rgb(230, 180, 40);
const PLAYHEAD_COLOR: Color32 = Color32::from_rgb(40, 90, 200);
const PLAYHEAD_HANDLE_PX: f32 = 6.0;

/// Minimum spacing between drawn sample marks. Denser samples are thinned to every few.
const MIN_MARK_SPACING_PX: f32 = 3.0;
/// Target spacing between time labels.
const LABEL_SPACING_PX: f64 = 90.0;

/// The visible time range `[start, end]` of the timeline.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimelineView {
    pub start: f64,
    pub end: f64,
}

impl Default for TimelineView {
    fn default() -> Self {
        Self {
            start: 0.0,
            end: 1.0,
        }
    }
}

impl TimelineView {
    /// The whole run `[0, duration]`, with a nonzero width even for an empty run.
    pub fn full(duration: f64, h: f64) -> Self {
        Self {
            start: 0.0,
            end: duration.max(h),
        }
    }

    pub fn span(&self) -> f64 {
        self.end - self.start
    }

    /// Zoom by `factor` (> 1 zooms in) keeping time `anchor` fixed, within `[0, duration]`, and
    /// never narrower than half a sample.
    pub fn zoom_at(&mut self, factor: f64, anchor: f64, duration: f64, h: f64) {
        let full = Self::full(duration, h);
        let span = (self.span() / factor).clamp((0.5 * h).min(full.span()), full.span());
        let frac = (anchor - self.start) / self.span();
        self.start = anchor - frac * span;
        self.end = self.start + span;
        self.clamp(duration, h);
    }

    /// Shift by `dt` seconds, within `[0, duration]`.
    pub fn pan(&mut self, dt: f64, duration: f64, h: f64) {
        self.start += dt;
        self.end += dt;
        self.clamp(duration, h);
    }

    /// Scroll so that `t` is visible, paging forward like a timeline during playback.
    pub fn follow(&mut self, t: f64, duration: f64, h: f64) {
        let span = self.span();
        if t < self.start || t > self.end {
            self.start = t - 0.1 * span;
            self.end = self.start + span;
            self.clamp(duration, h);
        }
    }

    fn clamp(&mut self, duration: f64, h: f64) {
        let full = Self::full(duration, h);
        let span = self.span().min(full.span());
        self.start = self.start.clamp(full.start, full.end - span);
        self.end = self.start + span;
    }

    fn t_to_x(&self, t: f64, rect: Rect) -> f32 {
        rect.left() + ((t - self.start) / self.span()) as f32 * rect.width()
    }

    fn x_to_t(&self, x: f32, rect: Rect) -> f64 {
        self.start + f64::from((x - rect.left()) / rect.width()) * self.span()
    }
}

/// Draw every `stride`-th sample so marks are at least `min_px` apart.
pub fn mark_stride(h: f64, px_per_second: f64, min_px: f64) -> usize {
    let spacing = h * px_per_second;
    if spacing >= min_px || spacing <= 0.0 {
        1
    } else {
        (min_px / spacing).ceil() as usize
    }
}

/// Draw the timeline across the available width. Returns the time to seek to, if the user
/// clicked or dragged on it.
pub fn timeline_ui(
    ui: &mut egui::Ui,
    view: &mut TimelineView,
    duration: f64,
    h: f64,
    t_now: f64,
) -> Option<f64> {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), HEIGHT),
        Sense::click_and_drag(),
    );

    if response.double_clicked() {
        *view = TimelineView::full(duration, h);
    }
    if let Some(pos) = response.hover_pos() {
        let (scroll, pinch) = ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta()));
        let factor = f64::from(pinch * (scroll.y / 200.0).exp());
        if factor.is_finite() && factor > 0.0 && factor != 1.0 {
            view.zoom_at(factor, view.x_to_t(pos.x, rect), duration, h);
        }
        if scroll.x != 0.0 {
            let dt = -f64::from(scroll.x / rect.width()) * view.span();
            view.pan(dt, duration, h);
        }
    }

    draw(ui, rect, view, h, t_now);

    let seek = (response.clicked() || response.dragged())
        .then(|| response.interact_pointer_pos())
        .flatten()
        .map(|pos| view.x_to_t(pos.x, rect).clamp(0.0, duration));
    response.on_hover_text(
        "Click/drag: seek   Scroll/pinch: zoom   Shift+scroll: pan   Double-click: whole run\n\
         Yellow marks: controller samples t = k·h",
    );
    seek
}

fn draw(ui: &egui::Ui, rect: Rect, view: &TimelineView, h: f64, t_now: f64) {
    let painter = ui.painter_at(rect);
    let visuals = ui.visuals();
    painter.rect_filled(rect, 2.0, visuals.extreme_bg_color);
    let marks = Rect::from_min_max(
        rect.left_top() + egui::vec2(0.0, LABEL_HEIGHT),
        rect.right_bottom(),
    );
    painter.rect_filled(marks, 0.0, visuals.faint_bg_color);

    // Time labels.
    let px_per_second = f64::from(rect.width()) / view.span();
    let label_step = nice_step(LABEL_SPACING_PX / px_per_second);
    for t in major_ticks(view.start, view.end, label_step) {
        let x = view.t_to_x(t, rect);
        painter.line_segment(
            [egui::pos2(x, marks.top() - 4.0), egui::pos2(x, marks.top())],
            Stroke::new(1.0_f32, visuals.text_color()),
        );
        painter.text(
            egui::pos2(x + 2.0, rect.top()),
            egui::Align2::LEFT_TOP,
            format!("{} s", format_tick(t, label_step)),
            egui::FontId::monospace(10.0),
            visuals.text_color(),
        );
    }

    // Controller samples.
    let stride = mark_stride(h, px_per_second, f64::from(MIN_MARK_SPACING_PX));
    let first = (view.start / h).ceil().max(0.0) as usize;
    let last = (view.end / h).floor().max(0.0) as usize;
    let first = first.div_ceil(stride) * stride;
    for k in (first..=last).step_by(stride) {
        let x = view.t_to_x(k as f64 * h, rect);
        painter.line_segment(
            [egui::pos2(x, marks.top()), egui::pos2(x, marks.bottom())],
            Stroke::new(1.0_f32, SAMPLE_COLOR),
        );
    }
    if stride > 1 {
        painter.text(
            marks.right_top() + egui::vec2(-4.0, 2.0),
            egui::Align2::RIGHT_TOP,
            format!("every {stride} samples (scroll to zoom)"),
            egui::FontId::proportional(10.0),
            visuals.weak_text_color(),
        );
    }

    // Playhead.
    let x = view.t_to_x(t_now, rect);
    if rect.left() - 1.0 <= x && x <= rect.right() + 1.0 {
        painter.line_segment(
            [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
            Stroke::new(2.0_f32, PLAYHEAD_COLOR),
        );
        painter.add(egui::Shape::convex_polygon(
            vec![
                egui::pos2(x - PLAYHEAD_HANDLE_PX, marks.top() - PLAYHEAD_HANDLE_PX),
                egui::pos2(x + PLAYHEAD_HANDLE_PX, marks.top() - PLAYHEAD_HANDLE_PX),
                egui::pos2(x, marks.top()),
            ],
            PLAYHEAD_COLOR,
            Stroke::NONE,
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: f64 = 0.1;

    #[test]
    fn full_view_has_nonzero_width() {
        assert!(
            (TimelineView::full(0.0, H).span() - H).abs() < 1e-12,
            "empty run"
        );
        assert!(
            (TimelineView::full(21.8, H).span() - 21.8).abs() < 1e-12,
            "whole run"
        );
    }

    #[test]
    fn zoom_keeps_the_anchor_and_stays_inside() {
        let mut v = TimelineView::full(20.0, H);
        v.zoom_at(4.0, 10.0, 20.0, H);
        assert!((v.span() - 5.0).abs() < 1e-12, "zoomed in 4x");
        assert!(
            (v.start - 7.5).abs() < 1e-12,
            "anchor kept at the same fraction"
        );
        v.zoom_at(1e9, 10.0, 20.0, H);
        assert!((v.span() - 0.05).abs() < 1e-12, "at most half a sample");
        v.zoom_at(1e-9, 10.0, 20.0, H);
        assert_eq!(
            v,
            TimelineView::full(20.0, H),
            "zooming out stops at the whole run"
        );
    }

    #[test]
    fn pan_and_follow_stay_inside() {
        let mut v = TimelineView {
            start: 0.0,
            end: 5.0,
        };
        v.pan(100.0, 20.0, H);
        assert_eq!((v.start, v.end), (15.0, 20.0), "clamped at the end");
        v.follow(2.0, 20.0, H);
        assert!(v.start <= 2.0 && 2.0 <= v.end, "playhead visible");
        assert!((v.span() - 5.0).abs() < 1e-12, "span kept");
    }

    #[test]
    fn marks_are_thinned_when_dense() {
        assert_eq!(mark_stride(H, 100.0, 3.0), 1, "10 px apart");
        assert_eq!(mark_stride(H, 10.0, 3.0), 3, "1 px apart → every 3rd");
        assert_eq!(mark_stride(H, 0.0, 3.0), 1, "degenerate");
    }
}
