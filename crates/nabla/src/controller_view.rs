//! A wireframe of the controller, drawn with egui's painter over a pure black panel (black on an
//! AMOLED screen is the pixels off), lit by the controller's state: the sticks move in their
//! wells, the face buttons glow in their own colors, the touchpad, the D-pad, the bumpers and
//! the small buttons light while held, and the triggers' gauges fill as far as they are
//! pulled. Above, a callout for each shoulder button and trigger (L1, L2, R1, R2) points down at
//! it with an elbow line, and lights with it. A heading names the controller.
//!
//! The outlines come from `controller_shape`, traced from a photo; the callouts are laid out
//! here, in the same coordinates. Each line is drawn twice, wide and faint under thin and
//! bright, so it glows.

use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, Vec2, pos2, vec2};

use crate::controller_shape as shape;
use crate::gamepad::{Button, State};
use crate::theme::palette;

/// The part of the drawing shown: the controller, and the callouts above it. Symmetric about
/// the controller's axis.
pub(crate) const VIEW: Rect = Rect::from_min_max(pos2(90.0, 10.0), pos2(662.0, 422.0));

/// A callout, on the left (the right's is mirrored): its labels, its box, and its elbow line,
/// which leaves the box's bottom at `from`, goes straight down to the height of `to`, the point
/// it points at, then across to it.
struct Callout {
    label: &'static str,
    right_label: &'static str,
    rect: Rect,
    from: Pos2,
    to: Pos2,
}

/// L1, out to the bumper's outer end, and L2, onto the bumper's top, where the trigger is
/// behind it. L1's box is further out and lower, L2's further in and higher, so that their
/// lines never cross.
const L1: Callout = Callout {
    label: "L1",
    right_label: "R1",
    rect: Rect::from_min_max(pos2(126.0, 53.0), pos2(172.0, 75.0)),
    from: pos2(149.0, 75.0),
    to: pos2(196.5, 113.0),
};
const L2: Callout = Callout {
    label: "L2",
    right_label: "R2",
    rect: Rect::from_min_max(pos2(178.0, 17.0), pos2(254.0, 39.0)),
    from: pos2(192.0, 39.0),
    to: pos2(230.0, 98.0),
};
/// In a box, the room for the label at its outer end; and a trigger's gauge's height.
const LABEL_ROOM: f32 = 30.0;
const GAUGE_HEIGHT: f32 = 8.0;

/// How far a stick's cap shifts in its well, and how far the dot showing where it points goes.
const CAP_TRAVEL: f32 = 8.0;
const DOT_TRAVEL: f32 = shape::WELL_RADIUS - 4.0;
/// The PS button, on the axis between the sticks: its center's height, and its radius.
const PS_Y: f32 = 247.0;
const PS_RADIUS: f32 = 8.0;

/// The drawing's width at most, in points, its margin in the panel, and the height of the heading
/// above it (down to the rule under it).
const MAX_WIDTH: f32 = 420.0;
const PADDING: f32 = 10.0;
const HEADING: f32 = 26.0;

/// The wireframe's color: white. The light bars', which the touchpad lights in too when clicked.
const LINE: Color32 = Color32::from_gray(235);
const LIGHT: Color32 = palette::BLUE;
/// The rule under the heading.
const RULE: Color32 = Color32::from_gray(55);
/// The lines' width and their glow's, in points.
const LINE_WIDTH: f32 = 1.3;
const GLOW_WIDTH: f32 = 5.0;
/// The labels' text size, in points.
const LABEL_SIZE: f32 = 10.0;

/// The panel's size where `width` is available.
pub fn panel_size(width: f32) -> Vec2 {
    let drawing = MAX_WIDTH.min(width - 2.0 * PADDING);
    vec2(
        drawing + 2.0 * PADDING,
        drawing * VIEW.height() / VIEW.width() + HEADING + 2.0 * PADDING,
    )
}

