//! The networked flight: the client machine of `nabla-net` on a connection, and what a screen
//! needs of it.
//!
//! The machine predicts the own ship, `Pred(k)`, and takes in snapshots. This turns that into
//! things to draw. The own ship is drawn between its last two predicted ticks, and moved by a
//! smoothing offset that absorbs a correction and then decays (`docs/multiplayer.typ`,
//! "Hiding corrections"). Other ships are drawn in the past, between two snapshots, by cubic
//! Hermite interpolation (`prop-hermite`). An autopilot flies by the prediction and sends
//! `Direct` commands (`Autopilots`).
//!
//! Nothing here draws, so it runs under test: the transport is a trait, and the clock is an
//! argument of [`Online::frame_at`].

use std::collections::BTreeMap;

use nabla_core::problem::{Case, Outcome, Trajectory};
use nabla_core::solutions::StoredSolution;
use nabla_core::{Control, Params, State, Task, flow};
use nabla_net::client::{
    Correction, CorrectionStats, LobbyInfo, Remote, SNAP_HEADING, SNAP_POSITION, Stage, Step,
};
use nabla_net::link::Incoming;
use nabla_net::wire::{ShipId, Tick};
use nabla_net::{ClientMachine, Config};
use nabla_sim::command::Command;
use nabla_sim::steering::{Input, Steering};
use nabla_sim::world::{ShipRequest, World};
use web_time::Instant;

use crate::game::{Autopilot, FlightController, Objective, forecast_of, run_controller};
use crate::net::Transport;
use crate::orbit::{self, Pilot, Prediction};
use crate::plots::{Plots, window_steps};
use crate::replay::Replay;

/// `tau_s`: how long a correction takes to fade from the picture, in seconds.
pub const SMOOTHING: f64 = 0.1;
/// `q`: ticks between the snapshots of a remote ship. The server sends every ship every tick.
const Q: f64 = 1.0;
/// The allowance for jitter in `D_r h = q h + 40 ms`, the delay remote ships are drawn in.
const REMOTE_JITTER: f64 = 0.040;
/// How many ticks of its own flight the client keeps, for the plume, the path and the plots:
/// half a minute.
const TRAIL: Tick = 1800;
/// The longest a remote ship is extrapolated past its newest snapshot, in seconds.
const MAX_EXTRAPOLATION: f64 = 0.1;
/// How far into a tick, as a fraction of it, the plume and the sound read its control at most.
const LAST_OF_TICK: f64 = 0.999;
/// The longest frame the offset decays over, in seconds: a stall is not a long frame.
const MAX_FRAME: f64 = 0.25;

/// What the pilot holds.
#[derive(Clone, Copy, Debug, Default)]
pub struct Hands {
    pub input: Input,
    pub steering: Steering,
}

/// A ship to draw.
pub struct View {
    pub ship: ShipId,
    pub own: bool,
    /// Where it is drawn.
    pub state: State,
    pub params: Params,
    /// Its recent flight, for the plume and the thrusters' sound: `t` seconds into it is now.
    pub replay: Replay,
    pub t: f64,
}

/// What a screen can say about the connection.
#[derive(Clone, Copy, Debug)]
pub struct Info {
    pub rtt: Option<f64>,
    /// `mu - mu*` in seconds.
    pub margin_error: Option<f64>,
    pub tick: Option<Tick>,
    pub corrections: CorrectionStats,
    /// Ships in the world, this one included.
    pub ships: usize,
}

