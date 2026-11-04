# Nabla

A game on the 2D ship problem ([`nabla-core`](../nabla-core)), rendered with [egui](https://github.com/emilk/egui): fly the ship to rest at the goal `B` of a test case, yourself or with a control policy as **autopilot**. It is also the web app deployed to GitHub Pages: https://vini-fda.github.io/nabla/

Vacuum flight uses the same exact sampled-data model the solutions are scored on. At each sample `t_k = k h`, the controls held then become `u_k`, held for the step, and `s_(k+1) = step(s_k, u_k)`. Between samples the canvas shows the exact state at the present time, one partial `step` from `s_k`: computed, not interpolated.

## Worlds and saves

Choose **Pure vacuum**, **Planet**, or **Planet + moon** (the default) on the title screen, or switch from the **World** dropdown in flight, which starts the flight over. Each preset is an embedded TOML file in [`assets/worlds`](../nabla-sim/assets/worlds). The existing starting-condition presets and custom ship controls work inside the selected world.

**Save world...** on the title screen saves the setup; in flight Settings it saves the player's current position, velocity, heading and spin. **Load world...** opens a TOML file and returns to the title screen; press Play to resume from that state. Native builds use file dialogs; the browser uploads and downloads the same portable format. Restart restores the selected world's starting state. Saves include ship parameters, size, goal and tolerances, but not replay history or UI/autopilot settings.

A version 1 world contains `name`, `gravity_constant`, `players` (currently exactly one), and `bodies`. Each player has `state`, `task` and `radius`; each celestial body has `name`, `position = [x, y]`, `radius`, and gravitational `mass`. The preset files are complete examples.

The **gravitational constant** `G` on the title screen sets the world's gravity strength before flight; it is preserved by save/load and restart. Zero disables attraction. The planet presets use `G = 1` in world units, with planet mass `343` and moon mass `15.625`. Older version 1 saves without these fields retain `G = 0` and default body masses to `1`.

Planets and moons remain static and attract only the player. Their Newtonian accelerations add: each contributes `G * mass / distance^2` toward its center, independently of the player's mass. Inside a body's radius, the field follows a uniform solid sphere (linear in distance, zero at the center). Debug draws white circles and includes gravity in the acceleration arrow; 3D draws shaded spheres with depth occlusion. The ship uses a circular collision envelope sized by the ship-size control, which is set before flight. Worlds with bodies run Rapier at fixed substeps of at most 1/120 s, recalculating gravity at each substep, with continuous collision detection and recorded substeps for rendering and plumes. Vacuum retains the exact control model. Orbit assist accounts for the summed gravitational field. The original point-goal policies still plan in vacuum and neither compensate for gravity nor avoid planets; their comparison scores remain vacuum scores.

**Collision meshes** in Display options overlays the ship and celestial-body collision circles in orange in either Debug or 3D. It starts off and follows the displayed ship state, including while paused. The overlay is display-only and uses the world's existing validated geometry.

Security: loaded TOML is schema-validated and limited to 1 MiB, one player and 128 bodies. Nonfinite or out-of-range physical values and unsupported versions are rejected before creating physics resources. Gravity accepts finite `G` from `0` to `10000` and body masses from `0` to `1e12`; the interior field avoids a singularity at a body's center. File access uses paths selected in the file dialog; saves contain data only. Orbit requests validate body indices and finite altitude bounds before simulation; prediction work is capped at 6000 control steps and 120 simulated seconds. Failed requests preserve the active flight mode. Display forecasts are separately capped at 500 control steps and five simulated seconds, run without mutating the live controller, and are not computed while hidden.

## What's on screen

- **Left panel**: the case picker, its sampling period `h`, parameters and initial state, and how the autopilot's policy does on it; the **Autopilot** (its policy, its switch, and **Show code**, a window with the policy's source, syntax highlighted and with line numbers); the state and control at present; display options; and sound: master/ion/RCS volume knobs, and buttons to hear each thruster on its own.
- **Main canvas**: the ship, in 3D or as a flat triangle with velocity (green) and acceleration (red) arrows (**Debug** in Settings), in a Desmos-style coordinate grid you can pan (drag) and zoom (scroll, anchored at the cursor). The camera follows the ship from the start. It starts as a top-down orthographic view; the axes gizmo in its top right corner turns it to look from any side. Right of it, floating buttons show or hide the gizmo (sphere), follow the ship (rocket) and show or hide the path flown (a curve between two dots), turning light blue while on, switch dragging the canvas between panning and rotating the camera (arrows), and switch between orthographic and perspective projection (grid). Perspective uses the field of view of Blender's default 50 mm camera, and keeps the plane through the view's center at the same zoom as orthographic. It shows the path, the start, the goal `B` with its position tolerance, and a marker at the canvas's edge pointing to `B` while it is off screen.
- **Bottom bar**: pause and restart, the **SAS**, **Autopilot** and **Point** switches, and the time.
- **Right panel**: choose **State plots** or **Objective** in its vertical title bar, where the one showing is white and the other gray. **State plots**: x, y, vx, vy, θ, ω and the controls F, τ over the last 60 s of the flight, with a cursor at the present. **Objective**: a mutually exclusive selector (**Reach B** or **Orbit**), with orbit target, altitude, preview and engagement controls when Orbit is selected.