/// Paints the controller in `panel`, lit by `state`, or dim without a controller.
pub fn paint(painter: &Painter, panel: Rect, state: Option<&State>) {
    painter.rect(
        panel,
        12.0,
        Color32::BLACK,
        Stroke::new(1.0, Color32::from_gray(38)),
        egui::StrokeKind::Inside,
    );
    // The heading: the controller's name, and a rule under it.
    let name = state.map_or("No controller", |s| s.name.as_str());
    // On one row, cut short with an ellipsis if wider than the panel.
    let mut job = egui::text::LayoutJob::simple_singleline(
        name.to_owned(),
        egui::FontId::proportional(LABEL_SIZE),
        palette::WHITE,
    );
    job.wrap = egui::text::TextWrapping {
        max_width: panel.width() - 2.0 * PADDING - 2.0,
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('\u{2026}'),
    };
    let galley = painter.layout_job(job);
    let top_left = pos2(
        panel.left() + PADDING + 2.0,
        panel.top() + HEADING / 2.0 + 2.0 - galley.size().y / 2.0,
    );
    painter.galley(top_left, galley, palette::WHITE);
    painter.hline(
        (panel.left() + PADDING)..=(panel.right() - PADDING),
        panel.top() + HEADING,
        Stroke::new(1.0, RULE),
    );

    let drawing = drawing_rect(panel);
    let view = View {
        painter,
        origin: drawing.min,
        scale: drawing.width() / VIEW.width(),
        dim: state.is_none(),
    };
    paint_wireframe(&view, state);

    // The callouts for the bumpers and the triggers.
    let idle = State::default();
    let s = state.unwrap_or(&idle);
    for (right, bumper, trigger, depth) in [
        (
            false,
            Button::LeftTrigger,
            Button::LeftTrigger2,
            s.triggers[0],
        ),
        (
            true,
            Button::RightTrigger,
            Button::RightTrigger2,
            s.triggers[1],
        ),
    ] {
        view.callout(&L1, right, bumper_lit(s, bumper), None);
        let pulled = (depth > 0.0 || s.is_held(trigger)).then_some(palette::PEACH);
        view.callout(&L2, right, pulled, Some(depth));
    }
}

/// The light of a bumper held in `s`.
fn bumper_lit(s: &State, bumper: Button) -> Option<Color32> {
    // Mauve rather than a blue, which would barely stand out from the wireframe's.
    s.is_held(bumper).then_some(palette::MAUVE)
}

/// The controller itself, lit by `state`, or as if idle without a controller.
pub(crate) fn paint_wireframe(view: &View<'_>, state: Option<&State>) {
    let idle = State::default();
    let s = state.unwrap_or(&idle);
    let held = |button| s.is_held(button);

    view.line(points(shape::BODY, false), true, LINE);
    view.line(points(shape::BAND, false), true, LINE.gamma_multiply(0.6));
    view.shape(points(shape::TOUCHPAD, false), s.touchpad.then_some(LIGHT));
    for right in [false, true] {
        view.line(points(shape::LIGHT_BAR, right), false, LIGHT);
    }
    for (right, bumper) in [(false, Button::LeftTrigger), (true, Button::RightTrigger)] {
        view.shape(points(shape::BUMPER, right), bumper_lit(s, bumper));
    }

    buttons(view, s);

    // The sticks: the cap shifts with the stick, and a dot shows where it points.
    for (right, stick, thumb) in [
        (false, s.stick, Button::LeftThumb),
        (true, s.right_stick, Button::RightThumb),
    ] {
        let center = side(at(shape::STICK), right);
        // Screen y points down; the stick's y up.
        let push = vec2(stick[0], -stick[1]);
        view.circle(center, shape::WELL_RADIUS, None);
        view.circle(
            center + push * CAP_TRAVEL,
            shape::CAP_RADIUS,
            held(thumb).then_some(palette::SAPPHIRE),
        );
        view.dot(center + push * DOT_TRAVEL, palette::SAPPHIRE);
    }
}

/// Where the drawing goes in `panel`: under the heading, within the margin.
fn drawing_rect(panel: Rect) -> Rect {
    Rect::from_min_max(
        pos2(panel.left() + PADDING, panel.top() + HEADING + PADDING),
        panel.max - Vec2::splat(PADDING),
    )
}

