//! The game: the player flies the ship to the goal, and time runs freely. An [`Autopilot`] can
//! fly it instead, with one of the control policies.
//!
//! Vacuum flights run on the same exact sampled-data model a solution is scored on.
//! Worlds with celestial bodies use Rapier substeps with collision-aware replay samples. At each sample
//! `t_k = k h` the controls the player holds become `u_k`, held for the whole step, and
//! `s_(k+1) = step(s_k, u_k)`. So a flight is a [`Replay`] recorded as it goes, of which only the
//! last minute is kept, and the canvas, plumes and sound show it as they show a solution's run. The
//! plots show the same minute, from [`Plots`] recorded along with it.
//!
//! Thrust is all or nothing, as a key or a button is. Torque is all or nothing from the keyboard
//! too, and as strong as the joystick is pushed on a touch screen. Fine control scales both down
//! to [`FINE`] of their maxima, since one step at full power can change the speed or the spin by
//! more than the target set allows.
//!
//! [`Steering`] picks what letting go of the turn does. With [`Steering::Raw`], nothing: the
//! ship spins on. With [`Steering::Sas`], a stability assist like Kerbal Space Program's SAS
//! brakes the spin and holds the heading the ship comes to, until the player turns again.
//!
//! The player can also point instead, with the mouse or a finger, while the camera follows the
//! ship: an [`Aim`] turns the ship toward the pointer and thrusts once it faces it, within a
//! cone. Both the assist and the aim use the attitude law of `docs/main.typ` ("Problem:
//! pointing").
//!
//! Engaged, the [`Autopilot`] takes over from the player: at every sample, the policy's
//! controller reads the exact state and chooses the control, as when a solution is scored. It is
//! made afresh when engaged, from wherever the ship is then, and again whenever the ship leaves
//! the target set, in which it holds no control.

use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};

use egui::{Color32, Pos2, Rect, Sense, Vec2, pos2, vec2};
use log::Level;
use nabla_core::problem::Case;
use nabla_core::solutions::StoredSolution;
use nabla_core::{Control, Controller, State, Task, step};
use web_time::Instant;

use crate::orbit::{self, Pilot, Prediction, Target};
use crate::physics::Physics;
use crate::plots::{Plots, window_steps};
use crate::replay::Replay;
use crate::theme::{icons, palette};
use crate::world::World;
use nabla_sim::span;

pub use nabla_sim::steering::{Aim, FINE, Input, Steering, wrap};
use nabla_sim::steering::{steer, stopping_heading};

/// Longest frame time simulated at once: after a longer frame (a stall, a hidden tab), the
/// flight goes on from where it was rather than jumping ahead.
const MAX_FRAME_TIME: f64 = 0.1;

/// Least half width and height of the view [`Game::view_bounds`] frames around the ship.
const MIN_VIEW_HALF_SIZE: f64 = 3.0;

/// A control policy flying the ship, in place of the player.
pub(crate) struct Autopilot {
    solution: &'static StoredSolution,
    /// The policy's controller, from the sample it was made at on: `None` until the next sample
    /// it flies outside the target set.
    pub(crate) controller: Option<Box<dyn Controller>>,
    /// The samples the controller has flown: the `k` it is queried at next.
    k: usize,
}

impl Autopilot {
    pub(crate) fn new(solution: &'static StoredSolution) -> Self {
        Self {
            solution,
            controller: None,
            k: 0,
        }
    }

    /// The policy's control in state `s`, of the ship with `task`: none in the target set, where
    /// the controller is dropped, to be made afresh once the ship drifts out of it. An error
    /// says why the policy can't fly on: it panicked, or chose an inadmissible control.
    pub(crate) fn control(&mut self, task: &Task, s: &State) -> Result<Control, String> {
        if task.reached(s) {
            self.controller = None;
            return Ok(Control::default());
        }
        let solution = self.solution;
        let controller = if let Some(controller) = self.controller.take() {
            controller
        } else {
            self.k = 0;
            let _span = span!(Level::Debug, "{}: new controller", solution.name);
            catch_unwind(|| (solution.controller)(task))
                .map_err(|_panic| format!("{} panicked starting up", solution.name))?
        };
        let controller = self.controller.insert(controller);
        let k = self.k;
        let started = Instant::now();
        let u = {
            let _span = span!(Level::Trace, "{}: control at step {k}", solution.name);
            catch_unwind(AssertUnwindSafe(|| controller.control(k, s)))
        };
        let took = started.elapsed().as_secs_f64();
        if took > task.params.h {
            log::debug!(
                "{}: control at step {k} took {:.1} ms, over the {:.0} ms sampling period",
                solution.name,
                took * 1e3,
                task.params.h * 1e3
            );
        }
        let u = u.map_err(|_panic| format!("{} panicked at its step {k}", solution.name))?;
        if !u.is_admissible(&task.params) {
            return Err(format!(
                "{} chose an inadmissible control at its step {k}: \
                 F = {}, tau = {}",
                solution.name, u.thrust, u.torque
            ));
        }
        self.k += 1;
        Ok(u)
    }
}

/// The one objective of a flight. Manual control does not change what the ship is aiming for.
/// Point B and its tolerances come from the flight's case; an orbit carries its own target.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Objective {
    #[default]
    ReachB,
    Orbit(Target),
}

impl Objective {
    pub fn label(self) -> &'static str {
        match self {
            Self::ReachB => "Reach B",
            Self::Orbit(_) => "Orbit",
        }
    }
}

/// Only one controller can own the flight's controls at a time.
pub(crate) enum FlightController {
    ReachB(Autopilot),
    Orbit(Pilot),
}

/// The control `controller` chooses in state `s`, of the ship with `task` in `world`.
///
/// # Errors
/// Why it can't fly on: a policy panicked or chose an inadmissible control, or the orbit assist
/// lost its clearance.
pub(crate) fn run_controller(
    controller: &mut FlightController,
    world: &World,
    task: &Task,
    s: &State,
) -> Result<Control, String> {
    match controller {
        FlightController::ReachB(pilot) => pilot.control(task, s),
        FlightController::Orbit(pilot) => {
            let u = pilot.control(world, s, &task.params);
            if u.is_admissible(&task.params) && orbit::clear(world, s) {
                Ok(u)
            } else {
                Err("Orbit assist lost clearance or control authority".to_owned())
            }
        }
    }
}

