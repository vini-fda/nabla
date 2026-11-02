//! How the pilot's input becomes a control.
//!
//! The pieces are [`Input`], the stability assist ([`Steering`]), the aim ([`Aim`]), and
//! [`steer`], which applies them to a state. The client flies one ship by them, and a server
//! flies many.
//!
//! Both the assist and the aim use the attitude law of `docs/main.typ` ("Problem: pointing").

use std::f64::consts::TAU;

use nabla_core::{Control, Params, State};
use serde::{Deserialize, Serialize};

/// Fraction of the maximum thrust and torque applied under fine control.
pub const FINE: f64 = 0.1;

/// Fraction of the ship's angular acceleration the stability assist brakes with, leaving the
/// rest to make up for the samples' lag.
const SAS_AUTHORITY: f64 = 0.8;
/// Time constant of the stability assist's approach to the held heading, at least this many
/// sampling periods, so that it settles without overshooting at any `h`.
const SAS_TIME_CONSTANT: f64 = 0.2;
const SAS_MIN_SAMPLES: f64 = 4.0;

/// What letting go of the turn does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Aim {
    pub heading: f64,
    pub cone: f64,
}

impl Aim {
    /// The heading is within `[-2 pi, 2 pi]` and the cone within `[0, pi]`, both finite.
    pub fn in_range(&self) -> bool {
        self.heading.is_finite()
            && self.heading.abs() <= TAU
            && self.cone.is_finite()
            && (0.0..=TAU / 2.0).contains(&self.cone)
    }
}

/// The controls the player holds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Input {
    /// How hard to thrust, from 0 to 1 (full thrust).
    pub thrust: f64,
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
    /// Both's controls held: the harder thrust counts, and the turns add up, to at most a full
    /// turn either way.
    pub fn or(self, other: Self) -> Self {
        Self {
            thrust: self.thrust.max(other.thrust),
            turn: self.turn + other.turn,
            fine: self.fine || other.fine,
            aim: self.aim.or(other.aim),
        }
    }

    /// These inputs with every value in its range, whatever they were: the input of a client
    /// that cannot be trusted, made into one that can.
    ///
    /// A number that is not finite becomes `0`, and the others are clamped to their ranges. An
    /// aim whose heading is not finite or outside `[-2 pi, 2 pi]`, or whose cone is not finite or
    /// outside `[0, pi]`, becomes no aim: a pointer gives headings in `[-pi, pi]`.
    #[must_use]
    pub fn sanitized(self) -> Self {
        let unit = |x: f64, lo: f64| if x.is_finite() { x.clamp(lo, 1.0) } else { 0.0 };
        Self {
            thrust: unit(self.thrust, 0.0),
            turn: unit(self.turn, -1.0),
            fine: self.fine,
            aim: self.aim.filter(Aim::in_range),
        }
    }

    /// The control these inputs ask of a ship with `params`: always admissible.
    pub fn control(self, params: &Params) -> Control {
        let power = if self.fine { FINE } else { 1.0 };
        // NaN, which neither should ever be, thrusts and turns not at all.
        let turn = if self.turn.is_nan() {
            0.0
        } else {
            self.turn.clamp(-1.0, 1.0)
        };
        let thrust = if self.thrust.is_nan() {
            0.0
        } else {
            self.thrust.clamp(0.0, 1.0)
        };
        Control {
            thrust: power * thrust * params.f_max,
            torque: power * turn * params.tau_max,
        }
    }
}

/// The control for `input` in state `s`.
///
/// It is the input's own, except that an aim, or else the stability assist while the player is
/// not turning, takes over the torque. The aim turns the
/// ship toward its heading, the shorter way, and thrusts within its cone, as well as when the
/// player thrusts. The assist holds `hold`, which it sets, when it takes over, to where the
/// ship's spin will stop, and which is cleared while the player turns or aims.
pub fn steer(
    input: Input,
    steering: Steering,
    hold: &mut Option<f64>,
    params: &Params,
    s: &State,
) -> Control {
    let mut u = input.control(params);
    // A heading that is not finite would make the torque NaN, and one step of it would leave
    // the ship's heading NaN for good.
    let aim = input
        .aim
        .filter(|aim| aim.heading.is_finite() && aim.cone.is_finite());
    if let Some(aim) = aim {
        *hold = None;
        let error = wrap(aim.heading - s.theta);
        u.torque = hold_torque(params, s, s.theta + error);
        if error.abs() <= aim.cone {
            let thrust = Input {
                thrust: 1.0,
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
pub fn stopping_heading(params: &Params, s: &State) -> f64 {
    s.theta + s.omega * s.omega.abs() / (2.0 * sas_acceleration(params))
}

/// The torque that turns the ship in state `s` toward `heading` and holds it there.
///
/// Over the next step, it brings the angular velocity to the speed at which braking at the assist's
/// acceleration stops the ship on the heading, or, near it, to an exponential approach.
pub fn hold_torque(params: &Params, s: &State, heading: f64) -> f64 {
    let error = heading - s.theta;
    let time_constant = SAS_TIME_CONSTANT.max(SAS_MIN_SAMPLES * params.h);
    let braking = (2.0 * sas_acceleration(params) * error.abs()).sqrt();
    let omega = (error.abs() / time_constant).min(braking).copysign(error);
    let torque = params.inertia * (omega - s.omega) / params.h;
    torque.clamp(-params.tau_max, params.tau_max)
}
