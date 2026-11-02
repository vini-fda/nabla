//! The game controller's guide: the wireframe of `controller_view`, with a callout for every
//! control in use, each its icon and what it does, joined to the control by an elbow line. All of
//! them light while the control is used. Too narrow for that, it is a list of the same.
//!
//! The icons are from "PS5 Button Icons and Controls" by Zacksly (CC BY 3.0,
//! <https://zacksly.itch.io>), in `assets/controller_icons/`: outlined, and solid while lit.

use std::borrow::Cow;

use egui::{Color32, Pos2, Rect, Stroke, Ui, Vec2, pos2};

use crate::controller_view::{self as view, View};
use crate::game;
use crate::gamepad::{Button, State};
use crate::theme::palette;

/// One of the pack's icons: white, for tinting.
pub struct Icon {
    outline: (&'static str, &'static [u8]),
    solid: (&'static str, &'static [u8]),
}

macro_rules! icon {
    ($name:literal) => {
        Icon {
            outline: (
                concat!("bytes://controller_icons/outline/", $name, ".svg"),
                include_bytes!(concat!(
                    "../assets/controller_icons/outline/",
                    $name,
                    ".svg"
                )),
            ),
            solid: (
                concat!("bytes://controller_icons/solid/", $name, ".svg"),
                include_bytes!(concat!("../assets/controller_icons/solid/", $name, ".svg")),
            ),
        }
    };
}

pub const CROSS: Icon = icon!("cross");
pub const CIRCLE: Icon = icon!("circle");
pub const SQUARE: Icon = icon!("square");
const TRIANGLE: Icon = icon!("triangle");
const L1: Icon = icon!("l1");
pub const L2: Icon = icon!("l2");
const R1: Icon = icon!("r1");
const R2: Icon = icon!("r2");
const OPTIONS: Icon = icon!("options");
const CREATE: Icon = icon!("create");
pub const DPAD_UP: Icon = icon!("dpad-up");
pub const DPAD_DOWN: Icon = icon!("dpad-down");
pub const DPAD_LEFT: Icon = icon!("dpad-left");
pub const DPAD_RIGHT: Icon = icon!("dpad-right");
const LEFT_STICK: Icon = icon!("left-stick-left-right");
const RIGHT_STICK: Icon = icon!("right-stick-all");
pub const RIGHT_STICK_CLICK: Icon = icon!("right-stick-click");

impl Icon {
    fn image(&self, lit: bool) -> egui::Image<'static> {
        let (uri, svg) = if lit { self.solid } else { self.outline };
        egui::Image::from_bytes(uri, svg)
    }

    /// The icon, as big as a row of `ui`'s text, in place of a key's name.
    pub fn inline(&self, ui: &Ui) -> egui::Image<'static> {
        self.image(false)
            .fit_to_exact_size(Vec2::splat(ui.spacing().interact_size.y))
            .tint(palette::WHITE)
    }
}

/// [`Icon::inline`], on its own.
pub fn icon_ui(ui: &mut Ui, icon: &Icon) {
    ui.add(icon.inline(ui));
}

/// The map, in the units of the wireframe's outlines: the controller in the middle, at
/// [`view::VIEW`], the labels at either side, past [`LEFT_EDGE`] and [`RIGHT_EDGE`].
const MAP: Rect = Rect::from_min_max(pos2(-239.0, -5.0), pos2(991.0, 445.0));
const LEFT_EDGE: f32 = 32.0;
const RIGHT_EDGE: f32 = 720.0;
/// Points per unit at the map's full size.
const FULL_SCALE: f32 = 0.7;
/// The map's width at full size, in points. It scales to the width it's given.
pub const WIDTH: f32 = 861.0;
/// Narrowest the map gets, at which its text is still legible, in points: below, it's a list.
const MIN_WIDTH: f32 = 560.0;
/// At the map's full size, in points: its text, a callout's icon, and the gap beside it.
const TEXT_SIZE: f32 = 12.0;
const ICON_SIZE: f32 = 30.0;
const GAP: f32 = 6.0;
/// How far a stick must be pushed to count as used.
const STICK_USED: f32 = 0.15;

const LINE: Color32 = Color32::from_gray(235);