/// A short forecast of the ship's path from `start`, the next control sample, under `controller`
/// if one is engaged, without advancing it. `h` is the sampling period.
pub(crate) fn forecast_of(
    world: &World,
    start: State,
    controller: Option<&FlightController>,
    h: f64,
) -> Vec<[f64; 2]> {
    match controller {
        Some(FlightController::Orbit(pilot)) => orbit::forecast(world, start, pilot.clone()),
        Some(FlightController::ReachB(pilot)) => {
            let steps = orbit::forecast_steps(h);
            let states = pilot
                .controller
                .as_ref()
                .map_or_else(Vec::new, |controller| {
                    catch_unwind(AssertUnwindSafe(|| controller.prediction(&start, steps)))
                        .unwrap_or_default()
                });
            states
                .into_iter()
                .take(steps + 1)
                .take_while(|s| s.x.is_finite() && s.y.is_finite())
                .map(|s| [s.x, s.y])
                .collect()
        }
        None => Vec::new(),
    }
}

/// A flight on one case.
pub struct Game {
    pub world: World,
    physics: Option<Physics>,
    /// The last minute of the flight: the samples up to the present, and the state at the end of
    /// the step under way. A flight has no step budget, and [`Self::landed`] says when it reached
    /// the target set. Allocated here, for the flight's `h`.
    pub replay: Replay,
    /// The same minute, for the state plots: also allocated here.
    pub plots: Plots,
    /// Time since the start, in simulated seconds.
    time: f64,
    pub paused: bool,
    /// The first sample in the target set.
    landed: Option<usize>,
    /// The heading the stability assist holds, while it does.
    hold: Option<f64>,
    objective: Objective,
    autopilot: Option<FlightController>,
    /// Some sample before the landing was flown by the player rather than the autopilot.
    manual: bool,
    /// Why the autopilot stopped flying, if it failed: to show the player, once.
    fault: Option<String>,
    pub pads: Pads,
}

impl Game {
    pub fn new(case: Case) -> Self {
        let world = World::vacuum(&case, 1.0);
        Self::in_world(case, world)
    }

    pub fn in_world(case: Case, world: World) -> Self {
        let steps = window_steps(case.task.params.h);
        let mut plots = Plots::new(steps);
        plots.record_state(0.0, &case.start);
        Self {
            physics: (!world.bodies.is_empty()).then(|| Physics::new(&world)),
            world,
            replay: Replay::recording(case, steps),
            plots,
            time: 0.0,
            paused: false,
            landed: None,
            hold: None,
            autopilot: None,
            objective: Objective::ReachB,
            manual: false,
            fault: None,
            pads: Pads::default(),
        }
    }

    pub fn time(&self) -> f64 {
        self.time
    }

    pub fn save(&self) -> World {
        let mut world = self.world.clone();
        world.players[0].state = self.state();
        world
    }

    pub fn objective(&self) -> Objective {
        self.objective
    }

    pub fn assisted(&self) -> bool {
        self.autopilot.is_some()
    }

    pub fn orbit_pilot(&self) -> Option<&Pilot> {
        match &self.autopilot {
            Some(FlightController::Orbit(pilot)) => Some(pilot),
            _ => None,
        }
    }

    /// Selecting a different objective returns to manual control. Feasibility is checked
    /// separately on engagement, so an obstructed orbit can still be selected and edited.
    pub fn select_objective(&mut self, objective: Objective) -> Result<(), String> {
        if let Objective::Orbit(target) = objective {
            target.validate_input(&self.world)?;
        }
        if self.objective != objective {
            self.disengage();
            log::info!("Objective: {}", objective.label());
            self.objective = objective;
            self.landed = None;
            self.manual |= self.time > 0.0;
        }
        Ok(())
    }

    /// Validate the entire request before replacing the current objective or controller.
    pub fn engage(
        &mut self,
        objective: Objective,
        solution: &'static StoredSolution,
    ) -> Result<Option<Prediction>, String> {
        let (controller, prediction) = match objective {
            Objective::ReachB => (FlightController::ReachB(Autopilot::new(solution)), None),
            Objective::Orbit(target) => {
                let prediction = orbit::predict(&self.world, self.next_state(), target)?;
                (
                    FlightController::Orbit(Pilot::new(target)),
                    Some(prediction),
                )
            }
        };
        self.select_objective(objective)?;
        log::info!(
            "Autopilot engaged: {}, policy {}",
            objective.label(),
            solution.name
        );
        self.autopilot = Some(controller);
        self.fault = None;
        self.hold = None;
        if matches!(objective, Objective::Orbit(_)) {
            self.manual = true; // Orbit captures are not vacuum policy comparison runs.
        }
        Ok(prediction)
    }

    pub fn disengage(&mut self) {
        if self.autopilot.take().is_some() {
            log::info!("Autopilot off");
        }
        self.hold = None;
    }

    pub fn next_state(&self) -> State {
        *self.replay.state(self.replay.last_frame())
    }

    /// A short forecast from the next control sample, without advancing the live controller.
    pub fn prediction(&self) -> Vec<[f64; 2]> {
        forecast_of(
            &self.world,
            self.next_state(),
            self.autopilot.as_ref(),
            self.replay.h(),
        )
    }

    /// The present as `(k, tau)`, with `t = k h + tau` and `0 <= tau < h`.
    pub fn moment(&self) -> (usize, f64) {
        let r = &self.replay;
        let k = ((self.time / r.h()).floor() as usize).min(r.last_frame());
        (k, self.time - r.time(k))
    }

    /// The exact state at present.
    pub fn state(&self) -> State {
        let (k, tau) = self.moment();
        self.replay.state_at(k, tau)
    }

    /// The number of steps the flight took to first reach the target set, once it has.
    pub fn landed(&self) -> Option<usize> {
        self.landed
    }

    /// Whether the autopilot flew every sample up to the landing (or up to now, before it).
    pub fn autopiloted(&self) -> bool {
        !self.manual
    }

    /// Why the autopilot last stopped flying on its own, if it failed since the last call.
    pub fn take_fault(&mut self) -> Option<String> {
        self.fault.take()
    }

    /// With the stability assist, the heading it holds, or, while the player turns, the one it
    /// would hold if they let go now: where braking would stop the spin. `None` without it.
    pub fn sas_heading(&self, steering: Steering) -> Option<f64> {
        if steering == Steering::Raw {
            return None;
        }
        let params = &self.replay.case.task.params;
        Some(
            self.hold
                .unwrap_or_else(|| stopping_heading(params, &self.state())),
        )
    }

