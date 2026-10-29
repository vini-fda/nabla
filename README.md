# Nabla

A ship in the plane with thrust along its heading and a torque. Find a controller that brings it to rest at a goal point in the fewest steps.

This is a finite, discretized version of a control problem described in `docs/main.typ`. The workspace has four crates:

- **[`nabla-core`](crates/nabla-core):** the exact sampled-data state-space model of the ship, with the control held constant over each step, and the problem: the target set, the `Controller` interface and the fixed test cases every solution must solve.
- **[`nabla-optimization`](crates/nabla-optimization):** evaluation and scoring of control policies, and the harness that compiles a solution given as Rust source and scores it.
- **[`nabla-solutions`](crates/nabla-solutions):** the solutions found so far ([`solutions/`](solutions)) and their database.
- **[`nabla`](crates/nabla):** the app, which replays and plots the solutions.

See [`TASK.md`](TASK.md) for the rules, the interface a solution implements, and the commands.

**Web app:** https://vini-fda.github.io/nabla/ replays the stored solutions on every test case in continuous time, with plots, a timeline and the solution's source. The same app runs natively ([`crates/nabla/`](crates/nabla)), where it can also compile and replay any solution file, recompiling when you save it.

```sh
cargo xtask eval path/to/solution.rs     # score a solution (or several at once)
cargo xtask leaderboard                  # stored solutions, best first
cargo test --workspace --release         # model, app and every stored solution
cargo run --release -p nabla             # the app, natively
```

Development tasks are in the `xtask` crate (`cargo xtask help`). `cargo xtask check` runs the same checks as CI, plus the web build. Every push to `main` deploys the web app.

## Releasing

Pushing a version tag `vX.Y.Z` builds the native app and publishes a [GitHub Release](https://github.com/vini-fda/nabla/releases). The builds are for macOS on Apple silicon (an app bundle), Linux x86-64 and Windows 64-bit. Pushes to `main` only run CI and deploy the web app; they never release.

1. Set the workspace `version` in `Cargo.toml` (`[workspace.package]`, shared by every crate), and commit.
2. Tag and push:

   ```sh
   git tag -a v0.2.0 -m "v0.2.0"
   git push origin v0.2.0
   ```

`.github/workflows/release.yml` first checks that the tag matches the workspace version and reruns CI. It then builds each target on a native runner and publishes the release with checksums and the notes from `.github/release-notes.md`. Released apps compile solution sources against the tagged version of `nabla-core` and `nabla-optimization` in this repository, so a version's tag must stay in place once released.

### Signing the macOS app

With these secrets in the repository's `release` environment, the release signs `Nabla.app` with a Developer ID, notarizes it and staples the ticket, so Gatekeeper opens it without a warning. Without `MACOS_CERTIFICATE`, the app is signed ad-hoc and the release notes tell users how to allow it.

The signing runs in its own job, `sign-macos`, after the build: it checks out nothing and runs no cargo, so no build script or dependency ever runs next to the certificate. Only that job uses the environment, and the environment only deploys tags matching `v*.*.*` (Settings → Environments → release), so a workflow pushed on a branch cannot read the secrets.

| Secret | Value |
|---|---|
| `MACOS_CERTIFICATE` | A *Developer ID Application* certificate with its private key, exported from Keychain Access as `.p12`, in base64 |
| `MACOS_CERTIFICATE_PASSWORD` | The password of that `.p12` |
| `APPLE_API_KEY` | The contents of an App Store Connect API key, `AuthKey_<id>.p8` |
| `APPLE_API_KEY_ID` | That key's ID |
| `APPLE_API_ISSUER_ID` | The issuer ID shown above the list of keys |

1. Create the certificate in Xcode → Settings → Accounts → the team → Manage Certificates → + → Developer ID Application (only the team's Account Holder can), which puts it and its key in the login keychain. Export both from Keychain Access → My Certificates as `.p12`. An *Apple Distribution* certificate is for the App Store and cannot be notarized.
2. Create the API key in [App Store Connect](https://appstoreconnect.apple.com/access/integrations/api) → Users and Access → Integrations → Team Keys, with the Developer role. The `.p8` downloads only once.
3. Store them in the environment:

   ```sh
   base64 -i DeveloperID.p12 | gh secret set MACOS_CERTIFICATE --env release
   gh secret set MACOS_CERTIFICATE_PASSWORD --env release
   gh secret set APPLE_API_KEY --env release < AuthKey_XXXXXXXXXX.p8
   gh secret set APPLE_API_KEY_ID --env release
   gh secret set APPLE_API_ISSUER_ID --env release
   ```

To sign a local bundle the same way: `cargo xtask bundle --sign "Developer ID Application: <name> (<team>)"`.
