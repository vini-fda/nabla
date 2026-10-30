//! Gameplay mode: the player flies the ship to the goal, in place of a control policy, and time
//! runs freely instead of along a timeline.
//!
//! The flight runs on the same exact sampled-data model a solution is scored on. At each sample
//! `t_k = k h` the controls the player holds become `u_k`, held for the whole step, and
//! `s_(k+1) = step(s_k, u_k)`. So a flight is a [`Replay`] that grows as it goes, and the canvas,
//! plumes, plots and sound show it as they show a solution's run.
//!
//! Thrust is all or nothing, as a key or a button is. Torque is all or nothing from the keyboard
//! too, and as strong as the joystick is pushed on a touch screen. Fine control scales both down
//! to [`FINE`] of their maxima, since one step at full power can change the speed or the spin by
//! more than the target set allows.
//!
//! [`Steering`] picks what letting go of the turn does. With [`Steering::Raw`], nothing: the
//! ship spins on. With [`Steering::Sas`], a stability assist like Kerbal Space Program's SAS
//! brakes the spin and holds the heading the ship comes to, until the player turns again.
//!
//! The player can also point instead, with the mouse or a finger, while the camera follows the
//! ship: an [`Aim`] turns the ship toward the pointer and thrusts once it faces it, within a
//! cone. Both the assist and the aim use the attitude law of `docs/main.typ` ("Problem:
//! pointing").

use std::collections::HashMap;
use std::f64::consts::TAU;

use egui::{Color32, Pos2, Rect, Sense, Vec2, pos2, vec2};
use nabla_core::problem::{Case, Outcome, Trajectory};
use nabla_core::{Control, Params, State, step};

use crate::replay::Replay;
use crate::theme::{icons, palette};

/// Name of the flight's "solution", as the replay shows it.
const PLAYER: &str = "you";

/// Fraction of the maximum thrust and torque applied under fine control.
pub const FINE: f64 = 0.1;

/// Longest frame time simulated at once: after a longer frame (a stall, a hidden tab), the
/// flight goes on from where it was rather than jumping ahead.
const MAX_FRAME_TIME: f64 = 0.1;

/// Least half width and height of the view [`Game::view_bounds`] frames around the ship.
const MIN_VIEW_HALF_SIZE: f64 = 3.0;

/// Fraction of the ship's angular acceleration the stability assist brakes with, leaving the
/// rest to make up for the samples' lag.
const SAS_AUTHORITY: f64 = 0.8;
/// Time constant of the stability assist's approach to the held heading, at least this many
/// sampling periods, so that it settles without overshooting at any `h`.
const SAS_TIME_CONSTANT: f64 = 0.2;
const SAS_MIN_SAMPLES: f64 = 4.0;

/// What letting go of the turn does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Steering {
    /// Nothing: the turn input is the torque, and the ship spins on when it is let go.
    #[default]
    Raw,
    /// Stability assist, as KSP's SAS: while the player turns, the turn input is the torque, as
    /// with [`Steering::Raw`]. Let go, the assist brakes the spin, with the ship's full torque
    /// whatever the fine control, and holds the heading the ship comes to.
    Sas,
}

/// Where the player points: the commanded heading `psi`, toward the pointer from the ship, and
/// the half-angle `delta` of the cone around it within which the ship thrusts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aim {
    pub heading: f64,
    pub cone: f64,
}

/// The controls the player holds.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Input {
    pub thrust: bool,
    /// How hard to turn, from -1 (full torque to the right, clockwise) to 1 (full torque to the
    /// left, counterclockwise).
    pub turn: f64,
    /// Scale thrust and torque down to [`FINE`]. Under an aim, the thrust only.
    pub fine: bool,
    /// Where the player points, while they do: it takes over the torque, and thrusts within its
    /// cone.
    pub aim: Option<Aim>,
}

impl Input {
    /// Both's controls held: the turns add up, to at most a full turn either way.
    pub fn or(self, other: Self) -> Self {
        Self {
            thrust: self.thrust || other.thrust,
            turn: self.turn + other.turn,
            fine: self.fine || other.fine,
            aim: self.aim.or(other.aim),
        }
    }

