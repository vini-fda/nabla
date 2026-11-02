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

The state then advances by the exact sampled-data map `step` in
`crates/nabla-core/src/model.rs`: closed-form rotation, and translation computed by
Gauss–Legendre quadrature, accurate to rounding.

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

## What a solution is

A solution is a module of `crates/nabla-core/src/solutions/` that defines `pub struct Solution`
implementing `Controller`:

```rust
use crate::{Control, Controller, State, Task};

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
- Only the standard library and `nabla-core` are available.

## Test cases and score

`crates/nabla-core/src/suite.rs` defines two suites. Every case of both must be solved.

- **standard** (`suite::standard`): hand-picked cases plus seeded random ones. The hand-picked
  cases cover starts that are at rest, moving fast, spinning, already at the goal, very far away,
  and very sluggish or very agile ships.
- **holdout** (`suite::holdout`): random cases with a different seed. They are not scored. They
  exist so that a solution cannot just memorize the standard cases.

Random cases (`suite::random`) vary the ship (`m, I, F_max, τ_max`), the goal, and the start
position, speed, heading and spin, each uniformly within the ranges set there.

**Score = the sum of `N` over the standard suite. Lower is better.**

To check robustness beyond the suites, run your controller on
`suite::random(prefix, seed, count)` with other seeds.

## The solutions

| name | idea |
|---|---|
| `flip_and_burn_tuned` | two-layer feedback: braking-aware velocity setpoint, square-root heading law, constants tuned by coordinate descent |
| `mpc` | shrinking-horizon MPC on the exact model: seed plans from feedback laws, shortened by projected Gauss–Newton on the 5 terminal residuals, re-solved every step; every plan verified with `step` |

To add one, list it in `SOLUTIONS` (`crates/nabla-core/src/solutions.rs`). `cargo test` checks
that every solution still solves every case of both suites, and the app picks it up.
