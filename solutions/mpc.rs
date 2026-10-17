//! Shrinking-horizon MPC on the exact model.
//!
//! The controller predicts with the same `step` the simulator uses, so a control sequence whose
//! rollout from the current state enters the target set after `n` steps is guaranteed to finish
//! in `n` steps when replayed. The controller keeps such a verified plan at all times and spends
//! its effort on making it shorter:
//!
//! * At `k = 0`, from each of a few seed laws (copies of the stored feedback solutions, the first
//!   being `tolerance_aware_tuned`), the plan is the law's rollout, shortened by a continuation
//!   on the horizon `n`: each candidate is the plan resampled in time to `n` steps, corrected by
//!   projected Gauss–Newton until its rollout ends in the target set. The shortening doubles
//!   after a success and halves after a failure. The shortest plan over the seeds is kept, so no
//!   case takes longer than with `tolerance_aware_tuned`.
//! * At every later step (receding horizon), a working candidate a few steps shorter than the
//!   remaining plan gets a few more Gauss–Newton iterations. It is shifted along with the plan,
//!   so its iterations accumulate over steps. A stalled candidate is dropped, and after a failed
//!   one-step shortening the next attempt waits (exponential backoff).
//!
//! A candidate replaces the plan only if its exact rollout (the crate's `step`) reaches the
//! target set sooner.
//!
//! Gauss–Newton: the 5 residuals are the excess of `‖p − B‖`, `‖v‖` and `|ω|` over a fraction
//! `margin` of their tolerances at the end of the horizon (θ is free). Their Jacobian with respect
//! to all `2n` controls comes from one backward pass through the sensitivities of `step`, which
//! `step_sens` computes along with the prediction in one quadrature pass. The step is the
//! minimum-norm solution of the linearized equations over the controls not held at a bound, in
//! controls scaled by their limits, with Levenberg–Marquardt damping and a backtracking line
//! search; an iteration that cannot lower the residual stalls the solve.
//!
//! Portability: the planner uses its own model of `step`, built on portable `sin_cos` and
//! `atan2` (IEEE-exact operations only), and plans from the start state quantized to 32
//! significant bits. Its plans are then the same on every platform; only their confirmation uses
//! the platform's `step`. With libm's functions instead, the optimizer amplified their last-bit
//! differences into scores that differed between macOS and Linux.
//!
//! Every buffer is allocated once, in `new`, with room for `max_steps` controls.

use ship2d_optimization::*;
use std::f64::consts::PI;

/// The tunable constants.
#[derive(Clone, Copy, Debug)]
pub struct Gains {
    /// Aim for this fraction of each tolerance at the end of the horizon.
    pub margin: f64,
    /// Levenberg–Marquardt damping, relative to the mean diagonal of `J Jᵀ`.
    pub damping: f64,
    /// Gauss–Newton iterations per attempt of the initial continuation.
    pub init_iters: f64,
    /// First shortening of the initial continuation, as a fraction of the baseline's steps.
    pub init_shorten: f64,
    /// Attempts of the initial continuation.
    pub init_attempts: f64,
    /// Gauss–Newton iterations per receding-horizon attempt.
    pub step_iters: f64,
    /// Longest wait (steps) between receding-horizon attempts after failures.
    pub patience: f64,
    /// Seed laws of the initial search (1 to 4).
    pub seeds: f64,
}

impl Gains {
    /// Tuned by coordinate descent on the standard suite (27596 → 27378), accepting only
    /// improvements that also solve 300 random validation cases (seed 12345); the result solves
    /// 2000 cases it never saw (seed 777).
    pub const DEFAULT: Gains = Gains {
        margin: 0.6336000000000002,
        damping: 7.182000000000001e-10,
        init_iters: 30.0,
        init_shorten: 0.012,
        init_attempts: 59.59008,
        step_iters: 3.0,
        patience: 7.040000000000001,
        seeds: 4.0,
    };
}

/// Halvings of the line search before an iteration gives up.
const LINE_SEARCH: usize = 4;