    /// The control these inputs ask of a ship with `params`: always admissible.
    pub fn control(self, params: &Params) -> Control {
        let power = if self.fine { FINE } else { 1.0 };
        // NaN, which a turn should never be, turns not at all.
        let turn = if self.turn.is_nan() {
            0.0
        } else {
            self.turn.clamp(-1.0, 1.0)
        };
        Control {
            thrust: if self.thrust {
                power * params.f_max
            } else {
                0.0
            },
            torque: power * turn * params.tau_max,
        }
    }
}

/// A flight on one case.
pub struct Game {
    /// The flight so far: every sample up to the present, and the state at the end of the
    /// step under way. Its outcome stays [`Outcome::Timeout`]: a flight has no step budget, and
    /// [`Self::landed`] says when it reached the target set.
    pub replay: Replay,
    /// Time since the start, in simulated seconds.
    time: f64,
    pub paused: bool,
    /// The first sample in the target set.
    landed: Option<usize>,
    /// The heading the stability assist holds, while it does.
    hold: Option<f64>,
    pub pads: Pads,
}

impl Game {
    pub fn new(case: Case) -> Self {
        let trajectory = Trajectory {
            states: vec![case.start],
            controls: Vec::new(),
            outcome: Outcome::Timeout,
        };
        Self {
            replay: Replay::new(PLAYER, case, trajectory),
            time: 0.0,
            paused: false,
            landed: None,
            hold: None,
            pads: Pads::default(),
        }
    }

    pub fn time(&self) -> f64 {
        self.time
    }

    /// The present as `(k, tau)`, with `t = k h + tau` and `0 <= tau < h`.
    pub fn moment(&self) -> (usize, f64) {
        let r = &self.replay;
        let k = ((self.time / r.h()).floor() as usize).min(r.last_frame());
        (k, self.time - r.time(k))
    }

    /// The exact state at present.
    pub fn state(&self) -> State {
        let (k, tau) = self.moment();
        self.replay.state_at(k, tau)
    }

    /// The number of steps the flight took to first reach the target set, once it has.
    pub fn landed(&self) -> Option<usize> {
        self.landed
    }

    /// With the stability assist, the heading it holds, or, while the player turns, the one it
    /// would hold if they let go now: where braking would stop the spin. `None` without it.
    pub fn sas_heading(&self, steering: Steering) -> Option<f64> {
        if steering == Steering::Raw {
            return None;
        }
        let params = &self.replay.case.task.params;
        Some(
            self.hold
                .unwrap_or_else(|| stopping_heading(params, &self.state())),
        )
    }

    /// Run the flight `dt` real seconds on, unless paused, with `input` held at the samples
    /// this passes, steering as `steering` says.
    pub fn advance(&mut self, dt: f64, input: Input, steering: Steering) {
        if self.paused {
            return;
        }
        self.time += dt.clamp(0.0, MAX_FRAME_TIME);
        let h = self.replay.h();
        let task = self.replay.case.task;
        let trajectory = &mut self.replay.trajectory;
        // Every sample up to the present gets its control, so the step under way is known.
        while trajectory.controls.len() as f64 * h <= self.time {
            let k = trajectory.controls.len();
            let s = trajectory.states[k];
            if self.landed.is_none() && task.reached(&s) {
                self.landed = Some(k);
            }
            let u = steer(input, steering, &mut self.hold, &task.params, &s);
            trajectory.controls.push(u);
            trajectory.states.push(step(&task.params, &s, &u));
        }
    }

    /// A view centered on the ship that shows the goal too, as `([x_min, y_min], [x_max, y_max])`.
    pub fn view_bounds(&self) -> ([f64; 2], [f64; 2]) {
        let s = self.state();
        let goal = self.replay.case.task.goal;
        let dx = (goal.x - s.x).abs().max(MIN_VIEW_HALF_SIZE);
        let dy = (goal.y - s.y).abs().max(MIN_VIEW_HALF_SIZE);
        ([s.x - dx, s.y - dy], [s.x + dx, s.y + dy])
    }
}