/// `a` and `b`, the states of one ship `span` seconds apart, joined by the cubic Hermite
/// interpolant of the positions and the headings, which uses the velocities and the spin, at
/// `s` in `[0, 1]` of the way.
pub fn hermite(a: &State, b: &State, s: f64, span: f64) -> State {
    let (s2, s3) = (s * s, s * s * s);
    let (h00, h10, h01, h11) = (
        2.0 * s3 - 3.0 * s2 + 1.0,
        s3 - 2.0 * s2 + s,
        -2.0 * s3 + 3.0 * s2,
        s3 - s2,
    );
    let (d00, d10, d11) = (
        6.0 * s2 - 6.0 * s,
        3.0 * s2 - 4.0 * s + 1.0,
        3.0 * s2 - 2.0 * s,
    );
    let along = |p0: f64, v0: f64, p1: f64, v1: f64| {
        (
            h00 * p0 + h10 * span * v0 + h01 * p1 + h11 * span * v1,
            (d00 * p0 + d10 * span * v0 - d00 * p1 + d11 * span * v1) / span,
        )
    };
    let (x, vx) = along(a.x, a.vx, b.x, b.vx);
    let (y, vy) = along(a.y, a.vy, b.y, b.vy);
    let (theta, omega) = along(a.theta, a.omega, b.theta, b.omega);
    State {
        x,
        y,
        vx,
        vy,
        theta,
        omega,
    }
}

/// `epsilon`: what the picture adds to the prediction to hide the jumps in it. When a snapshot
/// moves the prediction by `Delta`, the offset takes `-Delta`, so that the drawing does not move,
/// and then fades.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Smoothing {
    pub x: f64,
    pub y: f64,
    pub theta: f64,
}

impl Smoothing {
    /// Take in a correction, or drop the offset if it is too large to hide.
    pub fn absorb(&mut self, correction: &Correction) {
        let d = &correction.delta;
        if correction.jumped || d.x.hypot(d.y) > SNAP_POSITION || d.theta.abs() > SNAP_HEADING {
            *self = Self::default();
        } else {
            self.x -= d.x;
            self.y -= d.y;
            self.theta -= d.theta;
        }
    }

    /// Let `elapsed` seconds pass: `epsilon` fades by `exp(-elapsed / tau_s)`.
    pub fn decay(&mut self, elapsed: f64) {
        let keep = (-elapsed / SMOOTHING).exp();
        self.x *= keep;
        self.y *= keep;
        self.theta *= keep;
    }

    fn apply(&self, state: &mut State) {
        state.x += self.x;
        state.y += self.y;
        state.theta += self.theta;
    }
}

/// The tick the other ships are drawn at, which trails the newest snapshot by `D_r`.
#[derive(Clone, Copy, Debug, Default)]
struct Playout {
    tick: Option<f64>,
    seen: Tick,
    last: f64,
}

impl Playout {
    /// The newest snapshot is for tick `newest`, and it is `now`.
    fn update(&mut self, now: f64, h: f64, newest: Tick) {
        let target = f64::from(newest) - Q - REMOTE_JITTER / h;
        let advance = ((now - self.last) / h).clamp(0.0, MAX_FRAME / h);
        let mut tick = self.tick.map_or(target, |tick| tick + advance);
        if newest != self.seen {
            // Steer toward the target by stretching or shrinking the ticks a little, as the own
            // ship's margin is steered; a gap too large for that is a jump.
            let error = target - tick;
            if error.abs() > 8.0 {
                tick = target;
            } else {
                let nudge = 0.05 * advance;
                tick += (0.1 * error).clamp(-nudge, nudge);
            }
        }
        self.tick = Some(tick.min(f64::from(newest)));
        self.seen = newest;
        self.last = now;
    }
}

/// An autopilot flying the ship.
struct Engaged {
    controller: FlightController,
    task: Task,
}

/// A flight on a server.
pub struct Online {
    transport: Box<dyn Transport>,
    machine: ClientMachine,
    closed: Option<String>,
    started: Instant,
    last: f64,
    smoothing: Smoothing,
    /// The own ship's predicted flight, tick by tick.
    trail: BTreeMap<Tick, Step>,
    /// The buffers of the own ship's plots, kept from frame to frame.
    plots: Option<Plots>,
    playout: Playout,
    engaged: Option<Engaged>,
    /// What the ship is aiming for, which an autopilot flies to once engaged.
    objective: Objective,
    fault: Option<String>,
    landed: bool,
}