// ---------------------------------------------------------------------------------------------
// Portable math.
//
// `f64::sin_cos`, `atan2` and `hypot` come from the platform's libm, whose last bits differ
// between macOS, Linux and Windows. The planner iterates thousands of times on its own
// predictions, which amplifies a one-ulp difference into a different plan, and so a different
// score per platform. So everything the planner computes uses only IEEE-exact operations
// (`+ − × ÷`, `sqrt`, rounding), which give the same bits everywhere; the crate's `step` and
// `Task::reached` only confirm plans.

/// `(sin x, cos x)`, within a few ulps: fdlibm's kernels on `[−π/4, π/4]` after a Cody–Waite
/// reduction by `π/2` (exact for `|x| < 2²⁰ π/2`).
#[allow(clippy::excessive_precision)]
pub fn sin_cos(x: f64) -> (f64, f64) {
    // π/2 in three 33-bit parts, so that `n · PIO2_i` is exact.
    const PIO2_1: f64 = 1.570_796_326_734_125_614_17;
    const PIO2_2: f64 = 6.077_100_506_303_965_976_60e-11;
    const PIO2_3: f64 = 2.022_266_248_711_166_455_80e-21;
    let n = (x * std::f64::consts::FRAC_2_PI).round();
    let r = ((x - n * PIO2_1) - n * PIO2_2) - n * PIO2_3;
    let (s, c) = (kernel_sin(r), kernel_cos(r));
    match (n as i64).rem_euclid(4) {
        0 => (s, c),
        1 => (c, -s),
        2 => (-s, -c),
        _ => (-c, s),
    }
}

#[allow(clippy::excessive_precision)]
fn kernel_sin(x: f64) -> f64 {
    const S1: f64 = -1.666_666_666_666_663_243_48e-01;
    const S2: f64 = 8.333_333_333_322_489_461_24e-03;
    const S3: f64 = -1.984_126_982_985_794_931_34e-04;
    const S4: f64 = 2.755_731_370_707_006_767_89e-06;
    const S5: f64 = -2.505_076_025_340_686_341_95e-08;
    const S6: f64 = 1.589_690_995_211_550_102_21e-10;
    let z = x * x;
    let r = S2 + z * (S3 + z * (S4 + z * (S5 + z * S6)));
    x + z * x * (S1 + z * r)
}

#[allow(clippy::excessive_precision)]
fn kernel_cos(x: f64) -> f64 {
    const C1: f64 = 4.166_666_666_666_660_190_37e-02;
    const C2: f64 = -1.388_888_888_887_410_957_49e-03;
    const C3: f64 = 2.480_158_728_947_672_941_78e-05;
    const C4: f64 = -2.755_731_435_139_066_330_35e-07;
    const C5: f64 = 2.087_572_321_298_174_827_90e-09;
    const C6: f64 = -1.135_964_755_778_819_482_65e-11;
    let z = x * x;
    let r = z * (C1 + z * (C2 + z * (C3 + z * (C4 + z * (C5 + z * C6)))));
    let hz = 0.5 * z;
    let w = 1.0 - hz;
    w + (((1.0 - w) - hz) + z * r)
}