/// The control for `input` in state `s`: the input's own, except that an aim, or else the
/// stability assist while the player is not turning, takes over the torque. The aim turns the
/// ship toward its heading, the shorter way, and thrusts within its cone, as well as when the
/// player thrusts. The assist holds `hold`, which it sets, when it takes over, to where the
/// ship's spin will stop, and which is cleared while the player turns or aims.
fn steer(
    input: Input,
    steering: Steering,
    hold: &mut Option<f64>,
    params: &Params,
    s: &State,
) -> Control {
    let mut u = input.control(params);
    if let Some(aim) = input.aim {
        *hold = None;
        let error = wrap(aim.heading - s.theta);
        u.torque = hold_torque(params, s, s.theta + error);
        if error.abs() <= aim.cone {
            let thrust = Input {
                thrust: true,
                ..input
            };
            u.thrust = thrust.control(params).thrust;
        }
        return u;
    }
    if steering == Steering::Raw || input.turn != 0.0 {
        *hold = None;
        return u;
    }
    let heading = *hold.get_or_insert_with(|| stopping_heading(params, s));
    u.torque = hold_torque(params, s, heading);
    u
}

/// `angle` turned into `[-pi, pi]` by whole turns: the turn to it the shorter way.
pub fn wrap(angle: f64) -> f64 {
    angle - TAU * (angle / TAU).round()
}

/// The stability assist's angular acceleration: a fraction of the ship's.
fn sas_acceleration(params: &Params) -> f64 {
    SAS_AUTHORITY * params.tau_max / params.inertia
}

/// The heading at which the stability assist brings the ship's spin in state `s` to a stop.
fn stopping_heading(params: &Params, s: &State) -> f64 {
    s.theta + s.omega * s.omega.abs() / (2.0 * sas_acceleration(params))
}

/// The torque that turns the ship in state `s` toward `heading` and holds it there: over the
/// next step, it brings the angular velocity to the speed at which braking at the assist's
/// acceleration stops the ship on the heading, or, near it, to an exponential approach.
fn hold_torque(params: &Params, s: &State, heading: f64) -> f64 {
    let error = heading - s.theta;
    let time_constant = SAS_TIME_CONSTANT.max(SAS_MIN_SAMPLES * params.h);
    let braking = (2.0 * sas_acceleration(params) * error.abs()).sqrt();
    let omega = (error.abs() / time_constant).min(braking).copysign(error);
    let torque = params.inertia * (omega - s.omega) / params.h;
    torque.clamp(-params.tau_max, params.tau_max)
}

/// Radius of the joystick's base, and of its knob, which travels sideways within the base.
const STICK_RADIUS: f32 = 52.0;
const KNOB_RADIUS: f32 = 24.0;
/// Fraction of the joystick's travel around its center that turns not at all, so a thumb
/// resting on it does not spin the ship.
const DEAD_ZONE: f32 = 0.08;
/// Radius of the thrust pad, and of the fine control latch.
const THRUST_RADIUS: f32 = 44.0;
const FINE_RADIUS: f32 = 22.0;
/// Space between the controls and the canvas's edges, and between neighboring controls.
const PAD_MARGIN: f32 = 24.0;
const PAD_GAP: f32 = 16.0;
/// How far past its edge a touch still lands on a control, as a fraction of its radius: thumbs
/// are not precise.
const PAD_REACH: f32 = 1.25;
const PAD_ICON_SIZE: f32 = 28.0;
const STICK_ICON_SIZE: f32 = 16.0;
/// Height of the strip along the canvas's bottom the controls take.
pub const PADS_HEIGHT: f32 = PAD_MARGIN + 2.0 * STICK_RADIUS;
/// A control's color, a translucent grey as the canvas buttons'.
const PAD_COLOR: Color32 = Color32::from_rgba_unmultiplied_const(46, 46, 46, 170);

/// An on-screen control.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pad {
    Stick,
    Thrust,
    Fine,
}

impl Pad {
    const ALL: [Self; 3] = [Self::Stick, Self::Thrust, Self::Fine];