/// The D-pad, the face buttons, each with its symbol and lit in its color, and the small
/// buttons, lit while `s` holds them.
fn buttons(view: &View<'_>, s: &State) {
    let held = |button| s.is_held(button);
    for (button, arm) in [
        (Button::DPadUp, shape::DPAD_UP),
        (Button::DPadDown, shape::DPAD_DOWN),
        (Button::DPadLeft, shape::DPAD_LEFT),
        (Button::DPadRight, shape::DPAD_RIGHT),
    ] {
        view.shape(points(arm, false), held(button).then_some(LINE));
    }

    let face = at(shape::FACE);
    for (button, offset, color) in [
        (Button::North, vec2(0.0, -1.0), palette::TEAL),
        (Button::West, vec2(-1.0, 0.0), palette::PINK),
        (Button::East, vec2(1.0, 0.0), palette::RED),
        (Button::South, vec2(0.0, 1.0), palette::SKY),
    ] {
        let center = face + offset * shape::FACE_REACH;
        let lit = held(button);
        view.circle(center, shape::FACE_RADIUS, lit.then_some(color));
        let symbol = if lit {
            color
        } else {
            color.gamma_multiply(0.5)
        };
        view.symbol(button, center, symbol);
    }

    for (right, button) in [(false, Button::Select), (true, Button::Start)] {
        view.shape(points(shape::CREATE, right), held(button).then_some(LINE));
    }
    view.circle(
        pos2(shape::AXIS, PS_Y),
        PS_RADIUS,
        held(Button::Mode).then_some(LINE),
    );
    let mute = Rect::from_center_size(at(shape::MUTE), 2.0 * Vec2::from(shape::MUTE_HALF));
    view.rounded(mute, shape::MUTE_HALF[1], None);
}

/// Where to draw: the drawing's coordinates mapped to the screen, and whether to draw dimly.
pub(crate) struct View<'a> {
    pub painter: &'a Painter,
    /// Where `VIEW.min` lands, and the points per drawing unit.
    pub origin: Pos2,
    pub scale: f32,
    /// No controller: everything at half strength.
    pub dim: bool,
}

