# Nabla

A ship in the plane with thrust along its heading and a torque. Find a controller that brings it to rest at a goal point in the fewest steps.

This is a finite, discretized version of a control problem, formally specified in [`specs/main.typ`](specs/main.typ). The workspace has these crates:

- **[`nabla-core`](crates/nabla-core):** the exact sampled-data state-space model of the ship, with the control held constant over each step; the problem: the target set, the `Controller` interface and the fixed test cases every solution must solve; and the [solutions](crates/nabla-core/src/solutions) found so far.
- **[`nabla-sim`](crates/nabla-sim):** the simulation of a world with no display, sound or network: the world format, the ship's physics among bodies, and how a pilot's input becomes a control. It builds for the web, and is what a multiplayer server shares with the client (`specs/multiplayer.typ`).
- **[`nabla-net`](crates/nabla-net):** the multiplayer protocol as sans-IO state machines (server, client, margin controller), and a seeded simulated network that checks the theorems of `specs/multiplayer.typ` as assertions.
- **[`nabla-server`](crates/nabla-server):** the multiplayer server, over WebTransport (HTTP/3 on QUIC), for native and browser pilots alike. It also holds the headless bots and the soak test that fly sixteen of them through an impaired network.
- **[`nabla-synth`](crates/nabla-synth):** the thruster sounds' synthesizer, also built alone for the web app's AudioWorklet.
- **[`nabla-client`](crates/nabla-client):** the app, a game: fly the ship to the goal yourself, or let a solution fly it as autopilot.

```sh
cargo test --workspace --profile ci      # model, app and every solution
cargo run --release -p nabla-client     # the app, natively
```

**Web app:** https://vini-fda.github.io/nabla/ lets you fly the ship on every test case, with the keyboard, touch or a game controller, with plots of the state and controls. An autopilot can fly it instead with any of the solutions, whose source it shows. The same app runs natively ([`crates/nabla-client/`](crates/nabla-client)).

## Flying together

To fly with other pilots (or to try the online mode alone), run the multiplayer server on your machine and connect from the app (Online on the title screen) or the web app with the address it logs:

```sh
cargo dev-server                          # the planet-and-moon world, on 127.0.0.1:4433
cargo dev-server --world vacuum --port 4440
```

See [`docs/multiplayer.md`](docs/multiplayer.md) for hosting a server on your own network: all its flags, passwords, certificates, and what to do when a connection fails. `cargo dev-server` is `cargo xtask dev-server`; `cargo xtask help` lists the development tasks, `cargo xtask check` runs the same checks as CI, and `cargo xtask web-dev` serves the web app locally, rebuilding it on every change. Every push to `main` deploys the web app.

On Linux, builds link with [mold](https://github.com/rui314/mold) through clang (see `.cargo/config.toml`), so both need to be installed: `sudo apt install mold clang` on Debian and Ubuntu.

## Documentation

- **[`docs/`](docs/)** — further knowledge for users and developers: [`docs/task.md`](docs/task.md) states the optimization task and the interface a solution implements; [`docs/multiplayer.md`](docs/multiplayer.md) covers running and joining a multiplayer server; [`docs/releasing.md`](docs/releasing.md) covers cutting a release.
- **[`specs/`](specs/)** — the formal specifications: Typst write-ups of the control problem ([`specs/main.typ`](specs/main.typ)) and of the multiplayer protocol ([`specs/multiplayer.typ`](specs/multiplayer.typ)), the thruster audio worklet and text fields on iOS Safari, and a TLA+ module ([`specs/tla/`](specs/tla/)) checked with SANY and TLC.
