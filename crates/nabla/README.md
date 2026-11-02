# Nabla

A viewer for the control policies of the 2D ship problem ([`nabla-core`](../nabla-core)), rendered with [egui](https://github.com/emilk/egui). It is also the web app deployed to GitHub Pages: https://vini-fda.github.io/nabla/

Pick a solution and a test case. The viewer runs the solution on that case with the exact sampled-data model of `nabla-core`. It then replays the run on a pannable, zoomable canvas.

Playback is continuous in time. Between controller samples the control is held constant, so the state at any `t = k h + τ` is exactly one partial `step` of length `τ` from `s_k`. It is computed, not interpolated. The display sampling time `Δt = h/n` can be made much finer than `h`, for smooth playback on high-refresh displays.

## What's on screen

- **Left panel**: the solution picker, the case picker, the case's parameters and initial state, the outcome (steps to reach the target set), the state and control at the current frame, display options, and sound: the ion thruster's sound (five synthesized styles to choose from), master/ion/RCS volume knobs, and buttons to hear each thruster on its own.
- **Main canvas**: the ship drawn as an isosceles triangle in a Desmos-style coordinate grid you can pan (drag) and zoom (scroll, anchored at the cursor). It starts as a top-down orthographic view; the axes gizmo in its top right corner turns it to look from any side. Right of it, floating buttons show or hide the gizmo (sphere), follow the ship (rocket) and show or hide the path (a curve between two dots), turning light blue while on, switch dragging the canvas between panning and rotating the camera (arrows), and switch between orthographic and perspective projection (grid). Perspective uses the field of view of Blender's default 50 mm camera, and keeps the plane through the view's center at the same zoom as orthographic. It shows the full path (grey), the part travelled so far (blue), the start, the goal `B` with its position tolerance, and the velocity (green) and acceleration (red) arrows.
- **Bottom panel**: playback controls, the playback speed (relative to real time), the display sampling time `Δt`, and an After Effects-style timeline. The timeline has a thin yellow mark at every controller sample `t_k = k h` and a playhead. Click or drag to seek, scroll to zoom, shift+scroll to pan, double-click to show the whole run.
- **Right panel**: x, y, vx, vy, θ, ω and the controls F, τ over the whole run, with a cursor at the current time. Click a plot to jump to that time.

| Input | Effect |
| --- | --- |
| `Space` | Play / pause |
| `←` / `→` | Back / forward by `Δt` |
| `Shift` + `←` / `→` | Previous / next controller sample |
| `Home` / `End` | Start / end of the run |
| `F` | Follow the ship (keep it centered) / stop following |
| Mouse drag | Pan the view, or orbit the camera in rotate mode |
| Scroll wheel | Zoom (cursor-anchored) |
| Drag the axes gizmo (top right) | Orbit the camera around the view's center |
| Click an axis ball | Look along that axis from its side (click again for the opposite side) |
| **Top-down view** button under the gizmo | Back to the default top-down orthographic view |
| Sphere button | Show / hide the gizmo |
| Rocket button | Follow the ship, as `F` |
| Path button | Show / hide the path, as the **Path** checkbox in Settings |
| Arrows button | Switch what dragging the canvas does: pan (four arrows, the default) or rotate the camera (circular arrow) |
| Grid button | Switch between orthographic (flat grid, the default) and perspective projection |

## Flying it yourself

**Fly it yourself** (the game controller button over the canvas, or **Mode** at the top of the left panel) switches to gameplay: no control policy runs, and you fly the ship to `B` on the selected case. There is no timeline: time runs freely from the start, and the bottom panel holds only pause, restart and exit, the **SAS** and **Point** switches and the time. Once the ship lands, a banner gives the step count, next to the policy's on the same case.

The flight uses the same exact sampled-data model as the solutions. At each sample `t_k = k h`, the controls you hold become `u_k`, held for the step. Thrust is all or nothing, and so is torque from the keyboard; the touch joystick gives as much torque as it is pushed. Fine control scales both to a tenth, since one step at full power can change the speed or the spin by more than the target set allows. **SAS** (`T`, as in Kerbal Space Program) switches what letting go of the turn does. Off, the default, nothing: the ship spins on, and stopping a spin is up to you. On, a stability assist like KSP's takes over the torque whenever you are not turning: it brakes the spin, with the ship's full torque whatever the fine control, and holds the heading the ship comes to. Turning again overrides it, and it holds the new heading once you let go. A faint line from the ship ending in a ring shows that heading all along: while you turn, it is where the ship would come to rest if you let go now. The choice is kept from flight to flight.

**Point** lets you fly with the mouse or a finger instead. Hold it on the canvas, and the ship turns toward it, the shorter way, and thrusts once it faces it within a cone, whose half-angle (10 degrees by default) is set next to the switch. A line from the ship to the pointer, between the cone's edges, turns green while it thrusts. Point works only while the camera follows the ship: then a still pointer holds a direction relative to the ship, while otherwise it would mark a fixed point the ship flies past. While it steers, the on-screen controls (joystick, thrust and fine) go, and a finger anywhere on the canvas points. Dragging the canvas points rather than turning the camera, and a touch on the buttons in the corner never points. The policy, and why it settles without ringing, is modeled in [`docs/main.typ`](../../docs/main.typ) ("Problem: pointing").

A **game controller** works too, natively and in the browser (read with [gilrs](https://crates.io/crates/gilrs); a browser only lists one once a button has been pressed with the page open). The left stick turns, as hard as it is pushed sideways, Cross or R2 thrusts, L2 held is fine control, R1 and L1 zoom in and out while held, Triangle switches SAS, Options pauses and Create shows or hides the controller view. The right stick aims like **Point**: the ship turns toward where it points on screen and thrusts once it faces it within the cone. Being a direction rather than a spot, it needs neither Point nor the camera following the ship. Buttons are named as on a Sony controller; on an Xbox one, Cross is A, Triangle Y, L1 and R1 LB and RB, and L2 and R2 LT and RT. The **controller view** (off by default: **Controller view, while flying** in Settings, or the controller's Create button) draws a wireframe of the controller under the flight, on a pure black panel, lit as you use it: the sticks move in their wells, the face buttons glow in their colors, the triggers fill as far as they are pulled, and the other buttons light while held.

The plots, plumes and sound follow the flight, and a marker at the canvas's edge points to `B` while it is off screen.

| Input | Effect |
| --- | --- |
| `Space` (or `↑`) held | Thrust |
| `←` / `→` held | Turn left / right |
| `Shift` held | Fine control (a tenth of the thrust and torque) |
| `P` / `R` | Pause / restart |
| `T` | SAS on / off |
| Hold the mouse on the canvas, with **Point** on | Turn toward the pointer, and thrust when facing it |
| Controller: left stick / Cross or R2 / L2 held | Turn / thrust / fine control |
| Controller: R1 / L1 held | Zoom in / out, about the canvas's center |
| Controller: right stick | Aim, as **Point** |
| Controller: Triangle / Options / Create | SAS on / off, pause / resume, controller view on / off |
| `F` | Follow the ship / stop following |

On phones and tablets (and once a touch screen is used), controls appear over the canvas. In the bottom left corner, a joystick turns the ship: push it sideways, and the further it goes the harder it turns (left counterclockwise, right clockwise), with a small dead zone in the middle. It follows the finger that took it until that finger lifts, even off its base. In the bottom right, a thrust pad is held while a finger is on it, and a **fine** latch above it turns on or off with a tap. So one thumb steers while the other thrusts.

## Solutions

The viewer offers the solutions compiled into `nabla-core` (see [`TASK.md`](../../TASK.md)), and starts on the first, `mpc`. **Show code** opens a window with the source, syntax highlighted and with line numbers.

## Architecture

```
src/
├── main.rs       # eframe entry point (native + WASM)
├── lib.rs        # module wiring
├── app.rs        # egui UI: solution & case, playback, canvas, plots, code window
├── replay.rs    ← a solution run on a case, exact states between samples, playback on the Δt grid
├── game.rs      ← gameplay mode: a flight that grows a sample at a time from the player's controls, touch joystick and pads
├── timeline.rs  ← After Effects-style timeline: sample marks, playhead, zoom
├── camera.rs    ← orthographic orbit camera: world↔screen transform, pan, cursor-anchored zoom
├── gizmo.rs     ← Blender-style axes gizmo: orbit by dragging, snap to an axis by clicking
├── canvas_buttons.rs ← floating icon toggles over the canvas (gizmo, follow ship)
├── grid.rs      ← Desmos-style grid with 1/2/5×10ⁿ adaptive spacing
├── synth.rs     ← thruster sound synthesizer: ion thruster styles, RCS hiss
└── sound.rs     ← audio output (cpal: native or Web Audio), sound settings and volume knobs
```

`replay`, `camera` and `grid` (and the timeline's view geometry) are free of egui widget types, so they're independently unit-testable. `cargo test --lib` runs the test suite.

## Running

From the repository root:

```sh
cargo run --release -p nabla
```

While working on the viewer, use `cargo nabla`. It builds with the `fast` profile, which keeps release's optimization level but skips its link-time optimization, so after an edit the viewer rebuilds in seconds instead of about a minute.

On macOS, `cargo nabla` (`cargo xtask run --profile fast`) runs the viewer inside an app bundle, `Nabla.app`, with the layered Icon Composer icon (`crates/nabla/macos/nabla_macOS.icon`) that macOS 26 renders with Liquid Glass; a bare executable, as from `cargo run`, can only show a flat rounded icon. `cargo xtask bundle` builds `target/release/Nabla.app` on its own. The Liquid Glass icon needs Xcode 26, for `actool`; without it the bundle gets a flat icon. See `cargo xtask help`.

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

## Development

See the repository root: `cargo xtask check` runs the same checks as CI, plus the web build, and `.githooks/pre-commit` runs the format and lint checks before each commit (enable it with `git config core.hooksPath .githooks`).

## Acknowledgments

Built on top of [emilk/eframe_template](https://github.com/emilk/eframe_template). The adaptive grid-spacing algorithm in `src/grid.rs` is adapted from [`egui_plot`](https://github.com/emilk/egui_plot) (MIT/Apache).
