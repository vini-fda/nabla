# ship2d_optimization

A ship in the plane with thrust along its heading and a torque. Find a controller that brings it to rest at a goal point in the fewest steps.

This is a finite, discretized version of a control problem described in `docs/main.typ`:

- **Model:** the exact sampled-data model of the ship, with the control held constant over each step.
- **Test suites:** fixed test cases that every solution must solve.
- **Harness:** compiles a solution given as Rust source and scores it.
- **Database:** the solutions found so far.

See [`TASK.md`](TASK.md) for the rules, the interface a solution implements, and the commands.

**Web app:** https://vini-fda.github.io/ship2d_optimization/ replays the stored solutions on every test case in continuous time, with plots, a timeline and the solution's source. The same app runs natively ([`viewer/`](viewer)), where it can also compile and replay any solution file, recompiling when you save it.

```sh
cargo xtask eval path/to/solution.rs     # score a solution (or several at once)
cargo xtask leaderboard                  # stored solutions, best first
cargo test --workspace --release         # model, viewer and every stored solution
cargo run --release -p ship2d_graphics   # the viewer, natively
```

Development tasks are in the `xtask` crate (`cargo xtask help`). `cargo xtask check` runs the same checks as CI, plus the web build. Every push to `main` deploys the web app.

## Releasing

Pushing a version tag `vX.Y.Z` builds the native viewer and publishes a [GitHub Release](https://github.com/vini-fda/ship2d_optimization/releases). The builds are for macOS on Apple silicon (an app bundle), Linux x86-64 and Windows 64-bit. Pushes to `main` only run CI and deploy the web app; they never release.

1. Set the same `version` in `Cargo.toml` and `viewer/Cargo.toml`, and commit.
2. Tag and push:

   ```sh
   git tag -a v0.2.0 -m "v0.2.0"
   git push origin v0.2.0
   ```

`.github/workflows/release.yml` first checks that the tag matches both versions and reruns CI. It then builds each target on a native runner and publishes the release with checksums and the notes from `.github/release-notes.md`. Released viewers compile solution sources against the tagged version of this repository, so a version's tag must stay in place once released.
