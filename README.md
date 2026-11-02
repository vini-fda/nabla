# Nabla

A ship in the plane with thrust along its heading and a torque. Find a controller that brings it to rest at a goal point in the fewest steps.

This is a finite, discretized version of a control problem described in `docs/main.typ`. The workspace has these crates:

- **[`nabla-core`](crates/nabla-core):** the exact sampled-data state-space model of the ship, with the control held constant over each step; the problem: the target set, the `Controller` interface and the fixed test cases every solution must solve; and the [solutions](crates/nabla-core/src/solutions) found so far.
- **[`nabla-sim`](crates/nabla-sim):** the simulation of a world with no display, sound or network: the world format, the ship's physics among bodies, and how a pilot's input becomes a control. It builds for the web, and is what a multiplayer server shares with the client (`docs/multiplayer.typ`).
- **[`nabla-net`](crates/nabla-net):** the multiplayer protocol as sans-IO state machines (server, client, margin controller), and a seeded simulated network that checks the theorems of `docs/multiplayer.typ` as assertions.
- **[`nabla-server`](crates/nabla-server):** the multiplayer server, over WebTransport (HTTP/3 on QUIC), for native and browser pilots alike. It also holds the headless bots and the soak test that fly sixteen of them through an impaired network.
- **[`nabla-synth`](crates/nabla-synth):** the thruster sounds' synthesizer, also built alone for the web app's AudioWorklet.
- **[`nabla-client`](crates/nabla-client):** the app, a game: fly the ship to the goal yourself, or let a solution fly it as autopilot.

See [`TASK.md`](TASK.md) for the rules and the interface a solution implements.

**Web app:** https://vini-fda.github.io/nabla/ lets you fly the ship on every test case, with the keyboard, touch or a game controller, with plots of the state and controls. An autopilot can fly it instead with any of the solutions, whose source it shows. The same app runs natively ([`crates/nabla-client/`](crates/nabla-client)).

```sh
cargo test --workspace --release         # model, app and every solution
cargo run --release -p nabla-client             # the app, natively
```

## Running a dev server

To fly with other pilots (or to try the online mode alone), run the multiplayer server on your machine:

```sh
cargo dev-server                          # the planet-and-moon world, on 127.0.0.1:4433
cargo dev-server --world vacuum --port 4440
cargo dev-server --release -- --max-pilots 4   # anything after `--` goes to nabla-server
```

(`cargo dev-server` is `cargo xtask dev-server`; see `cargo xtask help`. `--world` is `vacuum`, `planet`, `planet-moon` or a world file.) It makes a self-signed certificate, good for two weeks, and logs the address to connect to, which has the certificate's hash after a `#`:

```text
connect to https://127.0.0.1:4433#sha256=dea539c5...
in the web app of `cargo xtask web-dev`: http://localhost:8080/?server=...&connect#dev
```

Then:

- **Native app:** `cargo nabla`, then Online on the title screen, paste the address, Connect, and take a ship in the lobby. Open the app again, or run `cargo run -p nabla-server --bin nabla-bot -- --server '<address>'` for a headless pilot, to have company.
- **Web app:** `cargo xtask web-dev`, then open the link the server logged. The page's `?server=` is the address, percent-encoded, and `&connect` connects at once. Browsers need WebTransport, which Chrome, Edge and Firefox have, and Safari from 26.4; only Chromium has been tried so far.
- **Soak test:** `cargo run --release -p nabla-server --bin nabla-soak` flies sixteen bots through a lossy, delaying proxy for ten minutes, and fails if the expectations of `docs/multiplayer.typ`, M4, are not met (`--help` there lists the flags).

For a server others can reach, give it a name and a real certificate: `cargo run --release -p nabla-server -- --bind 0.0.0.0:4433 --cert cert.pem --key key.pem`. Without `--cert` it makes a self-signed one for `--name` hosts, and each client needs the hash. UDP port 4433 must be open, since WebTransport runs on QUIC.

Development tasks are in the `xtask` crate (`cargo xtask help`). `cargo xtask check` runs the same checks as CI, plus the web build. `cargo xtask web-dev` serves the web app locally, rebuilding it on every change. Every push to `main` deploys the web app.

On Linux, builds link with [mold](https://github.com/rui314/mold) through clang (see `.cargo/config.toml`), so both need to be installed: `sudo apt install mold clang` on Debian and Ubuntu.

## Releasing

Pushing a version tag `vX.Y.Z` builds the native app and publishes a [GitHub Release](https://github.com/vini-fda/nabla/releases). The builds are for macOS on Apple silicon (an app bundle), Linux x86-64 and Windows 64-bit. Pushes to `main` only run CI and deploy the web app; they never release.

1. Bump the workspace `version` in `Cargo.toml` (`[workspace.package]`, shared by every crate) and `Cargo.lock`, then commit and get the commit onto `main`:

   ```sh
   cargo bump minor  # or major, or patch
   ```

2. After the merge, on `main`, run:

   ```sh
   cargo tag
   ```

   It only tags if the checkout is `main`, has no uncommitted changes and is level with `origin/main`, and if the version in `Cargo.toml` is newer than every existing tag. It then asks whether to push the tag, which publishes the release.

`.github/workflows/release.yml` first checks that the tag matches the workspace version and reruns CI. It then builds each target on a native runner and publishes the release with checksums and the notes from `.github/release-notes.md`.

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
