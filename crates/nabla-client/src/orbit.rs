//! Circular-orbit capture with bounded thrust/attitude feedback and a finite prediction.
//! This is a practical flight assist, not a global feasibility or stability certificate.

use std::ops::RangeInclusive;
use std::time::Duration;

use log::Level;
use nabla_core::{Control, Params, State};
use web_time::Instant;

use crate::physics::{Physics, gravity_acceleration};
use crate::world::{CelestialBody, World};
use nabla_sim::span;
use nabla_sim::steering::{hold_torque, wrap};

pub const MAX_ALTITUDE: f64 = 100_000.0;
/// How many steps of a [`Check`] run between looks at the clock: well under a millisecond.
const STEPS_PER_CLOCK_READ: usize = 32;
const CLEARANCE: f64 = 0.5;
const HOLD_SECONDS: f64 = 2.0;
/// How far above another body's keep-out band a suggested orbit sits. A ring that only just
/// clears the band passes the geometry check, but the other body's pull and the overshoot of
/// the capture can still carry the ship into it.
const SUGGESTED_MARGIN: f64 = 3.0;

/// The orbit radii around `source` at which a ship would touch `other`.
fn keep_out(world: &World, source: &CelestialBody, other: &CelestialBody) -> RangeInclusive<f64> {
    let distance = (f64::from(other.position[0]) - f64::from(source.position[0]))
        .hypot(f64::from(other.position[1]) - f64::from(source.position[1]));
    let reach = f64::from(other.radius + world.players[0].radius) + CLEARANCE;
    distance - reach..=distance + reach
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Target {
    pub body: usize,
    /// Distance of the ship's center above the body's surface, in world units.
    pub altitude: f64,
    pub clockwise: bool,
}

impl Target {
    pub fn at_current_altitude(world: &World, state: &State, body: usize) -> Option<Self> {
        let source = world.bodies.get(body)?;
        let x = state.x - f64::from(source.position[0]);
        let y = state.y - f64::from(source.position[1]);
        Some(Self {
            body,
            altitude: (x.hypot(y) - f64::from(source.radius))
                .clamp(Self::min_altitude(world), MAX_ALTITUDE),
            clockwise: x * state.vy - y * state.vx < 0.0,
        })
    }

    /// The target to offer first: the current altitude, or if that ring would run into another
    /// body, the lowest ring above it that stays clear of them all.
    pub fn suggested(world: &World, state: &State, body: usize) -> Option<Self> {
        let mut target = Self::at_current_altitude(world, state, body)?;
        let source = &world.bodies[body];
        let mut bands: Vec<_> = world
            .bodies
            .iter()
            .enumerate()
            .filter(|&(i, _)| i != body)
            .map(|(_, other)| keep_out(world, source, other))
            .collect();
        // In order of their lower edges, one climb past each is enough even where they overlap.
        bands.sort_by(|a, b| a.start().total_cmp(b.start()));
        let surface = f64::from(source.radius);
        for band in bands {
            let above = band.end() - surface + SUGGESTED_MARGIN;
            if (band.start() - surface..above).contains(&target.altitude) {
                target.altitude = above;
            }
        }
        target.altitude = target.altitude.min(MAX_ALTITUDE);
        Some(target)
    }

    pub fn min_altitude(world: &World) -> f64 {
        f64::from(world.players[0].radius) + 2.0 * CLEARANCE
    }

    pub fn radius(self, world: &World) -> f64 {
        f64::from(world.bodies[self.body].radius) + self.altitude
    }

    pub fn direction(self) -> &'static str {
        if self.clockwise {
            "Clockwise"
        } else {
            "Counterclockwise"
        }
    }

    fn speed(self, world: &World) -> f64 {
        (world.gravity_constant * world.bodies[self.body].mass / self.radius(world)).sqrt()
            * if self.clockwise { -1.0 } else { 1.0 }
    }

    /// Whether the target names a body and an altitude the world has room for.
    ///
    /// # Errors
    /// Says what is wrong with it.
    pub fn validate_input(self, world: &World) -> Result<(), String> {
        if world.bodies.get(self.body).is_none() {
            return Err("Select a planet or moon".to_owned());
        }
        if !self.altitude.is_finite()
            || !(Self::min_altitude(world)..=MAX_ALTITUDE).contains(&self.altitude)
        {
            return Err("Choose an altitude with room for the ship".to_owned());
        }
        Ok(())
    }

    /// Check target geometry and sampled force requirements before attempting capture.
    ///
    /// # Errors
    /// Says what is wrong with it.
    pub fn validate(self, world: &World) -> Result<(), String> {
        self.validate_input(world)?;
        let source = &world.bodies[self.body];
        let mu = world.gravity_constant * source.mass;
        if !mu.is_finite() || mu <= 0.0 {
            return Err("This body has no gravity to orbit".to_owned());
        }
        let radius = self.radius(world);
        let center = source.position.map(f64::from);
        for (i, other) in world.bodies.iter().enumerate() {
            if i != self.body && keep_out(world, source, other).contains(&radius) {
                return Err(format!("Orbit crosses {}. Change altitude", other.name));
            }
        }
        let params = world.players[0].task.params;
        for i in 0..64 {
            let angle = std::f64::consts::TAU * f64::from(i) / 64.0;
            let (s, c) = angle.sin_cos();
            let g = gravity_acceleration(
                &world.bodies,
                world.gravity_constant,
                [center[0] + radius * c, center[1] + radius * s],
            );
            let acceleration = mu / radius.powi(2);
            if (g[0] + acceleration * c).hypot(g[1] + acceleration * s)
                > 0.7 * params.f_max / params.mass
                || self.speed(world).abs() > 1e4
            {
                return Err("Not enough thrust margin at this altitude".to_owned());
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Turning,
    Burning,
    Coasting,
    Holding,
}

impl Phase {
    pub fn label(self) -> &'static str {
        match self {
            Self::Turning => "Turning",
            Self::Burning => "Burning",
            Self::Coasting => "Coasting",
            Self::Holding => "Holding orbit",
        }
    }
}

#[derive(Clone)]
pub struct Pilot {
    pub target: Target,
    pub phase: Phase,
    settled: f64,
    coasting: bool,
}

impl Pilot {
    pub fn new(target: Target) -> Self {
        Self {
            target,
            phase: Phase::Turning,
            settled: 0.0,
            coasting: false,
        }
    }

    /// Radial position, radial velocity and tangential velocity errors, in tolerance units.
    fn errors(&self, world: &World, state: &State) -> ([f64; 2], f64, f64, f64, f64) {
        let source = &world.bodies[self.target.body];
        let d = [
            state.x - f64::from(source.position[0]),
            state.y - f64::from(source.position[1]),
        ];
        let radius = d[0].hypot(d[1]).max(1e-6);
        let radial = [d[0] / radius, d[1] / radius];
        let vr = state.vx * radial[0] + state.vy * radial[1];
        let vt = -state.vx * radial[1] + state.vy * radial[0];
        let position_tolerance = (0.015 * self.target.radius(world)).max(0.2);
        let velocity_tolerance = (0.025 * self.target.speed(world).abs()).max(0.05);
        let error = ((radius - self.target.radius(world)) / position_tolerance)
            .abs()
            .max(vr.abs() / velocity_tolerance)
            .max((vt - self.target.speed(world)).abs() / velocity_tolerance);
        (radial, radius, vr, vt, error)
    }

    pub fn control(&mut self, world: &World, state: &State, params: &Params) -> Control {
        let (er, radius, vr, vt, error) = self.errors(world, state);
        self.settled = if error <= 1.0 {
            self.settled + params.h
        } else {
            0.0
        };
        if error < 0.1 {
            self.coasting = true;
        }
        if error > 1.0 {
            self.coasting = false;
        }
        let mut u = Control::default();
        if self.coasting {
            u.torque =
                (-params.inertia * state.omega / params.h).clamp(-params.tau_max, params.tau_max);
            self.phase = Phase::Coasting;
        } else {
            // Slow the translational loop enough to allow bounded reorientation.
            let time =
                (2.0 * (std::f64::consts::PI * params.inertia / params.tau_max).sqrt()).max(3.0);
            let limit = 0.4 * params.f_max / params.mass;
            let ar = -vt * vt / radius
                + ((self.target.radius(world) - radius) / time.powi(2) - 2.0 * vr / time)
                    .clamp(-limit, limit);
            let at =
                vr * vt / radius + ((self.target.speed(world) - vt) / time).clamp(-limit, limit);
            let g = gravity_acceleration(&world.bodies, world.gravity_constant, [state.x, state.y]);
            let command = [
                ar * er[0] - at * er[1] - g[0],
                ar * er[1] + at * er[0] - g[1],
            ];
            let heading = command[1].atan2(command[0]);
            let error = wrap(heading - state.theta);
            u.torque = hold_torque(params, state, state.theta + error);
            if error.abs() < std::f64::consts::PI / 6.0 {
                u.thrust = (params.mass
                    * (command[0] * state.theta.cos() + command[1] * state.theta.sin()))
                .clamp(0.0, params.f_max);
            }
            self.phase = if u.thrust > 0.001 * params.f_max {
                Phase::Burning
            } else {
                Phase::Turning
            };
        }
        if self.settled >= HOLD_SECONDS {
            self.phase = Phase::Holding;
        }
        u
    }
}

pub struct Prediction {
    pub capture_seconds: f64,
}

/// Display forecasts are intentionally shorter and cheaper than the capture feasibility check.
pub fn forecast_steps(h: f64) -> usize {
    ((5.0 / h).floor() as usize).min(500)
}

/// Run a copy of the controller, including its coast/hold state, on a separate physics plant.
pub fn forecast(world: &World, mut state: State, mut pilot: Pilot) -> Vec<[f64; 2]> {
    let _span = span!(Level::Trace, "orbit forecast");
    if pilot.target.validate_input(world).is_err() || !clear(world, &state) {
        return Vec::new();
    }
    let mut save = world.clone();
    save.players[0].state = state;
    let p = save.players[0].task.params;
    let mut physics = Physics::new(&save);
    let mut path = vec![[state.x, state.y]];
    for _ in 0..forecast_steps(p.h) {
        let u = pilot.control(world, &state, &p);
        if !u.is_admissible(&p) {
            break;
        }
        let samples = physics.step(p.h, u);
        if samples.iter().any(|s| !clear(world, s)) {
            break;
        }
        state = *samples.last().expect("physics samples");
        path.push([state.x, state.y]);
    }
    path
}

pub fn clear(world: &World, s: &State) -> bool {
    [s.x, s.y, s.vx, s.vy, s.theta, s.omega]
        .into_iter()
        .all(f64::is_finite)
        && world.bodies.iter().all(|body| {
            (s.x - f64::from(body.position[0])).hypot(s.y - f64::from(body.position[1]))
                > f64::from(body.radius + world.players[0].radius) + CLEARANCE
        })
}

/// A capture check that runs a little at a time: at once it takes longer than a tick, and a frame
/// that late costs a flight on a server its commands. How it is cut up changes nothing but when
/// the verdict comes.
pub struct Check {
    world: World,
    params: Params,
    physics: Physics,
    pilot: Pilot,
    state: State,
    captured: Option<f64>,
    step: usize,
    steps: usize,
}

impl Check {
    /// Bound both the duration and work.
    ///
    /// # Errors
    /// If the target is not valid here, or `state` is too close to a body.
    pub fn new(world: &World, state: State, target: Target) -> Result<Self, String> {
        target.validate(world)?;
        if !clear(world, &state) {
            return Err("Too close to a body to capture an orbit".to_owned());
        }
        let mut world = world.clone();
        world.players[0].state = state;
        let params = world.players[0].task.params;
        let physics = Physics::new(&world);
        Ok(Self {
            world,
            params,
            physics,
            pilot: Pilot::new(target),
            state,
            captured: None,
            step: 0,
            steps: ((120.0 / params.h).ceil() as usize).min(6000),
        })
    }

    /// Run up to `steps` more steps, and say the verdict once there is one. Reject unsafe substeps
    /// and captures that do not settle.
    pub fn advance(&mut self, steps: usize) -> Option<Result<Prediction, String>> {
        let end = self.step.saturating_add(steps).min(self.steps);
        while self.step < end {
            let u = self.pilot.control(&self.world, &self.state, &self.params);
            if !u.is_admissible(&self.params) {
                return Some(Err("Orbit controls exceed ship limits".to_owned()));
            }
            let samples = self.physics.step(self.params.h, u);
            if samples.iter().any(|s| !clear(&self.world, s)) {
                return Some(Err(
                    "No safe capture found. Try another altitude or fly clear".to_owned(),
                ));
            }
            self.state = *samples.last().expect("physics samples");
            self.step += 1;
            if self.pilot.phase == Phase::Holding {
                self.captured
                    .get_or_insert(self.step as f64 * self.params.h);
            }
        }
        (self.step == self.steps).then(|| self.verdict())
    }

    /// Run for about `budget`, and at least a few steps, so that a check always ends. Says the
    /// verdict once there is one.
    pub fn run_for(&mut self, budget: Duration) -> Option<Result<Prediction, String>> {
        let _span = span!(Level::Trace, "orbit capture check, a slice");
        let started = Instant::now();
        loop {
            let verdict = self.advance(STEPS_PER_CLOCK_READ);
            if verdict.is_some() || started.elapsed() >= budget {
                return verdict;
            }
        }
    }

    fn verdict(&self) -> Result<Prediction, String> {
        if self.pilot.phase != Phase::Holding {
            return Err(
                "No settled capture found in the prediction. Adjust altitude or approach"
                    .to_owned(),
            );
        }
        Ok(Prediction {
            capture_seconds: self.captured.expect("holding capture"),
        })
    }
}

/// A whole [`Check`] at once, for when the answer is needed now.
pub fn predict(world: &World, state: State, target: Target) -> Result<Prediction, String> {
    let _span = span!(
        Level::Debug,
        "orbit capture check, body {}, altitude {:.1}",
        target.body,
        target.altitude
    );
    let mut check = Check::new(world, state, target)?;
    check.advance(check.steps).expect("the last step")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::Preset;

    #[test]
    fn captures_default_planet_and_holds_for_several_orbits() {
        let world = Preset::Planet.world();
        let mut s = world.players[0].state;
        let target = Target::at_current_altitude(&world, &s, 0).expect("planet");
        assert!(target.clockwise, "preserve direction");
        let prediction = predict(&world, s, target).expect("default capture");
        assert!(prediction.capture_seconds < 120.0, "finite capture");
        let mut physics = Physics::new(&world);
        let mut pilot = Pilot::new(target);
        let p = world.players[0].task.params;
        let mut held = 0;
        for k in 0..15000 {
            let u = pilot.control(&world, &s, &p);
            assert!(u.is_admissible(&p), "bounded controls");
            let samples = physics.step(p.h, u);
            assert!(
                samples.iter().all(|s| clear(&world, s)),
                "clearance at step {k}"
            );
            s = *samples.last().expect("samples");
            if k > 6000 && pilot.phase == Phase::Holding {
                held += 1;
            }
        }
        assert!(
            held > 7500,
            "maintains orbit: {held} holding samples; state {s:?}"
        );
    }

    #[test]
    fn suggests_a_ring_clear_of_the_other_bodies_that_can_be_captured() {
        let world = Preset::PlanetAndMoon.world();
        let start = world.players[0].state;
        let around = |body| Target::suggested(&world, &start, body).expect("a body");
        // The ship starts where a ring around the planet would run through the moon.
        let current = Target::at_current_altitude(&world, &start, 0).expect("planet");
        assert!(current.validate(&world).is_err(), "crosses the moon");
        assert!((around(0).altitude - 20.0).abs() < 1e-9, "{:?}", around(0));
        assert_eq!(around(0).clockwise, current.clockwise, "keeps direction");
        for body in [0, 1] {
            predict(&world, start, around(body)).expect("a capture from the start");
        }
        assert_eq!(Target::suggested(&world, &start, 99), None, "unknown body");
    }

    #[test]
    fn a_suggestion_keeps_a_clear_altitude_and_climbs_only_when_it_must() {
        let world = Preset::PlanetAndMoon.world();
        let at = |x: f64, y: f64, body| {
            let state = State {
                x,
                y,
                ..State::default()
            };
            let current = Target::at_current_altitude(&world, &state, body).expect("a body");
            let suggested = Target::suggested(&world, &state, body).expect("a body");
            (current.altitude, suggested.altitude)
        };
        // Altitudes around the planet, centered at (0, -12): the moon rules out 9 to 17.
        for clear in [5.0, 25.0] {
            let (current, suggested) = at(0.0, 7.0 + clear - 12.0, 0);
            assert!((current - clear).abs() < 1e-9);
            assert_eq!(suggested, current, "nothing in the way at {clear}");
        }
        for blocked in [10.0, 17.5] {
            let (_, suggested) = at(0.0, 7.0 + blocked - 12.0, 0);
            assert!((suggested - 20.0).abs() < 1e-9, "from {blocked}");
        }
    }

    #[test]
    fn a_suggestion_climbs_past_bands_that_overlap_and_stays_in_range() {
        let mut world = Preset::PlanetAndMoon.world();
        let start = State {
            x: 0.0,
            y: 8.0,
            ..State::default()
        };
        // Listed out of order, with the second moon's band reaching well past the first's.
        let mut outer = world.bodies[1].clone();
        outer.position = [0.0, 12.0];
        world.bodies.insert(1, outer);
        let target = Target::suggested(&world, &start, 0).expect("planet");
        target.validate(&world).expect("clear of both moons");
        assert!((target.altitude - 24.0).abs() < 1e-9, "{target:?}");
        // A band straddling the top of the range: the climb stops at the limit.
        world.bodies[1].position = [0.0, 99_995.0];
        world.bodies[2].position = [0.0, 99_995.0];
        let high = State {
            x: 0.0,
            y: 99_993.0,
            ..start
        };
        let capped = Target::suggested(&world, &high, 0).expect("planet");
        assert!((capped.altitude - MAX_ALTITUDE).abs() < 1e-9, "{capped:?}");
    }

    #[test]
    fn captures_both_directions_and_changed_altitudes() {
        for clockwise in [false, true] {
            for altitude in [8.0, 20.0] {
                let mut world = Preset::Planet.world();
                world.players[0].state = State {
                    x: 20.0,
                    y: -12.0,
                    vy: if clockwise { -3.0 } else { 3.0 },
                    theta: 1.5,
                    ..State::default()
                };
                let target = Target {
                    body: 0,
                    altitude,
                    clockwise,
                };
                predict(&world, world.players[0].state, target)
                    .unwrap_or_else(|error| panic!("{target:?}: {error}"));
            }
        }
    }

    #[test]
    fn accounts_for_the_other_bodys_gravity_when_orbiting_the_moon() {
        let mut world = Preset::PlanetAndMoon.world();
        world.players[0].state = State {
            x: 18.0,
            y: 4.0,
            vy: 1.5,
            ..State::default()
        };
        let target = Target {
            body: 1,
            altitude: 3.5,
            clockwise: false,
        };
        predict(&world, world.players[0].state, target).expect("powered moon orbit");
    }

    #[test]
    fn rejects_invalid_targets_and_unsafe_captures() {
        let mut world = Preset::Planet.world();
        let s = world.players[0].state;
        let target = Target::at_current_altitude(&world, &s, 0).expect("planet");
        for altitude in [f64::NAN, f64::INFINITY, -1.0, 0.5, MAX_ALTITUDE + 1.0] {
            assert!(
                predict(&world, s, Target { altitude, ..target }).is_err(),
                "reject {altitude}"
            );
        }
        assert!(
            predict(&world, s, Target { body: 99, ..target }).is_err(),
            "unknown body"
        );
        world.gravity_constant = 0.0;
        assert!(predict(&world, s, target).is_err(), "no gravity");
        world = Preset::Planet.world();
        world.bodies[0].mass = 0.0;
        assert!(predict(&world, s, target).is_err(), "massless target");
        world = Preset::PlanetAndMoon.world();
        assert!(target.validate(&world).is_err(), "ring intersects moon");
        world = Preset::Planet.world();
        let incoming = State {
            x: -9.0,
            y: -12.0,
            vx: 100.0,
            ..State::default()
        };
        assert!(
            predict(&world, incoming, target).is_err(),
            "cannot brake before collision"
        );
    }

    /// The capture time's bits, or the refusal, of a check run in slices of `slice` steps.
    fn sliced(world: &World, state: State, target: Target, slice: usize) -> Result<u64, String> {
        let mut check = Check::new(world, state, target)?;
        loop {
            if let Some(verdict) = check.advance(slice) {
                return verdict.map(|p| p.capture_seconds.to_bits());
            }
        }
    }

    #[test]
    fn a_check_says_the_same_however_it_is_cut_up() {
        let planet = Preset::Planet.world();
        let start = planet.players[0].state;
        let capture = Target::at_current_altitude(&planet, &start, 0).expect("planet");
        let incoming = State {
            x: -9.0,
            y: -12.0,
            vx: 100.0,
            ..State::default()
        };
        let moon = Preset::PlanetAndMoon.world();
        let moon_start = moon.players[0].state;
        let around_moon = Target {
            body: 0,
            altitude: 34.0,
            ..capture
        };
        let cases = [
            (&planet, start, capture),
            (&planet, incoming, capture),
            (&moon, moon_start, around_moon),
        ];
        let mut outcomes = [0, 0];
        for (world, state, target) in cases {
            let whole = predict(world, state, target).map(|p| p.capture_seconds.to_bits());
            outcomes[usize::from(whole.is_err())] += 1;
            for slice in [1, 7, STEPS_PER_CLOCK_READ, 1000, usize::MAX] {
                assert_eq!(
                    sliced(world, state, target, slice),
                    whole,
                    "{target:?} in slices of {slice}"
                );
            }
        }
        assert_eq!(outcomes, [2, 1], "captures, and a refusal partway through");
    }

    #[test]
    fn a_check_ends_however_short_its_slices() {
        let world = Preset::Planet.world();
        let start = world.players[0].state;
        let target = Target::at_current_altitude(&world, &start, 0).expect("planet");
        let mut check = Check::new(&world, start, target).expect("a target");
        assert!(check.advance(0).is_none(), "no steps, and no verdict yet");
        assert_eq!(check.step, 0);
        assert!(check.run_for(Duration::ZERO).is_none());
        assert_eq!(check.step, STEPS_PER_CLOCK_READ, "a slice with no time");
        let mut slices = 1;
        while check.run_for(Duration::ZERO).is_none() {
            slices += 1;
        }
        assert_eq!(slices + 1, check.steps.div_ceil(STEPS_PER_CLOCK_READ));
    }

    #[test]
    fn holding_requires_sustained_orbital_accuracy_and_resets_after_a_disturbance() {
        let world = Preset::Planet.world();
        let target = Target {
            body: 0,
            altitude: 13.0,
            clockwise: false,
        };
        let mut pilot = Pilot::new(target);
        let p = world.players[0].task.params;
        let s = State {
            x: 20.0,
            y: -12.0,
            vy: (343.0_f64 / 20.0).sqrt(),
            ..State::default()
        };
        for _ in 0..50 {
            pilot.control(&world, &s, &p);
        }
        assert_ne!(
            pilot.phase,
            Phase::Holding,
            "one second is not a settled orbit"
        );
        for _ in 0..60 {
            pilot.control(&world, &s, &p);
        }
        assert_eq!(pilot.phase, Phase::Holding);
        pilot.control(&world, &State { vx: 2.0, ..s }, &p);
        assert_ne!(
            pilot.phase,
            Phase::Holding,
            "radial disturbance resets holding"
        );
    }
}