    /// The control's center and radius on `canvas`: the joystick in its bottom left corner,
    /// under the left thumb, thrust in the bottom right, level with it, and the fine control
    /// latch above thrust.
    fn circle(self, canvas: Rect) -> (Pos2, f32) {
        let thrust = pos2(
            canvas.right() - PAD_MARGIN - THRUST_RADIUS,
            canvas.bottom() - PAD_MARGIN - STICK_RADIUS,
        );
        match self {
            Self::Stick => (
                canvas.left_bottom() + vec2(1.0, -1.0) * (PAD_MARGIN + STICK_RADIUS),
                STICK_RADIUS,
            ),
            Self::Thrust => (thrust, THRUST_RADIUS),
            Self::Fine => (
                thrust - vec2(0.0, THRUST_RADIUS + PAD_GAP + FINE_RADIUS),
                FINE_RADIUS,
            ),
        }
    }

    fn contains(self, canvas: Rect, p: Pos2) -> bool {
        let (center, radius) = self.circle(canvas);
        center.distance(p) <= PAD_REACH * radius
    }
}

/// How hard the joystick on `canvas` turns with its knob held at `p`: from -1 (all the way
/// right) to 1 (all the way left). Only the knob's sideways travel counts, past the dead zone.
fn stick_turn(canvas: Rect, p: Pos2) -> f32 {
    let (center, radius) = Pad::Stick.circle(canvas);
    let x = ((p.x - center.x) / radius).clamp(-1.0, 1.0);
    let past = ((x.abs() - DEAD_ZONE) / (1.0 - DEAD_ZONE)).max(0.0);
    -past.copysign(x)
}

/// On-screen controls for touch screens: a joystick in the canvas's bottom left corner that
/// turns the ship, as hard as it is pushed sideways, thrust in its bottom right, and a latch for
/// fine control. The joystick follows the finger that took it until that finger lifts, even off
/// its base; thrust is held while any finger is on it. So one thumb can turn while the other
/// thrusts. While the player points, the controls are hidden, and the whole canvas points.
#[derive(Default)]
pub struct Pads {
    /// Where each touch on the screen is, by device and touch id.
    touches: HashMap<(u64, u64), Pos2>,
    /// The touch that holds the joystick.
    stick: Option<(u64, u64)>,
    /// The touch that points, for an [`Aim`]: the first to land on the canvas off the controls
    /// and off the areas the caller keeps for other widgets.
    aim: Option<(u64, u64)>,
    /// Where the joystick is held, while it is.
    knob: Option<Pos2>,
    /// A touch screen has been used, so the controls show even where the system is not a
    /// phone's or a tablet's.
    touched: bool,
    /// Fine control is latched on.
    fine: bool,
    /// What the controls held at the last [`Self::update`].
    held: Input,
    /// A touch was on a control at the last [`Self::update`].
    on_pad: bool,
    /// The controls are hidden, at the last [`Self::update`].
    hidden: bool,
}

impl Pads {
    /// Whether the controls show: on phones and tablets, and once a touch screen has been used.
    pub fn visible(&self, touch_device: bool) -> bool {
        touch_device || self.touched
    }

    /// The controls held at the last [`Self::update`].
    pub fn held(&self) -> Input {
        self.held
    }

    /// Whether a touch screen has been used: then the touches point, rather than the mouse.
    pub fn touched(&self) -> bool {
        self.touched
    }

    /// Where the touch that points is, while one does.
    pub fn aim_touch(&self) -> Option<Pos2> {
        let key = self.aim?;
        self.touches.get(&key).copied()
    }

    /// Whether a touch is on a control, so a second finger is not taken for a pinch.
    pub fn touching(&self) -> bool {
        self.on_pad
    }

    /// The controls that show: all of them, or none while they are hidden.
    fn shown(&self) -> impl Iterator<Item = Pad> + use<> {
        let hidden = self.hidden;
        Pad::ALL.into_iter().filter(move |_| !hidden)
    }

