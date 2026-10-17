//! Tune the constants of one solution by coordinate descent, in a single process.
//!
//! Copy to `examples/tune_<name>.rs`, fill in the three `TODO`s, then run:
//!
//!     cargo run --profile fast --example tune_<name>
//!
//! See TASK.md, "Tuning parameters".

// TODO 1: the solution file, relative to this file (`examples/`).
#[path = "../path/to/solution.rs"]
mod solution;

use ship2d_optimization::{
    Control, Controller, State, Task, eval, parallel,
    problem::{Case, run},
    suite,
};
use solution::{Gains, Solution};
use std::sync::RwLock;
use std::time::Instant;

/// A tunable field of `Gains`: its name, and how to reach it.
type Field = (&'static str, fn(&mut Gains) -> &mut f64);

// TODO 2: every tunable field of `Gains`.
const FIELDS: &[Field] = &[
    // ("brake_fraction", |g| &mut g.brake_fraction),
    // ("align", |g| &mut g.align),
];

// TODO 3 (optional): the search budget.
/// First relative step: each field is tried at `x · (1 ± STEP)`.
const STEP: f64 = 0.1;
/// Stop once the relative step is below this.
const MIN_STEP: f64 = 1e-3;
/// Stop after this many evaluations.
const MAX_EVALS: usize = 2000;
/// Random cases every improvement must also solve, so that the search cannot trade
/// robustness for score. Seeds must differ from the suites' (1 and 2) and from each other.
const VALIDATION_SEED: u64 = 12345;
const VALIDATION_COUNT: usize = 1000;
/// Random cases never seen during the search: the final, honest robustness check.
const CHECK_SEED: u64 = 777;
const CHECK_COUNT: usize = 2000;

/// The gains of the evaluation in progress. `Controller::new` only receives the `Task`, so
/// they reach the solution through this global, read once per case.
static GAINS: RwLock<Gains> = RwLock::new(Gains::DEFAULT);

struct Tuned(Solution);

impl Controller for Tuned {
    fn new(task: &Task) -> Self {
        Tuned(Solution::with_gains(task, *GAINS.read().expect("gains lock")))
    }

    fn control(&mut self, k: usize, state: &State) -> Control {
        self.0.control(k, state)
    }
}

/// The score with these gains (total steps on the standard suite), or infinity if any case
/// of either suite fails. Evaluations run one at a time: each uses every core.
fn score(gains: Gains) -> f64 {
    *GAINS.write().expect("gains lock") = gains;
    eval::evaluate::<Tuned>()
        .score()
        .map_or(f64::INFINITY, |s| s as f64)
}

/// The names of the `cases` that these gains fail.
fn failures(gains: Gains, cases: &[Case]) -> Vec<String> {
    *GAINS.write().expect("gains lock") = gains;
    let outcomes = parallel::map(cases, run::<Tuned>);
    cases
        .iter()
        .zip(&outcomes)
        .filter(|(_, o)| o.steps().is_none())
        .map(|(c, _)| c.name.clone())
        .collect()
}

fn main() {
    assert!(!FIELDS.is_empty(), "list the tunable fields in FIELDS");
    let started = Instant::now();
    let mut best = Gains::DEFAULT;
    let mut best_score = score(best);
    let mut evals = 1;
    println!(
        "default: score {best_score} ({:.0} ms per evaluation)",
        started.elapsed().as_secs_f64() * 1e3
    );
    assert!(
        best_score.is_finite(),
        "Gains::DEFAULT does not solve every case: fix the solution before tuning it"
    );
    let validation = suite::random("validation", VALIDATION_SEED, VALIDATION_COUNT);
    let failed = failures(best, &validation);
    if !failed.is_empty() {
        println!(
            "warning: Gains::DEFAULT already fails {} validation cases; \
             improvements may not fail more",
            failed.len()
        );
    }
    let allowed_failures = failed.len();
    let mut rejected = 0;

    // Coordinate descent with multiplicative steps: scale free, and robust to the score being
    // piecewise constant (it counts steps). Halve the step after a sweep without progress.
    let mut step = STEP;
    'search: while step >= MIN_STEP {
        let mut improved = false;
        for (name, field) in FIELDS {
            for factor in [1.0 + step, 1.0 - step] {
                if evals >= MAX_EVALS {
                    break 'search;
                }
                let mut candidate = best;
                *field(&mut candidate) *= factor;
                let s = score(candidate);
                evals += 1;
                // An improvement counts only if it keeps solving the validation cases.
                if s < best_score {
                    if failures(candidate, &validation).len() > allowed_failures {
                        rejected += 1;
                        continue;
                    }
                    (best, best_score, improved) = (candidate, s, true);
                    println!("{best_score}  {name} = {:?}  (step {step})", *field(&mut best));
                    break;
                }
            }
        }
        if !improved {
            step /= 2.0;
        }
    }
    println!(
        "\nbest score {best_score} after {evals} evaluations in {:.1} s \
         ({rejected} improvements rejected for failing validation cases)",
        started.elapsed().as_secs_f64()
    );

    // The honest check: cases the search never saw.
    let check = suite::random("check", CHECK_SEED, CHECK_COUNT);
    let failed = failures(best, &check);
    println!(
        "{} of {CHECK_COUNT} check cases (seed {CHECK_SEED}) failed{}",
        failed.len(),
        if failed.is_empty() { String::new() } else { format!(": {}", failed.join(", ")) }
    );

    // Paste into the solution.
    println!("\npub const DEFAULT: Gains = {best:?};");
}