    /// Run the flight `dt` real seconds on, unless paused, with `input` held at the samples
    /// this passes, steering as `steering` says. With an engaged controller, it flies instead,
    /// and the input does nothing; should the policy fail, the player takes over, and
    /// [`Self::take_fault`] says why.
    pub fn advance(&mut self, dt: f64, input: Input, steering: Steering) {
        if self.paused {
            return;
        }
        let _span = span!(Level::Trace, "advance, t = {:.3} s + {dt:.3} s", self.time);
        self.time += dt.clamp(0.0, MAX_FRAME_TIME);
        let h = self.replay.h();
        let task = self.replay.case.task;
        // Every sample up to the present gets its control, so the step under way is known.
        while self.replay.last_frame() as f64 * h <= self.time {
            let k = self.replay.last_frame();
            let s = self.next_state();
            if self.objective == Objective::ReachB && self.landed.is_none() && task.reached(&s) {
                self.landed = Some(k);
                log::info!("Landed at step {k}");
            }
            let piloted = self.autopilot_control(&task, &s);
            if piloted.is_none() && self.landed.is_none() {
                self.manual = true;
            }
            let u =
                piloted.unwrap_or_else(|| steer(input, steering, &mut self.hold, &task.params, &s));
            let (next, samples) = if let Some(physics) = &mut self.physics {
                let mut samples = physics.step(h, u);
                samples[0] = s;
                (*samples.last().expect("physics samples"), Some(samples))
            } else {
                (step(&task.params, &s, &u), None)
            };
            self.replay.record(u, next, samples);
            self.plots
                .record_step(self.replay.time(k), self.replay.time(k + 1), &u, &next);
        }
    }

    /// The control the autopilot flying `solution`, if any, chooses in state `s`. A new one
    /// starts afresh, and one that fails is dropped, with the fault kept.
    fn autopilot_control(&mut self, task: &Task, s: &State) -> Option<Control> {
        let result = run_controller(self.autopilot.as_mut()?, &self.world, task, s);
        match result {
            Ok(u) => {
                // The assist holds the heading the autopilot leaves, not one from before it.
                self.hold = None;
                Some(u)
            }
            Err(fault) => {
                log::warn!("Autopilot off: {fault}");
                self.autopilot = None;
                self.fault = Some(fault);
                None
            }
        }
    }

    /// A view centered on the ship that includes the goal and celestial bodies.
    pub fn view_bounds(&self) -> ([f64; 2], [f64; 2]) {
        let s = self.state();
        let (mut dx, mut dy) = match self.objective {
            Objective::ReachB => {
                let goal = self.replay.case.task.goal;
                ((goal.x - s.x).abs(), (goal.y - s.y).abs())
            }
            Objective::Orbit(target) => {
                let body = &self.world.bodies[target.body];
                (
                    (f64::from(body.position[0]) - s.x).abs() + target.radius(&self.world),
                    (f64::from(body.position[1]) - s.y).abs() + target.radius(&self.world),
                )
            }
        };
        dx = dx.max(MIN_VIEW_HALF_SIZE);
        dy = dy.max(MIN_VIEW_HALF_SIZE);
        for body in &self.world.bodies {
            dx = dx.max((f64::from(body.position[0]) - s.x).abs() + f64::from(body.radius));
            dy = dy.max((f64::from(body.position[1]) - s.y).abs() + f64::from(body.radius));
        }
        ([s.x - dx, s.y - dy], [s.x + dx, s.y + dy])
    }
}

/// Radius of the joystick's base, and of its knob, which travels sideways within the base.
const STICK_RADIUS: f32 = 52.0;
const KNOB_RADIUS: f32 = 24.0;
/// Fraction of the joystick's travel around its center that turns not at all, so a thumb
/// resting on it does not spin the ship.
const DEAD_ZONE: f32 = 0.08;
/// Radius of the thrust pad, and of the fine control latch.
const THRUST_RADIUS: f32 = 44.0;
const FINE_RADIUS: f32 = 22.0;
/// Space between the controls and the canvas's edges, and between neighboring controls.
const PAD_MARGIN: f32 = 24.0;
const PAD_GAP: f32 = 16.0;
/// How far past its edge a touch still lands on a control, as a fraction of its radius: thumbs
/// are not precise.
const PAD_REACH: f32 = 1.25;
const PAD_ICON_SIZE: f32 = 28.0;
const STICK_ICON_SIZE: f32 = 16.0;
/// Height of the strip along the canvas's bottom the controls take.
pub const PADS_HEIGHT: f32 = PAD_MARGIN + 2.0 * STICK_RADIUS;
/// A control's color, a translucent grey as the canvas buttons'.
const PAD_COLOR: Color32 = Color32::from_rgba_unmultiplied_const(46, 46, 46, 170);

/// An on-screen control.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pad {
    Stick,
    Thrust,
    Fine,
}

impl Pad {
    const ALL: [Self; 3] = [Self::Stick, Self::Thrust, Self::Fine];

    /// The control's center and radius on `canvas`: the joystick in its bottom left corner,
    /// under the left thumb, thrust in the bottom right, level with it, and the fine control
    /// latch above thrust.
    fn circle(self, canvas: Rect) -> (Pos2, f32) {
        let thrust = pos2(
            canvas.right() - PAD_MARGIN - THRUST_RADIUS,
            canvas.bottom() - PAD_MARGIN - STICK_RADIUS,
        );
        match self {
            Self::Stick => (
                canvas.left_bottom() + vec2(1.0, -1.0) * (PAD_MARGIN + STICK_RADIUS),
                STICK_RADIUS,
            ),
            Self::Thrust => (thrust, THRUST_RADIUS),
            Self::Fine => (
                thrust - vec2(0.0, THRUST_RADIUS + PAD_GAP + FINE_RADIUS),
                FINE_RADIUS,
            ),
        }
    }

    fn contains(self, canvas: Rect, p: Pos2) -> bool {
        let (center, radius) = self.circle(canvas);
        center.distance(p) <= PAD_REACH * radius
    }
}

/// How hard the joystick on `canvas` turns with its knob held at `p`: from -1 (all the way
/// right) to 1 (all the way left). Only the knob's sideways travel counts, past the dead zone.
fn stick_turn(canvas: Rect, p: Pos2) -> f32 {
    let (center, radius) = Pad::Stick.circle(canvas);
    let x = ((p.x - center.x) / radius).clamp(-1.0, 1.0);
    let past = ((x.abs() - DEAD_ZONE) / (1.0 - DEAD_ZONE)).max(0.0);
    -past.copysign(x)
}

/// On-screen controls for touch screens: a joystick in the canvas's bottom left corner that
/// turns the ship, as hard as it is pushed sideways, thrust in its bottom right, and a latch for
/// fine control. The joystick follows the finger that took it until that finger lifts, even off
/// its base; thrust is held while any finger is on it. So one thumb can turn while the other
/// thrusts. While the player points, the controls are hidden, and the whole canvas points.
#[derive(Default)]
pub struct Pads {
    /// Where each touch on the screen is, by device and touch id.
    touches: HashMap<(u64, u64), Pos2>,
    /// The touch that holds the joystick.
    stick: Option<(u64, u64)>,
    /// The touch that points, for an [`Aim`]: the first to land on the canvas off the controls
    /// and off the areas the caller keeps for other widgets.
    aim: Option<(u64, u64)>,
    /// Where the joystick is held, while it is.
    knob: Option<Pos2>,
    /// A touch screen has been used, so the controls show even where the system is not a
    /// phone's or a tablet's.
    touched: bool,
    /// Fine control is latched on.
    fine: bool,
    /// What the controls held at the last [`Self::update`].
    held: Input,
    /// A touch was on a control at the last [`Self::update`].
    on_pad: bool,
    /// The controls are hidden, at the last [`Self::update`].
    hidden: bool,
}

