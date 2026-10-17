# ship2d_graphics

A viewer for the control policies of [`ship2d_optimization`](..) (the crate at the root of this repository), rendered with [egui](https://github.com/emilk/egui). It is also the web app deployed to GitHub Pages: https://vini-fda.github.io/ship2d_optimization/

Open a solution's Rust source and pick a test case. The viewer compiles and runs the solution on that case with the exact sampled-data model of `ship2d_optimization`. It then replays the run on a pannable, zoomable canvas.

Playback is continuous in time. Between controller samples the control is held constant, so the state at any `t = k h + τ` is exactly one partial `step` of length `τ` from `s_k`. It is computed, not interpolated. The display sampling time `Δt = h/n` can be made much finer than `h`, for smooth playback on high-refresh displays.

## What's on screen

- **Left panel**: the solution source (open, reload, compile status and errors), the case picker, the case's parameters and initial state, the outcome (steps to reach the target set), the state and control at the current frame, display options, and sound: the ion thruster's sound (five synthesized styles to choose from), master/ion/RCS volume knobs, and buttons to hear each thruster on its own.
- **Main canvas**: the ship drawn as an isosceles triangle in a Desmos-style coordinate grid you can pan (drag) and zoom (scroll, anchored at the cursor). It starts as a top-down orthographic view; the axes gizmo in its top right corner turns it to look from any side. It shows the full path (grey), the part travelled so far (blue), the start, the goal `B` with its position tolerance, and the velocity (green) and acceleration (red) arrows.
- **Bottom panel**: playback controls, the playback speed (relative to real time), the display sampling time `Δt`, and an After Effects-style timeline. The timeline has a thin yellow mark at every controller sample `t_k = k h` and a playhead. Click or drag to seek, scroll to zoom, shift+scroll to pan, double-click to show the whole run.
- **Right panel**: x, y, vx, vy, θ, ω and the controls F, τ over the whole run, with a cursor at the current time. Click a plot to jump to that time.

| Input | Effect |
| --- | --- |
| `Space` | Play / pause |
| `←` / `→` | Back / forward by `Δt` |
| `Shift` + `←` / `→` | Previous / next controller sample |
| `Home` / `End` | Start / end of the run |
| `F` | Follow the ship (keep it centered) / stop following |
| Mouse drag | Pan the view |
| Scroll wheel | Zoom (cursor-anchored) |
| Drag the axes gizmo (top right) | Orbit the camera around the view's center |
| Click an axis ball | Look along that axis from its side (click again for the opposite side) |
| **Top-down view** button under the gizmo | Back to the default top-down orthographic view |

## Solutions are Rust source files

A solution is a `.rs` file defining `pub struct Solution` implementing `ship2d_optimization::Controller` (see [`TASK.md`](../TASK.md)). To open one:

- use **Open .rs…**,
- drop the file on the window,
- pass its path on the command line, or
- pick a stored solution from the database list.

**Show code** opens a window with the source, syntax highlighted and with line numbers.

To run a source on the selected case, the viewer compiles it with `ship2d_optimization`'s harness on a background thread, then asks the compiled binary for the trajectory of that case. Compile errors and crashes are shown in the left panel. With **Recompile when the file is saved**, editing the file in your editor recompiles it and updates the replay in place, keeping the playhead and the view.

Sources that are byte-identical to a stored solution skip compilation: the viewer runs the copy compiled into it (`ship2d_optimization`'s `registry` feature). That is the only way to run solutions in the web build, which cannot compile Rust.

## Architecture

```
src/
├── main.rs       # eframe entry point (native + WASM)
├── lib.rs        # module wiring
├── app.rs        # egui UI: source & case, playback, canvas, plots, code window
├── source.rs    ← solution source files; compiling and running them in the background
├── replay.rs    ← a solution run on a case, exact states between samples, playback on the Δt grid
├── timeline.rs  ← After Effects-style timeline: sample marks, playhead, zoom
├── camera.rs    ← orthographic orbit camera: world↔screen transform, pan, cursor-anchored zoom
├── gizmo.rs     ← Blender-style axes gizmo: orbit by dragging, snap to an axis by clicking
├── grid.rs      ← Desmos-style grid with 1/2/5×10ⁿ adaptive spacing
├── synth.rs     ← thruster sound synthesizer: ion thruster styles, RCS hiss
└── sound.rs     ← audio output (cpal: native or Web Audio), sound settings and volume knobs
```

`replay`, `camera` and `grid` (and the timeline's view geometry) are free of egui widget types, so they're independently unit-testable. `cargo test --lib` runs the test suite.

## Running

From the repository root:

```sh
cargo run --release -p ship2d_graphics                        # opens the best stored solution
cargo run --release -p ship2d_graphics -- path/to/solution.rs # opens this source
```

While working on the viewer, use `cargo viewer` (arguments go after `--` as above). It builds with the `fast` profile, which keeps release's optimization level but skips its link-time optimization, so after an edit the viewer rebuilds in seconds instead of about a minute.

On macOS, `cargo viewer` (`cargo xtask run --profile fast`) runs the viewer inside an app bundle, `Nabla.app`, with the layered Icon Composer icon (`viewer/macos/nabla_macOS.icon`) that macOS 26 renders with Liquid Glass; a bare executable, as from `cargo run`, can only show a flat rounded icon. `cargo xtask bundle` builds `target/release/Nabla.app` on its own. The Liquid Glass icon needs Xcode 26, for `actool`; without it the bundle gets a flat icon. See `cargo xtask help`.

```sh
cargo xtask bundle && open "target/release/Nabla.app"
```

On Linux you may need:

```sh
sudo apt-get install libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev libxkbcommon-dev libssl-dev libasound2-dev
```

For a web build with [Trunk](https://trunkrs.dev/), from this directory:

```sh
cargo install --locked trunk
trunk serve   # then open http://127.0.0.1:8080/index.html#dev
```

The `#dev` suffix skips the service worker, which otherwise caches the app for offline use.

Every push to `main` deploys the web build to GitHub Pages (`.github/workflows/pages.yml` at the repository root).

## Development

See the repository root: `cargo xtask check` runs the same checks as CI, plus the web build, and `.githooks/pre-commit` runs the format and lint checks before each commit (enable it with `git config core.hooksPath .githooks`).

## Acknowledgments

Built on top of [emilk/eframe_template](https://github.com/emilk/eframe_template). The adaptive grid-spacing algorithm in `src/grid.rs` is adapted from [`egui_plot`](https://github.com/emilk/egui_plot) (MIT/Apache).
