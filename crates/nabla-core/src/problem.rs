//! The finite, discretized version of Problem 2: come to rest at a goal point.
//!
//! A case fixes the parameters, the goal and the initial state. Starting from `s₀`, the
//! controller is queried once per step for `u_k`, and the state advances by the exact
//! sampled-data map [`step`]. The case is solved at the first step `N` such that `s_N` lies in
//! the target set
//!
//! ```text
//! ‖p − B‖ ≤ ε_p,   ‖v‖ ≤ ε_v,   |ω| ≤ ε_ω   (θ free)
//! ```
//!
//! with `N ≤ max_steps`, and every control applied before that is admissible.

use crate::model::{Control, Params, State, step};
use std::panic::{AssertUnwindSafe, catch_unwind};

/// Target point `B`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Goal {
    pub x: f64,
    pub y: f64,
}

/// Size of the target set around `B`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tolerance {
    /// `ε_p`: maximum distance to `B`.
    pub position: f64,
    /// `ε_v`: maximum speed.
    pub velocity: f64,
    /// `ε_ω`: maximum angular speed.
    pub omega: f64,
}

/// Everything a controller is told about a case, except the state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Task {
    pub params: Params,
    pub goal: Goal,
    pub tolerance: Tolerance,
    /// Step budget: the target must be reached at some `N ≤ max_steps`.
    pub max_steps: usize,
}

impl Task {
    /// Whether `s` lies in the target set.
    pub fn reached(&self, s: &State) -> bool {
        let t = &self.tolerance;
        (s.x - self.goal.x).hypot(s.y - self.goal.y) <= t.position
            && s.vx.hypot(s.vy) <= t.velocity
            && s.omega.abs() <= t.omega
    }
}

/// A control policy. A solution is Rust source defining `pub struct Solution` implementing this.
///
/// One `Solution` is created per case and then queried at steps `k = 0, 1, 2, …` with the exact
/// current state, until the target is reached. It may keep internal state (e.g. a plan), and may
/// call [`step`] to predict the motion.
pub trait Controller {
    fn new(task: &Task) -> Self
    where
        Self: Sized;

    /// The control `u_k` to hold during step `k`, given the state `s_k`.
    fn control(&mut self, k: usize, state: &State) -> Control;
}

/// A test case: a task and an initial state.
#[derive(Clone, Debug, PartialEq)]
pub struct Case {
    pub name: String,
    pub task: Task,
    pub start: State,
}

/// Result of running a controller on one case.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    /// Target set reached after this many steps.
    Reached(usize),
    /// Still outside the target set after `max_steps` steps.
    Timeout,
    /// The control returned at `step` violates the constraints.
    Inadmissible { step: usize, control: Control },
    /// The controller panicked at `step`.
    Panic { step: usize },
}

impl Outcome {
    pub fn steps(&self) -> Option<usize> {
        match self {
            Outcome::Reached(n) => Some(*n),
            _ => None,
        }
    }
}

/// States and controls of one simulated case.
#[derive(Clone, Debug, PartialEq)]
pub struct Trajectory {
    /// `s_0, …, s_N`: one more state than controls.
    pub states: Vec<State>,
    /// `u_0, …, u_{N-1}`: the admissible controls that were applied.
    pub controls: Vec<Control>,
    pub outcome: Outcome,
}

/// Simulate the controller `C` on one case, calling `observe(s_k, u_k)` for every applied step.
fn drive<C: Controller>(case: &Case, mut observe: impl FnMut(&State, &Control)) -> Outcome {
    let task = &case.task;
    let Ok(mut controller) = catch_unwind(|| C::new(task)) else {
        return Outcome::Panic { step: 0 };
    };
    let mut s = case.start;
    for k in 0..task.max_steps {
        if task.reached(&s) {
            return Outcome::Reached(k);
        }
        let Ok(u) = catch_unwind(AssertUnwindSafe(|| controller.control(k, &s))) else {
            return Outcome::Panic { step: k };
        };
        if !u.is_admissible(&task.params) {
            return Outcome::Inadmissible {
                step: k,
                control: u,
            };
        }
        observe(&s, &u);
        s = step(&task.params, &s, &u);
    }
    if task.reached(&s) {
        Outcome::Reached(task.max_steps)
    } else {
        Outcome::Timeout
    }
}

/// Simulate the controller `C` on one case.
pub fn run<C: Controller>(case: &Case) -> Outcome {
    drive::<C>(case, |_, _| ())
}

/// Simulate the controller `C` on one case, recording the trajectory.
pub fn simulate<C: Controller>(case: &Case) -> Trajectory {
    let (mut states, mut controls) = (Vec::new(), Vec::new());
    let outcome = drive::<C>(case, |s, u| {
        states.push(*s);
        controls.push(*u);
    });
    let last = match states.last().zip(controls.last()) {
        Some((s, u)) => step(&case.task.params, s, u),
        None => case.start,
    };
    states.push(last);
    Trajectory {
        states,
        controls,
        outcome,
    }
}