impl Pads {
    /// Whether the controls show: on phones and tablets, and once a touch screen has been used.
    pub fn visible(&self, touch_device: bool) -> bool {
        touch_device || self.touched
    }

    /// The controls held at the last [`Self::update`].
    pub fn held(&self) -> Input {
        self.held
    }

    /// Whether a touch screen has been used: then the touches point, rather than the mouse.
    pub fn touched(&self) -> bool {
        self.touched
    }

    /// Where the touch that points is, while one does.
    pub fn aim_touch(&self) -> Option<Pos2> {
        let key = self.aim?;
        self.touches.get(&key).copied()
    }

    /// Whether a touch is on a control, so a second finger is not taken for a pinch.
    pub fn touching(&self) -> bool {
        self.on_pad
    }

    /// The controls that show: all of them, or none while they are hidden.
    fn shown(&self) -> impl Iterator<Item = Pad> + use<> {
        let hidden = self.hidden;
        Pad::ALL.into_iter().filter(move |_| !hidden)
    }

    /// Reads this frame's touches, and the mouse, on the controls on `canvas`, while they show
    /// (if `visible`). A touch that lands in one of the `reserved` areas, which other widgets
    /// take, never points. Call it after the canvas has been allocated, so the controls, and not
    /// the canvas, take the pointer's drags.
    pub fn update(&mut self, ui: &egui::Ui, canvas: Rect, visible: bool, reserved: &[Rect]) {
        self.hidden = !visible;
        if self.hidden {
            self.stick = None;
        }
        let taps = self.read_touches(ui, canvas, reserved);
        self.held = Input::default();
        self.knob = None;
        self.on_pad = false;
        if !visible {
            return;
        }

        let mut clicked_fine = false;
        let mut dragged_stick = None;
        for pad in self.shown() {
            let (center, radius) = pad.circle(canvas);
            let rect = Rect::from_center_size(center, Vec2::splat(2.0 * PAD_REACH * radius));
            let id = ui.scope_id().with(("game_pad", pad as u8));
            let response = ui.interact(rect, id, Sense::click_and_drag());
            clicked_fine |= pad == Pad::Fine && response.clicked();
            if pad == Pad::Stick && response.is_pointer_button_down_on() {
                dragged_stick = response.interact_pointer_pos();
            }
        }
        // A touch also moves the pointer, so on a touch screen the latch and the joystick go by
        // the touches alone: a tap would flip the latch twice, and a second finger on the
        // joystick could take the pointer from the first.
        let tapped_fine = if self.touched {
            taps.iter().any(|&p| Pad::Fine.contains(canvas, p))
        } else {
            clicked_fine
        };
        if tapped_fine {
            self.fine = !self.fine;
        }
        self.knob = if self.touched {
            self.stick.and_then(|key| self.touches.get(&key).copied())
        } else {
            dragged_stick
        };

        let pointer = ui.input(|i| {
            i.pointer
                .primary_down()
                .then(|| i.pointer.interact_pos())
                .flatten()
        });
        self.held = Input {
            thrust: f64::from(u8::from(
                self.touches
                    .values()
                    .copied()
                    .chain(pointer)
                    .any(|p| Pad::Thrust.contains(canvas, p)),
            )),
            turn: self.knob.map_or(0.0, |p| f64::from(stick_turn(canvas, p))),
            fine: self.fine,
            aim: None,
        };
        self.on_pad = self.stick.is_some()
            || self
                .touches
                .values()
                .any(|&p| self.shown().any(|pad| pad.contains(canvas, p)));
    }

    /// Follows this frame's touch events: where each touch is, which one holds the joystick,
    /// the first to land on it, and which one points, the first to land off the controls and
    /// the `reserved` areas. Returns where touches started.
    fn read_touches(&mut self, ui: &egui::Ui, canvas: Rect, reserved: &[Rect]) -> Vec<Pos2> {
        let mut taps = Vec::new();
        ui.input(|i| {
            for event in &i.events {
                let egui::Event::Touch {
                    device_id,
                    id,
                    phase,
                    pos,
                    ..
                } = *event
                else {
                    continue;
                };
                self.touched = true;
                let key = (device_id.0, id.0);
                match phase {
                    egui::TouchPhase::Start => {
                        self.touches.insert(key, pos);
                        taps.push(pos);
                        let on_pad = self.shown().any(|pad| pad.contains(canvas, pos));
                        if self.stick.is_none() && !self.hidden && Pad::Stick.contains(canvas, pos)
                        {
                            self.stick = Some(key);
                        } else if self.aim.is_none()
                            && !on_pad
                            && canvas.contains(pos)
                            && !reserved.iter().any(|r| r.contains(pos))
                        {
                            self.aim = Some(key);
                        }
                    }
                    egui::TouchPhase::Move => {
                        self.touches.insert(key, pos);
                    }
                    egui::TouchPhase::End | egui::TouchPhase::Cancel => {
                        self.touches.remove(&key);
                        if self.stick == Some(key) {
                            self.stick = None;
                        }
                        if self.aim == Some(key) {
                            self.aim = None;
                        }
                    }
                }
            }
        });
        taps
    }

    /// Paints the controls on `canvas`, lit while held.
    pub fn paint(&self, painter: &egui::Painter, canvas: Rect) {
        self.paint_stick(painter, canvas);
        for (pad, held, color) in [
            (Pad::Thrust, self.held.thrust > 0.0, palette::PEACH),
            (Pad::Fine, self.fine, palette::GREEN),
        ] {
            let (center, radius) = pad.circle(canvas);
            paint_base(painter, center, radius, held.then_some(color));
            let (glyph, font) = match pad {
                Pad::Fine => ("fine", egui::FontId::proportional(13.0)),
                _ => (icons::outline::FIRE, icons::outline::font(PAD_ICON_SIZE)),
            };
            let glyph_color = if held { color } else { palette::WHITE };
            painter.text(
                center,
                egui::Align2::CENTER_CENTER,
                glyph,
                font,
                glyph_color,
            );
        }
    }