/// `atan x`, within an ulp or so: fdlibm's reduction to `|x| < 7/16` and odd polynomial.
#[allow(clippy::excessive_precision)]
fn atan(x: f64) -> f64 {
    const HI: [f64; 4] = [
        4.636_476_090_008_060_935_15e-01,
        std::f64::consts::FRAC_PI_4,
        9.827_937_232_473_290_540_82e-01,
        std::f64::consts::FRAC_PI_2,
    ];
    const LO: [f64; 4] = [
        2.269_877_745_296_168_709_24e-17,
        3.061_616_997_868_383_017_93e-17,
        1.390_331_103_123_099_845_16e-17,
        6.123_233_995_736_766_035_87e-17,
    ];
    const T: [f64; 11] = [
        3.333_333_333_333_293_180_27e-01,
        -1.999_999_999_987_648_324_76e-01,
        1.428_571_427_250_346_637_11e-01,
        -1.111_111_040_546_235_578_80e-01,
        9.090_887_133_436_506_561_96e-02,
        -7.691_876_205_044_829_994_95e-02,
        6.661_073_137_387_531_206_69e-02,
        -5.833_570_133_790_573_486_45e-02,
        4.976_877_994_615_932_360_17e-02,
        -3.653_157_274_421_691_552_70e-02,
        1.628_582_011_536_578_236_23e-02,
    ];
    let (sign, a) = if x < 0.0 { (-1.0, -x) } else { (1.0, x) };
    if a.is_nan() {
        return x;
    }
    if a > 1e17 {
        return sign * (HI[3] + LO[3]);
    }
    let (id, t) = if a < 0.4375 {
        (None, a)
    } else if a < 0.6875 {
        (Some(0), (2.0 * a - 1.0) / (2.0 + a))
    } else if a < 1.1875 {
        (Some(1), (a - 1.0) / (a + 1.0))
    } else if a < 2.4375 {
        (Some(2), (a - 1.5) / (1.0 + 1.5 * a))
    } else {
        (Some(3), -1.0 / a)
    };
    let z = t * t;
    let w = z * z;
    let s1 = z * (T[0] + w * (T[2] + w * (T[4] + w * (T[6] + w * (T[8] + w * T[10])))));
    let s2 = w * (T[1] + w * (T[3] + w * (T[5] + w * (T[7] + w * T[9]))));
    sign * match id {
        None => t - t * (s1 + s2),
        Some(i) => HI[i] - ((t * (s1 + s2) - LO[i]) - t),
    }
}

/// `atan2(y, x)` in `(−π, π]`, with `atan2(0, 0) = 0`.
pub fn atan2(y: f64, x: f64) -> f64 {
    use std::f64::consts::{FRAC_PI_2, PI};
    if x > 0.0 {
        atan(y / x)
    } else if x < 0.0 {
        if y >= 0.0 { atan(y / x) + PI } else { atan(y / x) - PI }
    } else if y > 0.0 {
        FRAC_PI_2
    } else if y < 0.0 {
        -FRAC_PI_2
    } else {
        0.0
    }
}

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

// ---------------------------------------------------------------------------------------------
// Seed laws: `tolerance_aware_tuned`, with its constants as a parameter.

/// Constants of a baseline law: brake fraction, turn fraction, turn margin, velocity steps,
/// heading steps, align, deadband, relative deadband (see `tolerance_aware_tuned`).
type Law = [f64; 8];

/// The seeds of the initial search. The first is `tolerance_aware_tuned`'s `Gains::DEFAULT`, so
/// that no case takes longer than with it; the others are the constants of
/// `flip_and_burn_tuned`, of `tolerance_aware`, and a slow, cautious law, to start the search
/// from differently shaped maneuvers.
const SEEDS: [Law; 4] = [
    [0.9189911838503996, 0.9506236609762766, 0.5972282731682141, 7.677088681433671, 1.9418379423190761, 0.29783295299101137, 0.16706250000000003, 1.1397686269921878],
    [0.8385254915624212, 1.00595096399606, 0.9871541705259741, 3.889969889864292, 2.087576904999343, 0.36943031601686555, 0.375, 1.1397686269921878],
    [0.9282739230812116, 1.056248512195863, 0.5429347937892856, 6.199639511971215, 1.5656826787495068, 0.36712137654176014, 0.1875, 0.81],
    [0.6, 0.8, 1.5, 10.0, 2.0, 0.2, 0.16706250000000003, 1.1397686269921878],
];

/// Longest coast the baseline's finisher looks ahead, in steps.
const FINISH_STEPS: usize = 40;

fn wrap(angle: f64) -> f64 {
    (angle + PI).rem_euclid(2.0 * PI) - PI
}

/// A seed feedback law, on the planning model. Stateless, so its rollout from any state is
/// reproducible.
struct Baseline {
    task: Task,
    accel: f64,
    alpha: f64,
    law: Law,
}

impl Baseline {
    fn new(task: &Task) -> Self {
        let p = &task.params;
        Baseline { task: *task, accel: p.f_max / p.mass, alpha: p.tau_max / p.inertia, law: SEEDS[0] }
    }

    fn spin_down(&self, s: &State) -> Control {
        let p = &self.task.params;
        Control { thrust: 0.0, torque: (-p.inertia * s.omega / p.h).clamp(-p.tau_max, p.tau_max) }
    }