impl Online {
    /// A client that will ask the server it is connected to for `request` when it is told to
    /// [`join`](Self::join).
    pub fn new(mut transport: Box<dyn Transport>, request: ShipRequest) -> Self {
        let mut machine =
            ClientMachine::new(Config::default(), request, crate::version::build_version!())
                .waiting_in_the_lobby();
        for kind in machine.start() {
            transport.send(kind);
        }
        Self {
            transport,
            machine,
            closed: None,
            started: Instant::now(),
            last: 0.0,
            smoothing: Smoothing::default(),
            trail: BTreeMap::new(),
            plots: None,
            playout: Playout::default(),
            engaged: None,
            objective: Objective::ReachB,
            fault: None,
            landed: false,
        }
    }

    pub fn stage(&self) -> Stage {
        self.machine.stage()
    }

    /// Why the connection is over, if it is.
    pub fn closed(&self) -> Option<&str> {
        self.closed.as_deref().or_else(|| self.machine.refusal())
    }

    pub fn lobby(&self) -> Option<LobbyInfo> {
        self.machine.lobby()
    }

    pub fn world(&self) -> Option<&World> {
        self.machine.world()
    }

    pub fn params(&self) -> Option<Params> {
        self.machine.params()
    }

    /// Ask for a ship. Does nothing outside the lobby.
    pub fn join(&mut self, request: ShipRequest) {
        self.join_at(self.started.elapsed().as_secs_f64(), request);
    }

    /// [`Self::join`] at local time `now`.
    pub fn join_at(&mut self, now: f64, request: ShipRequest) {
        for kind in self.machine.join(request, now) {
            self.transport.send(kind);
        }
    }

    /// Give the ship up, and go back to the lobby.
    pub fn leave(&mut self) {
        for kind in self.machine.leave() {
            self.transport.send(kind);
        }
        self.reset_flight();
    }

    fn reset_flight(&mut self) {
        self.trail.clear();
        self.smoothing = Smoothing::default();
        self.playout = Playout::default();
        self.engaged = None;
        self.landed = false;
    }

    /// `h`, the world's tick, in seconds.
    pub fn h(&self) -> Option<f64> {
        self.machine.h()
    }

    /// The ship's task: the world's goal, with the ship's own parameters.
    fn task(&self) -> Option<Task> {
        let (world, params) = (self.machine.world()?, self.machine.params()?);
        let mut task = world.spawn(0).task;
        task.params = params;
        Some(task)
    }

    pub fn objective(&self) -> Objective {
        self.objective
    }

    /// Aim for `objective`. Another one than before takes the ship back from an autopilot.
    ///
    /// # Errors
    /// If an orbit's target is not valid in this world.
    pub fn select_objective(&mut self, objective: Objective) -> Result<(), String> {
        if let (Objective::Orbit(target), Some(world)) = (objective, self.machine.world()) {
            target.validate_input(world)?;
        }
        if self.objective != objective {
            self.engaged = None;
            self.objective = objective;
            self.landed = false;
        }
        Ok(())
    }

    /// Fly the ship to `objective`, by `solution` for B and by the orbit assist for an orbit,
    /// from the ship's predicted state on. Says what an orbit is forecast to do.
    ///
    /// # Errors
    /// If there is no ship to fly, or the orbit cannot be flown from here.
    pub fn engage(
        &mut self,
        objective: Objective,
        solution: &'static StoredSolution,
    ) -> Result<Option<Prediction>, String> {
        let (Some(task), Some(world), Some(start)) =
            (self.task(), self.machine.world(), self.next_state())
        else {
            return Err("no ship to fly".to_owned());
        };
        let (controller, prediction) = match objective {
            Objective::ReachB => (FlightController::ReachB(Autopilot::new(solution)), None),
            Objective::Orbit(target) => {
                let prediction = orbit::predict(world, start, target)?;
                (
                    FlightController::Orbit(Pilot::new(target)),
                    Some(prediction),
                )
            }
        };
        self.select_objective(objective)?;
        self.engaged = Some(Engaged { controller, task });
        self.fault = None;
        self.landed = false;
        Ok(prediction)
    }