    /// Reads this frame's touches, and the mouse, on the controls on `canvas`, while they show
    /// (if `visible`). A touch that lands in one of the `reserved` areas, which other widgets
    /// take, never points. Call it after the canvas has been allocated, so the controls, and not
    /// the canvas, take the pointer's drags.
    pub fn update(&mut self, ui: &egui::Ui, canvas: Rect, visible: bool, reserved: &[Rect]) {
        self.hidden = !visible;
        if self.hidden {
            self.stick = None;
        }
        let taps = self.read_touches(ui, canvas, reserved);
        self.held = Input::default();
        self.knob = None;
        self.on_pad = false;
        if !visible {
            return;
        }

        let mut clicked_fine = false;
        let mut dragged_stick = None;
        for pad in self.shown() {
            let (center, radius) = pad.circle(canvas);
            let rect = Rect::from_center_size(center, Vec2::splat(2.0 * PAD_REACH * radius));
            let id = ui.scope_id().with(("game_pad", pad as u8));
            let response = ui.interact(rect, id, Sense::click_and_drag());
            clicked_fine |= pad == Pad::Fine && response.clicked();
            if pad == Pad::Stick && response.is_pointer_button_down_on() {
                dragged_stick = response.interact_pointer_pos();
            }
        }
        // A touch also moves the pointer, so on a touch screen the latch and the joystick go by
        // the touches alone: a tap would flip the latch twice, and a second finger on the
        // joystick could take the pointer from the first.
        let tapped_fine = if self.touched {
            taps.iter().any(|&p| Pad::Fine.contains(canvas, p))
        } else {
            clicked_fine
        };
        if tapped_fine {
            self.fine = !self.fine;
        }
        self.knob = if self.touched {
            self.stick.and_then(|key| self.touches.get(&key).copied())
        } else {
            dragged_stick
        };

        let pointer = ui.input(|i| {
            i.pointer
                .primary_down()
                .then(|| i.pointer.interact_pos())
                .flatten()
        });
        self.held = Input {
            thrust: self
                .touches
                .values()
                .copied()
                .chain(pointer)
                .any(|p| Pad::Thrust.contains(canvas, p)),
            turn: self.knob.map_or(0.0, |p| f64::from(stick_turn(canvas, p))),
            fine: self.fine,
            aim: None,
        };
        self.on_pad = self.stick.is_some()
            || self
                .touches
                .values()
                .any(|&p| self.shown().any(|pad| pad.contains(canvas, p)));
    }

    /// Follows this frame's touch events: where each touch is, which one holds the joystick,
    /// the first to land on it, and which one points, the first to land off the controls and
    /// the `reserved` areas. Returns where touches started.
    fn read_touches(&mut self, ui: &egui::Ui, canvas: Rect, reserved: &[Rect]) -> Vec<Pos2> {
        let mut taps = Vec::new();
        ui.input(|i| {
            for event in &i.events {
                let egui::Event::Touch {
                    device_id,
                    id,
                    phase,
                    pos,
                    ..
                } = *event
                else {
                    continue;
                };
                self.touched = true;
                let key = (device_id.0, id.0);
                match phase {
                    egui::TouchPhase::Start => {
                        self.touches.insert(key, pos);
                        taps.push(pos);
                        let on_pad = self.shown().any(|pad| pad.contains(canvas, pos));
                        if self.stick.is_none() && !self.hidden && Pad::Stick.contains(canvas, pos)
                        {
                            self.stick = Some(key);
                        } else if self.aim.is_none()
                            && !on_pad
                            && canvas.contains(pos)
                            && !reserved.iter().any(|r| r.contains(pos))
                        {
                            self.aim = Some(key);
                        }
                    }
                    egui::TouchPhase::Move => {
                        self.touches.insert(key, pos);
                    }
                    egui::TouchPhase::End | egui::TouchPhase::Cancel => {
                        self.touches.remove(&key);
                        if self.stick == Some(key) {
                            self.stick = None;
                        }
                        if self.aim == Some(key) {
                            self.aim = None;
                        }
                    }
                }
            }
        });
        taps
    }

    /// Paints the controls on `canvas`, lit while held.
    pub fn paint(&self, painter: &egui::Painter, canvas: Rect) {
        self.paint_stick(painter, canvas);
        for (pad, held, color) in [
            (Pad::Thrust, self.held.thrust, palette::PEACH),
            (Pad::Fine, self.fine, palette::GREEN),
        ] {
            let (center, radius) = pad.circle(canvas);
            paint_base(painter, center, radius, held.then_some(color));
            let (glyph, font) = match pad {
                Pad::Fine => ("fine", egui::FontId::proportional(13.0)),
                _ => (icons::outline::FIRE, icons::outline::font(PAD_ICON_SIZE)),
            };
            let glyph_color = if held { color } else { palette::WHITE };
            painter.text(
                center,
                egui::Align2::CENTER_CENTER,
                glyph,
                font,
                glyph_color,
            );
        }
    }

