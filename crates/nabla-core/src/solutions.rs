//! The control policies found so far, compiled in. The app's autopilot flies with them.

pub mod flip_and_burn_tuned;
pub mod mpc;

use crate::Controller;
use crate::problem::{Case, Task, Trajectory, simulate};

/// A control policy, with what the app shows of it.
pub struct StoredSolution {
    pub name: &'static str,
    /// Its Rust source.
    pub source: &'static str,
    /// Run this solution on a case, recording the trajectory.
    pub simulate: fn(&Case) -> Trajectory,
    /// A new controller of this solution for `task`, to query a step at a time.
    pub controller: fn(&Task) -> Box<dyn Controller>,
}

/// A new controller `C` for `task`, boxed.
fn boxed<C: Controller + 'static>(task: &Task) -> Box<dyn Controller> {
    Box::new(C::new(task))
}

/// Every solution. The app starts on the first.
pub static SOLUTIONS: &[StoredSolution] = &[
    StoredSolution {
        name: "mpc",
        source: include_str!("solutions/mpc.rs"),
        simulate: simulate::<mpc::Solution>,
        controller: boxed::<mpc::Solution>,
    },
    StoredSolution {
        name: "flip_and_burn_tuned",
        source: include_str!("solutions/flip_and_burn_tuned.rs"),
        simulate: simulate::<flip_and_burn_tuned::Solution>,
        controller: boxed::<flip_and_burn_tuned::Solution>,
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::suite;
    use std::thread;

    /// The cases of `cases` that `solution` fails. One thread per case: the slowest solutions
    /// take seconds per suite.
    fn failures(solution: &StoredSolution, cases: &[Case]) -> Vec<String> {
        thread::scope(|scope| {
            let runs: Vec<_> = cases
                .iter()
                .map(|case| scope.spawn(move || (solution.simulate)(case).outcome))
                .collect();
            cases
                .iter()
                .zip(runs)
                .filter_map(|(case, run)| {
                    let outcome = run.join().expect("worker thread panicked");
                    outcome
                        .steps()
                        .is_none()
                        .then(|| format!("{}: {outcome:?}", case.name))
                })
                .collect()
        })
    }

    #[test]
    fn every_solution_solves_every_case() {
        for solution in SOLUTIONS {
            for cases in [suite::standard(), suite::holdout()] {
                let failed = failures(solution, &cases);
                assert!(failed.is_empty(), "{}: {failed:?}", solution.name);
            }
        }
    }
}
