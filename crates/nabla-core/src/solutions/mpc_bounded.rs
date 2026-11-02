//! Receding-horizon MPC with a hard bound on the work of every sample.
//!
//! [`super::mpc`] plans from scratch whenever the state isn't exactly the one it predicted. Under
//! a plant its model doesn't match (gravity, single-precision physics), that is every sample, and
//! each plan takes many times the sampling period. This policy keeps its plan instead, and spreads
//! the same kind of search over the flight:
//!
//! * **The budget.** A call of `control` does at most [`WORK_PER_SAMPLE`] planning steps, plus
//!   one evaluation of the law. All the planning draws on that budget, and a search that runs out
//!   goes on at the next sample. The budget counts steps rather than time, so plans don't depend
//!   on how fast the machine is.
//! * **The law.** A two-layer feedback law flies whenever there is no plan: first, while the goal
//!   is further than [`HORIZON`] steps, and again whenever a plan is given up. It costs almost
//!   nothing, and being feedback it needs no prediction to stay right.
//! * **The plan.** Once the law's own rollout reaches the target set within the horizon, that
//!   rollout is the plan. While the state is the predicted one, a candidate a few steps shorter
//!   gets Gauss–Newton iterations (as in [`super::mpc`]), and replaces the plan if it reaches the
//!   target set sooner. Otherwise the iterations correct the plan from the true state.
//! * **The disturbance.** Whatever the model missed shows in the velocity: `(v − v_predicted)/h`
//!   estimates an acceleration the model doesn't know of, such as gravity. Both the law and the
//!   predictions take it as constant, which is what makes hovering at the goal possible. Where
//!   the plant is the model, as on the test suite, it stays exactly zero.

use super::mpc::{atan2, sin_cos};
use crate::{Control, Controller, Params, State, Task, step};
use std::f64::consts::PI;

/// Planning steps a sample may spend. At about 100 ns a step (measured natively), that's about
/// 2 ms, against a sampling period of 20 ms.
pub const WORK_PER_SAMPLE: usize = 20_000;
/// Longest plan, in steps. Further out the law flies, as a longer plan is too costly to correct.
pub const HORIZON: usize = 2048;
/// Samples between looking for a plan, while the goal is beyond the horizon.
const SEED_RETRY: usize = 16;
/// Part of the tolerances a plan must end within, to leave room for what the model misses.
const HIT: f64 = 0.8;
/// Aim for this fraction of each tolerance at the end of the horizon.
const MARGIN: f64 = 0.6336;
/// Levenberg–Marquardt damping, relative to the mean diagonal of `J Jᵀ`.
const DAMPING: f64 = 7.182e-10;
/// Halvings of the line search before an iteration gives up.
const LINE_SEARCH: usize = 4;
/// Gauss–Newton iterations of a candidate before it is dropped.
const CANDIDATE_ITERS: usize = 30;
/// First shortening of a new plan, as a fraction of its steps.
const FIRST_SHORTEN: f64 = 0.012;
/// Longest wait (samples) between shortenings after failures.
const PATIENCE: usize = 7;
/// How much of what the last prediction missed goes into the disturbance, at each sample.
const OBSERVER_GAIN: f64 = 0.25;
/// Longest coast the law looks ahead to finish, in steps.
const FINISH_STEPS: usize = 40;

// The law's constants: the first and best tuned of `mpc`'s seed laws.
/// Fraction of the available acceleration assumed for braking.
const BRAKE_FRACTION: f64 = 0.9189911838503996;
/// Fraction of full angular acceleration assumed by the heading loop.
const TURN_FRACTION: f64 = 0.9506236609762766;
/// Safety factor on the time needed to turn around.
const TURN_MARGIN: f64 = 0.5972282731682141;
/// Time constant (steps) of the velocity loop.
const VELOCITY_STEPS: f64 = 7.677088681433671;
/// Time constant (steps) of the heading loop near its setpoint.
const HEADING_STEPS: f64 = 1.9418379423190761;
/// Thrust only within this heading error (rad).
const ALIGN: f64 = 0.29783295299101137;
/// Velocity errors below this many velocity tolerances don't call for thrust...
const DEADBAND: f64 = 0.16706250000000003;
/// ...nor those below this many times the speed aimed for.
const RELATIVE_DEADBAND: f64 = 1.1397686269921878;
/// The law brakes with at least this fraction of full thrust, however strong the disturbance.
const MIN_AVAILABLE: f64 = 0.2;