    fn finishes(&self, s: &State) -> bool {
        let mut s = *s;
        for _ in 0..FINISH_STEPS {
            s = model_step(&self.task.params, &s, &self.spin_down(&s));
            if inside(&self.task, &s) {
                return true;
            }
        }
        false
    }

    fn control(&self, s: &State) -> Control {
        if self.finishes(s) {
            return self.spin_down(s);
        }
        let p = &self.task.params;
        let h = p.h;
        let [brake_fraction, turn_fraction, turn_margin, velocity_steps, heading_steps, align, deadband, relative_deadband] =
            self.law;
        let (rx, ry) = (self.task.goal.x - s.x, self.task.goal.y - s.y);
        let dist = norm(rx, ry);
        let a_brake = brake_fraction * self.accel;
        let t_turn = turn_margin * 2.0 * (PI / (turn_fraction * self.alpha)).sqrt() + 2.0 * h;
        let speed = a_brake * ((t_turn * t_turn + 2.0 * dist / a_brake).sqrt() - t_turn);
        let (vx_des, vy_des) = if dist > 0.0 { (speed * rx / dist, speed * ry / dist) } else { (0.0, 0.0) };
        let (ex, ey) = (vx_des - s.vx, vy_des - s.vy);
        let v_err = norm(ex, ey);
        let active = v_err > deadband * self.task.tolerance.velocity || v_err > relative_deadband * speed;
        let accel = (v_err / (velocity_steps * h)).min(self.accel);
        let heading = if active { atan2(ey, ex) } else { s.theta };
        let err = wrap(heading - s.theta);
        let omega_des = err.signum()
            * (2.0 * turn_fraction * self.alpha * err.abs()).sqrt().min(err.abs() / (heading_steps * h));
        let torque = (p.inertia * (omega_des - s.omega) / h).clamp(-p.tau_max, p.tau_max);
        let thrust = if active && err.abs() < align {
            (p.mass * accel * err.cos()).clamp(0.0, p.f_max)
        } else {
            0.0
        };
        Control { thrust, torque }
    }
}

// ---------------------------------------------------------------------------------------------
// Sensitivities of the exact step.

/// 8-point Gauss–Legendre nodes and weights on `[-1, 1]`, as in `src/model.rs`.
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

/// The entries of the step's sensitivities that depend on the state and control, rows
/// `x, y, vx, vy`: `∂/∂θ`, `∂/∂ω`, `∂/∂F`, `∂/∂τ`. The rest of `A = ∂s⁺/∂s` is the identity plus
/// `h` at `(x, vx)`, `(y, vy)`, `(θ, ω)`, and the rest of `B = ∂s⁺/∂u` is `h²/(2I)`, `h/I` in
/// the `τ` column at rows `θ`, `ω`.
type Sens = [[f64; 4]; 4];

/// The planning model: `step` and its sensitivities in one quadrature pass, with the same
/// panels, nodes and arithmetic as the crate's `step`, but the portable [`sin_cos`]. The states
/// agree with `step` to rounding, and are the same on every platform.
///
/// With `φ(r) = θ + ω r + τ/(2I) r²`, `c = ∫ e(φ)`, `d = ∫ (h − r) e(φ)` and `e⊥ = ∂e/∂φ`:
/// `∂c/∂θ = ∫ e⊥`, `∂c/∂ω = ∫ r e⊥`, `∂c/∂τ = ∫ r²/(2I) e⊥`, and likewise for `d`.
fn step_sens(p: &Params, s: &State, u: &Control) -> (State, Sens) {
    let h = p.h;
    let alpha = u.torque / p.inertia;
    let span = s.omega.abs() * h + 0.5 * alpha.abs() * h * h;
    let panels = ((span / 1.0).ceil() as usize).clamp(1, 4096);
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
        State { x: s.x + h * s.vx, y: s.y + h * s.vy, vx: s.vx, vy: s.vy, theta, omega }
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
    (next, sens)
}

/// The planning model's step, without the sensitivities.
fn model_step(p: &Params, s: &State, u: &Control) -> State {
    step_sens(p, s, u).0
}

