# Nabla

A game on the 2D ship problem ([`nabla-core`](../nabla-core)), rendered with [egui](https://github.com/emilk/egui): fly the ship to rest at the goal `B` of a test case, yourself or with a control policy as **autopilot**. It is also the web app deployed to GitHub Pages: https://vini-fda.github.io/nabla/

The flight uses the same exact sampled-data model the solutions are scored on. At each sample `t_k = k h`, the controls held then become `u_k`, held for the step, and `s_(k+1) = step(s_k, u_k)`. Between samples the canvas shows the exact state at the present time, one partial `step` from `s_k`: computed, not interpolated.

## What's on screen

- **Left panel**: the case picker, its sampling period `h`, parameters and initial state, and how the autopilot's policy does on it; the **Autopilot** (its policy, its switch, and **Show code**, a window with the policy's source, syntax highlighted and with line numbers); the state and control at present; display options; and sound: the ion thruster's sound (five synthesized styles to choose from), master/ion/RCS volume knobs, and buttons to hear each thruster on its own.
- **Main canvas**: the ship, in 3D or as a flat triangle with velocity (green) and acceleration (red) arrows (**Debug** in Settings), in a Desmos-style coordinate grid you can pan (drag) and zoom (scroll, anchored at the cursor). The camera follows the ship from the start. It starts as a top-down orthographic view; the axes gizmo in its top right corner turns it to look from any side. Right of it, floating buttons show or hide the gizmo (sphere), follow the ship (rocket) and show or hide the path flown (a curve between two dots), turning light blue while on, switch dragging the canvas between panning and rotating the camera (arrows), and switch between orthographic and perspective projection (grid). Perspective uses the field of view of Blender's default 50 mm camera, and keeps the plane through the view's center at the same zoom as orthographic. It shows the path, the start, the goal `B` with its position tolerance, and a marker at the canvas's edge pointing to `B` while it is off screen.
- **Bottom bar**: pause and restart, the **SAS**, **Autopilot** and **Point** switches, and the time.
- **Right panel**: x, y, vx, vy, θ, ω and the controls F, τ over the flight so far, with a cursor at the present.

Once the ship lands, a banner gives the step count, next to the policy's on the same case. The plots, plumes and sound follow the flight.

## Flying

Thrust is all or nothing, and so is torque from the keyboard; the touch joystick gives as much torque as it is pushed. Fine control scales both to a tenth, since one step at full power can change the speed or the spin by more than the target set allows. **SAS** (`T`, as in Kerbal Space Program) switches what letting go of the turn does. Off, the default, nothing: the ship spins on, and stopping a spin is up to you. On, a stability assist like KSP's takes over the torque whenever you are not turning: it brakes the spin, with the ship's full torque whatever the fine control, and holds the heading the ship comes to. Turning again overrides it, and it holds the new heading once you let go. A faint line from the ship ending in a ring shows that heading all along: while you turn, it is where the ship would come to rest if you let go now. The choice is kept from flight to flight.

**Point** lets you fly with the mouse or a finger instead. Hold it on the canvas, and the ship turns toward it, the shorter way, and thrusts once it faces it within a cone, whose half-angle (10 degrees by default) is set next to the switch. A line from the ship to the pointer, between the cone's edges, turns green while it thrusts. Point works only while the camera follows the ship: then a still pointer holds a direction relative to the ship, while otherwise it would mark a fixed point the ship flies past. While it steers, the on-screen controls (joystick, thrust and fine) go, and a finger anywhere on the canvas points. Dragging the canvas points rather than turning the camera, and a touch on the buttons in the corner never points. The policy, and why it settles without ringing, is modeled in [`docs/main.typ`](../../docs/main.typ) ("Problem: pointing").

## Autopilot

**Autopilot** (`A`, the switch in the bottom bar or in Settings, or Square on a controller) hands the ship to a control policy: one of the solutions compiled into `nabla-core` (see [`TASK.md`](../../TASK.md)), picked in Settings, `mpc` by default. At every sample, the policy's controller reads the exact state and chooses the control, as when it is scored; your flight controls do nothing meanwhile. Switch it off to take over again, and on to hand the ship back, from wherever it is: the controller is made afresh each time, as if the case started there. In the target set it holds no control, and should the ship drift out it is made afresh again to bring it back. Engaged from the start of a flight (switch it on, then restart with `R`), it flies exactly the run the policy is scored on. Should the policy panic or choose an inadmissible control, it switches itself off and says why in Settings. The choice is kept from flight to flight.

The `mpc` policy plans its whole way to the goal when it takes over, which can take up to a second on the farthest cases: the flight waits for it.

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
| Path button | Show / hide the path, as the **Path** checkbox in Settings |
| Arrows button | Switch what dragging the canvas does: pan (four arrows, the default) or rotate the camera (circular arrow) |
| Grid button | Switch between orthographic (flat grid, the default) and perspective projection |
| Controller: left stick / Cross or R2 / L2 held | Turn / thrust / fine control |
| Controller: R1 / L1 held | Zoom in / out, about the canvas's center |
| Controller: right stick | Aim, as **Point** |
| Controller: Triangle / Square / Options / Create | SAS on / off, autopilot on / off, pause / resume, controller view on / off |

A **game controller** works natively and in the browser (read with [gilrs](https://crates.io/crates/gilrs); a browser only lists one once a button has been pressed with the page open). The right stick aims like **Point**: the ship turns toward where it points on screen and thrusts once it faces it within the cone. Being a direction rather than a spot, it needs neither Point nor the camera following the ship. Buttons are named as on a Sony controller; on an Xbox one, Cross is A, Square X, Triangle Y, L1 and R1 LB and RB, and L2 and R2 LT and RT. The **controller view** (off by default: **Controller view** in Settings, or the controller's Create button) draws a wireframe of the controller under the flight, on a pure black panel, lit as you use it: the sticks move in their wells, the face buttons glow in their colors, the triggers fill as far as they are pulled, and the other buttons light while held.

On phones and tablets (and once a touch screen is used), controls appear over the canvas. In the bottom left corner, a joystick turns the ship: push it sideways, and the further it goes the harder it turns (left counterclockwise, right clockwise), with a small dead zone in the middle. It follows the finger that took it until that finger lifts, even off its base. In the bottom right, a thrust pad is held while a finger is on it, and a **fine** latch above it turns on or off with a tap. So one thumb steers while the other thrusts. They go while the autopilot flies.

## Architecture

```
src/
├── main.rs       # eframe entry point (native + WASM)
├── lib.rs        # module wiring
├── app.rs        # egui UI: case & autopilot, the game's bar, canvas, plots, code window
├── game.rs      ← the flight: grows a sample at a time from the player's controls or the autopilot's policy; touch joystick and pads
├── replay.rs    ← a recorded run on a case: exact states between samples, plot series
├── camera.rs    ← orthographic orbit camera: world↔screen transform, pan, cursor-anchored zoom
├── gizmo.rs     ← Blender-style axes gizmo: orbit by dragging, snap to an axis by clicking
├── canvas_buttons.rs ← floating icon toggles over the canvas (gizmo, follow ship)
├── grid.rs      ← Desmos-style grid with 1/2/5×10ⁿ adaptive spacing
├── synth.rs     ← thruster sound synthesizer: ion thruster styles, RCS hiss
└── sound.rs     ← audio output (cpal: native or Web Audio), sound settings and volume knobs
```

`replay`, `camera` and `grid` are free of egui widget types, so they're independently unit-testable. `cargo test --lib` runs the test suite.

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