Once the ship lands, a banner gives the step count, next to the policy's on the same case. The plots, plumes and sound follow the flight.

## Flying

Thrust is all or nothing from the keyboard, Cross and the touch pad, and so is torque from the keyboard; R2 gives as much thrust as it is pulled, and the sticks as much torque as they are pushed. Fine control scales both to a tenth, since one step at full power can change the speed or the spin by more than the target set allows. **SAS** (`T`, as in Kerbal Space Program) switches what letting go of the turn does. Off, the default, nothing: the ship spins on, and stopping a spin is up to you. On, a stability assist like KSP's takes over the torque whenever you are not turning: it brakes the spin, with the ship's full torque whatever the fine control, and holds the heading the ship comes to. Turning again overrides it, and it holds the new heading once you let go. A faint line from the ship ending in a ring shows that heading all along: while you turn, it is where the ship would come to rest if you let go now. The choice is kept from flight to flight.

**Point** lets you fly with the mouse or a finger instead. Hold it on the canvas, and the ship turns toward it, the shorter way, and thrusts once it faces it within a cone, whose half-angle (10 degrees by default) is set next to the switch. A line from the ship to the pointer, between the cone's edges, turns green while it thrusts. Point works only while the camera follows the ship: then a still pointer holds a direction relative to the ship, while otherwise it would mark a fixed point the ship flies past. While it steers, the on-screen controls (joystick, thrust and fine) go, and a finger anywhere on the canvas points. Dragging the canvas points rather than turning the camera, and a touch on the buttons in the corner never points. The policy, and why it settles without ringing, is modeled in [`docs/main.typ`](../../docs/main.typ) ("Problem: pointing").

## Objectives and autopilot

The flight has one `Objective` enum value: `ReachB` or `Orbit(Target)`. The active controller is also a single enum, so both controllers cannot run together. Select **Reach B** or **Orbit** in the right panel's **Objective** view, or use **D-pad Left/Right**, which also brings that view up and announces the change. Changing the objective disengages control and clears the previous objective's success state; **Square** engages the selected objective. **A** and the Autopilot switches control that same selection. **Circle** returns to manual flight while retaining the objective. B's marker, distance, landing status and policy comparison appear only for Reach B; Orbit shows its ring and orbit status. New flights default to Reach B in vacuum and Orbit in worlds with bodies. Objectives and engagement are session state, not part of world saves.

**Reach B** uses a control policy: one of the solutions compiled into `nabla-core` (see [`TASK.md`](../../TASK.md)), picked in Settings, `mpc` by default. At every sample, the policy's controller reads the exact state and chooses the control, as when it is scored; your flight controls do nothing meanwhile. Switch it off to take over again, and on to hand the ship back, from wherever it is: the controller is made afresh each time, as if the case started there. In the target set it holds no control, and should the ship drift out it is made afresh again to bring it back. Engaged from the start of a flight (switch it on, then restart with `R`), it flies exactly the run the policy is scored on. Should the policy panic or choose an inadmissible control, it switches itself off and says why in Settings. Restart preserves an engaged Reach B controller; the title-screen **Autopilot flies from the start** preference controls engagement when beginning a new flight.