/// `Task::reached`, with [`norm`] in place of `hypot`.
fn inside(task: &Task, s: &State) -> bool {
    let t = &task.tolerance;
    norm(s.x - task.goal.x, s.y - task.goal.y) <= t.position
        && norm(s.vx, s.vy) <= t.velocity
        && s.omega.abs() <= t.omega
}

// ---------------------------------------------------------------------------------------------
// The controller.

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

    /// Drop the first step: the track then starts from its second state.
    fn shift(&mut self, n: usize) {
        self.u.copy_within(1..n, 0);
        self.s.copy_within(1..=n, 0);
        self.sens.copy_within(1..n, 0);
    }
}

pub struct Solution {
    task: Task,
    model: Model,
    gains: Gains,
    baseline: Baseline,
    /// The verified plan: `plan[head..len]`, whose exact rollout from the current state enters
    /// the target set exactly at its end.
    plan: Vec<Control>,
    /// The best plan of the initial search so far.
    best: Vec<Control>,
    head: usize,
    len: usize,
    /// The true state the plan predicts next; anything else invalidates the plan.
    expected: State,
    /// The planning model's prediction of the current state, from the quantized start state.
    state: State,
    valid: bool,
    /// Gauss–Newton iterate and line-search trial.
    cand: Track,
    trial: Track,
    /// Scratch: Jacobian columns of the residual, and the Gauss–Newton step.
    jac: Vec<[[f64; 5]; 2]>,
    delta: Vec<[f64; 2]>,
    /// Receding horizon: the working candidate is `cand` over `work` steps (none if 0), with
    /// residual `work_r`; it aims to be `shorten` steps shorter than the plan.
    work: usize,
    work_r: [f64; 5],
    shorten: usize,
    /// After a failed one-step shortening, wait `cooldown` steps, then `backoff` after the
    /// next failure.
    cooldown: usize,
    backoff: usize,
}

/// How a Gauss–Newton solve ended.
#[derive(Clone, Copy, PartialEq)]
enum Solve {
    /// The exact rollout enters the target set at this step.
    Hit(usize),
    /// An iteration failed to lower the residual: a local minimum, most likely infeasible.
    Stalled,
    /// Out of iterations, still making progress.
    Running,
}

/// What the predictions need: the task and the residual's margin.
#[derive(Clone, Copy)]
struct Model {
    task: Task,
    margin: f64,
}

impl Model {
    /// The scaled terminal residual (5) and its Jacobian `∂r/∂s` (5×6).
    fn residual(&self, s: &State) -> ([f64; 5], [[f64; 6]; 5]) {
        let t = &self.task.tolerance;
        let mut r = [0.0; 5];
        let mut dr = [[0.0; 6]; 5];
        // Vector groups: position (state 0, 1) and velocity (state 2, 3).
        let groups = [
            (s.x - self.task.goal.x, s.y - self.task.goal.y, t.position, 0),
            (s.vx, s.vy, t.velocity, 2),
        ];
        for (ex, ey, eps, i) in groups {
            let n = norm(ex, ey);
            let threshold = self.margin * eps;
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
        let threshold = self.margin * t.omega;
        if s.omega.abs() > threshold {
            r[4] = (s.omega - threshold * s.omega.signum()) / t.omega;
            dr[4][5] = 1.0 / t.omega;
        }
        (r, dr)
    }

    /// Predict `track.u[..n]` from the model state `s0`, storing states and sensitivities. A
    /// predicted hit is confirmed from the true state `real` with the crate's `step` before it
    /// is reported (once per rollout: if the prediction ever disagrees with `step`, later
    /// predicted hits are not trusted).
    fn rollout(&self, track: &mut Track, s0: &State, real: &State, n: usize) -> Rollout {
        let p = self.task.params;
        track.s[0] = *s0;
        let mut trusted = true;
        for k in 0..=n {
            if trusted && inside(&self.task, &track.s[k]) {
                if self.confirm(&track.u, real, k) {
                    return Rollout::Hit(k);
                }
                trusted = false;
            }
            if k < n {
                (track.s[k + 1], track.sens[k]) = step_sens(&p, &track.s[k], &track.u[k]);
            }
        }
        Rollout::Miss(self.residual(&track.s[n]).0)
    }

    /// Whether the exact rollout of `u[..m]` from `s0` ends in the target set.
    fn confirm(&self, u: &[Control], s0: &State, m: usize) -> bool {
        let mut s = *s0;
        for u in &u[..m] {
            s = step(&self.task.params, &s, u);
        }
        self.task.reached(&s)
    }
}

/// Rollout result: the first step in the target set, or the scaled residual at the end.
enum Rollout {
    Hit(usize),
    Miss([f64; 5]),
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

impl Solution {
    pub fn with_gains(task: &Task, gains: Gains) -> Self {
        let cap = task.max_steps;
        Solution {
            task: *task,
            model: Model { task: *task, margin: gains.margin },
            gains,
            baseline: Baseline::new(task),
            plan: vec![Control::default(); cap],
            best: vec![Control::default(); cap],
            head: 0,
            len: 0,
            expected: State::default(),
            state: State::default(),
            valid: false,
            cand: Track::new(cap),
            trial: Track::new(cap),
            jac: vec![[[0.0; 5]; 2]; cap],
            delta: vec![[0.0; 2]; cap],
            work: 0,
            work_r: [0.0; 5],
            shorten: 1,
            cooldown: 0,
            backoff: 1,
        }
    }

