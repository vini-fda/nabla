//! `flip_and_burn_tuned`, made aware of the target-set tolerances, then retuned by coordinate
//! descent on the standard suite (score 12389 → 8058). Checked on 3000 extra random cases
//! (`suite::random`, seeds 12345, 777 and 4242).
//!
//! The target set only asks for `‖p − B‖ ≤ ε_p`, `‖v‖ ≤ ε_v` and `|ω| ≤ ε_ω`. Two changes use that:
//!
//! * Finisher: every step, predict with `step` whether coasting (no thrust) while braking the
//!   spin reaches the target set within `FINISH_STEPS`. If it does, do exactly that. Without it,
//!   the ship could park at the goal still spinning, its heading loop chasing the direction of a
//!   rounding-level velocity error for hundreds of steps (31.5 s instead of ~6 s on `agile`).
//! * Deadband: velocity errors below a fraction of `ε_v` are ignored, unless they are a large
//!   fraction of the desired speed (so a ship at rest away from the goal always moves). The old
//!   deadband was a fraction of `min(ε_v, desired speed)`, which vanishes at the goal.
//!
//! The finisher cleans up whatever the approach leaves, which lets the retuned constants be
//! much more aggressive (e.g. a far shorter turn-around margin).
//!
//! Otherwise this is the two-layer "flip and burn" feedback of `flip_and_burn`:
//!
//! Outer loop: aim for a velocity pointing at the goal, with the largest speed from which the
//! ship can still turn around (time `t_turn`) and brake at a fraction of full thrust before the
//! goal: `v²/(2 a_brake) + v t_turn = d`. The desired acceleration is the velocity error over a
//! time constant.
//!
//! Inner loop: point the ship along the desired acceleration. The angular-speed setpoint follows
//! the time-optimal braking curve `ω = sign(e) √(2 α' |e|)` (linear near zero), and the torque
//! reaches that setpoint in one step when not saturated. Thrust is only applied when the heading
//! is within `ALIGN` of the desired acceleration.

use ship2d_optimization::*;
use std::f64::consts::PI;

/// Fraction of full thrust assumed available for braking.
const BRAKE_FRACTION: f64 = 0.9282739230812116;
/// Fraction of full angular acceleration assumed by the heading loop.
const TURN_FRACTION: f64 = 1.056248512195863;
/// Safety factor on the time needed to turn around.
const TURN_MARGIN: f64 = 0.5429347937892856;
/// Time constant (in steps) of the velocity loop.
const VELOCITY_STEPS: f64 = 6.199639511971215;
/// Time constant (in steps) of the heading loop near its setpoint.
const HEADING_STEPS: f64 = 1.5656826787495068;
/// Thrust only within this heading error (rad).
const ALIGN: f64 = 0.36712137654176014;
/// Velocity errors below this fraction of the velocity tolerance are ignored...
const DEADBAND: f64 = 0.1875;
/// ...unless they exceed this fraction of the desired speed.
const RELATIVE_DEADBAND: f64 = 0.81;
/// Longest coast the finisher looks ahead, in steps.
const FINISH_STEPS: usize = 40;

pub struct Solution {
    task: Task,
    /// Full linear acceleration `F_max / m`.
    accel: f64,
    /// Full angular acceleration `τ_max / I`.
    alpha: f64,
}

fn wrap(angle: f64) -> f64 {
    (angle + PI).rem_euclid(2.0 * PI) - PI
}

impl Solution {
    /// No thrust, and the torque that stops the spin (in one step when not saturated).
    fn spin_down(&self, s: &State) -> Control {
        let p = &self.task.params;
        Control { thrust: 0.0, torque: (-p.inertia * s.omega / p.h).clamp(-p.tau_max, p.tau_max) }
    }

    /// Whether repeating `spin_down` reaches the target set within `FINISH_STEPS`.
    fn finishes(&self, s: &State) -> bool {
        let mut s = *s;
        for _ in 0..FINISH_STEPS {
            s = step(&self.task.params, &s, &self.spin_down(&s));
            if self.task.reached(&s) {
                return true;
            }
        }
        false
    }
}

impl Controller for Solution {
    fn new(task: &Task) -> Self {
        let p = &task.params;
        Solution { task: *task, accel: p.f_max / p.mass, alpha: p.tau_max / p.inertia }
    }

    fn control(&mut self, _k: usize, s: &State) -> Control {
        if self.finishes(s) {
            return self.spin_down(s);
        }
        let p = &self.task.params;
        let h = p.h;

        // Outer loop: desired velocity and acceleration.
        let (rx, ry) = (self.task.goal.x - s.x, self.task.goal.y - s.y);
        let dist = rx.hypot(ry);
        let a_brake = BRAKE_FRACTION * self.accel;
        let t_turn = TURN_MARGIN * 2.0 * (PI / (TURN_FRACTION * self.alpha)).sqrt() + 2.0 * h;
        let speed = a_brake * ((t_turn * t_turn + 2.0 * dist / a_brake).sqrt() - t_turn);
        let (vx_des, vy_des) = if dist > 0.0 { (speed * rx / dist, speed * ry / dist) } else { (0.0, 0.0) };
        let (ex, ey) = (vx_des - s.vx, vy_des - s.vy);
        let v_err = ex.hypot(ey);
        let active = v_err > DEADBAND * self.task.tolerance.velocity || v_err > RELATIVE_DEADBAND * speed;
        let accel = (v_err / (VELOCITY_STEPS * h)).min(self.accel);

        // Inner loop: heading.
        let heading = if active { ey.atan2(ex) } else { s.theta };
        let err = wrap(heading - s.theta);
        let omega_des = err.signum()
            * (2.0 * TURN_FRACTION * self.alpha * err.abs()).sqrt().min(err.abs() / (HEADING_STEPS * h));
        let torque = (p.inertia * (omega_des - s.omega) / h).clamp(-p.tau_max, p.tau_max);

        let thrust = if active && err.abs() < ALIGN {
            (p.mass * accel * err.cos()).clamp(0.0, p.f_max)
        } else {
            0.0
        };
        Control { thrust, torque }
    }
}
