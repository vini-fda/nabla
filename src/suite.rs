//! Test suites: the initial conditions every solution must solve.
//!
//! * [`standard`]: hand-picked cases plus seeded random ones. A solution's score is its total
//!   step count on this suite.
//! * [`holdout`]: more random cases with a different seed. They are not scored, but must be
//!   solved too, so that a solution cannot just memorize the standard cases.
//!
//! Changing any case changes scores, so bump [`SUITE_VERSION`] whenever this file changes.

use crate::model::{Params, State};
use crate::problem::{Case, Goal, Task, Tolerance};
use std::f64::consts::PI;

/// Version of the suites. Scores are only comparable between equal versions.
pub const SUITE_VERSION: u32 = 1;

/// Sampling period used by every case.
pub const H: f64 = 0.1;

/// Step budget of every case (`2000 s` of simulated time).
pub const MAX_STEPS: usize = 20_000;

/// Target-set size used by every case.
pub const TOLERANCE: Tolerance = Tolerance {
    position: 0.05,
    velocity: 0.05,
    omega: 0.05,
};

/// Unit ship: `m = I = F_max = τ_max = 1`.
pub const NOMINAL: Params = Params {
    mass: 1.0,
    inertia: 1.0,
    f_max: 1.0,
    tau_max: 1.0,
    h: H,
};

fn case(name: &str, params: Params, goal: (f64, f64), start: State) -> Case {
    Case {
        name: name.to_string(),
        task: Task {
            params,
            goal: Goal {
                x: goal.0,
                y: goal.1,
            },
            tolerance: TOLERANCE,
            max_steps: MAX_STEPS,
        },
        start,
    }
}

fn state(x: f64, y: f64, vx: f64, vy: f64, theta: f64, omega: f64) -> State {
    State {
        x,
        y,
        vx,
        vy,
        theta,
        omega,
    }
}

/// Hand-picked cases covering the typical difficulties.
fn hand_picked() -> Vec<Case> {
    let sluggish = Params {
        mass: 5.0,
        inertia: 5.0,
        f_max: 0.5,
        tau_max: 0.2,
        h: H,
    };
    let agile = Params {
        mass: 0.5,
        inertia: 0.2,
        f_max: 5.0,
        tau_max: 5.0,
        h: H,
    };
    vec![
        case(
            "at_goal",
            NOMINAL,
            (0.0, 0.0),
            state(0.0, 0.0, 0.0, 0.0, 1.0, 0.0),
        ),
        case(
            "rest_facing_goal",
            NOMINAL,
            (0.0, 0.0),
            state(10.0, 0.0, 0.0, 0.0, PI, 0.0),
        ),
        case(
            "rest_facing_away",
            NOMINAL,
            (0.0, 0.0),
            state(10.0, 0.0, 0.0, 0.0, 0.0, 0.0),
        ),
        case(
            "rest_sideways",
            NOMINAL,
            (0.0, 0.0),
            state(0.0, 10.0, 0.0, 0.0, 0.0, 0.0),
        ),
        case(
            "moving_away",
            NOMINAL,
            (0.0, 0.0),
            state(5.0, 0.0, 3.0, 0.0, 0.0, 0.0),
        ),
        case(
            "incoming_fast",
            NOMINAL,
            (0.0, 0.0),
            state(20.0, 0.0, -5.0, 0.0, PI, 0.0),
        ),
        case(
            "passing_by",
            NOMINAL,
            (0.0, 0.0),
            state(-10.0, 5.0, 2.0, 0.0, 0.5 * PI, 0.0),
        ),
        case(
            "spinning",
            NOMINAL,
            (0.0, 0.0),
            state(3.0, 4.0, 0.0, 0.0, 0.0, 5.0),
        ),
        case(
            "spinning_moving",
            NOMINAL,
            (0.0, 0.0),
            state(-8.0, -6.0, 1.0, -2.0, 1.0, -4.0),
        ),
        case(
            "drifting_at_goal",
            NOMINAL,
            (0.0, 0.0),
            state(0.04, 0.0, 0.2, 0.1, 2.0, 0.0),
        ),
        case(
            "far_away",
            NOMINAL,
            (0.0, 0.0),
            state(100.0, -50.0, 0.0, 0.0, 0.5, 0.0),
        ),
        case(
            "goal_offset",
            NOMINAL,
            (7.0, -3.0),
            state(-2.0, 4.0, 0.0, 1.0, -2.0, 0.5),
        ),
        case(
            "sluggish",
            sluggish,
            (0.0, 0.0),
            state(10.0, 10.0, 0.0, -1.0, 0.0, 0.0),
        ),
        case(
            "agile",
            agile,
            (0.0, 0.0),
            state(-15.0, 3.0, 4.0, 4.0, 0.0, 2.0),
        ),
    ]
}

/// SplitMix64: a small, portable, seeded generator.
struct Rng(u64);

impl Rng {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `[lo, hi)`.
    fn uniform(&mut self, lo: f64, hi: f64) -> f64 {
        let unit = (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64;
        lo + (hi - lo) * unit
    }
}

/// Random cases: varied ships, goals within 20 of the origin, starts within 30 of the goal,
/// speeds up to 5, any heading and angular speeds up to 3. Useful for stress-testing a solution
/// beyond the two suites.
pub fn random(prefix: &str, seed: u64, count: usize) -> Vec<Case> {
    let mut rng = Rng(seed);
    (0..count)
        .map(|i| {
            let params = Params {
                mass: rng.uniform(0.5, 5.0),
                inertia: rng.uniform(0.2, 5.0),
                f_max: rng.uniform(0.5, 5.0),
                tau_max: rng.uniform(0.2, 5.0),
                h: H,
            };
            let goal = (rng.uniform(-20.0, 20.0), rng.uniform(-20.0, 20.0));
            let (dist, bearing) = (rng.uniform(0.0, 30.0), rng.uniform(-PI, PI));
            let (speed, course) = (rng.uniform(0.0, 5.0), rng.uniform(-PI, PI));
            let start = State {
                x: goal.0 + dist * bearing.cos(),
                y: goal.1 + dist * bearing.sin(),
                vx: speed * course.cos(),
                vy: speed * course.sin(),
                theta: rng.uniform(-PI, PI),
                omega: rng.uniform(-3.0, 3.0),
            };
            case(&format!("{prefix}_{i:02}"), params, goal, start)
        })
        .collect()
}

/// The scored suite.
pub fn standard() -> Vec<Case> {
    let mut cases = hand_picked();
    cases.extend(random("random", 1, 26));
    cases
}

/// The unscored suite that must also be solved.
pub fn holdout() -> Vec<Case> {
    random("holdout", 2, 40)
}

/// The case of either suite with this name.
pub fn find(name: &str) -> Option<Case> {
    standard()
        .into_iter()
        .chain(holdout())
        .find(|c| c.name == name)
}
