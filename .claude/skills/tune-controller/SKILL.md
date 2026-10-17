---
name: tune-controller
description: Tune the constants of a ship2d_optimization controller (gains, margins, thresholds, time constants, any `const` that shapes the control law) to lower its score, by fast in-process search instead of re-running `cargo xtask eval` per value. Use this whenever the user wants to tune, optimize, sweep, calibrate or "find the best value of" a parameter of a solution in this repo — e.g. "tune alpha", "optimize the gains of flip_and_burn", "can we squeeze a better score by adjusting the constants", "run coordinate descent on my controller", "sweep BRAKE_FRACTION" — even if they don't say "tune" explicitly.
---

# Tuning a controller's constants

A score is the total steps over the standard suite (see `TASK.md`). Tuning means evaluating the
same controller with many values of its constants. `cargo xtask eval` is the wrong tool for that:
every distinct source is a new cargo build and every run a new process. Instead, compile the
solution once into a small example program that sets the constants through a static and calls
`eval::evaluate` directly. Each point then costs one evaluation (milliseconds for simple
controllers) and the scores are identical to `cargo xtask eval`'s.

`TASK.md`, section "Tuning parameters", documents the same pattern for humans; this skill adds a
ready tuner (`assets/tune.rs`) and the workflow around it.

## Workflow

### 1. Pick the solution and the constants

Work on a copy, not on `solutions/<name>.rs` itself: `cargo test` checks stored files against
`solutions/index.tsv`, so editing one in place breaks the tests until it is resubmitted. Copy it
to `tuning/<new_name>.rs` (gitignored, like `examples/tune_*.rs`).

Tune the constants the user named. If they named none, pick the ones that shape the control law
(their doc comments usually say so: fractions, margins, time constants, thresholds). Leave alone
anything that is not a free choice: mathematical constants, and the task's values (`h`, the
tolerance, the limits), which a controller must read from its `Task`.

### 2. Make the copy tunable, without changing its behavior

Gather the chosen constants into a struct whose default is exactly the current values, and route
construction through it:

```rust
/// The tunable constants.
#[derive(Clone, Copy, Debug)]
pub struct Gains {
    /// (keep each constant's doc comment)
    pub brake_fraction: f64,
    pub align: f64,
}

impl Gains {
    pub const DEFAULT: Gains = Gains { brake_fraction: 0.8385254915624212, align: 0.36943031601686555 };
}

impl Solution {
    pub fn with_gains(task: &Task, gains: Gains) -> Self { /* what `new` did, plus `gains` */ }
}

impl Controller for Solution {
    fn new(task: &Task) -> Self {
        Solution::with_gains(task, Gains::DEFAULT)
    }
    // `control` reads `self.gains.brake_fraction` where it used `BRAKE_FRACTION`.
}
```

Copy the literals exactly: a rounded default changes the starting score. Then confirm the
refactor is behavior-preserving: `cargo xtask eval` on the copy must give the same score as the
original. If it differs, fix the refactor before tuning — otherwise the tuning starts from a
different controller than the user thinks.

### 3. Create the tuner from the template

Copy `assets/tune.rs` (next to this file) to `examples/tune_<name>.rs` and fill in its three
`TODO`s:

1. `#[path = "..."]`: the working file, relative to `examples/` (e.g. `../tuning/<new_name>.rs`).
2. `FIELDS`: one `("name", |g| &mut g.name)` entry per tunable field.
3. Optionally the budget: `STEP`, `MIN_STEP`, `MAX_EVALS`, and the random case sets
   (`VALIDATION_*`, `CHECK_*`).

The template runs coordinate descent with multiplicative steps (each field tried at
`x·(1 ± step)`, step halved after a sweep without progress). An improvement only counts if the
new gains also solve a set of random **validation** cases. Without that, the search happily
trades robustness for score: on `tolerance_aware` an unconstrained run reached a better score
but failed 5 of 2000 fresh cases, because it pushed `RELATIVE_DEADBAND` (the clause that makes
a ship at rest always start moving) from 0.81 to 4.3. At the end, the tuner runs a separate set
of **check** cases that the search never saw, and prints a `DEFAULT` line to paste back.

### 4. Run it

```sh
cargo run --profile fast --example tune_<name>
```

The `fast` profile skips the release profile's whole-program LTO, so the tuner builds in seconds.
The first line reports the time per evaluation: use it to size `MAX_EVALS` before a long run
(heavier controllers such as MPC can take seconds per evaluation).

### 5. Judge the result before accepting it

- **Robustness.** Accept the gains only if the check cases all pass. The validation constraint
  makes failures rare. If some still fail, the search overfitted the validation set too: raise
  `VALIDATION_COUNT`, retry with a smaller `STEP`, or constrain the offending field. Change
  `CHECK_SEED` only to get another check set, never to make a failing check pass.
- **Meaning.** Look at each tuned value against its doc comment. A "fraction of full thrust"
  above 1, a negative time constant, a margin below its physical floor: these can be genuine
  improvements (the controller exploits slack somewhere) or exploit quirks of the cases. Tell the
  user, and keep such a value only if it passes the robustness check; consider clamping it in the
  search if the user prefers values that keep their meaning.
- **Local optimum.** Coordinate descent stops at the first local optimum. If the user wants more,
  restart from the result with a larger `STEP`, or from a few different starting points.

### 6. Write the result back

1. Paste the printed `DEFAULT` into the working file. `{:?}` prints `f64` values that round-trip
   exactly, so the stored solution reproduces the tuner's score.
2. Update the solution's top doc comment: what was tuned, the score before → after, and the
   robustness check (seed and count), as `solutions/flip_and_burn_tuned.rs` does.
3. `cargo xtask eval tuning/<new_name>.rs` must show the tuner's best score exactly.
4. Submit only when the user wants it, since it changes the database:
   `cargo xtask submit <new_name> tuning/<new_name>.rs` (names are `[a-z][a-z0-9_]*`; reusing an
   existing name replaces that solution).

The stored source must stay free of tuning hooks: no environment variables, no statics read by
`new`. The `Gains` struct with a fixed `DEFAULT` is fine to keep.

### 7. Clean up and report

`examples/tune_<name>.rs` and `tuning/` are scratch, and gitignored: say where they are, and
leave them unless the user asks to keep or delete them. Report:

- the constants tuned, with old → new values;
- the score before → after (and the longest case, from `cargo xtask eval`);
- evaluations run and time taken;
- the robustness results (improvements rejected by validation, check failures);
- anything from step 5 the user should know (values outside their meaning, local optimum).

## Adapting the search

The template's search suits positive constants of unknown scale. Change it when that doesn't fit:

- **Signed values, or values near zero:** multiplicative steps cannot cross or leave zero; use
  additive steps (`*field(&mut c) += step * scale`) for those fields.
- **Integers** (e.g. `FINISH_STEPS`, a horizon length): `FIELDS` holds `f64` fields only, so
  store the constant as an `f64` field, round it where it is used
  (`self.gains.finish_steps.round() as usize`), and make sure a step changes it by at least 1.
- **One or two constants:** a grid over a range shows the whole landscape. Print the scores as a
  table and pick the best point, or zoom in around it.
- **Many constants or a rugged landscape:** random restarts, or Nelder–Mead on the same `score`
  function.

Whatever the search, keep three properties of the template: evaluate one point at a time (each
evaluation already uses every core, and every case reads the same `GAINS`); treat any failed
case of either suite as an infinite score; and accept a point only if it also passes the
validation cases.