The `mpc` policy plans its whole way to the goal when it takes over, which can take up to a second on the farthest cases: the flight waits for it.

## Orbit assist

In **Planet**, press **Square** (or **Engage orbit**) to capture the previewed circular orbit. **R3** cycles planets/moons. Selecting a body sets the preview to the current altitude and direction of travel about it; zero angular momentum defaults to counterclockwise. **D-pad Up/Down** changes altitude by 10% (at least 1 world unit); hold **L2** for 0.1-unit adjustments. Altitude is measured from the body's surface to the ship's center. The target picker and altitude field provide the same controls with mouse/touch.

A dashed ring shows the selected orbit. The red line shows the controller's short forecast. A solid green ring and the **Active orbit** group identify the engaged orbit, with what the assist is doing. Editing the preview leaves the active orbit untouched until Square confirms it. **Circle** (or **Manual**) disengages either assist; manual thrust and turning then resume. Orbit assist ignores manual flight inputs while engaged. Pause preserves it; restart clears it and selects the nearest body again. To use a point-goal policy, select Reach B and engage it.

The assist turns toward corrective thrust, burns to capture, and coasts when close enough. **Holding orbit** means radial position, radial speed and tangential speed have stayed within tolerance for two simulated seconds; it resets if those errors leave tolerance. Position tolerance is the larger of 1.5% of orbital radius and 0.2 units; velocity tolerance is the larger of 2.5% of target speed and 0.05 units/s. The heading is free, and powered maintenance can still turn and thrust. The point-goal marker and landing detection are suppressed whenever Orbit is the objective, including in manual flight.

Each engagement runs a fresh prediction using the flight's physics, checking bounded controls, body clearance at substeps, and sustained capture at the end of the horizon. Zero gravity, obstructed rings, insufficient thrust margin and unsuccessful capture predictions produce an explanation and leave the current flight mode unchanged. The preview refreshes once per simulated second unless the selected orbit is already active, and after edits even while paused. Prediction is capped at 120 simulated seconds or 6000 control steps, whichever is shorter. This is a practical assist, not a proof of stability or indefinite collision avoidance; multiple static sources can require continuous corrections. The formulation and remaining proof obligations are in [`docs/main.typ`](../../docs/main.typ).

## Controller prediction

**Display options → Controller prediction (red)** toggles the red forecast independently of the flown path and target rings; it starts enabled. The line refreshes every 0.25 simulated seconds, including during orbit holding, and covers up to five seconds or 500 control steps. Changing objectives, confirming a target, or toggling the display clears stale forecasts. Switching it off does not alter control or capture validation.

For Orbit, the forecast rolls out a copy of the live controller, including its coast/hold state, with the world's gravity and physics; before engagement it previews the selected orbit controller. For Reach B, it displays the MPC controllers' remaining plan or a rollout of the flip-and-burn policy after engagement. These point-goal predictions use the policies' vacuum model; `mpc_bounded`, the default policy, adds the acceleration it has learned from what its predictions missed, and so is the one that lands in worlds with gravity. Predictions are estimates and can change as the controller replans. `Controller::prediction` is optional so future policies can expose their forecast without changing control execution.

## Inputs