/// What lights a row.
#[derive(Clone, Copy)]
enum Input {
    Held(Button),
    /// Either of two.
    EitherHeld(Button, Button),
    /// The trigger on that side (0 left, 1 right), with a gauge for how far it's pulled.
    Trigger(usize, Button),
    LeftStickSideways,
    RightStick,
}

impl Input {
    fn lit(self, s: &State) -> bool {
        match self {
            Self::Held(button) => s.is_held(button),
            Self::EitherHeld(one, other) => s.is_held(one) || s.is_held(other),
            Self::Trigger(side, button) => s.triggers[side] > 0.0 || s.is_held(button),
            Self::LeftStickSideways => s.stick[0].abs() >= STICK_USED,
            Self::RightStick => Vec2::from(s.right_stick).length() >= STICK_USED,
        }
    }
}

/// A row's icon, solid while `button` is held, or else while the row is lit.
struct Part {
    icon: Icon,
    button: Option<Button>,
}

/// An icon of one control, or of a direction of the D-pad, which lights on its own.
const fn part(icon: Icon) -> Part {
    Part { icon, button: None }
}
const fn dpad(icon: Icon, button: Button) -> Part {
    Part {
        icon,
        button: Some(button),
    }
}

/// A control and what it does.
struct Row {
    parts: Vec<Part>,
    action: Cow<'static, str>,
    input: Input,
    color: Color32,
    /// Which side of the controller its label is.
    right: bool,
    /// The height of the label's middle, where its line leaves it.
    y: f32,
    /// The line: across at `y` to `lane`, along it to the height `via`, across to `to`'s x, and
    /// along that to `to`, where it ends in a dot.
    lane: f32,
    via: f32,
    to: Pos2,
}

impl Row {
    /// The points of the elbow line.
    fn path(&self) -> Vec<Pos2> {
        let edge = if self.right { RIGHT_EDGE } else { LEFT_EDGE };
        vec![
            pos2(edge, self.y),
            pos2(self.lane, self.y),
            pos2(self.lane, self.via),
            pos2(self.to.x, self.via),
            self.to,
        ]
    }

    /// Whether `s` is using this control.
    fn lit(&self, s: &State) -> bool {
        self.input.lit(s)
    }

    /// How far the row's trigger is pulled, from 0 to 1, if it's a trigger.
    fn depth(&self, s: &State) -> Option<f32> {
        match self.input {
            Input::Trigger(side, _) => Some(s.triggers[side].clamp(0.0, 1.0)),
            _ => None,
        }
    }
}

