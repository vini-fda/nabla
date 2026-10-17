# Optimization task: stop the ship at a point in the fewest steps

This is a finite, discretized version of the ship control problem (`docs/main.typ`).

## The problem

A ship moves in the plane with state `s = (x, y, vx, vy, θ, ω)`:

```text
ẋ = vx,  ẏ = vy,  v̇x = F/m cos θ,  v̇y = F/m sin θ,  θ̇ = ω,  ω̇ = τ/I
```

Every `h` seconds the controller reads the exact state and chooses a control `u = (F, τ)`, held
constant until the next step. The control must satisfy

```text
0 ≤ F ≤ F_max,   |τ| ≤ τ_max      (checked exactly: no tolerance, no clamping)
```

The state then advances by the exact sampled-data map `step` in `src/model.rs`: closed-form
rotation, and translation computed by Gauss–Legendre quadrature, accurate to rounding.

A case is solved at the first step `N` where the state lies in the target set around the goal
`B`:

```text
‖p − B‖ ≤ ε_p,   ‖v‖ ≤ ε_v,   |ω| ≤ ε_ω      (θ free)
```

with `N ≤ N_max`. The case fails if the step budget runs out, if a control is inadmissible
(including NaN or infinite), or if the controller panics.

### Where the numbers are

This file uses symbols only. The values are in the code, and a controller gets every one of them
from its `Task`, so a solution should read them from there rather than hardcode them:

| symbol | meaning | in a `Task` | value for every case |
|---|---|---|---|
| `m, I, F_max, τ_max` | mass, inertia, thrust and torque limits | `task.params` | varies by case |
| `h` | sampling period | `task.params.h` | `suite::H` |
| `B` | goal | `task.goal` | varies by case |
| `ε_p, ε_v, ε_ω` | target-set size (position, speed, angular speed) | `task.tolerance` | `suite::TOLERANCE` |
| `N_max` | step budget | `task.max_steps` | `suite::MAX_STEPS` |
| `T_max` | wall-clock limit for evaluating both suites | — | `harness::TIME_LIMIT` |

## What a solution is

A solution is a Rust source file (any `&str`) that defines `pub struct Solution` implementing
`ship2d_optimization::Controller`:

```rust
use ship2d_optimization::*;

pub struct Solution { /* anything */ }

impl Controller for Solution {
    fn new(task: &Task) -> Self { /* parameters, goal, tolerance, step budget */ }
    fn control(&mut self, k: usize, state: &State) -> Control { /* u_k */ }
}
```

- One `Solution` is created per case, then `control` is called at `k = 0, 1, 2, …`.
- The solution may keep internal state (plans, modes), and may call `step` to predict the motion
  (e.g. for MPC).
- Cases run in parallel, each on one thread with its own `Solution`. Do not share mutable state
  between cases (e.g. through statics).
- Only the standard library and this crate are available.
- Evaluating both suites must take under `T_max` of wall-clock time.

## Test cases and score

`src/suite.rs` defines two suites. Every case of both must be solved.

- **standard** (`suite::standard`): hand-picked cases plus seeded random ones. The hand-picked
  cases cover starts that are at rest, moving fast, spinning, already at the goal, very far away,
  and very sluggish or very agile ships.
- **holdout** (`suite::holdout`): random cases with a different seed. They are not scored. They
  exist so that a solution cannot just memorize the standard cases.

Random cases (`suite::random`) vary the ship (`m, I, F_max, τ_max`), the goal, and the start
position, speed, heading and spin, each uniformly within the ranges set there. `cargo xtask eval`
lists every case of both suites.

**Score = the sum of `N` over the standard suite. Lower is better.** The longest case is reported
too.

There is only one version of the suites: the current one. Changing a case, `h`, the tolerance or
the budget changes every score, so resubmit the stored solutions afterwards; until then
`cargo test` and `cargo xtask verify` report their index entries as stale. To check robustness
beyond the suites, run your controller on `suite::random(prefix, seed, count)` with other seeds.

## Commands

Run these from this directory:

```sh
cargo xtask eval path/to/solution.rs              # compile, run, per-case table + score
cargo xtask eval a.rs b.rs c.rs                   # several: built in parallel, best first
cargo xtask submit <name> path/to/solution.rs     # store it if every case is solved
cargo xtask leaderboard                           # stored solutions, best first
cargo xtask verify                                # re-evaluate every stored solution
cargo test --workspace --release                  # model tests + one test per solution
cargo xtask help                                  # every task, including the viewer's
```

### How evaluation runs

- **Builds.** Each source becomes its own cargo project, `target/candidates/<source hash>/`,
  with its own target directory. Builds of different sources never wait on each other's cargo
  lock. An unchanged source is not rebuilt. A new one also compiles this crate, which takes about
  a second. `target/candidates/` is only a cache, and you can delete it at any time.
- **Cases.** The compiled candidate runs all cases of both suites in parallel, one thread per
  core. Workers take the next case from a shared atomic counter (`parallel::map`), so a few long
  cases do not leave the other cores idle.
- **Many solutions.** Pass them all to one `cargo xtask eval`. They compile in parallel, then
  run one at a time, so each run gets every core and the run times stay comparable.
  For many variants of one solution that differ only in constants, see "Tuning parameters"
  below instead.
- **Many processes at once** (e.g. several agents): build the tasks once with
  `cargo build -p xtask`, then call `target/debug/xtask eval …` directly. `cargo xtask` goes
  through `cargo run`, which replaces `target/debug/xtask` on every call, even when nothing
  changed. Several calls launched at the same instant can therefore fail to start.
