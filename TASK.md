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
- **Many processes at once** (e.g. several agents): build the tasks once with
  `cargo build -p xtask`, then call `target/debug/xtask eval …` directly. `cargo xtask` goes
  through `cargo run`, which replaces `target/debug/xtask` on every call, even when nothing
  changed. Several calls launched at the same instant can therefore fail to start.
- **Trust.** The candidate runs as ordinary native code, with no sandbox. Only evaluate code you
  trust.

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

`cargo xtask leaderboard` shows their current scores.

## Ideas for doing better

- **Bang-bang turns**: plan time-optimal turns instead of tracking a setpoint.
- **Better flip timing**: compute the exact flip-and-brake switching time instead of using a
  margin, and start the braking turn earlier.
- **Turn while moving**: thrust during the turn, not only once aligned.
- **MPC**: use `step` to optimize a short horizon, warm-started from the baseline.
- **Endgame**: the target set is enough; exact rest is not needed. Arrive with a speed just under
  `ε_v` and a spin just under `ε_ω` instead of braking to zero.