impl View<'_> {
    fn at(&self, q: Pos2) -> Pos2 {
        self.origin + (q - VIEW.min) * self.scale
    }

    fn color(&self, color: Color32) -> Color32 {
        if self.dim {
            color.gamma_multiply(0.45)
        } else {
            color
        }
    }

    /// A line through `points` (closed or not), with its glow.
    pub fn line(&self, points: Vec<Pos2>, closed: bool, color: Color32) {
        let color = self.color(color);
        let points: Vec<Pos2> = points.into_iter().map(|q| self.at(q)).collect();
        let path = |width: f32, color: Color32| {
            let stroke = Stroke::new(width, color);
            if closed {
                Shape::closed_line(points.clone(), stroke)
            } else {
                Shape::line(points.clone(), stroke)
            }
        };
        self.painter
            .add(path(GLOW_WIDTH, color.gamma_multiply(0.12)));
        self.painter.add(path(LINE_WIDTH, color));
    }

    /// A closed shape through `points`, outlined, and filled with `lit` if given. The fill is
    /// for convex shapes; the traced ones that light (the touchpad, the bumpers, the D-pad's
    /// arms and the small buttons) nearly are.
    fn shape(&self, points: Vec<Pos2>, lit: Option<Color32>) {
        if let Some(fill) = lit {
            let screen = points.iter().map(|&q| self.at(q)).collect();
            self.painter.add(Shape::convex_polygon(
                screen,
                self.color(fill).gamma_multiply(0.35),
                Stroke::NONE,
            ));
        }
        self.line(points, true, lit.unwrap_or(LINE));
    }

    /// A circle, filled with `lit` if given.
    fn circle(&self, center: Pos2, radius: f32, lit: Option<Color32>) {
        let (center, radius) = (self.at(center), radius * self.scale);
        let color = self.color(lit.unwrap_or(LINE));
        if lit.is_some() {
            self.painter
                .circle_filled(center, radius, color.gamma_multiply(0.35));
        }
        self.painter.circle_stroke(
            center,
            radius,
            Stroke::new(GLOW_WIDTH, color.gamma_multiply(0.12)),
        );
        self.painter
            .circle_stroke(center, radius, Stroke::new(LINE_WIDTH, color));
    }

    /// A small bright dot.
    pub fn dot(&self, center: Pos2, color: Color32) {
        let center = self.at(center);
        let color = self.color(color);
        self.painter
            .circle_filled(center, 5.0, color.gamma_multiply(0.2));
        self.painter.circle_filled(center, 2.5, color);
    }

    /// A rounded rectangle, filled with `lit` if given.
    fn rounded(&self, rect: Rect, radius: f32, lit: Option<Color32>) {
        let rect = Rect::from_min_max(self.at(rect.min), self.at(rect.max));
        let radius = radius * self.scale;
        let color = self.color(lit.unwrap_or(LINE));
        if lit.is_some() {
            self.painter
                .rect_filled(rect, radius, color.gamma_multiply(0.35));
        }
        for (width, color) in [
            (GLOW_WIDTH, color.gamma_multiply(0.12)),
            (LINE_WIDTH, color),
        ] {
            self.painter.rect_stroke(
                rect,
                radius,
                Stroke::new(width, color),
                egui::StrokeKind::Middle,
            );
        }
    }

    /// A callout, on the `right` (mirrored) or the left, lit with `lit` if given: its label in
    /// a box at the box's outer end, its elbow line down from the box and across to what it
    /// points at, ending in a dot, and, for a trigger, a gauge in the rest of the box, filled as
    /// far as the trigger is pulled, at `depth` from 0 to 1.
    fn callout(&self, c: &Callout, right: bool, lit: Option<Color32>, depth: Option<f32>) {
        let color = lit.unwrap_or_else(|| LINE.gamma_multiply(0.7));
        let corner = pos2(c.from.x, c.to.y);
        let elbow = [c.from, corner, c.to].map(|q| side(q, right));
        self.line(elbow.to_vec(), false, color);
        self.dot(side(c.to, right), color);

        let rect = Rect::from_two_pos(side(c.rect.min, right), side(c.rect.max, right));
        self.rounded(rect, 6.0, lit.map(|l| l.gamma_multiply(0.5)));
        let (outer, inward) = if right {
            (rect.right(), -1.0)
        } else {
            (rect.left(), 1.0)
        };
        self.painter.text(
            self.at(pos2(outer + inward * LABEL_ROOM / 2.0, rect.center().y)),
            egui::Align2::CENTER_CENTER,
            if right { c.right_label } else { c.label },
            egui::FontId::monospace(LABEL_SIZE),
            self.color(lit.unwrap_or(palette::OVERLAY1)),
        );

        let Some(depth) = depth else {
            return;
        };
        let start = outer + inward * LABEL_ROOM;
        let end = if right {
            rect.left() + 6.0
        } else {
            rect.right() - 6.0
        };
        let y = rect.center().y;
        let track = Rect::from_two_pos(
            pos2(start, y - GAUGE_HEIGHT / 2.0),
            pos2(end, y + GAUGE_HEIGHT / 2.0),
        );
        self.rounded(track, GAUGE_HEIGHT / 2.0, None);
        let depth = depth.clamp(0.0, 1.0);
        if depth > 0.0 {
            let reach = start + (end - start) * depth;
            let filled = Rect::from_two_pos(
                self.at(pos2(start, track.top())),
                self.at(pos2(reach, track.bottom())),
            );
            self.painter.rect_filled(
                filled,
                GAUGE_HEIGHT / 2.0 * self.scale,
                self.color(palette::PEACH).gamma_multiply(0.4 + 0.6 * depth),
            );
        }
    }

    /// A face button's symbol, centered on `center`.
    fn symbol(&self, button: Button, center: Pos2, color: Color32) {
        use std::f32::consts::TAU;
        const SIZE: f32 = 0.5 * shape::FACE_RADIUS;
        match button {
            Button::North => {
                let corner = |i: u8| {
                    let angle = -TAU / 4.0 + f32::from(i) * TAU / 3.0;
                    center + Vec2::angled(angle) * SIZE
                };
                self.line((0..3).map(corner).collect(), true, color);
            }
            Button::West => {
                let r = Rect::from_center_size(center, Vec2::splat(1.5 * SIZE));
                let corners = [
                    r.left_top(),
                    r.right_top(),
                    r.right_bottom(),
                    r.left_bottom(),
                ];
                self.line(corners.to_vec(), true, color);
            }
            Button::East => {
                let ring =
                    (0..24).map(|i| center + Vec2::angled(i as f32 * TAU / 24.0) * (0.85 * SIZE));
                self.line(ring.collect(), true, color);
            }
            _ => {
                let d = 0.75 * SIZE;
                self.line(vec![center - vec2(d, d), center + vec2(d, d)], false, color);
                self.line(
                    vec![center + vec2(-d, d), center + vec2(d, -d)],
                    false,
                    color,
                );
            }
        }
    }
}