/// The controls in use, from the top down on either side, in the order of what they point at so
/// that their lines never cross. The further down a label below what it points at, the further
/// in its line's lane.
#[expect(clippy::too_many_lines, reason = "a table of the controls")]
fn rows() -> Vec<Row> {
    let left = |y, lane, via, to: [f32; 2]| (false, y, lane, via, pos2(to[0], to[1]));
    let right = |y, lane, via, to: [f32; 2]| (true, y, lane, via, pos2(to[0], to[1]));
    let row =
        |parts, action: Cow<'static, str>, input, color, place: (bool, f32, f32, f32, Pos2)| {
            let (right, y, lane, via, to) = place;
            Row {
                parts,
                action,
                input,
                color,
                right,
                y,
                lane,
                via,
                to,
            }
        };
    vec![
        row(
            vec![part(CREATE)],
            "Show / hide the controller view".into(),
            Input::Held(Button::Select),
            LINE,
            left(20.0, 265.0, 20.0, [265.0, 123.7]),
        ),
        row(
            vec![part(L2)],
            format!("Fine control ({}x power)", game::FINE).into(),
            Input::Trigger(0, Button::LeftTrigger2),
            palette::PEACH,
            left(70.0, 52.0, 98.0, [230.0, 98.0]),
        ),
        row(
            vec![part(L1)],
            "Zoom out".into(),
            Input::Held(Button::LeftTrigger),
            palette::MAUVE,
            left(120.0, 52.0, 113.0, [196.5, 113.0]),
        ),
        row(
            vec![
                dpad(DPAD_UP, Button::DPadUp),
                dpad(DPAD_DOWN, Button::DPadDown),
            ],
            "Orbit altitude; L2 for fine steps".into(),
            Input::EitherHeld(Button::DPadUp, Button::DPadDown),
            LINE,
            left(170.0, 67.0, 160.0, [213.0, 160.0]),
        ),
        row(
            vec![
                dpad(DPAD_LEFT, Button::DPadLeft),
                dpad(DPAD_RIGHT, Button::DPadRight),
            ],
            "Select Reach B / Orbit objective".into(),
            Input::EitherHeld(Button::DPadLeft, Button::DPadRight),
            LINE,
            left(220.0, 82.0, 187.0, [187.5, 187.0]),
        ),
        row(
            vec![part(LEFT_STICK)],
            "Turn, harder the further".into(),
            Input::LeftStickSideways,
            palette::SAPPHIRE,
            left(270.0, 97.0, 252.2, [258.9, 252.2]),
        ),
        row(
            vec![part(OPTIONS)],
            "Pause / resume".into(),
            Input::Held(Button::Start),
            LINE,
            right(20.0, 487.0, 20.0, [487.0, 123.7]),
        ),
        row(
            vec![part(R2)],
            "Thrust".into(),
            Input::Trigger(1, Button::RightTrigger2),
            palette::PEACH,
            right(70.0, 700.0, 98.0, [522.0, 98.0]),
        ),
        row(
            vec![part(R1)],
            "Zoom in".into(),
            Input::Held(Button::RightTrigger),
            palette::MAUVE,
            right(120.0, 700.0, 113.0, [555.5, 113.0]),
        ),
        // Over Triangle, then down onto Square, which Triangle and Circle hide from the right.
        row(
            vec![part(SQUARE)],
            "Engage the selected objective".into(),
            Input::Held(Button::West),
            palette::PINK,
            right(170.0, 685.0, 128.0, [491.1, 170.6]),
        ),
        row(
            vec![part(TRIANGLE)],
            "SAS: hold the heading when not turning".into(),
            Input::Held(Button::North),
            palette::TEAL,
            right(220.0, 670.0, 152.0, [542.3, 152.0]),
        ),
        row(
            vec![part(CIRCLE)],
            "Return to manual flight".into(),
            Input::Held(Button::East),
            palette::RED,
            right(270.0, 655.0, 186.9, [577.2, 186.9]),
        ),
        row(
            vec![part(CROSS)],
            "Thrust".into(),
            Input::Held(Button::South),
            palette::SKY,
            right(320.0, 640.0, 221.8, [542.3, 221.8]),
        ),
        row(
            vec![part(RIGHT_STICK)],
            "Aim: turn that way; thrust when facing it".into(),
            Input::RightStick,
            palette::SAPPHIRE,
            right(370.0, 625.0, 252.2, [493.1, 252.2]),
        ),
        // Under the controller, then up between its grips onto the stick's cap.
        row(
            vec![part(RIGHT_STICK_CLICK)],
            "Select orbit target".into(),
            Input::Held(Button::RightThumb),
            palette::SAPPHIRE,
            right(420.0, 452.3, 420.0, [452.3, 282.0]),
        ),
    ]
}

/// The guide, `width` wide, lit by `state`, or dim without a controller.
pub fn ui(ui: &mut Ui, width: f32, state: Option<&State>) {
    ui.label(state.map_or(
        "No controller: connect one and press a button on it to see it light up here",
        |pad| pad.name.as_str(),
    ));
    if width < MIN_WIDTH {
        list_ui(ui, state);
    } else {
        map_ui(ui, width, state);
    }
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.label(
            egui::RichText::new("Button icons by Zacksly (CC BY 3.0), tinted: ")
                .small()
                .color(palette::OVERLAY1),
        );
        ui.hyperlink_to(
            egui::RichText::new("zacksly.itch.io").small(),
            "https://zacksly.itch.io",
        );
    });
}