| Input | Effect |
| --- | --- |
| `Space` (or `↑`) held | Thrust |
| `←` / `→` held | Turn left / right |
| `Shift` held | Fine control (a tenth of the thrust and torque) |
| `P` / `R` | Pause / restart |
| `T` | SAS on / off |
| `A` | Autopilot on / off |
| `F` | Follow the ship (keep it centered) / stop following |
| Hold the mouse on the canvas, with **Point** on | Turn toward the pointer, and thrust when facing it |
| Mouse drag | Pan the view, or orbit the camera in rotate mode |
| Scroll wheel | Zoom (cursor-anchored) |
| Drag the axes gizmo (top right) | Orbit the camera around the view's center |
| Click an axis ball | Look along that axis from its side (click again for the opposite side) |
| **Top-down view** button under the gizmo | Back to the default top-down orthographic view |
| Sphere button | Show / hide the gizmo |
| Rocket button | Follow the ship, as `F` |
| Path button | Show / hide the path of the last 60 s, as the **Path** checkbox in Settings |
| Arrows button | Switch what dragging the canvas does: pan (four arrows, the default) or rotate the camera (circular arrow) |
| Grid button | Switch between orthographic (flat grid, the default) and perspective projection |
| Controller: left stick / Cross / R2 / L2 held | Turn / full thrust / thrust as hard as it is pulled / fine control |
| Controller: R1 / L1 held | Zoom in / out, about the canvas's center |
| Controller: right stick | Aim, as **Point** |
| Controller: D-pad Left / Right | Select Reach B / Orbit objective |
| Controller: D-pad Up / Down; L2 held | Raise / lower selected orbit altitude; fine adjustment |
| Controller: R3 | Cycle orbit target body |
| Controller: Square | Engage the selected objective |
| Controller: Circle | Return to manual flight |
| Controller: Triangle / Options / Create | SAS on / off, pause / resume, **User inputs** window on / off |