/// A traced point.
fn at([x, y]: [f32; 2]) -> Pos2 {
    pos2(x, y)
}

/// Traced points, mirrored for the `right`.
fn points(traced: &[[f32; 2]], right: bool) -> Vec<Pos2> {
    traced.iter().map(|&q| side(at(q), right)).collect()
}

/// `q`, mirrored about the controller's axis for the `right`.
fn side(q: Pos2, right: bool) -> Pos2 {
    if right {
        pos2(2.0 * shape::AXIS - q.x, q.y)
    } else {
        q
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_symmetric_outlines_are_symmetric() {
        for (name, outline) in [
            ("body", shape::BODY),
            ("band", shape::BAND),
            ("touchpad", shape::TOUCHPAD),
        ] {
            let pts = points(outline, false);
            for &q in &pts {
                let nearest = pts
                    .iter()
                    .map(|&p| (p - side(q, true)).length())
                    .fold(f32::INFINITY, f32::min);
                assert!(nearest < 0.15, "{name}: {q:?} has its mirror image");
            }
        }
    }

    #[test]
    fn the_drawing_and_its_callouts_fit_the_view() {
        let traced = [
            shape::BODY,
            shape::BUMPER,
            shape::TOUCHPAD,
            shape::DPAD_LEFT,
            shape::CREATE,
        ];
        let corners = [L1.rect.min, L1.rect.max, L2.rect.min, L2.rect.max];
        for q in traced.iter().flat_map(|t| points(t, false)).chain(corners) {
            assert!(
                VIEW.contains(q) && VIEW.contains(side(q, true)),
                "{q:?} in view"
            );
        }
    }

    #[test]
    fn the_callouts_lines_go_down_then_across_and_do_not_cross() {
        for c in [&L1, &L2] {
            assert!(
                (c.from.y - c.rect.bottom()).abs() < 1e-3,
                "{}: from its box",
                c.label
            );
            assert!(
                c.rect.x_range().contains(c.from.x),
                "{}: under its box",
                c.label
            );
            assert!(
                c.to.y > c.from.y && c.to.x > c.from.x,
                "{}: down, then right",
                c.label
            );
        }
        // L2's line comes down inside L1's, above L1's box and turn: they never meet.
        assert!(L2.from.x > L1.rect.right() && L2.to.y < L1.to.y);
    }

    #[test]
    fn the_panel_keeps_the_drawing_in_proportion() {
        let panel = Rect::from_min_size(Pos2::ZERO, panel_size(1000.0));
        let inner = drawing_rect(panel);
        let ratio = inner.width() / inner.height();
        assert!(
            (ratio - VIEW.width() / VIEW.height()).abs() < 1e-3,
            "in proportion"
        );
        assert!(panel_size(200.0).x <= 200.0, "shrinks to fit");
        let mirrored = side(pos2(VIEW.left(), 0.0), true).x;
        assert!(
            (mirrored - VIEW.right()).abs() < 1e-3,
            "the view centered on the axis"
        );
    }
}
