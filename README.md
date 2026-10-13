# ship2d_optimization

A ship in the plane with thrust along its heading and a torque. Find a controller that brings it to rest at a goal point in the fewest steps.

This is a finite, discretized version of a control problem formalized in Lean 4 in the `ship2d_lean` project:

- **Model:** the exact sampled-data model of the ship, with the control held constant over each step.
- **Test suites:** fixed test cases that every solution must solve.
- **Harness:** compiles a solution given as Rust source and scores it.
- **Database:** the solutions found so far.

See [`TASK.md`](TASK.md) for the rules, the interface a solution implements, and the commands.

**Web app:** https://vini-fda.github.io/ship2d_optimization/ replays the stored solutions on every test case in continuous time, with plots, a timeline and the solution's source. The same app runs natively ([`viewer/`](viewer)), where it can also compile and replay any solution file, recompiling when you save it.

```sh
cargo run --release -- eval path/to/solution.rs   # score a solution
cargo run --release -- leaderboard                # stored solutions, best first
cargo test --workspace --release                  # model, viewer and every stored solution
cargo run --release -p ship2d_graphics            # the viewer, natively
```

`./check.sh` runs the same checks as CI, plus the web build. Every push to `main` deploys the web app.