    /// The joystick: its base, with the turn it makes on either side, and the knob, where the
    /// finger holds it (sideways only, and within the base) or else at the center.
    fn paint_stick(&self, painter: &egui::Painter, canvas: Rect) {
        use icons::outline::{ARROW_CLOCKWISE, ARROW_COUNTER_CLOCKWISE};
        const COLOR: Color32 = palette::SAPPHIRE;
        let (center, radius) = Pad::Stick.circle(canvas);
        let turn = self.knob.map_or(0.0, |p| stick_turn(canvas, p));
        paint_base(painter, center, radius, (turn != 0.0).then_some(COLOR));
        let travel = radius - KNOB_RADIUS;
        painter.line_segment(
            [center - vec2(travel, 0.0), center + vec2(travel, 0.0)],
            egui::Stroke::new(2.0, palette::OVERLAY0),
        );
        for (side, glyph, on) in [
            (-1.0, ARROW_COUNTER_CLOCKWISE, turn > 0.0),
            (1.0, ARROW_CLOCKWISE, turn < 0.0),
        ] {
            painter.text(
                // Above the knob's track, inside the base.
                center + vec2(side * 0.45 * radius, -0.68 * radius),
                egui::Align2::CENTER_CENTER,
                glyph,
                icons::outline::font(STICK_ICON_SIZE),
                if on { COLOR } else { palette::OVERLAY1 },
            );
        }
        let offset = self
            .knob
            .map_or(0.0, |p| (p.x - center.x).clamp(-travel, travel));
        let knob = center + vec2(offset, 0.0);
        let (fill, stroke) = if self.knob.is_some() {
            (COLOR.gamma_multiply(0.6), COLOR)
        } else {
            (palette::SURFACE2, palette::OVERLAY1)
        };
        painter.circle_filled(knob, KNOB_RADIUS, fill);
        painter.circle_stroke(knob, KNOB_RADIUS, egui::Stroke::new(1.5, stroke));
    }
}

/// A control's round base at `center`, tinted and outlined in `lit`, if given.
fn paint_base(painter: &egui::Painter, center: Pos2, radius: f32, lit: Option<Color32>) {
    painter.circle_filled(center, radius, PAD_COLOR);
    if let Some(color) = lit {
        painter.circle_filled(center, radius, color.gamma_multiply(0.3));
    }
    let stroke = lit.unwrap_or(palette::OVERLAY0);
    painter.circle_stroke(center, radius, egui::Stroke::new(1.5, stroke));
}

#[cfg(test)]
mod tests {
    use super::*;
    use nabla_core::suite;
    use std::f64::consts::PI;

    fn game() -> Game {
        Game::new(suite::find("agile").expect("case exists"))
    }

    #[test]
    fn a_flight_and_its_plots_hold_only_the_last_minute_and_fly_on_exactly() {
        let h = 0.1;
        let mut case = suite::find("agile").expect("case exists");
        case.task.params.h = h;
        let params = case.task.params;
        let mut exact = case.start;
        let mut g = Game::new(case);
        let allocations = g.plots.allocations();
        let capacities = |g: &Game| {
            (
                g.replay.states().capacity(),
                g.replay.controls().capacity(),
                g.replay.collision_steps().capacity(),
            )
        };
        let capacity = capacities(&g);
        assert_eq!(g.plots.x.points().len(), 1, "from the start");
        let thrust = Input {
            thrust: 1.0,
            turn: 0.5,
            ..Input::default()
        };
        let u = thrust.control(&params);

        let mut flown = 0.0;
        let mut flown_steps = 0;
        while flown < 100.0 {
            g.advance(h, thrust, Steering::Raw);
            flown += h;
            assert_eq!(g.plots.allocations(), allocations, "at {flown} s");
            assert_eq!(capacities(&g), capacity, "at {flown} s");
            assert!(g.replay.controls().len() <= 600, "at {flown} s");
            while flown_steps < g.replay.last_frame() {
                exact = step(&params, &exact, &u);
                flown_steps += 1;
            }
            // The newest sample is the flight's own, whatever has been discarded.
            assert_eq!(g.next_state(), exact, "at {flown} s");
            let newest = g.plots.x.points().last().expect("a sample").x;
            let oldest = g.plots.x.points()[0].x;
            assert!((newest - g.replay.duration()).abs() < 1e-9);
            assert!(newest - oldest < 60.0 + 1e-9, "at {flown} s");
            assert!(oldest <= (newest - 60.0).max(0.0) + 1e-9, "at {flown} s");
        }

        // Frames still count from the start, and the present is among those held.
        let last = g.replay.last_frame();
        assert!(last >= 1000, "{last} samples");
        let first = last - 600;
        assert_eq!(g.replay.first_frame(), first);
        assert_eq!(
            (g.replay.states().len(), g.replay.controls().len()),
            (601, 600)
        );
        let (k, tau) = g.moment();
        assert!(k >= first && tau < h, "at sample {k} + {tau}");
        assert!((g.replay.time(k) + tau - g.time()).abs() < 1e-9);

        // The plots hold the very samples the replay does.
        assert_eq!(g.plots.vx.points().len(), 601);
        for (i, (x, omega)) in g
            .plots
            .x
            .points()
            .iter()
            .zip(g.plots.omega.points())
            .enumerate()
        {
            let s = g.replay.state(first + i);
            assert_eq!([x.x, x.y], [g.replay.time(first + i), s.x]);
            assert_eq!([omega.x, omega.y], [g.replay.time(first + i), s.omega]);
        }
        assert_eq!(g.plots.thrust.points().len(), 1200);
        for (pair, u) in g.plots.thrust.points().chunks(2).zip(g.replay.controls()) {
            assert_eq!([pair[0].y, pair[1].y], [u.thrust, u.thrust]);
        }
        assert!(g.plots.thrust.points().iter().any(|p| p.y > 0.0), "thrusts");
        let torque = g.plots.torque.points();
        assert!(torque.iter().any(|p| p.y != 0.0), "turns");

        // The sound reads the present, and the plume half a second back: both still held.
        let t = g.time();
        let full = u.thrust / params.f_max;
        assert!((g.replay.thrust_fraction(t) - full).abs() < 1e-9);
        assert!((g.replay.thrust_fraction(t - 0.5) - full).abs() < 1e-9);
        assert_eq!(g.replay.thrust_fraction(1.0), 0.0, "long discarded");
    }

    #[test]
    fn a_flight_with_collisions_holds_only_the_last_minute_of_substeps() {
        let mut world = crate::world::Preset::PlanetAndMoon.world();
        world.players[0].task.params.h = 0.1;
        let mut g = Game::in_world(world.case(), world);
        let capacity = g.replay.collision_steps().capacity();
        while g.time() < 65.0 {
            g.advance(0.1, Input::default(), Steering::Raw);
            assert_eq!(
                g.replay.collision_steps().len(),
                g.replay.controls().len(),
                "a step's substeps go with it"
            );
        }
        assert_eq!(g.replay.collision_steps().len(), 600);
        assert_eq!(g.replay.collision_steps().capacity(), capacity);
        let r = &g.replay;
        for k in [r.first_frame(), r.last_frame() - 1] {
            assert_eq!(r.state_at(k, 0.0), *r.state(k), "substeps of frame {k}");
        }
        let (k, tau) = g.moment();
        assert_eq!(g.state(), r.state_at(k, tau));
    }