    /// The orbit assist, while it flies.
    pub fn orbit_pilot(&self) -> Option<&Pilot> {
        match &self.engaged.as_ref()?.controller {
            FlightController::Orbit(pilot) => Some(pilot),
            FlightController::ReachB(_) => None,
        }
    }

    /// The own ship's state at the next tick to issue, `Pred(k)`: where the controllers fly from.
    pub fn next_state(&self) -> Option<State> {
        self.trail.get(&self.machine.tick()?).map(|step| step.state)
    }

    /// The flight's time, in seconds: the next tick to issue, times `h`.
    pub fn time(&self) -> f64 {
        match (self.machine.tick(), self.machine.h()) {
            (Some(k), Some(h)) => f64::from(k) * h,
            _ => 0.0,
        }
    }

    /// A short forecast of the ship's path under the engaged controller, or none.
    pub fn forecast(&self) -> Vec<[f64; 2]> {
        match (self.machine.world(), self.next_state(), self.machine.h()) {
            (Some(world), Some(start), Some(h)) => forecast_of(
                world,
                start,
                self.engaged.as_ref().map(|e| &e.controller),
                h,
            ),
            _ => Vec::new(),
        }
    }

    pub fn disengage(&mut self) {
        self.engaged = None;
    }

    pub fn assisted(&self) -> bool {
        self.engaged.is_some()
    }

    /// Why the autopilot stopped flying on its own, once.
    pub fn take_fault(&mut self) -> Option<String> {
        self.fault.take()
    }

    /// Whether the autopilot's ship has arrived at the goal since the last call.
    pub fn take_landed(&mut self) -> bool {
        std::mem::take(&mut self.landed)
    }

    /// Local time now, in seconds since this was made: the clock [`Self::frame`] runs by.
    pub fn local_time(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }

    /// One frame, by the real clock.
    pub fn frame(&mut self, hands: Hands) {
        self.frame_at(self.started.elapsed().as_secs_f64(), hands);
    }

    /// One frame at local time `now`: take in what the network brought, issue the commands that
    /// are due, and bring the smoothing, the trail and the playout up to date.
    pub fn frame_at(&mut self, now: f64, hands: Hands) {
        let frame_time = (now - self.last).clamp(0.0, MAX_FRAME);
        self.last = now;
        while let Some(incoming) = self.transport.recv() {
            match incoming {
                Incoming::Stream(bytes) => {
                    for kind in self.machine.on_stream(&bytes, now) {
                        self.transport.send(kind);
                    }
                }
                Incoming::Datagram(bytes) => {
                    self.machine.on_datagram(&bytes, now);
                }
                Incoming::Closed(why) => self.closed = Some(why),
            }
        }
        if !self.machine.is_playing() {
            if !self.trail.is_empty() {
                self.reset_flight();
            }
            return;
        }
        let (Some(params), Some(h)) = (self.machine.params(), self.machine.h()) else {
            return;
        };
        let world = self.machine.world().cloned();
        let Self {
            machine,
            transport,
            engaged,
            fault,
            ..
        } = self;
        let issued = machine.poll(now, &mut |_stamp, predicted| {
            let manual = Command::Manual {
                input: hands.input,
                steering: hands.steering,
            };
            let command = match (engaged.as_mut(), world.as_ref()) {
                (Some(flying), Some(world)) => {
                    match run_controller(&mut flying.controller, world, &flying.task, predicted) {
                        Ok(u) => Command::Direct {
                            thrust: u.thrust,
                            torque: u.torque,
                        },
                        Err(why) => {
                            *fault = Some(why);
                            *engaged = None;
                            manual
                        }
                    }
                }
                _ => manual,
            };
            command.sanitize(&params)
        });
        for kind in issued {
            transport.send(kind);
        }
        if let Some(correction) = self.machine.take_correction() {
            self.smoothing.absorb(&correction);
        }
        self.smoothing.decay(frame_time);
        let steps = self.machine.trajectory();
        let newest = steps.last().map(|step| step.tick);
        for step in steps {
            self.trail.insert(step.tick, step);
        }
        if let Some(newest) = newest {
            let keep_from = newest.saturating_sub(TRAIL);
            self.trail = self.trail.split_off(&keep_from);
            if let Some(flying) = &self.engaged
                && self.objective == Objective::ReachB
                && let Some(step) = self.trail.get(&newest)
                && flying.task.reached(&step.state)
            {
                self.landed = true;
            }
        }
        if let Some(base) = self.machine.base() {
            self.playout.update(now, h, base.tick);
        }
    }