    /// Correct `cand` over `n` steps from the model state `s0` (true state `real`) by projected
    /// Gauss–Newton, starting from a fresh
    /// prediction if `fresh`, else from the stored one with residual `work_r`. The first
    /// control stays fixed if `fix_first`. Returns the first step in the target set (of the
    /// exact rollout); otherwise `cand` and `work_r` hold the last accepted iterate.
    fn solve(&mut self, s0: &State, real: &State, n: usize, iters: usize, fresh: bool, fix_first: bool) -> Solve {
        let p = self.task.params;
        if fresh {
            match self.model.rollout(&mut self.cand, s0, real, n) {
                Rollout::Hit(m) => return Solve::Hit(m),
                Rollout::Miss(r) => self.work_r = r,
            }
        }
        let mut r = self.work_r;
        let mut merit = norm2(&r);
        let scale = [p.f_max, p.tau_max];
        let (h, first) = (p.h, fix_first as usize);
        let (b4, b5) = (0.5 * h * h / p.inertia, h / p.inertia);
        let damping = self.gains.damping;
        for _ in 0..iters {
            // Backward pass: Jacobian columns of the residual, in scaled controls.
            let (_, mut lam) = self.model.residual(&self.cand.s[n]);
            for k in (0..n).rev() {
                let sn = &self.cand.sens[k];
                for (i, l) in lam.iter().enumerate() {
                    let gf = l[0] * sn[0][2] + l[1] * sn[1][2] + l[2] * sn[2][2] + l[3] * sn[3][2];
                    let gt = l[0] * sn[0][3] + l[1] * sn[1][3] + l[2] * sn[2][3] + l[3] * sn[3][3]
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
                    row[5] = l5 + h * l4 + l0 * sn[0][1] + l1 * sn[1][1] + l2 * sn[2][1] + l3 * sn[3][1];
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
                    row[i] += damping * mean + 1e-300;
                }
                let Some(sol) = solve5(&m, &r) else { return Solve::Stalled };
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
                for k in 0..n {
                    let (u, d) = (self.cand.u[k], self.delta[k]);
                    self.trial.u[k] = Control {
                        thrust: (u.thrust + t * d[0]).clamp(0.0, p.f_max),
                        torque: (u.torque + t * d[1]).clamp(-p.tau_max, p.tau_max),
                    };
                }
                match self.model.rollout(&mut self.trial, s0, real, n) {
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
            self.work_r = r;
        }
        Solve::Running
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

    /// One receding-horizon attempt from the true state `real`: a few more Gauss–Newton
    /// iterations on the working candidate, started from the plan resampled `shorten` steps
    /// shorter when there is none. Its first control is the plan's, which is applied next either
    /// way, so that the candidate's prediction stays valid after the step.
    fn improve(&mut self, real: &State) {
        if self.cooldown > 0 {
            self.cooldown -= 1;
            return;
        }
        let l = self.len - self.head;
        let fresh = self.work == 0;
        if fresh {
            self.work = l.saturating_sub(self.shorten).max(1);
            self.resample(self.work);
            self.cand.u[0] = self.plan[self.head];
        }
        let s = self.state;
        match self.solve(&s, real, self.work, self.gains.step_iters as usize, fresh, true) {
            Solve::Hit(m) if m < l => {
                self.accept(m);
                self.shorten *= 2;
                self.backoff = 1;
            }
            Solve::Running => {}
            _ => {
                self.work = 0;
                if self.shorten > 1 {
                    self.shorten /= 2;
                } else {
                    self.cooldown = self.backoff;
                    self.backoff = (2 * self.backoff).min(self.gains.patience as usize);
                }
            }
        }
    }

    /// Plan from scratch from the true state `real`: from each seed law, its rollout shortened
    /// by a continuation on the horizon; keep the shortest. The planning starts from `real`
    /// quantized, so that it does not depend on the last bits of the state.
    fn init(&mut self, real: &State) {
        self.state = quantize_state(real);
        let mut best = None;
        for law in SEEDS.iter().take(self.gains.seeds as usize) {
            // The first seed is the fallback: it gets the whole step budget.
            let limit = best.map_or(self.task.max_steps, |b: usize| 2 * b);
            self.baseline.law = *law;
            if !self.seed(real, limit) {
                continue;
            }
            self.shorten_plan(real);
            if best.is_none_or(|b| self.len < b) {
                std::mem::swap(&mut self.plan, &mut self.best);
                best = Some(self.len);
            }
        }
        self.baseline.law = SEEDS[0];
        (self.work, self.shorten, self.cooldown, self.backoff) = (0, 1, 0, 1);
        match best {
            Some(n) => {
                std::mem::swap(&mut self.plan, &mut self.best);
                (self.head, self.len, self.valid) = (0, n, true);
            }
            None => self.valid = false,
        }
    }

    /// Make the rollout of the current seed law on the model the plan, if it reaches the target
    /// set within `limit` steps, as confirmed from the true state `real`.
    fn seed(&mut self, real: &State, limit: usize) -> bool {
        let mut x = self.state;
        for k in 0..limit {
            if inside(&self.task, &x) {
                (self.head, self.len) = (0, k);
                return self.model.confirm(&self.plan, real, k);
            }
            let u = self.baseline.control(&x);
            self.plan[k] = u;
            x = model_step(&self.task.params, &x, &u);
        }
        false
    }

    /// Shorten the plan by `shorten` steps, doubling it after a success and halving it after a
    /// failure, until a one-step shortening fails or the attempts run out.
    fn shorten_plan(&mut self, real: &State) {
        let iters = self.gains.init_iters as usize;
        let s = self.state;
        let mut shorten = ((self.gains.init_shorten * self.len as f64) as usize).max(1);
        for _ in 0..self.gains.init_attempts as usize {
            let l = self.len;
            if l <= 1 {
                break;
            }
            let target = l.saturating_sub(shorten).max(1);
            self.resample(target);
            match self.solve(&s, real, target, iters, true, false) {
                Solve::Hit(m) if m < l => {
                    self.accept(m);
                    shorten *= 2;
                }
                _ if shorten == 1 => break,
                _ => shorten /= 2,
            }
        }
    }
}

impl Controller for Solution {
    fn new(task: &Task) -> Self {
        Solution::with_gains(task, Gains::DEFAULT)
    }

    fn control(&mut self, k: usize, s: &State) -> Control {
        if k == 0 || !self.valid || *s != self.expected {
            self.init(s);
        } else if self.gains.step_iters >= 1.0 && self.len - self.head > 1 {
            self.improve(s);
        }
        if !self.valid || self.head >= self.len {
            return self.baseline.control(s);
        }
        let u = self.plan[self.head];
        self.head += 1;
        self.expected = step(&self.task.params, s, &u);
        self.state = model_step(&self.task.params, &self.state, &u);
        // The working candidate continues from the next state, one step shorter.
        if self.work > 1 {
            self.cand.shift(self.work);
            self.work -= 1;
        } else {
            self.work = 0;
        }
        u
    }
}