    #[test]
    fn the_plots_follow_a_flight_with_collisions_to_its_last_sample() {
        let world = crate::world::Preset::PlanetAndMoon.world();
        let mut g = Game::in_world(world.case(), world);
        for _ in 0..100 {
            g.advance(0.05, Input::default(), Steering::Raw);
        }
        let newest = g.plots.y.points().last().expect("a sample");
        assert_eq!(newest.y, g.next_state().y);
        assert_eq!(g.plots.y.points().len(), g.replay.last_frame() + 1);
    }

    #[test]
    fn saves_current_state_and_reloads_collision_world() {
        let mut world = crate::world::Preset::PlanetAndMoon.world();
        world.gravity_constant = 2.5;
        world.players[0].state = State {
            x: -20.0,
            y: -12.0,
            vx: 100.0,
            ..State::default()
        };
        let mut game = Game::in_world(world.case(), world.clone());
        let (lo, hi) = game.view_bounds();
        assert!(
            lo[1] <= -19.0 && hi[0] >= 14.5,
            "initial view includes planet and moon"
        );
        for _ in 0..30 {
            game.advance(0.017, Input::default(), Steering::Raw);
        }
        assert!(
            game.state().x <= -7.95,
            "visible ship does not pass into the planet"
        );
        let saved = World::from_toml(&game.save().to_toml().expect("save")).expect("load");
        assert_eq!(
            saved.players[0].state,
            game.state(),
            "save the present, not the predicted next sample"
        );
        assert_eq!(
            saved.bodies, world.bodies,
            "static bodies survive flight and serialization"
        );
        assert_eq!(
            saved.gravity_constant, world.gravity_constant,
            "save custom gravity"
        );
        let mut resumed = Game::in_world(saved.case(), saved.clone());
        assert_eq!(
            resumed.state(),
            saved.players[0].state,
            "load restores the visible state"
        );
        resumed.paused = true;
        resumed.advance(0.1, Input::default(), Steering::Raw);
        assert_eq!(resumed.save(), saved, "pause leaves save state unchanged");
        resumed.paused = false;
        resumed.advance(
            0.1,
            Input {
                thrust: 1.0,
                ..Input::default()
            },
            Steering::Raw,
        );
        assert!(
            resumed.state().x <= -7.95,
            "rebuilt colliders stop thrust into the planet"
        );
        let restarted = Game::in_world(world.case(), world.clone());
        assert_eq!(
            restarted.save(),
            world,
            "restart rebuilds the original world"
        );
    }

    #[test]
    fn input_controls_are_admissible_and_signed() {
        let params = game().replay.case.task.params;
        let turn = |turn| Input {
            turn,
            ..Input::default()
        };
        let full = Input {
            thrust: 1.0,
            ..turn(1.0)
        };
        let u = full.control(&params);
        assert!(u.is_admissible(&params), "full power");
        assert_eq!((u.thrust, u.torque), (params.f_max, params.tau_max), "full");
        let thrust = |thrust| Input {
            thrust,
            ..Input::default()
        };
        let halfway = thrust(0.5).control(&params).thrust;
        assert!((halfway - 0.5 * params.f_max).abs() < 1e-12, "{halfway}");
        let fine_halfway = Input {
            fine: true,
            ..thrust(0.5)
        };
        let fine_halfway = fine_halfway.control(&params).thrust;
        assert!(
            (fine_halfway - FINE * 0.5 * params.f_max).abs() < 1e-12,
            "{fine_halfway}"
        );
        assert_eq!(thrust(2.0).control(&params).thrust, params.f_max, "at most");
        for out_of_range in [-1.0, f64::NAN] {
            let u = thrust(out_of_range).control(&params);
            assert_eq!(u.thrust, 0.0, "{out_of_range}");
        }
        assert!(
            (thrust(0.25).or(thrust(0.5)).thrust - 0.5).abs() < 1e-12,
            "the harder counts"
        );
        let fine = Input {
            fine: true,
            ..turn(-1.0)
        };
        let u = fine.control(&params);
        assert_eq!(u.thrust, 0.0, "no thrust");
        assert!(
            (u.torque + FINE * params.tau_max).abs() < 1e-12,
            "fine, rightward"
        );
        let both = turn(1.0).or(turn(-1.0));
        assert_eq!(both.control(&params).torque, 0.0, "left and right cancel");
        let over = turn(1.0).or(turn(0.7)).control(&params);
        assert_eq!(over.torque, params.tau_max, "clamped to a full turn");
        assert_eq!(
            turn(f64::NAN).control(&params).torque,
            0.0,
            "NaN does not turn"
        );
    }

    #[test]
    fn the_joystick_turns_as_hard_as_it_is_pushed_sideways() {
        let canvas = Rect::from_min_size(Pos2::ZERO, vec2(400.0, 800.0));
        let (center, radius) = Pad::Stick.circle(canvas);
        assert_eq!(stick_turn(canvas, center), 0.0, "centered");
        assert_eq!(
            stick_turn(canvas, center + vec2(0.5 * DEAD_ZONE * radius, 30.0)),
            0.0,
            "in the dead zone; up and down do not count"
        );
        assert_eq!(
            stick_turn(canvas, center - vec2(radius, 0.0)),
            1.0,
            "full left"
        );
        assert_eq!(
            stick_turn(canvas, center + vec2(3.0 * radius, 0.0)),
            -1.0,
            "past full right"
        );
        let half = stick_turn(canvas, center - vec2(0.5 * radius, 0.0));
        assert!(0.4 < half && half < 0.5, "part way: {half}");
    }

    #[test]
    fn flight_follows_the_model_one_sample_at_a_time() {
        let mut g = game();
        let h = g.replay.h();
        let thrust = Input {
            thrust: 1.0,
            ..Input::default()
        };
        g.advance(2.5 * h, thrust, Steering::Raw);
        let r = &g.replay;
        assert_eq!(r.controls().len(), 3, "samples 0, 1 and 2 have controls");
        assert_eq!(r.states().len(), 4, "and the step under way ends");
        let params = r.case.task.params;
        let u = thrust.control(&params);
        let s1 = step(&params, &r.case.start, &u);
        assert_eq!(*r.state(1), s1, "exact step");
        let (k, tau) = g.moment();
        assert_eq!(k, 2, "in the third step");
        assert!((tau - 0.5 * h).abs() < 1e-12, "half way through it");
    }