    /// What a screen can say about the connection.
    pub fn info(&self) -> Info {
        Info {
            rtt: self.transport.rtt().or_else(|| self.machine.round_trip()),
            margin_error: self.machine.margin_error(),
            tick: self.machine.tick(),
            corrections: self.machine.correction_stats().unwrap_or_default(),
            ships: self.machine.roster().len(),
        }
    }

    /// The ships to draw at local time `now`: the own ship first, then the others.
    pub fn views(&mut self, now: f64) -> Vec<View> {
        let mut views = Vec::new();
        let (Some(world), Some(own), Some(params), Some(h)) = (
            self.machine.world().cloned(),
            self.machine.ship(),
            self.machine.params(),
            self.machine.h(),
        ) else {
            return views;
        };
        let mut task = world.spawn(0).task;
        task.params = params;
        if let Some(view) = self.own_view(&task, own, h, now) {
            views.push(view);
        }
        let Some(playout) = self.playout.tick else {
            return views;
        };
        let roster: Vec<(ShipId, Params)> = self
            .machine
            .roster()
            .iter()
            .map(|(id, entry)| (*id, entry.params))
            .collect();
        let Some(remotes) = self.machine.remotes() else {
            return views;
        };
        for (ship, params) in roster {
            if ship == own {
                continue;
            }
            let Some(records) = remotes.get(&ship) else {
                continue;
            };
            let records: Vec<Remote> = records.iter().copied().collect();
            if let Some(view) = remote_view(&task, ship, params, &records, playout, h) {
                views.push(view);
            }
        }
        views
    }

    /// The plots of the own ship's recent flight, `own`. Predicted ticks are corrected after the
    /// fact, so they are recorded anew each frame, in buffers allocated the first time. They need
    /// not hold more than the trail, which also bounds them whatever the server's `h`.
    pub fn plots_of_own(&mut self, own: &View) -> &Plots {
        let steps = window_steps(own.replay.h()).min(TRAIL as usize);
        let plots = self
            .plots
            .take()
            .filter(|plots| plots.steps() == steps)
            .unwrap_or_else(|| Plots::new(steps));
        let plots = self.plots.insert(plots);
        plots.rerecord(&own.replay);
        plots
    }

    /// The own ship, drawn between its last two predicted ticks, and moved by the smoothing.
    fn own_view(&self, task: &Task, ship: ShipId, h: f64, now: f64) -> Option<View> {
        let k = self.machine.tick()?;
        let due = self.machine.next_deadline()?;
        // The ship is in tick `k - 1`, which began a tick's length before the next issue.
        let fraction = (1.0 - (due - now) / h).clamp(0.0, 1.0);
        let (from, to) = (self.trail.get(&k.checked_sub(1)?)?, self.trail.get(&k)?);
        let mut state = hermite(&from.state, &to.state, fraction, h);
        self.smoothing.apply(&mut state);
        // The contiguous run of the trail that ends at `k`.
        let mut run: Vec<&Step> = Vec::new();
        for (tick, step) in self.trail.range(..=k).rev() {
            if run.last().is_some_and(|last| last.tick != tick + 1) {
                break;
            }
            run.push(step);
        }
        run.reverse();
        let states: Vec<State> = run.iter().map(|step| step.state).collect();
        // A tick the snapshots overtook before it was issued has no control, and the ship held the
        // one before: skipping it would move every later control a tick early, leaving none for
        // now, and the sound silent until that tick leaves the trail.
        let mut held = Control::default();
        let controls: Vec<Control> = run[..run.len().saturating_sub(1)]
            .iter()
            .map(|step| {
                held = step.control.unwrap_or(held);
                held
            })
            .collect();
        // Inside the tick, never at its end: a frame later than the tick's due time has run past
        // the last control, and the plume and the sound would find none (so silence) there.
        let t = (run.len().saturating_sub(2)) as f64 * h + fraction.min(LAST_OF_TICK) * h;
        Some(View {
            ship,
            own: true,
            state,
            params: task.params,
            replay: replay_of(task, states, controls),
            t,
        })
    }
}