    /// The joystick: its base, with the turn it makes on either side, and the knob, where the
    /// finger holds it (sideways only, and within the base) or else at the center.
    fn paint_stick(&self, painter: &egui::Painter, canvas: Rect) {
        use icons::outline::{ARROW_CLOCKWISE, ARROW_COUNTER_CLOCKWISE};
        const COLOR: Color32 = palette::SAPPHIRE;
        let (center, radius) = Pad::Stick.circle(canvas);
        let turn = self.knob.map_or(0.0, |p| stick_turn(canvas, p));
        paint_base(painter, center, radius, (turn != 0.0).then_some(COLOR));
        let travel = radius - KNOB_RADIUS;
        painter.line_segment(
            [center - vec2(travel, 0.0), center + vec2(travel, 0.0)],
            egui::Stroke::new(2.0, palette::OVERLAY0),
        );
        for (side, glyph, on) in [
            (-1.0, ARROW_COUNTER_CLOCKWISE, turn > 0.0),
            (1.0, ARROW_CLOCKWISE, turn < 0.0),
        ] {
            painter.text(
                // Above the knob's track, inside the base.
                center + vec2(side * 0.45 * radius, -0.68 * radius),
                egui::Align2::CENTER_CENTER,
                glyph,
                icons::outline::font(STICK_ICON_SIZE),
                if on { COLOR } else { palette::OVERLAY1 },
            );
        }
        let offset = self
            .knob
            .map_or(0.0, |p| (p.x - center.x).clamp(-travel, travel));
        let knob = center + vec2(offset, 0.0);
        let (fill, stroke) = if self.knob.is_some() {
            (COLOR.gamma_multiply(0.6), COLOR)
        } else {
            (palette::SURFACE2, palette::OVERLAY1)
        };
        painter.circle_filled(knob, KNOB_RADIUS, fill);
        painter.circle_stroke(knob, KNOB_RADIUS, egui::Stroke::new(1.5, stroke));
    }
}

/// A control's round base at `center`, tinted and outlined in `lit`, if given.
fn paint_base(painter: &egui::Painter, center: Pos2, radius: f32, lit: Option<Color32>) {
    painter.circle_filled(center, radius, PAD_COLOR);
    if let Some(color) = lit {
        painter.circle_filled(center, radius, color.gamma_multiply(0.3));
    }
    let stroke = lit.unwrap_or(palette::OVERLAY0);
    painter.circle_stroke(center, radius, egui::Stroke::new(1.5, stroke));
}

#[cfg(test)]
mod tests {
    use super::*;
    use nabla_core::suite;
    use std::f64::consts::PI;

    fn game() -> Game {
        Game::new(suite::find("agile").expect("case exists"))
    }

    #[test]
    fn input_controls_are_admissible_and_signed() {
        let params = game().replay.case.task.params;
        let turn = |turn| Input {
            turn,
            ..Input::default()
        };
        let full = Input {
            thrust: true,
            ..turn(1.0)
        };
        let u = full.control(&params);
        assert!(u.is_admissible(&params), "full power");
        assert_eq!((u.thrust, u.torque), (params.f_max, params.tau_max), "full");
        let fine = Input {
            fine: true,
            ..turn(-1.0)
        };
        let u = fine.control(&params);
        assert_eq!(u.thrust, 0.0, "no thrust");
        assert!(
            (u.torque + FINE * params.tau_max).abs() < 1e-12,
            "fine, rightward"
        );
        let both = turn(1.0).or(turn(-1.0));
        assert_eq!(both.control(&params).torque, 0.0, "left and right cancel");
        let over = turn(1.0).or(turn(0.7)).control(&params);
        assert_eq!(over.torque, params.tau_max, "clamped to a full turn");
        assert_eq!(
            turn(f64::NAN).control(&params).torque,
            0.0,
            "NaN does not turn"
        );
    }