    #[test]
    fn long_frames_are_capped_and_pause_stops_time() {
        let mut g = game();
        g.advance(10.0, Input::default(), Steering::Raw);
        assert!((g.time() - MAX_FRAME_TIME).abs() < 1e-12, "capped");
        g.paused = true;
        g.advance(0.05, Input::default(), Steering::Raw);
        assert!((g.time() - MAX_FRAME_TIME).abs() < 1e-12, "paused");
    }

    /// Runs `g` for `seconds` with `input` held, steering as `steering` says.
    fn fly(g: &mut Game, seconds: f64, input: Input, steering: Steering) {
        for _ in 0..(seconds / 0.05).round() as usize {
            g.advance(0.05, input, steering);
        }
    }

    #[test]
    fn without_the_assist_a_spinning_ship_spins_on() {
        let mut g = game();
        let omega = g.replay.case.start.omega;
        fly(&mut g, 2.0, Input::default(), Steering::Raw);
        assert!((g.state().omega - omega).abs() < 1e-12, "no torque");
    }

    #[test]
    fn the_assist_stops_the_spin_and_holds_the_heading_on_any_ship() {
        for name in ["agile", "sluggish", "random_00", "random_07"] {
            for h in [0.01, 0.02, 0.05, 0.1] {
                let mut case = suite::find(name).expect("case exists");
                case.task.params.h = h;
                let params = case.task.params;
                let mut g = Game::new(case);
                let stop = stopping_heading(&params, &g.replay.case.start);
                fly(&mut g, 30.0, Input::default(), Steering::Sas);
                let s = g.state();
                let at = format!("{name}, h = {h}");
                assert!(s.omega.abs() < 1e-3, "{at}: still spinning at {}", s.omega);
                assert!(
                    (s.theta - stop).abs() < 0.05,
                    "{at}: held {} for {stop}",
                    s.theta
                );
                assert_eq!(g.hold, Some(stop), "{at}: held where it stopped");
                for u in g.replay.controls() {
                    assert!(u.is_admissible(&params), "{at}: admissible");
                }
            }
        }
    }

    #[test]
    fn turning_overrides_the_assist_which_then_holds_the_new_heading() {
        let mut g = game();
        fly(&mut g, 3.0, Input::default(), Steering::Sas);
        let held = g.hold.expect("holding");
        let left = Input {
            turn: 1.0,
            ..Input::default()
        };
        fly(&mut g, 0.2, left, Steering::Sas);
        assert_eq!(g.hold, None, "the player turns");
        let params = g.replay.case.task.params;
        let guide = g.sas_heading(Steering::Sas).expect("guide while turning");
        assert_eq!(
            guide,
            stopping_heading(&params, &g.state()),
            "where it would stop"
        );
        assert!(guide > held, "ahead of the turn");
        assert_eq!(
            g.sas_heading(Steering::Raw),
            None,
            "no guide without the assist"
        );
        let tau_max = g.replay.case.task.params.tau_max;
        let last = g.replay.controls().back().expect("controls");
        assert_eq!(last.torque, tau_max, "the player's torque, unchanged");
        fly(&mut g, 5.0, Input::default(), Steering::Sas);
        let s = g.state();
        let new = g.hold.expect("holding again");
        assert!(new > held, "turned left");
        assert!(
            s.omega.abs() < 1e-3 && (s.theta - new).abs() < 0.05,
            "and stopped there"
        );
    }

    #[test]
    fn wrap_turns_the_shorter_way() {
        assert!(
            (wrap(3.0 * PI / 2.0) + PI / 2.0).abs() < 1e-12,
            "three quarters left"
        );
        assert!(
            (wrap(-5.0 * PI / 2.0) + PI / 2.0).abs() < 1e-12,
            "more than a turn right"
        );
        assert_eq!(wrap(0.3), 0.3, "already short");
    }

    /// An input that points at `heading`, thrusting within `cone`.
    fn aim(heading: f64, cone: f64) -> Input {
        Input {
            aim: Some(Aim { heading, cone }),
            ..Input::default()
        }
    }

    #[test]
    fn aiming_turns_the_ship_and_thrusts_only_within_the_cone() {
        for name in ["agile", "sluggish", "random_00", "random_07"] {
            for h in [0.01, 0.02, 0.05, 0.1] {
                let mut case = suite::find(name).expect("case exists");
                case.task.params.h = h;
                let params = case.task.params;
                let heading = case.start.theta + 2.0;
                let cone = 0.1;
                let mut g = Game::new(case);
                fly(&mut g, 30.0, aim(heading, cone), Steering::Sas);
                let at = format!("{name}, h = {h}");
                let r = &g.replay;
                for (s, u) in r.states().iter().zip(r.controls()) {
                    assert!(u.is_admissible(&params), "{at}: admissible");
                    let error = wrap(heading - s.theta).abs();
                    let thrusting = u.thrust > 0.0;
                    assert_eq!(thrusting, error <= cone, "{at}: thrust at {error}");
                }
                let s = g.state();
                assert!(wrap(heading - s.theta).abs() < 1e-3, "{at}: faces the aim");
                assert!(s.omega.abs() < 1e-3, "{at}: and stays");
                assert_eq!(g.hold, None, "{at}: the aim, not the assist");
            }
        }
    }

    #[test]
    fn aiming_turns_the_shorter_way_and_overrides_the_turn() {
        let mut case = suite::find("sluggish").expect("case exists");
        case.start.omega = 0.0;
        let start = case.start.theta;
        let mut g = Game::new(case);
        // A quarter turn right, as three quarters left; the player turning left besides.
        let input = Input {
            turn: 1.0,
            ..aim(start + 1.5 * PI, 0.1)
        };
        fly(&mut g, 0.5, input, Steering::Raw);
        assert!(g.state().theta < start, "turns right");
        assert!(
            g.replay.controls().iter().all(|u| u.torque <= 0.0),
            "the aim's torque, not the turn's"
        );
    }

    #[test]
    fn the_autopilot_flies_as_the_policy_is_scored() {
        use nabla_core::solutions::SOLUTIONS;
        for solution in SOLUTIONS {
            let case = suite::find("agile").expect("case exists");
            let scored = (solution.simulate)(&case);
            let n = scored.outcome.steps().expect("solved");
            let mut g = Game::new(case);
            // Pressing everything: the policy flies, not the player.
            let input = Input {
                thrust: 1.0,
                turn: 1.0,
                ..aim(1.0, 0.1)
            };
            let h = g.replay.h();
            g.engage(Objective::ReachB, solution).expect("engage");
            while g.landed().is_none() && g.time() < 60.0 {
                g.advance(h, input, Steering::Sas);
            }
            assert_eq!(g.landed(), Some(n), "{}: as scored", solution.name);
            assert!(g.autopiloted(), "{}: all the way", solution.name);
            let flown: Vec<Control> = g.replay.controls().iter().take(n).copied().collect();
            assert_eq!(flown, scored.controls, "{}", solution.name);
        }
    }