/// Another ship, drawn at the playout tick `at` from its snapshots: between two of them by
/// Hermite interpolation, and past the newest by a short extrapolation under its control.
fn remote_view(
    task: &Task,
    ship: ShipId,
    params: Params,
    records: &[Remote],
    at: f64,
    h: f64,
) -> Option<View> {
    let (first, last) = (records.first()?, records.last()?);
    let state = state_at(records, at, h, &params)?;
    // Contiguous ticks from the oldest to the newest, filling the gaps losses leave.
    let ticks = (first.tick..=last.tick).collect::<Vec<_>>();
    let mut states = Vec::with_capacity(ticks.len());
    let mut controls = Vec::with_capacity(ticks.len());
    for &tick in &ticks {
        states.push(state_at(records, f64::from(tick), h, &params)?);
        // The control a record carries was applied in the tick before its own.
        let next = records.iter().find(|r| r.tick > tick).unwrap_or(last);
        controls.push(Control {
            thrust: f64::from(next.record.thrust),
            torque: f64::from(next.record.torque),
        });
    }
    controls.pop();
    let mut task = *task;
    task.params = params;
    let t = ((at - f64::from(first.tick)).max(0.0)) * h;
    Some(View {
        ship,
        own: false,
        state,
        params,
        replay: replay_of(&task, states, controls),
        t,
    })
}

/// The state at tick `at` (a fraction included) from snapshots at ticks of their own.
fn state_at(records: &[Remote], at: f64, h: f64, params: &Params) -> Option<State> {
    let first = records.first()?;
    let last = records.last()?;
    if at <= f64::from(first.tick) {
        return Some(first.record.state);
    }
    if at >= f64::from(last.tick) {
        let ahead = ((at - f64::from(last.tick)) * h).min(MAX_EXTRAPOLATION);
        let control = Control {
            thrust: f64::from(last.record.thrust),
            torque: f64::from(last.record.torque),
        };
        return Some(if ahead > 0.0 {
            flow(params, &last.record.state, &control, ahead)
        } else {
            last.record.state
        });
    }
    let i = records.partition_point(|r| f64::from(r.tick) <= at);
    let (a, b) = (records.get(i.checked_sub(1)?)?, records.get(i)?);
    let span = f64::from(b.tick - a.tick);
    Some(hermite(
        &a.record.state,
        &b.record.state,
        (at - f64::from(a.tick)) / span,
        span * h,
    ))
}

/// A run of states and controls as a replay, for the plume and the sound: `controls[i]` is held
/// from `states[i]` to `states[i + 1]`.
fn replay_of(task: &Task, states: Vec<State>, controls: Vec<Control>) -> Replay {
    let start = states.first().copied().unwrap_or_default();
    let mut controls = controls;
    controls.truncate(states.len().saturating_sub(1));
    while controls.len() + 1 < states.len() {
        controls.push(Control::default());
    }
    Replay::new(
        Case {
            name: "online".to_owned(),
            task: *task,
            start,
        },
        Trajectory {
            states: if states.is_empty() {
                vec![start]
            } else {
                states
            },
            controls,
            outcome: Outcome::Timeout,
        },
    )
}

#[cfg(test)]
mod tests;