/// Gauss–Legendre nodes and weights on `[-1, 1]`, as in `src/model.rs`.
#[allow(clippy::excessive_precision)]
const GL_NODES: [f64; 8] = [
    -0.960_289_856_497_536_3,
    -0.796_666_477_413_626_7,
    -0.525_532_409_916_329_0,
    -0.183_434_642_495_649_8,
    0.183_434_642_495_649_8,
    0.525_532_409_916_329_0,
    0.796_666_477_413_626_7,
    0.960_289_856_497_536_3,
];
#[allow(clippy::excessive_precision)]
const GL_WEIGHTS: [f64; 8] = [
    0.101_228_536_290_376_3,
    0.222_381_034_453_374_5,
    0.313_706_645_877_887_3,
    0.362_683_783_378_362_0,
    0.362_683_783_378_362_0,
    0.313_706_645_877_887_3,
    0.222_381_034_453_374_5,
    0.101_228_536_290_376_3,
];

/// `√(x² + y²)`, without libm's `hypot`.
fn norm(x: f64, y: f64) -> f64 {
    (x * x + y * y).sqrt()
}

/// `x` rounded to 32 significant bits, so that states that differ only in their last bits (as
/// computed by different platforms) plan identically.
fn quantize(x: f64) -> f64 {
    const DROP: u32 = 20;
    if !x.is_finite() {
        return x;
    }
    let bits = x.to_bits();
    f64::from_bits(((bits >> DROP) + ((bits >> (DROP - 1)) & 1)) << DROP)
}

fn quantize_state(s: &State) -> State {
    State {
        x: quantize(s.x),
        y: quantize(s.y),
        vx: quantize(s.vx),
        vy: quantize(s.vy),
        theta: quantize(s.theta),
        omega: quantize(s.omega),
    }
}

fn wrap(angle: f64) -> f64 {
    (angle + PI).rem_euclid(2.0 * PI) - PI
}

/// The entries of the step's sensitivities that depend on the state and control, rows
/// `x, y, vx, vy`: `∂/∂θ`, `∂/∂ω`, `∂/∂F`, `∂/∂τ`. The rest of `A = ∂s⁺/∂s` is the identity plus
/// `h` at `(x, vx)`, `(y, vy)`, `(θ, ω)`, and the rest of `B = ∂s⁺/∂u` is `h²/(2I)`, `h/I` in
/// the `τ` column at rows `θ`, `ω`.
type Sens = [[f64; 4]; 4];

/// An acceleration, `(x, y)`.
type Accel = [f64; 2];

/// `s` after a step's worth of the constant acceleration `d`, on top of the ship's own motion.
fn drift(mut s: State, h: f64, d: Accel) -> State {
    s.x += 0.5 * h * h * d[0];
    s.y += 0.5 * h * h * d[1];
    s.vx += h * d[0];
    s.vy += h * d[1];
    s
}