    #[test]
    fn the_autopilot_takes_over_mid_flight_and_holds_still_at_the_goal() {
        let solution = &nabla_core::solutions::SOLUTIONS[0];
        let mut g = game();
        let thrust = Input {
            thrust: 1.0,
            turn: 0.5,
            ..Input::default()
        };
        fly(&mut g, 1.0, thrust, Steering::Raw);
        let mut flown = 0.0;
        g.engage(Objective::ReachB, solution).expect("engage");
        while g.landed().is_none() && flown < 60.0 {
            g.advance(0.05, Input::default(), Steering::Raw);
            flown += 0.05;
        }
        assert!(g.landed().is_some(), "lands");
        assert!(!g.autopiloted(), "the player flew first");
        // The ship stays at the goal: the autopilot holds no control in the target set, and
        // brings the ship back should it drift out.
        let landed = g.landed().expect("landed");
        fly_with(&mut g, 10.0, Some(solution));
        let task = g.replay.case.task;
        for s in g.replay.states().iter().skip(landed) {
            let distance = (s.x - task.goal.x).hypot(s.y - task.goal.y);
            assert!(
                distance < 3.0 * task.tolerance.position,
                "off by {distance}"
            );
        }
        assert_eq!(g.take_fault(), None, "no fault");
    }

    /// A policy that thrusts harder than any ship can.
    struct Overthrust;

    impl Controller for Overthrust {
        fn new(_: &Task) -> Self {
            Self
        }

        fn control(&mut self, _: usize, _: &State) -> Control {
            Control {
                thrust: f64::INFINITY,
                torque: 0.0,
            }
        }
    }

    static OVERTHRUST: StoredSolution = StoredSolution {
        name: "overthrust",
        source: "",
        simulate: nabla_core::problem::simulate::<Overthrust>,
        controller: |task| Box::new(Overthrust::new(task)),
    };

    #[test]
    fn an_inadmissible_policy_hands_the_ship_back() {
        let mut g = game();
        g.engage(Objective::ReachB, &OVERTHRUST).expect("engage");
        g.advance(0.05, Input::default(), Steering::Raw);
        let fault = g.take_fault().expect("a fault");
        assert!(fault.contains("inadmissible"), "{fault}");
        let params = g.replay.case.task.params;
        for u in g.replay.controls() {
            assert!(u.is_admissible(&params), "the player's controls instead");
        }
        assert!(!g.autopiloted(), "the player flew");
    }

    #[test]
    fn the_default_policy_lands_in_the_default_world() {
        let world = crate::world::Preset::PlanetAndMoon.world();
        let mut g = Game::in_world(world.case(), world);
        g.engage(Objective::ReachB, &nabla_core::solutions::SOLUTIONS[0])
            .expect("engages");
        for _ in 0..600 {
            g.advance(0.02, Input::default(), Steering::Raw);
        }
        assert!(g.landed().is_some(), "landed within 12 s");
        assert_eq!(g.take_fault(), None);
    }

    /// Runs `g` for `seconds` with no input, `autopilot` flying.
    fn fly_with(g: &mut Game, seconds: f64, autopilot: Option<&'static StoredSolution>) {
        if let Some(solution) = autopilot {
            if !g.assisted() {
                g.engage(Objective::ReachB, solution).expect("engage");
            }
        } else {
            g.disengage();
        }
        for _ in 0..(seconds / 0.05).round() as usize {
            g.advance(0.05, Input::default(), Steering::Raw);
        }
    }

    #[test]
    fn a_flight_starting_in_the_target_set_lands_at_once() {
        let mut g = Game::new(suite::find("at_goal").expect("case exists"));
        assert_eq!(g.landed(), None, "not before it starts");
        g.advance(0.001, Input::default(), Steering::Raw);
        assert_eq!(g.landed(), Some(0), "at the first sample");
    }

    #[test]
    fn only_the_selected_objective_can_report_reaching_b() {
        let mut world = crate::world::Preset::Planet.world();
        world.players[0].state = State {
            x: 20.0,
            y: -12.0,
            ..State::default()
        };
        world.players[0].task.goal = nabla_core::problem::Goal { x: 20.0, y: -12.0 };
        let mut g = Game::in_world(world.case(), world);
        g.advance(0.0, Input::default(), Steering::Raw);
        assert_eq!(g.landed(), Some(0));
        let target = Target::at_current_altitude(&g.world, &g.state(), 0).expect("target");
        g.select_objective(Objective::Orbit(target))
            .expect("select");
        g.advance(0.02, Input::default(), Steering::Raw);
        assert_eq!(
            g.landed(),
            None,
            "B is irrelevant even in manual orbit flight"
        );
        g.select_objective(Objective::ReachB).expect("select B");
        g.advance(0.02, Input::default(), Steering::Raw);
        assert!(g.landed().is_some(), "B becomes the objective again");
    }

    #[test]
    fn a_failed_objective_change_preserves_the_current_controller() {
        let world = crate::world::Preset::Planet.world();
        let mut g = Game::in_world(world.case(), world);
        let solution = &nabla_core::solutions::SOLUTIONS[0];
        g.engage(Objective::ReachB, solution).expect("engage B");
        let invalid = Objective::Orbit(Target {
            body: 999,
            altitude: 10.0,
            clockwise: true,
        });
        assert!(g.engage(invalid, solution).is_err());
        assert!(g.select_objective(invalid).is_err());
        assert_eq!(g.objective(), Objective::ReachB);
        assert!(matches!(g.autopilot, Some(FlightController::ReachB(_))));
    }

    #[test]
    fn orbit_forecast_uses_a_copy_of_the_live_controller_and_matches_the_next_step() {
        let world = crate::world::Preset::Planet.world();
        let mut g = Game::in_world(world.case(), world);
        let target = Target::at_current_altitude(&g.world, &g.state(), 0).expect("target");
        g.engage(
            Objective::Orbit(target),
            &nabla_core::solutions::SOLUTIONS[0],
        )
        .expect("engage");
        g.advance(0.0, Input::default(), Steering::Raw);
        let k = g.replay.last_frame();
        let before = g.next_state();
        let path = g.prediction();
        assert_eq!(
            path,
            g.prediction(),
            "prediction does not consume live controls"
        );
        assert_eq!(g.next_state(), before);
        assert_eq!(path[0], [before.x, before.y]);
        g.advance(g.replay.h(), Input::default(), Steering::Raw);
        let next = *g.replay.state(k + 1);
        assert!((path[1][0] - next.x).hypot(path[1][1] - next.y) < 1e-4);
    }
}