    #[test]
    fn the_joystick_turns_as_hard_as_it_is_pushed_sideways() {
        let canvas = Rect::from_min_size(Pos2::ZERO, vec2(400.0, 800.0));
        let (center, radius) = Pad::Stick.circle(canvas);
        assert_eq!(stick_turn(canvas, center), 0.0, "centered");
        assert_eq!(
            stick_turn(canvas, center + vec2(0.5 * DEAD_ZONE * radius, 30.0)),
            0.0,
            "in the dead zone; up and down do not count"
        );
        assert_eq!(
            stick_turn(canvas, center - vec2(radius, 0.0)),
            1.0,
            "full left"
        );
        assert_eq!(
            stick_turn(canvas, center + vec2(3.0 * radius, 0.0)),
            -1.0,
            "past full right"
        );
        let half = stick_turn(canvas, center - vec2(0.5 * radius, 0.0));
        assert!(0.4 < half && half < 0.5, "part way: {half}");
    }

    #[test]
    fn flight_follows_the_model_one_sample_at_a_time() {
        let mut g = game();
        let h = g.replay.h();
        let thrust = Input {
            thrust: true,
            ..Input::default()
        };
        g.advance(2.5 * h, thrust, Steering::Raw);
        let r = &g.replay;
        assert_eq!(
            r.trajectory.controls.len(),
            3,
            "samples 0, 1 and 2 have controls"
        );
        assert_eq!(r.trajectory.states.len(), 4, "and the step under way ends");
        let params = r.case.task.params;
        let u = thrust.control(&params);
        let s1 = step(&params, &r.case.start, &u);
        assert_eq!(*r.state(1), s1, "exact step");
        let (k, tau) = g.moment();
        assert_eq!(k, 2, "in the third step");
        assert!((tau - 0.5 * h).abs() < 1e-12, "half way through it");
    }

    #[test]
    fn long_frames_are_capped_and_pause_stops_time() {
        let mut g = game();
        g.advance(10.0, Input::default(), Steering::Raw);
        assert!((g.time() - MAX_FRAME_TIME).abs() < 1e-12, "capped");
        g.paused = true;
        g.advance(0.05, Input::default(), Steering::Raw);
        assert!((g.time() - MAX_FRAME_TIME).abs() < 1e-12, "paused");
    }

    /// Runs `g` for `seconds` with `input` held, steering as `steering` says.
    fn fly(g: &mut Game, seconds: f64, input: Input, steering: Steering) {
        for _ in 0..(seconds / 0.05).round() as usize {
            g.advance(0.05, input, steering);
        }
    }

    #[test]
    fn without_the_assist_a_spinning_ship_spins_on() {
        let mut g = game();
        let omega = g.replay.case.start.omega;
        fly(&mut g, 2.0, Input::default(), Steering::Raw);
        assert!((g.state().omega - omega).abs() < 1e-12, "no torque");
    }

    #[test]
    fn the_assist_stops_the_spin_and_holds_the_heading_on_any_ship() {
        for name in ["agile", "sluggish", "random_00", "random_07"] {
            for h in [0.01, 0.02, 0.05, 0.1] {
                let mut case = suite::find(name).expect("case exists");
                case.task.params.h = h;
                let params = case.task.params;
                let mut g = Game::new(case);
                let stop = stopping_heading(&params, &g.replay.case.start);
                fly(&mut g, 30.0, Input::default(), Steering::Sas);
                let s = g.state();
                let at = format!("{name}, h = {h}");
                assert!(s.omega.abs() < 1e-3, "{at}: still spinning at {}", s.omega);
                assert!(
                    (s.theta - stop).abs() < 0.05,
                    "{at}: held {} for {stop}",
                    s.theta
                );
                assert_eq!(g.hold, Some(stop), "{at}: held where it stopped");
                for u in &g.replay.trajectory.controls {
                    assert!(u.is_admissible(&params), "{at}: admissible");
                }
            }
        }
    }

