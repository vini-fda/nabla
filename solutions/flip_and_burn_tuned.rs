//! `flip_and_burn` with its constants tuned by coordinate descent on the standard suite
//! (score 19805 → 12389). Checked on 2000 extra random cases (`suite::random`, seed 12345).
//!
//! Two-layer "flip and burn" feedback.
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

use nabla_core::*;
use std::f64::consts::PI;

/// Fraction of full thrust assumed available for braking.
const BRAKE_FRACTION: f64 = 0.8385254915624212;
/// Fraction of full angular acceleration assumed by the heading loop.
const TURN_FRACTION: f64 = 1.00595096399606;
/// Safety factor on the time needed to turn around.
const TURN_MARGIN: f64 = 0.9871541705259741;
/// Time constant (in steps) of the velocity loop.
const VELOCITY_STEPS: f64 = 3.889969889864292;
/// Time constant (in steps) of the heading loop near its setpoint.
const HEADING_STEPS: f64 = 2.087576904999343;
/// Thrust only within this heading error (rad).
const ALIGN: f64 = 0.36943031601686555;
/// Velocity errors below this fraction of the velocity tolerance (or of the desired speed, if
/// smaller) are ignored.
const DEADBAND: f64 = 0.375;

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

impl Controller for Solution {
    fn new(task: &Task) -> Self {
        let p = &task.params;
        Solution { task: *task, accel: p.f_max / p.mass, alpha: p.tau_max / p.inertia }
    }

    fn control(&mut self, _k: usize, s: &State) -> Control {
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
        let active = v_err > DEADBAND * self.task.tolerance.velocity.min(speed);
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