A **game controller** works natively and in the browser (read with [gilrs](https://crates.io/crates/gilrs); a browser only lists one once a button has been pressed with the page open, and that press counts). A small controller icon, at the title screen's top right and at the right of the flight's bottom bar (on wide windows), is green with one connected and gray without, and a banner tells when one connects or disconnects. The right stick aims like **Point**: the ship turns toward where it points on screen and thrusts once it faces it within the cone. Being a direction rather than a spot, it needs neither Point nor the camera following the ship. Buttons are named as on a Sony controller; on an Xbox one, Cross is A, Square X, Triangle Y, L1 and R1 LB and RB, and L2 and R2 LT and RT. **User inputs** in Help has a **Controller** tab, shown first while one is connected: a map of the controller with an elbow line from each control in use to its icon and what it does, all lighting as you use it (a list of the same on narrow screens). Switch off **Guides** there for only the controller, lit as you use it. Its button icons are from [PS5 Button Icons and Controls](https://zacksly.itch.io) by Zacksly, licensed under [CC BY 3.0](http://creativecommons.org/licenses/by/3.0/) (see [`assets/controller_icons/LICENSE.txt`](assets/controller_icons/LICENSE.txt)), renamed and tinted when drawn. The **controller view** (off by default: **Controller view** in Settings, or the controller's Create button) draws a wireframe of the controller under the flight, on a pure black panel, lit as you use it: the sticks move in their wells, the face buttons glow in their colors, the triggers fill as far as they are pulled, and the other buttons light while held.

On phones and tablets (and once a touch screen is used), controls appear over the canvas. In the bottom left corner, a joystick turns the ship: push it sideways, and the further it goes the harder it turns (left counterclockwise, right clockwise), with a small dead zone in the middle. It follows the finger that took it until that finger lifts, even off its base. In the bottom right, a thrust pad is held while a finger is on it, and a **fine** latch above it turns on or off with a tap. So one thumb steers while the other thrusts. They go while the autopilot flies.

## Architecture

```
src/
├── main.rs       # eframe entry point (native + WASM)
├── lib.rs        # module wiring
├── app.rs        # egui UI: case & autopilot, the game's bar, canvas, plots, code window
├── game.rs      ← the flight: grows a sample at a time from the player's controls or the autopilot's policy; touch joystick and pads
├── replay.rs    ← a recorded run on a case: exact states between samples; a flight keeps its last 60 s, in buffers of a fixed size
├── plots.rs     ← what the state plots show: the same 60 s as points to draw, in buffers of a fixed size
├── camera.rs    ← orthographic orbit camera: world↔screen transform, pan, cursor-anchored zoom
├── gizmo.rs     ← Blender-style axes gizmo: orbit by dragging, snap to an axis by clicking
├── canvas_buttons.rs ← floating icon toggles over the canvas (gizmo, follow ship)
├── grid.rs      ← Desmos-style grid with 1/2/5×10ⁿ adaptive spacing
├── synth.rs     ← thruster sound synthesizer: ion thruster thrum, RCS hiss
└── sound.rs     ← audio output (cpal: native or Web Audio), sound settings and volume knobs
```

`replay`, `camera` and `grid` are free of egui widget types, so they're independently unit-testable. `cargo test --lib` runs the test suite.

## Running

From the repository root:

```sh
cargo run --release -p nabla-client
```

While working on the viewer, use `cargo nabla`. It builds with the `fast` profile, which keeps release's optimization level but skips its link-time optimization, so after an edit the viewer rebuilds in seconds instead of about a minute.

On macOS, `cargo nabla` (`cargo xtask run --profile fast`) runs the viewer inside an app bundle, `Nabla.app`, with the layered Icon Composer icon (`crates/nabla-client/macos/nabla_macOS.icon`) that macOS 26 renders with Liquid Glass; a bare executable, as from `cargo run`, can only show a flat rounded icon. `cargo xtask bundle` builds `target/release/Nabla.app` on its own. The Liquid Glass icon needs Xcode 26, for `actool`; without it the bundle gets a flat icon. See `cargo xtask help`.

```sh
cargo xtask bundle && open "target/release/Nabla.app"
```

On Linux you may need:

```sh
sudo apt-get install libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev libxkbcommon-dev libssl-dev libasound2-dev libudev-dev
```

For a web build with [Trunk](https://trunkrs.dev/):

```sh
cargo install --locked trunk
cargo xtask web-dev   # then open http://127.0.0.1:8080/#dev
```

`cargo xtask web-dev` runs `trunk serve` in this directory; pass `--port` for another port, or `--release` for an optimized build.

The `#dev` suffix skips the service worker, which otherwise caches the app for offline use.

Every push to `main` deploys the web build to GitHub Pages (`.github/workflows/pages.yml` at the repository root).

## Logging

Natively, the app logs to stderr, by default its warnings and what happens in a session. `RUST_LOG` says what else, as in:

```sh
RUST_LOG=nabla_client=debug cargo nabla                 # also the occasional costly operations
RUST_LOG=nabla_client=trace cargo nabla                 # also those of every frame and sample
RUST_LOG=warn,nabla_client::game=trace cargo nabla      # one module only
```

`cargo nabla` logs in a development format: UTC timestamps, the thread (if it is not the main one) and the source line, and `debug` for this workspace's crates, as in

```text
2026-11-02T20:56:46.472Z INFO  nabla_client::app::online crates/nabla-client/src/app/online.rs:67: Connecting to https://127.0.0.1:4433
```

Set `RUST_LOG` as above to say otherwise; with it unset, the filter is `info` for everything and `debug` for `nabla-*`. Any other way of starting the app (`cargo run -p nabla-client`, a release build) keeps the lean format, unless `NABLA_LOG_STYLE=dev` is in the environment.

The sound logs under `nabla_client::sound`, every line beginning with `audio:`: changes at `debug` (sound on or off, the output opened, a thruster starting or stopping, the knobs, the output device and its format), a status line from the backend every two seconds (natively how many times the audio callback ran, the frames it rendered, its peak level and whether it went quiet; in a browser the context's state and whether the processor has loaded), and the controls published every frame at `trace`:

```sh
cargo nabla 2>&1 | grep audio:                          # what the sound is doing
RUST_LOG=nabla_client::sound=trace cargo nabla          # also every frame's controls
```

On the web, the page's address says it, as in `?log=debug`, and the log is in the browser's console.

Costly operations log their start and their end, with how long they took: `let _span = span!(Level::Debug, "...");` (`src/span.rs`). Frames over 100 ms are logged as warnings, at most once a second, and with `debug`, so is any control sample over the sampling period.

## Development

See the repository root: `cargo xtask check` runs the same checks as CI, plus the web build, and `.githooks/pre-commit` runs the format and lint checks before each commit (enable it with `git config core.hooksPath .githooks`).

## Acknowledgments

Built on top of [emilk/eframe_template](https://github.com/emilk/eframe_template). The adaptive grid-spacing algorithm in `src/grid.rs` is adapted from [`egui_plot`](https://github.com/emilk/egui_plot) (MIT/Apache).