/// The wireframe with the callouts, `width` wide.
fn map_ui(ui: &mut Ui, width: f32, state: Option<&State>) {
    let fit = width / WIDTH;
    let scale = FULL_SCALE * fit;
    let (rect, _) = ui.allocate_exact_size(MAP.size() * scale, egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect(
        rect,
        12.0,
        Color32::BLACK,
        Stroke::new(1.0, Color32::from_gray(38)),
        egui::StrokeKind::Inside,
    );
    let view = View {
        painter: &painter,
        origin: rect.min + (view::VIEW.min - MAP.min) * scale,
        scale,
        dim: state.is_none(),
    };
    let screen = |q: Pos2| rect.min + (q - MAP.min) * scale;
    view::paint_wireframe(&view, state);

    let idle = State::default();
    let s = state.unwrap_or(&idle);
    let rows = rows();
    // Lit last, over the others.
    for lit in [false, true] {
        for row in rows.iter().filter(|row| row.lit(s) == lit) {
            let color = if lit {
                row.color
            } else {
                LINE.gamma_multiply(0.4)
            };
            view.line(row.path(), false, color);
            view.dot(row.to, color);
        }
    }
    for row in &rows {
        label(ui, &painter, row, s, state.is_none(), screen, fit);
    }
}

/// A row's label on the map: its icons by its line, and what it does beyond them.
fn label(
    ui: &Ui,
    painter: &egui::Painter,
    row: &Row,
    s: &State,
    dim: bool,
    screen: impl Fn(Pos2) -> Pos2,
    fit: f32,
) {
    let lit = row.lit(s);
    let strength = if dim { 0.45 } else { 1.0 };
    let icon_size = ICON_SIZE * fit;
    let gap = GAP * fit;
    let edge = screen(pos2(if row.right { RIGHT_EDGE } else { LEFT_EDGE }, row.y));
    let outward = if row.right { 1.0 } else { -1.0 };
    let side_of = |x: f32, width: f32| {
        // The rectangle from `x` outward, `width` wide and an icon tall.
        let far = x + outward * width;
        Rect::from_x_y_ranges(
            x.min(far)..=x.max(far),
            (edge.y - icon_size / 2.0)..=(edge.y + icon_size / 2.0),
        )
    };

    let mut x = edge.x + outward * gap;
    let icons_start = x;
    for part in &row.parts {
        let solid = part.button.map_or(lit, |button| s.is_held(button));
        let tint = if lit { row.color } else { palette::WHITE };
        part.icon
            .image(solid)
            .tint(tint.gamma_multiply(strength))
            .paint_at(ui, side_of(x, icon_size));
        x += outward * icon_size;
    }
    if let Some(depth) = row.depth(s).filter(|&depth| depth > 0.0) {
        let track = side_of(icons_start, icon_size * depth);
        painter.rect_filled(
            Rect::from_min_max(
                pos2(track.left(), track.bottom()),
                pos2(track.right(), track.bottom() + 2.0 * fit),
            ),
            1.0,
            row.color,
        );
    }

    let color = if lit { row.color } else { palette::SUBTEXT }.gamma_multiply(strength);
    let map_edge = screen(pos2(
        if row.right { MAP.right() } else { MAP.left() },
        row.y,
    ));
    let text_x = x + outward * gap;
    let wrap = (map_edge.x - text_x).abs() - 2.0 * gap;
    let mut job = egui::text::LayoutJob::simple(
        row.action.clone().into_owned(),
        egui::FontId::proportional(TEXT_SIZE * fit),
        color,
        wrap,
    );
    if !row.right {
        job.halign = egui::Align::RIGHT;
    }
    let galley = painter.layout_job(job);
    painter.galley(pos2(text_x, edge.y - galley.size().y / 2.0), galley, color);
}

/// The same as the map, as a list, for narrow screens.
fn list_ui(ui: &mut Ui, state: Option<&State>) {
    let idle = State::default();
    let s = state.unwrap_or(&idle);
    egui::Grid::new("controller_guide")
        .num_columns(2)
        .show(ui, |ui| {
            for row in rows() {
                let lit = row.lit(s);
                let color = if lit { row.color } else { palette::WHITE };
                let side = ui.spacing().interact_size.y;
                ui.horizontal(|ui| {
                    for part in &row.parts {
                        let solid = part.button.map_or(lit, |button| s.is_held(button));
                        ui.add(
                            part.icon
                                .image(solid)
                                .fit_to_exact_size(Vec2::splat(side))
                                .tint(color),
                        );
                    }
                });
                let text = egui::RichText::new(row.action.clone());
                ui.label(if lit { text.color(row.color) } else { text });
                ui.end_row();
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whether the segments `a` and `b`, each across or along, meet.
    fn cross(a: (Pos2, Pos2), b: (Pos2, Pos2)) -> bool {
        let range = |p: f32, q: f32| p.min(q)..=p.max(q);
        range(a.0.x, a.1.x).start() <= range(b.0.x, b.1.x).end()
            && range(b.0.x, b.1.x).start() <= range(a.0.x, a.1.x).end()
            && range(a.0.y, a.1.y).start() <= range(b.0.y, b.1.y).end()
            && range(b.0.y, b.1.y).start() <= range(a.0.y, a.1.y).end()
    }

    fn segments(row: &Row) -> Vec<(Pos2, Pos2)> {
        let path = row.path();
        path.windows(2)
            .map(|pair| (pair[0], pair[1]))
            .filter(|(p, q)| p != q)
            .collect()
    }

    #[test]
    fn the_callouts_lines_never_cross() {
        let rows = rows();
        for (i, one) in rows.iter().enumerate() {
            for other in &rows[i + 1..] {
                for a in segments(one) {
                    for b in segments(other) {
                        assert!(
                            !cross(a, b),
                            "{:?} and {:?}: {a:?} meets {b:?}",
                            one.action,
                            other.action
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_callouts_lines_are_elbows_that_stay_on_the_map() {
        for row in rows() {
            for (p, q) in segments(&row) {
                assert!(
                    (p.x - q.x).abs() < 1e-3 || (p.y - q.y).abs() < 1e-3,
                    "{:?}: across or along, not slanted",
                    row.action
                );
            }
            for q in row.path() {
                assert!(MAP.contains(q), "{:?}: {q:?} on the map", row.action);
            }
        }
    }

    #[test]
    fn the_labels_have_room_and_stay_on_the_map() {
        let rows = rows();
        let spacing = ICON_SIZE / FULL_SCALE;
        for (i, one) in rows.iter().enumerate() {
            assert!(
                one.y - spacing / 2.0 >= MAP.top() && one.y + spacing / 2.0 <= MAP.bottom(),
                "{:?} on the map",
                one.action
            );
            for other in rows[i + 1..].iter().filter(|o| o.right == one.right) {
                assert!(
                    (one.y - other.y).abs() >= spacing,
                    "{:?} and {:?} apart",
                    one.action,
                    other.action
                );
            }
        }
    }

    #[test]
    fn the_map_is_centered_on_the_controller_and_as_wide_as_said() {
        assert!((MAP.center().x - crate::controller_shape::AXIS).abs() < 1e-3);
        assert!((MAP.width() * FULL_SCALE - WIDTH).abs() < 1e-3);
        assert!(MAP.contains_rect(view::VIEW));
    }

    #[test]
    fn a_control_lights_its_row_alone() {
        let lit = |s: &State| -> Vec<String> {
            rows()
                .iter()
                .filter(|row| row.lit(s))
                .map(|row| row.action.to_string())
                .collect()
        };
        assert!(lit(&State::default()).is_empty(), "nothing used");
        let cross = State {
            held: vec![Button::South],
            ..State::default()
        };
        assert_eq!(lit(&cross), ["Thrust"]);
        let r2 = State {
            triggers: [0.0, 0.3],
            ..State::default()
        };
        assert_eq!(lit(&r2), ["Thrust"], "pulled, short of its threshold");
        let stick = State {
            stick: [-0.5, 0.0],
            ..State::default()
        };
        assert_eq!(lit(&stick), ["Turn, harder the further"]);
        let up = State {
            stick: [0.0, 0.9],
            ..State::default()
        };
        assert!(lit(&up).is_empty(), "up on the left stick does nothing");
    }

    #[test]
    fn the_guide_says_what_the_flight_does() {
        let actions: Vec<_> = rows().iter().map(|row| row.action.to_string()).collect();
        assert!(actions.iter().any(|a| a.contains(&game::FINE.to_string())));
        assert_eq!(actions.len(), 15, "every control in use");
    }
}