#[cfg(test)]
thread_local! {
    /// Planning steps taken: a count of the work that the budget's own bookkeeping can't fool.
    static STEPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The planning model: `step` under the disturbance `d`, and its sensitivities, in one quadrature
/// pass. A constant acceleration adds nothing to the sensitivities. As in [`super::mpc`], the
/// portable `sin_cos` makes the plans the same on every platform.
///
/// With `φ(r) = θ + ω r + τ/(2I) r²`, `c = ∫ e(φ)`, `d = ∫ (h − r) e(φ)` and `e⊥ = ∂e/∂φ`:
/// `∂c/∂θ = ∫ e⊥`, `∂c/∂ω = ∫ r e⊥`, `∂c/∂τ = ∫ r²/(2I) e⊥`, and likewise for `d`.
fn step_sens(p: &Params, s: &State, u: &Control, disturbance: Accel) -> (State, Sens) {
    #[cfg(test)]
    STEPS.with(|steps| steps.set(steps.get() + 1));
    let h = p.h;
    let alpha = u.torque / p.inertia;
    let span = s.omega.abs() * h + 0.5 * alpha.abs() * h * h;
    let panels = (span.ceil() as usize).clamp(1, 4096);
    let width = h / panels as f64;
    // c, d, then ∫ r e⊥, ∫ (h − r) r e⊥, ∫ r²/2 e⊥, ∫ (h − r) r²/2 e⊥, as (x, y) pairs.
    let mut q = [[0.0; 2]; 6];
    for j in 0..panels {
        let mid = (j as f64 + 0.5) * width;
        for (node, weight) in GL_NODES.iter().zip(GL_WEIGHTS) {
            let r = mid + 0.5 * width * node;
            let w = 0.5 * width * weight;
            let (sin, cos) = sin_cos(s.theta + s.omega * r + 0.5 * alpha * r * r);
            let (wd, r2) = (w * (h - r), 0.5 * r * r);
            q[0][0] += w * cos;
            q[0][1] += w * sin;
            q[1][0] += wd * cos;
            q[1][1] += wd * sin;
            q[2][0] -= w * r * sin;
            q[2][1] += w * r * cos;
            q[3][0] -= wd * r * sin;
            q[3][1] += wd * r * cos;
            q[4][0] -= w * r2 * sin;
            q[4][1] += w * r2 * cos;
            q[5][0] -= wd * r2 * sin;
            q[5][1] += wd * r2 * cos;
        }
    }
    let [c, d, c1, d1, c2, d2] = q;
    let a = u.thrust / p.mass;
    let theta = s.theta + h * s.omega + 0.5 * alpha * h * h;
    let omega = s.omega + alpha * h;
    let next = if u.thrust == 0.0 {
        State {
            x: s.x + h * s.vx,
            y: s.y + h * s.vy,
            vx: s.vx,
            vy: s.vy,
            theta,
            omega,
        }
    } else {
        State {
            x: s.x + h * s.vx + a * d[0],
            y: s.y + h * s.vy + a * d[1],
            vx: s.vx + a * c[0],
            vy: s.vy + a * c[1],
            theta,
            omega,
        }
    };
    let (im, ii) = (1.0 / p.mass, 1.0 / p.inertia);
    // ∂/∂θ: the e⊥ integrals of c and d are (−c_y, c_x) and (−d_y, d_x).
    let sens = [
        [-a * d[1], a * d1[0], d[0] * im, a * d2[0] * ii],
        [a * d[0], a * d1[1], d[1] * im, a * d2[1] * ii],
        [-a * c[1], a * c1[0], c[0] * im, a * c2[0] * ii],
        [a * c[0], a * c1[1], c[1] * im, a * c2[1] * ii],
    ];
    (drift(next, h, disturbance), sens)
}

/// Whether `s` is well inside the target set: within [`HIT`] of each tolerance.
fn inside(task: &Task, s: &State) -> bool {
    let t = &task.tolerance;
    norm(s.x - task.goal.x, s.y - task.goal.y) <= HIT * t.position
        && norm(s.vx, s.vy) <= HIT * t.velocity
        && s.omega.abs() <= HIT * t.omega
}

/// The scaled terminal residual (5) and its Jacobian `∂r/∂s` (5×6).
fn residual(task: &Task, s: &State) -> ([f64; 5], [[f64; 6]; 5]) {
    let t = &task.tolerance;
    let mut r = [0.0; 5];
    let mut dr = [[0.0; 6]; 5];
    // Vector groups: position (state 0, 1) and velocity (state 2, 3).
    let groups = [
        (s.x - task.goal.x, s.y - task.goal.y, t.position, 0),
        (s.vx, s.vy, t.velocity, 2),
    ];
    for (ex, ey, eps, i) in groups {
        let n = norm(ex, ey);
        let threshold = MARGIN * eps;
        if n > threshold {
            let f = 1.0 - threshold / n;
            r[i] = f * ex / eps;
            r[i + 1] = f * ey / eps;
            let (ux, uy, g) = (ex / n, ey / n, threshold / n);
            dr[i][i] = (f + g * ux * ux) / eps;
            dr[i][i + 1] = g * ux * uy / eps;
            dr[i + 1][i] = g * ux * uy / eps;
            dr[i + 1][i + 1] = (f + g * uy * uy) / eps;
        }
    }
    let threshold = MARGIN * t.omega;
    if s.omega.abs() > threshold {
        r[4] = (s.omega - threshold * s.omega.signum()) / t.omega;
        dr[4][5] = 1.0 / t.omega;
    }
    (r, dr)
}

fn norm2(r: &[f64; 5]) -> f64 {
    r.iter().map(|x| x * x).sum()
}

fn add_outer(m: &mut [[f64; 5]; 5], g: &[f64; 5]) {
    for i in 0..5 {
        for j in i..5 {
            m[i][j] += g[i] * g[j];
        }
    }
}

/// Solve `M y = r` for symmetric positive definite `M` (upper triangle used), by Cholesky.
fn solve5(m: &[[f64; 5]; 5], r: &[f64; 5]) -> Option<[f64; 5]> {
    let mut l = [[0.0; 5]; 5];
    for i in 0..5 {
        for j in 0..=i {
            let mut sum = m[j][i];
            for (a, b) in l[i][..j].iter().zip(&l[j][..j]) {
                sum -= a * b;
            }
            if i == j {
                if sum <= 0.0 || !sum.is_finite() {
                    return None;
                }
                l[i][i] = sum.sqrt();
            } else {
                l[i][j] = sum / l[j][j];
            }
        }
    }
    let mut y = [0.0; 5];
    for i in 0..5 {
        let mut sum = r[i];
        for k in 0..i {
            sum -= l[i][k] * y[k];
        }
        y[i] = sum / l[i][i];
    }
    for i in (0..5).rev() {
        let mut sum = y[i];
        for k in i + 1..5 {
            sum -= l[k][i] * y[k];
        }
        y[i] = sum / l[i][i];
    }
    Some(y)
}

/// A control sequence with its predicted states and sensitivities.
struct Track {
    u: Vec<Control>,
    s: Vec<State>,
    sens: Vec<Sens>,
}

impl Track {
    fn new(cap: usize) -> Self {
        Track {
            u: vec![Control::default(); cap],
            s: vec![State::default(); cap + 1],
            sens: vec![[[0.0; 4]; 4]; cap],
        }
    }
}

/// The feedback law flying while there is no plan, and seeding the plans. Stateless but for the
/// disturbance it is given, so its rollout from any state is reproducible.
struct Law {
    task: Task,
    accel: f64,
    alpha: f64,
}

impl Law {
    fn new(task: &Task) -> Self {
        let p = &task.params;
        Law {
            task: *task,
            accel: p.f_max / p.mass,
            alpha: p.tau_max / p.inertia,
        }
    }

    fn spin_down(&self, s: &State) -> Control {
        let p = &self.task.params;
        Control {
            thrust: 0.0,
            torque: (-p.inertia * s.omega / p.h).clamp(-p.tau_max, p.tau_max),
        }
    }

    /// Whether the goal is near enough for a coast to reach it, as it can't be from afar.
    fn within_coast(&self, s: &State, disturbance: Accel) -> bool {
        let time = FINISH_STEPS as f64 * self.task.params.h;
        let reach = (norm(s.vx, s.vy) + norm(disturbance[0], disturbance[1]) * time) * time;
        norm(s.x - self.task.goal.x, s.y - self.task.goal.y) <= reach + self.task.tolerance.position
    }

    /// Whether spinning down from `s`, thrust off, ends in the target set. Costs up to
    /// [`FINISH_STEPS`] planning steps, within coasting distance of the goal.
    fn finishes(&self, s: &State, disturbance: Accel) -> bool {
        let mut s = *s;
        for _ in 0..FINISH_STEPS {
            let u = self.spin_down(&s);
            s = step_sens(&self.task.params, &s, &u, disturbance).0;
            if inside(&self.task, &s) {
                return true;
            }
        }
        false
    }

    fn control(&self, s: &State, disturbance: Accel) -> Control {
        let near = self.within_coast(s, disturbance);
        if near && self.finishes(s, disturbance) {
            return self.spin_down(s);
        }
        let p = &self.task.params;
        let h = p.h;
        let (rx, ry) = (self.task.goal.x - s.x, self.task.goal.y - s.y);
        let dist = norm(rx, ry);
        let [dx, dy] = disturbance;
        // What's left to brake with, after pushing back the disturbance.
        let available = (self.accel - norm(dx, dy)).max(MIN_AVAILABLE * self.accel);
        let a_brake = BRAKE_FRACTION * available;
        let t_turn = TURN_MARGIN * 2.0 * (PI / (TURN_FRACTION * self.alpha)).sqrt() + 2.0 * h;
        let speed = a_brake * ((t_turn * t_turn + 2.0 * dist / a_brake).sqrt() - t_turn);
        let (vx_des, vy_des) = if dist > 0.0 {
            (speed * rx / dist, speed * ry / dist)
        } else {
            (0.0, 0.0)
        };
        let (ex, ey) = (vx_des - s.vx, vy_des - s.vy);
        let v_err = norm(ex, ey);
        let active =
            v_err > DEADBAND * self.task.tolerance.velocity || v_err > RELATIVE_DEADBAND * speed;
        // The acceleration asked of the thruster: toward the velocity wanted, less the disturbance.
        let push = if active {
            (v_err / (VELOCITY_STEPS * h)).min(self.accel) / v_err
        } else {
            0.0
        };
        let (ax, ay) = (push * ex - dx, push * ey - dy);
        let wanted = norm(ax, ay);
        let heading = if wanted > 0.0 { atan2(ay, ax) } else { s.theta };
        let err = wrap(heading - s.theta);
        let omega_des = err.signum()
            * (2.0 * TURN_FRACTION * self.alpha * err.abs())
                .sqrt()
                .min(err.abs() / (HEADING_STEPS * h));
        let torque = (p.inertia * (omega_des - s.omega) / h).clamp(-p.tau_max, p.tau_max);
        let thrust = if err.abs() < ALIGN {
            (p.mass * wanted.min(self.accel) * sin_cos(err).1).clamp(0.0, p.f_max)
        } else {
            0.0
        };
        Control { thrust, torque }
    }
}

/// How a Gauss–Newton solve ended.
#[derive(Clone, Copy, PartialEq)]
enum Solve {
    /// The rollout enters the target set at this step.
    Hit(usize),
    /// An iteration failed to lower the residual, or the candidate ran out of iterations.
    Stalled,
    /// Out of work for this sample, still making progress: `cand` holds the last iterate.
    Running,
}

/// Rollout result: the first step in the target set, or the scaled residual at the end.
enum Rollout {
    Hit(usize),
    Miss([f64; 5]),
}

pub struct Solution {
    task: Task,
    law: Law,
    /// The plan: `plan[head..len]`, from the current state. None if `head >= len`.
    plan: Vec<Control>,
    head: usize,
    len: usize,
    /// The true state predicted for the next sample.
    expected: State,
    /// The planning model's prediction of the next state, from the last quantized one.
    next: State,
    /// The planning model's state now.
    state: State,
    /// The estimate of the acceleration the model doesn't know of.
    disturbance: Accel,
    /// Planning steps left for this sample.
    work_left: usize,
    /// Samples until the next look for a plan.
    seed_wait: usize,
    /// Gauss–Newton iterate and line-search trial.
    cand: Track,
    trial: Track,
    /// Scratch: Jacobian columns of the residual, and the Gauss–Newton step.
    jac: Vec<[[f64; 5]; 2]>,
    delta: Vec<[f64; 2]>,
    /// The working candidate is `cand.u` over `work` steps (none if 0), after `iters` iterations;
    /// it aims to be `shorten` steps shorter than the plan.
    work: usize,
    iters: usize,
    shorten: usize,
    /// After a failed one-step shortening, wait `cooldown` samples, then `backoff` after the
    /// next failure.
    cooldown: usize,
    backoff: usize,
}

impl Solution {
    /// Spend `n` planning steps of this sample's budget.
    fn spend(&mut self, n: usize) {
        self.work_left = self.work_left.saturating_sub(n);
    }

    /// Predict `cand` or `trial`'s controls over `n` steps from the model state `s0`, storing
    /// states and sensitivities, at a cost of the steps taken.
    fn rollout(&mut self, trial: bool, s0: &State, n: usize) -> Rollout {
        let p = self.task.params;
        let disturbance = self.disturbance;
        let track = if trial {
            &mut self.trial
        } else {
            &mut self.cand
        };
        track.s[0] = *s0;
        let mut taken = n;
        let mut hit = None;
        for k in 0..=n {
            if inside(&self.task, &track.s[k]) {
                taken = k;
                hit = Some(k);
                break;
            }
            if k < n {
                (track.s[k + 1], track.sens[k]) =
                    step_sens(&p, &track.s[k], &track.u[k], disturbance);
            }
        }
        let outcome = match hit {
            Some(k) => Rollout::Hit(k),
            None => Rollout::Miss(residual(&self.task, &track.s[n]).0),
        };
        self.spend(taken);
        outcome
    }

    /// Correct `cand` over `n` steps from the model state `s0` by projected Gauss–Newton, as far
    /// as this sample's work goes. The first control stays fixed if `fix_first`. The caller makes
    /// sure the work left pays for the first prediction.
    fn solve(&mut self, s0: &State, n: usize, fix_first: bool) -> Solve {
        let p = self.task.params;
        let mut r = match self.rollout(false, s0, n) {
            Rollout::Hit(m) => return Solve::Hit(m),
            Rollout::Miss(r) => r,
        };
        let mut merit = norm2(&r);
        let scale = [p.f_max, p.tau_max];
        let (h, first) = (p.h, fix_first as usize);
        let (b4, b5) = (0.5 * h * h / p.inertia, h / p.inertia);
        loop {
            // An iteration: the linearization, charged as a rollout, and at least one trial.
            if self.work_left < 2 * n {
                return Solve::Running;
            }
            if self.iters >= CANDIDATE_ITERS {
                return Solve::Stalled;
            }
            self.iters += 1;
            self.spend(n);
            // Backward pass: Jacobian columns of the residual, in scaled controls.
            let (_, mut lam) = residual(&self.task, &self.cand.s[n]);
            for k in (0..n).rev() {
                let sn = &self.cand.sens[k];
                for (i, l) in lam.iter().enumerate() {
                    let gf = l[0] * sn[0][2] + l[1] * sn[1][2] + l[2] * sn[2][2] + l[3] * sn[3][2];
                    let gt = l[0] * sn[0][3]
                        + l[1] * sn[1][3]
                        + l[2] * sn[2][3]
                        + l[3] * sn[3][3]
                        + l[4] * b4
                        + l[5] * b5;
                    self.jac[k][0][i] = gf * scale[0];
                    self.jac[k][1][i] = gt * scale[1];
                }
                // λ ← λ A.
                for row in lam.iter_mut() {
                    let [l0, l1, l2, l3, l4, l5] = *row;
                    row[2] = l2 + h * l0;
                    row[3] = l3 + h * l1;
                    row[4] = l4 + l0 * sn[0][0] + l1 * sn[1][0] + l2 * sn[2][0] + l3 * sn[3][0];
                    row[5] =
                        l5 + h * l4 + l0 * sn[0][1] + l1 * sn[1][1] + l2 * sn[2][1] + l3 * sn[3][1];
                }
            }

            // Free set: controls not at a bound, or at one with the unconstrained step pointing
            // inward. `y` solves (J_f J_fᵀ + λ) y = r over the free set; the step is −J_fᵀ y.
            let mut y = [0.0; 5];
            let mut y0 = [0.0; 5];
            for pass in 0..2 {
                let mut m = [[0.0; 5]; 5];
                for k in first..n {
                    for c in 0..2 {
                        if pass == 0 || self.free(k, c, &y0) {
                            add_outer(&mut m, &self.jac[k][c]);
                        }
                    }
                }
                let mean = (0..5).map(|i| m[i][i]).sum::<f64>() / 5.0;
                for (i, row) in m.iter_mut().enumerate() {
                    row[i] += DAMPING * mean + 1e-300;
                }
                let Some(sol) = solve5(&m, &r) else {
                    return Solve::Stalled;
                };
                y = sol;
                if pass == 0 {
                    y0 = y;
                }
            }
            for k in 0..n {
                for (c, sc) in scale.iter().enumerate() {
                    self.delta[k][c] = if k >= first && self.free(k, c, &y0) {
                        let g = &self.jac[k][c];
                        -(0..5).map(|i| g[i] * y[i]).sum::<f64>() * sc
                    } else {
                        0.0
                    };
                }
            }

            // Line search on the merit.
            let mut t = 1.0;
            let mut accepted = false;
            for _ in 0..=LINE_SEARCH {
                if self.work_left < n {
                    return Solve::Running;
                }
                for k in 0..n {
                    let (u, d) = (self.cand.u[k], self.delta[k]);
                    self.trial.u[k] = Control {
                        thrust: (u.thrust + t * d[0]).clamp(0.0, p.f_max),
                        torque: (u.torque + t * d[1]).clamp(-p.tau_max, p.tau_max),
                    };
                }
                match self.rollout(true, s0, n) {
                    Rollout::Hit(m) => {
                        std::mem::swap(&mut self.cand, &mut self.trial);
                        return Solve::Hit(m);
                    }
                    Rollout::Miss(r2) if norm2(&r2) < merit => {
                        (r, merit) = (r2, norm2(&r2));
                        accepted = true;
                        break;
                    }
                    Rollout::Miss(..) => t *= 0.5,
                }
            }
            if !accepted {
                return Solve::Stalled;
            }
            std::mem::swap(&mut self.cand, &mut self.trial);
        }
    }

    /// Whether component `c` (thrust, torque) of step `k` is free, given the unconstrained `y`.
    fn free(&self, k: usize, c: usize, y: &[f64; 5]) -> bool {
        let p = &self.task.params;
        let g = &self.jac[k][c];
        let delta = -(0..5).map(|i| g[i] * y[i]).sum::<f64>();
        let (v, lo, hi) = match c {
            0 => (self.cand.u[k].thrust, 0.0, p.f_max),
            _ => (self.cand.u[k].torque, -p.tau_max, p.tau_max),
        };
        !((v <= lo && delta < 0.0) || (v >= hi && delta > 0.0))
    }

    /// Resample `plan[head..len]` in time to `n` steps, into `cand`.
    fn resample(&mut self, n: usize) {
        let l = self.len - self.head;
        for j in 0..n {
            let i = ((j as f64 + 0.5) * l as f64 / n as f64) as usize;
            self.cand.u[j] = self.plan[self.head + i.min(l - 1)];
        }
    }

    /// Make `cand.u[..m]` the plan.
    fn accept(&mut self, m: usize) {
        std::mem::swap(&mut self.plan, &mut self.cand.u);
        self.head = 0;
        self.len = m;
        self.work = 0;
    }

    /// With the state as predicted, the plan still holds: spend the sample's work on a candidate
    /// that's shorter, keeping the first control so that it stays valid after the step.
    fn improve(&mut self) {
        if self.cooldown > 0 {
            self.cooldown -= 1;
            return;
        }
        let s = self.state;
        loop {
            let l = self.len - self.head;
            if l <= 1 {
                return;
            }
            if self.work_left < 3 * l {
                return;
            }
            if self.work == 0 {
                self.work = l.saturating_sub(self.shorten).max(1);
                self.resample(self.work);
                self.iters = 0;
            }
            // The plan's control is the one applied next, whichever way this goes.
            self.cand.u[0] = self.plan[self.head];
            match self.solve(&s, self.work, true) {
                Solve::Hit(m) if m < l => {
                    self.accept(m);
                    self.shorten *= 2;
                    self.backoff = 1;
                }
                Solve::Running => return,
                _ => {
                    self.work = 0;
                    if self.shorten > 1 {
                        self.shorten /= 2;
                    } else {
                        self.cooldown = self.backoff;
                        self.backoff = (2 * self.backoff).min(PATIENCE);
                        return;
                    }
                }
            }
        }
    }

    /// With the state not as predicted, correct the plan from it, as far as the sample's work
    /// goes. An iterate still improving is the plan to go on with; a stalled one means the plan
    /// no longer works, and is given up.
    fn correct(&mut self) {
        let n = self.len - self.head;
        self.work = 0;
        self.cand.u[..n].copy_from_slice(&self.plan[self.head..self.len]);
        self.iters = 0;
        let s = self.state;
        match self.solve(&s, n, false) {
            Solve::Hit(m) => self.accept(m),
            Solve::Running => self.accept(n),
            Solve::Stalled => self.len = self.head,
        }
    }

    /// Make the law's rollout from the current state the plan, if it reaches the target set
    /// within the horizon and the sample's work.
    fn seed(&mut self) {
        self.head = 0;
        self.len = 0;
        let (p, cap) = (self.task.params, self.plan.len());
        let mut s = self.state;
        for k in 0..cap {
            if inside(&self.task, &s) {
                self.len = k;
                self.shorten = ((FIRST_SHORTEN * k as f64) as usize).max(1);
                (self.work, self.cooldown, self.backoff) = (0, 0, 1);
                return;
            }
            let near = self.law.within_coast(&s, self.disturbance);
            let cost = 1 + if near { FINISH_STEPS } else { 0 };
            if self.work_left < cost {
                break;
            }
            self.spend(cost);
            let u = self.law.control(&s, self.disturbance);
            self.plan[k] = u;
            s = step_sens(&p, &s, &u, self.disturbance).0;
        }
        self.seed_wait = SEED_RETRY;
    }

    /// Take in what the last prediction missed, in the disturbance.
    fn observe(&mut self, s: &State) {
        let p = &self.task.params;
        let limit = p.f_max / p.mass;
        let [ax, ay] = [
            (s.vx - self.expected.vx) / p.h,
            (s.vy - self.expected.vy) / p.h,
        ];
        self.disturbance[0] += OBSERVER_GAIN * ax.clamp(-limit, limit);
        self.disturbance[1] += OBSERVER_GAIN * ay.clamp(-limit, limit);
    }
}

impl Controller for Solution {
    fn new(task: &Task) -> Self {
        let cap = task.max_steps.min(HORIZON);
        Solution {
            task: *task,
            law: Law::new(task),
            plan: vec![Control::default(); cap],
            head: 0,
            len: 0,
            expected: State::default(),
            next: State::default(),
            state: State::default(),
            disturbance: [0.0; 2],
            work_left: 0,
            seed_wait: 0,
            cand: Track::new(cap),
            trial: Track::new(cap),
            jac: vec![[[0.0; 5]; 2]; cap],
            delta: vec![[0.0; 2]; cap],
            work: 0,
            iters: 0,
            shorten: 1,
            cooldown: 0,
            backoff: 1,
        }
    }

    fn control(&mut self, k: usize, s: &State) -> Control {
        self.work_left = WORK_PER_SAMPLE;
        let on_course = k > 0 && *s == self.expected;
        if k > 0 && !on_course {
            self.observe(s);
        }
        self.state = if on_course {
            self.next
        } else {
            quantize_state(s)
        };
        if self.head < self.len {
            if on_course {
                self.improve();
            } else {
                self.correct();
            }
        }
        if self.head >= self.len {
            if self.seed_wait == 0 {
                self.seed();
            } else {
                self.seed_wait -= 1;
            }
        }
        let p = self.task.params;
        let u = if self.head < self.len {
            let u = self.plan[self.head];
            self.head += 1;
            u
        } else {
            self.law.control(&self.state, self.disturbance)
        };
        self.expected = drift(step(&p, s, &u), p.h, self.disturbance);
        self.next = step_sens(&p, &self.state, &u, self.disturbance).0;
        // The working candidate continues from the next state, one step shorter.
        if self.work > 1 {
            self.cand.u.copy_within(1..self.work, 0);
            self.work -= 1;
        } else {
            self.work = 0;
        }
        u
    }

    /// The plan's rollout, or the law's without one. No optimizing, and nothing kept.
    fn prediction(&self, state: &State, steps: usize) -> Vec<State> {
        let p = self.task.params;
        let mut states = vec![*state];
        let mut s = *state;
        if self.head < self.len {
            for u in self.plan[self.head..self.len].iter().take(steps.min(500)) {
                s = drift(step(&p, &s, u), p.h, self.disturbance);
                states.push(s);
            }
        } else {
            for _ in 0..steps.min(500) {
                let u = self.law.control(&s, self.disturbance);
                s = drift(step(&p, &s, &u), p.h, self.disturbance);
                states.push(s);
            }
        }
        states
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::suite;

    /// The plant the planner's model doesn't quite match: `step` under the acceleration `g`,
    /// in single precision, as the app's physics is.
    fn plant(p: &Params, s: &State, u: &Control, g: Accel) -> State {
        let n = drift(step(p, s, u), p.h, g);
        State {
            x: f64::from(n.x as f32),
            y: f64::from(n.y as f32),
            vx: f64::from(n.vx as f32),
            vy: f64::from(n.vy as f32),
            theta: f64::from(n.theta as f32),
            omega: f64::from(n.omega as f32),
        }
    }

    /// Flies `start` under `g`: the sample it lands at, if within `samples`, and the most
    /// planning steps any sample took.
    fn fly(task: &Task, start: State, g: Accel, samples: usize) -> (Option<usize>, usize) {
        let mut policy = <Solution as Controller>::new(task);
        let (mut s, mut most) = (start, 0);
        for k in 0..samples {
            if task.reached(&s) {
                return (Some(k), most);
            }
            STEPS.with(|steps| steps.set(0));
            let u = policy.control(k, &s);
            most = most.max(STEPS.with(std::cell::Cell::get));
            assert!(u.is_admissible(&task.params), "at {k}: {u:?}");
            s = plant(&task.params, &s, &u, g);
        }
        (None, most)
    }

    /// The ship of the Planet + moon world, at rest at `x` on the x axis, going to the origin.
    fn world_ship(x: f64) -> (Task, State) {
        let task = Task {
            params: Params {
                mass: 0.5,
                inertia: 0.2,
                f_max: 5.0,
                tau_max: 5.0,
                h: 0.02,
            },
            goal: crate::Goal { x: 0.0, y: 0.0 },
            tolerance: crate::Tolerance {
                position: 0.1,
                velocity: 0.1,
                omega: 0.1,
            },
            max_steps: suite::MAX_STEPS,
        };
        let start = State {
            x,
            ..State::default()
        };
        (task, start)
    }

    #[test]
    fn no_sample_takes_more_than_the_budget() {
        // Its budget, the law's look ahead, the law's step, and the planning model's next state.
        let bound = WORK_PER_SAMPLE + FINISH_STEPS + 2;
        for x in [-30.0, -100.0, -300.0, -1_000.0, -10_000.0, -100_000.0] {
            let (task, start) = world_ship(x);
            let (_, most) = fly(&task, start, [0.3, -2.4], 300);
            assert!(most <= bound, "from {x}: {most} steps in a sample");
        }
    }

    #[test]
    fn it_lands_under_a_disturbance_it_is_not_told_of() {
        for x in [-30.0, -100.0, -300.0, -1_000.0] {
            for g in [[0.0, -2.4], [1.5, 1.0], [-0.1, 0.2]] {
                let (task, start) = world_ship(x);
                let (landed, _) = fly(&task, start, g, suite::MAX_STEPS);
                assert!(landed.is_some(), "from {x} under {g:?}");
            }
        }
    }

    #[test]
    fn where_the_plant_is_the_model_there_is_no_disturbance_and_the_plan_holds() {
        let case = suite::find("agile").expect("case");
        let mut policy = <Solution as Controller>::new(&case.task);
        let mut s = case.start;
        let mut lengths = Vec::new();
        for k in 0..40 {
            let u = policy.control(k, &s);
            assert_eq!(policy.disturbance, [0.0; 2]);
            lengths.push(policy.len - policy.head);
            s = step(&case.task.params, &s, &u);
        }
        assert!(lengths.iter().all(|&left| left > 0), "{lengths:?}");
    }
}