- **Trust.** The candidate runs as ordinary native code, with no sandbox. Only evaluate code you
  trust.

## Tuning parameters

To tune a solution's constants (gains, margins, thresholds), evaluate it many times in a single
process. `cargo xtask eval` is built for scoring finished solutions: every distinct source is a
new build, and every run is a new process. A tuner instead calls `eval::evaluate` directly, so
each point costs one evaluation (the suites in parallel on every core) and nothing else.

**1. Make the solution tunable.** Gather the constants in a struct, with their current values as
the default, and add a constructor that takes them. `new` uses the default, so the file stays a
valid solution:

```rust
#[derive(Clone, Copy, Debug)]
pub struct Gains {
    pub brake_fraction: f64,
    pub align: f64,
}

impl Gains {
    pub const DEFAULT: Gains = Gains { brake_fraction: 0.84, align: 0.37 };
}

impl Solution {
    pub fn with_gains(task: &Task, gains: Gains) -> Self { /* … */ }
}

impl Controller for Solution {
    fn new(task: &Task) -> Self {
        Solution::with_gains(task, Gains::DEFAULT)
    }
    // …
}
```

**2. Write the tuner as an example program**, e.g. `examples/tune_mine.rs`, that includes the
solution file as a module. `Controller::new` only receives the `Task`, so the gains under test
go through a static, which `new` reads once per case:

```rust
#[path = "../path/to/mine.rs"]
mod solution;

use ship2d_optimization::{Control, Controller, State, Task, eval, parallel, problem, suite};
use solution::{Gains, Solution};
use std::sync::RwLock;

static GAINS: RwLock<Gains> = RwLock::new(Gains::DEFAULT);

struct Tuned(Solution);

impl Controller for Tuned {
    fn new(task: &Task) -> Self {
        Tuned(Solution::with_gains(task, *GAINS.read().unwrap()))
    }
    fn control(&mut self, k: usize, state: &State) -> Control {
        self.0.control(k, state)
    }
}

/// The score with these gains: infinite if any case of either suite fails.
fn score(gains: Gains) -> f64 {
    *GAINS.write().unwrap() = gains;
    eval::evaluate::<Tuned>().score().map_or(f64::INFINITY, |s| s as f64)
}

fn main() {
    // Any search over `score`: coordinate descent, Nelder–Mead, a grid, …
}
```

**3. Run it** with the `fast` profile, which skips the release profile's slow whole-program
LTO:

```sh
cargo run --profile fast --example tune_mine
```

The scores are the same as with `cargo xtask eval`, since optimization levels do not change
floating-point results in Rust. For the current solutions, one point takes a few tens of
milliseconds.

**4. Store the result.** Copy the best values into `Gains::DEFAULT`, then run
`cargo xtask eval` and `cargo xtask submit`. The tuner itself is not part of the solution.

Things to keep in mind:

- **Evaluate one point at a time.** Each evaluation already uses every core, and all the cases
  of one evaluation read the same `GAINS`.
- **Watch for overfitting.** Only the standard suite is scored, and the holdout cases are just
  pass/fail. `score` returns infinity when a holdout case fails, so the search avoids those
  gains, but it can still trade robustness for score on cases outside the suites. Accept an
  improvement only if it also solves a set of random validation cases. Before submitting, check
  the result on another set of random cases that the search never saw:

  ```rust
  *GAINS.write().unwrap() = best;
  let extra = suite::random("extra", 12345, 2000);
  let outcomes = parallel::map(&extra, problem::run::<Tuned>);
  let failed = outcomes.iter().filter(|o| o.steps().is_none()).count();
  ```

- **Keep the stored source free of tuning hooks.** A stored solution must not read the
  environment or any other outside state. Its score has to be reproducible from its source
  alone.

For Claude, the `tune-controller` skill (`.claude/skills/tune-controller/`) follows this
workflow with a ready-made tuner: coordinate descent with validation cases and a final check.

## The database

- `solutions/<name>.rs`: the source of every stored solution.
- `solutions/index.tsv`: name, score, longest case and source hash.

`cargo test` checks that every stored solution still solves every case, and that its index entry
matches its current source and score. Do not edit stored files by hand. Resubmit instead.

| name | idea |
|---|---|
| `flip_and_burn` | two-layer feedback: braking-aware velocity setpoint, square-root heading law |
| `flip_and_burn_tuned` | same, constants tuned by coordinate descent |
| `tolerance_aware` | same, plus a coasting spin-down finisher and an `ε_v` deadband, retuned |
| `tolerance_aware_tuned` | `tolerance_aware` retuned in-process, with validation cases (`tune-controller` skill) |
| `mpc` | shrinking-horizon MPC on the exact model: seed plans from the feedback laws, shortened by projected Gauss–Newton on the 5 terminal residuals, re-solved every step; every plan verified with `step` |

`cargo xtask leaderboard` shows their current scores.

## Ideas for doing better

- **Bang-bang turns**: plan time-optimal turns instead of tracking a setpoint.
- **Better flip timing**: compute the exact flip-and-brake switching time instead of using a
  margin, and start the braking turn earlier.
- **Turn while moving**: thrust during the turn, not only once aligned.
- **MPC**: use `step` to optimize a short horizon, warm-started from the baseline.
- **Endgame**: the target set is enough; exact rest is not needed. Arrive with a speed just under
  `ε_v` and a spin just under `ε_ω` instead of braking to zero.