    #[test]
    fn turning_overrides_the_assist_which_then_holds_the_new_heading() {
        let mut g = game();
        fly(&mut g, 3.0, Input::default(), Steering::Sas);
        let held = g.hold.expect("holding");
        let left = Input {
            turn: 1.0,
            ..Input::default()
        };
        fly(&mut g, 0.2, left, Steering::Sas);
        assert_eq!(g.hold, None, "the player turns");
        let params = g.replay.case.task.params;
        let guide = g.sas_heading(Steering::Sas).expect("guide while turning");
        assert_eq!(
            guide,
            stopping_heading(&params, &g.state()),
            "where it would stop"
        );
        assert!(guide > held, "ahead of the turn");
        assert_eq!(
            g.sas_heading(Steering::Raw),
            None,
            "no guide without the assist"
        );
        let tau_max = g.replay.case.task.params.tau_max;
        let last = g.replay.trajectory.controls.last().expect("controls");
        assert_eq!(last.torque, tau_max, "the player's torque, unchanged");
        fly(&mut g, 5.0, Input::default(), Steering::Sas);
        let s = g.state();
        let new = g.hold.expect("holding again");
        assert!(new > held, "turned left");
        assert!(
            s.omega.abs() < 1e-3 && (s.theta - new).abs() < 0.05,
            "and stopped there"
        );
    }

    #[test]
    fn wrap_turns_the_shorter_way() {
        assert!(
            (wrap(3.0 * PI / 2.0) + PI / 2.0).abs() < 1e-12,
            "three quarters left"
        );
        assert!(
            (wrap(-5.0 * PI / 2.0) + PI / 2.0).abs() < 1e-12,
            "more than a turn right"
        );
        assert_eq!(wrap(0.3), 0.3, "already short");
    }

    /// An input that points at `heading`, thrusting within `cone`.
    fn aim(heading: f64, cone: f64) -> Input {
        Input {
            aim: Some(Aim { heading, cone }),
            ..Input::default()
        }
    }

    #[test]
    fn aiming_turns_the_ship_and_thrusts_only_within_the_cone() {
        for name in ["agile", "sluggish", "random_00", "random_07"] {
            for h in [0.01, 0.02, 0.05, 0.1] {
                let mut case = suite::find(name).expect("case exists");
                case.task.params.h = h;
                let params = case.task.params;
                let heading = case.start.theta + 2.0;
                let cone = 0.1;
                let mut g = Game::new(case);
                fly(&mut g, 30.0, aim(heading, cone), Steering::Sas);
                let at = format!("{name}, h = {h}");
                let r = &g.replay.trajectory;
                for (s, u) in r.states.iter().zip(&r.controls) {
                    assert!(u.is_admissible(&params), "{at}: admissible");
                    let error = wrap(heading - s.theta).abs();
                    let thrusting = u.thrust > 0.0;
                    assert_eq!(thrusting, error <= cone, "{at}: thrust at {error}");
                }
                let s = g.state();
                assert!(wrap(heading - s.theta).abs() < 1e-3, "{at}: faces the aim");
                assert!(s.omega.abs() < 1e-3, "{at}: and stays");
                assert_eq!(g.hold, None, "{at}: the aim, not the assist");
            }
        }
    }

    #[test]
    fn aiming_turns_the_shorter_way_and_overrides_the_turn() {
        let mut case = suite::find("sluggish").expect("case exists");
        case.start.omega = 0.0;
        let start = case.start.theta;
        let mut g = Game::new(case);
        // A quarter turn right, as three quarters left; the player turning left besides.
        let input = Input {
            turn: 1.0,
            ..aim(start + 1.5 * PI, 0.1)
        };
        fly(&mut g, 0.5, input, Steering::Raw);
        assert!(g.state().theta < start, "turns right");
        assert!(
            g.replay.trajectory.controls.iter().all(|u| u.torque <= 0.0),
            "the aim's torque, not the turn's"
        );
    }

    #[test]
    fn a_flight_starting_in_the_target_set_lands_at_once() {
        let mut g = Game::new(suite::find("at_goal").expect("case exists"));
        assert_eq!(g.landed(), None, "not before it starts");
        g.advance(0.001, Input::default(), Steering::Raw);
        assert_eq!(g.landed(), Some(0), "at the first sample");
    }
}
