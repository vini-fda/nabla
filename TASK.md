# Optimization task: stop the ship at a point in the fewest steps

This is a finite, discretized version of Problem 2 of the `ship2d_lean` write-up (`docs/main.typ`, section "Discrete-time
version of the second problem").

## The problem

A ship moves in the plane with state `s = (x, y, vx, vy, θ, ω)`:

```text
ẋ = vx,  ẏ = vy,  v̇x = F/m cos θ,  v̇y = F/m sin θ,  θ̇ = ω,  ω̇ = τ/I
```

Every `h = 0.1` s the controller reads the exact state and chooses a control `u = (F, τ)`, held
constant until the next step. It must satisfy

```text
0 ≤ F ≤ F_max,   |τ| ≤ τ_max      (checked exactly: no tolerance, no clamping)
```

The state then advances by the exact sampled-data map `step` in `src/model.rs`: closed-form
rotation, and translation computed by Gauss–Legendre quadrature, accurate to rounding.

A case is solved at the first step `N` where

```text
‖p − B‖ ≤ 0.05,   ‖v‖ ≤ 0.05,   |ω| ≤ 0.05      (θ free)
```

with `N ≤ 20000`. The case fails if the budget runs out, if a control is inadmissible (including
NaN), or if the controller panics.

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
- Only the standard library and this crate are available.
- Evaluating both suites must take under 300 s of wall-clock time.

## Test cases and score

`src/suite.rs` defines two suites. Every case of both must be solved.

- **standard** (40 cases): 14 hand-picked cases plus 26 random ones. They cover starts that are
  at rest, moving fast, spinning, already at the goal, very far away, and very sluggish or very
  agile ships.
- **holdout** (40 random cases, different seed): not scored. It exists so that a solution cannot
  just memorize the standard cases.

Random cases vary the ship (`m, I, F_max, τ_max`), the goal, the start position (within 30 of the
goal), the speed (up to 5), the heading and the spin (up to 3 rad/s).

**Score = total steps `N` over the standard suite. Lower is better.**

Scores are only comparable on the same `SUITE_VERSION`. Bump it whenever the suite, tolerance,
budget or `h` changes. To check robustness beyond the suites, run your controller on
`suite::random(prefix, seed, count)` with other seeds.

## Commands

Run these from this directory:

```sh
cargo run --release -- eval path/to/solution.rs          # compile, run, per-case table + score
cargo run --release -- submit <name> path/to/solution.rs # store it if every case is solved
cargo run --release -- leaderboard                       # stored solutions, best first
cargo run --release -- verify                            # re-evaluate every stored solution
cargo test --release                                     # model tests + one test per solution
```

`eval` and `submit` compile the source in a separate cargo project under `target/candidates/` and
run it as a separate process. There is no sandbox, so only evaluate code you trust.

## The database

- `solutions/<name>.rs`: the source of every stored solution.
- `solutions/index.tsv`: name, score, longest case, suite version and source hash.

`cargo test` checks that every stored solution still solves every case, and that its index entry
matches its current source and score. Do not edit stored files by hand; resubmit instead.

| name | idea | score |
|---|---|---|
| `flip_and_burn` | two-layer feedback: braking-aware velocity setpoint, square-root heading law | 19805 |
| `flip_and_burn_tuned` | same, constants tuned by coordinate descent | 12389 |

## Ideas for doing better

- **Bang-bang turns**: plan time-optimal turns instead of tracking a setpoint.
- **Better flip timing**: compute the exact flip-and-brake switching time instead of using a
  margin, and start the braking turn earlier.
- **Turn while moving**: thrust during the turn, not only once aligned.
- **MPC**: use `step` to optimize a short horizon, warm-started from the baseline.
- **Endgame**: reaching the tolerance set, not exact rest, is enough. Arrive at just under 0.05
  speed instead of braking to zero.
