//! The measurements of `docs/multiplayer.typ` that depend on the platform.
//!
//! They are code that runs the same on every platform: natively
//! (`cargo run --release -p nabla-sim --example measure`) and in a browser (`measure_report`
//! from the web app), so that the two can be put side by side.
//!
//! The replay table restores a ship from a recorded state, as a client does at each snapshot,
//! and compares the replay to the flight that was not interrupted. The digest hashes the bits of
//! such a flight: if two platforms agree on it, they agree on every tick. The timings are those
//! of one `Controller::control`, which a client must produce within a tick.

use std::fmt::Write as _;

use nabla_core::solutions::SOLUTIONS;
use nabla_core::{Controller, State, step, suite};
use web_time::Instant;

use crate::command::Command;
use crate::rng::Rng;
use crate::ship::Ship;
use crate::world::{Preset, World};

/// Replay lengths of the table, in ticks.
pub const LENGTHS: [usize; 4] = [1, 6, 12, 30];
/// Within this distance of a body's surface, a window "touched" it.
const CONTACT: f64 = 0.05;
const H: f64 = 1.0 / 60.0;

/// The worst error of a kind of window.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Worst {
    pub windows: usize,
    pub position: f64,
    pub velocity: f64,
    pub heading: f64,
    pub spin: f64,
}

impl Worst {
    fn take(&mut self, replayed: &State, live: &State) {
        self.windows += 1;
        self.position = self
            .position
            .max((replayed.x - live.x).hypot(replayed.y - live.y));
        self.velocity = self
            .velocity
            .max((replayed.vx - live.vx).hypot(replayed.vy - live.vy));
        self.heading = self.heading.max((replayed.theta - live.theta).abs());
        self.spin = self.spin.max((replayed.omega - live.omega).abs());
    }
}

/// One row of the replay table.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Row {
    pub length: usize,
    pub free: Worst,
    pub contact: Worst,
}

fn world(preset: Preset) -> World {
    preset
        .world()
        .multiplayer(60, 1000.0, 16)
        .expect("a bundled world is a server world")
}

/// A flight of `ticks` ticks under random commands, from `start`: the commands, and the states
/// before each of them and after the last.
fn flight(
    world: &World,
    seed: u64,
    start: State,
    ticks: usize,
    hops: bool,
) -> (Vec<Command>, Vec<State>) {
    let params = world.default_ship();
    let mut rng = Rng::new(seed);
    let mut ship = Ship::new(world, params, start);
    let (mut commands, mut states) = (Vec::with_capacity(ticks), vec![start]);
    let mut held = Command::default();
    for tick in 0..ticks {
        if tick % 20 == 0 {
            held = Command::Direct {
                thrust: if rng.chance(if hops { 0.7 } else { 0.5 }) {
                    rng.range(0.0, params.f_max)
                } else {
                    0.0
                },
                torque: rng.range(-params.tau_max, params.tau_max),
            };
        }
        ship.tick(world, &held);
        commands.push(held);
        states.push(ship.state());
    }
    (commands, states)
}

/// Whether the ship at `s` is within [`CONTACT`] of a body's surface.
fn touching(world: &World, s: &State) -> bool {
    let hull = f64::from(world.hull_radius());
    world.bodies.iter().any(|b| {
        let d = (s.x - f64::from(b.position[0])).hypot(s.y - f64::from(b.position[1]));
        d - f64::from(b.radius) - hull < CONTACT
    })
}

/// Restore at every tick of the flight, replay `length` ticks, and file the error by whether the
/// window touched a body.
fn windows(world: &World, commands: &[Command], states: &[State], rows: &mut [Row]) {
    let params = world.default_ship();
    for row in rows {
        let mut ship = Ship::new(world, params, states[0]);
        for start in 0..commands.len().saturating_sub(row.length) {
            ship.restore(world, states[start], None);
            for command in &commands[start..start + row.length] {
                ship.tick(world, command);
            }
            let end = start + row.length;
            let kind = if states[start..=end].iter().any(|s| touching(world, s)) {
                &mut row.contact
            } else {
                &mut row.free
            };
            kind.take(&ship.state(), &states[end]);
        }
    }
}

/// The replay table: the planet-and-moon world under random commands, and the planet world from
/// its surface under random hops (which touches).
pub fn replay_table(seed: u64, ticks: usize) -> Vec<Row> {
    let mut rows: Vec<Row> = LENGTHS
        .iter()
        .map(|&length| Row {
            length,
            ..Row::default()
        })
        .collect();
    let moon = world(Preset::PlanetAndMoon);
    let (commands, states) = flight(&moon, seed, moon.spawn(0).state, ticks, false);
    windows(&moon, &commands, &states, &mut rows);
    let planet = world(Preset::Planet);
    let body = &planet.bodies[0];
    let surface = State {
        x: f64::from(body.position[0]),
        y: f64::from(body.position[1])
            + f64::from(body.radius)
            + f64::from(planet.hull_radius())
            + 0.01,
        vx: 0.0,
        vy: 0.0,
        theta: std::f64::consts::FRAC_PI_2,
        omega: 0.0,
    };
    let (commands, states) = flight(&planet, seed ^ 0x5eed, surface, ticks, true);
    windows(&planet, &commands, &states, &mut rows);
    rows
}

/// An FNV-1a hash of the bits of every state of two flights, in a vacuum and among bodies. Two
/// platforms that agree on it agree on every tick of both.
pub fn digest(seed: u64, ticks: usize) -> [u64; 2] {
    [Preset::Vacuum, Preset::PlanetAndMoon].map(|preset| {
        let world = world(preset);
        let (_, states) = flight(&world, seed, world.spawn(0).state, ticks, false);
        states.iter().fold(0xcbf2_9ce4_8422_2325_u64, |hash, s| {
            [s.x, s.y, s.vx, s.vy, s.theta, s.omega]
                .iter()
                .fold(hash, |hash, x| {
                    x.to_bits().to_le_bytes().iter().fold(hash, |h, b| {
                        (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3)
                    })
                })
        })
    })
}

/// Where the vacuum flight of [`digest`] ends, in every component, to the last digit: two
/// platforms that differ in the digest can be set against each other here.
pub fn vacuum_end(seed: u64, ticks: usize) -> State {
    let world = world(Preset::Vacuum);
    let (_, states) = flight(&world, seed, world.spawn(0).state, ticks, false);
    states[states.len() - 1]
}

/// What one policy's `control` took, over the cases.
#[derive(Clone, Debug, PartialEq)]
pub struct Timing {
    pub policy: &'static str,
    pub calls: usize,
    pub p99_us: f64,
    pub worst_us: f64,
    /// Calls that took longer than a tick.
    pub over: usize,
}

/// Fly 12 cases of the suite at 60 Hz with each policy, and time each call of `control`. A
/// clock that is coarse (a browser's) shows in the smallest times.
pub fn timings(steps: usize) -> Vec<Timing> {
    SOLUTIONS
        .iter()
        .map(|solution| {
            let mut times: Vec<f64> = Vec::new();
            for mut case in suite::standard().into_iter().take(12) {
                case.task.params.h = H;
                let mut controller: Box<dyn Controller> = (solution.controller)(&case.task);
                let mut s = case.start;
                for k in 0..steps {
                    let began = Instant::now();
                    let u = controller.control(k, &s);
                    times.push(began.elapsed().as_secs_f64() * 1e6);
                    s = step(&case.task.params, &s, &u);
                    if case.task.reached(&s) {
                        break;
                    }
                }
            }
            times.sort_by(f64::total_cmp);
            let at = |p: f64| times[((times.len() - 1) as f64 * p).round() as usize];
            Timing {
                policy: solution.name,
                calls: times.len(),
                p99_us: at(0.99),
                worst_us: at(1.0),
                over: times.iter().filter(|&&t| t > H * 1e6).count(),
            }
        })
        .collect()
}

/// All of it, as text.
pub fn report(seed: u64, ticks: usize, steps: usize) -> String {
    let mut out = String::new();
    let [vacuum, bodies] = digest(seed, ticks);
    writeln!(
        out,
        "flight digests: vacuum {vacuum:016x}, bodies {bodies:016x}"
    )
    .ok();
    let e = vacuum_end(seed, ticks);
    writeln!(
        out,
        "vacuum flight ends at x {:e} y {:e} vx {:e} vy {:e} theta {:e} omega {:e}",
        e.x, e.y, e.vx, e.vy, e.theta, e.omega
    )
    .ok();
    writeln!(
        out,
        "replay error, worst over windows (position, velocity, heading, spin):"
    )
    .ok();
    for row in replay_table(seed, ticks) {
        for (name, w) in [("free", row.free), ("contact", row.contact)] {
            writeln!(
                out,
                "  L={:<2} {name:<7} {:>5} windows  {:.1e}  {:.1e}  {:.1e}  {:.1e}",
                row.length, w.windows, w.position, w.velocity, w.heading, w.spin
            )
            .ok();
        }
    }
    writeln!(out, "control time per call, at 60 Hz:").ok();
    for t in timings(steps) {
        writeln!(
            out,
            "  {:<22} {:>6} calls  p99 {:>8.1} us  worst {:>10.1} us  over a tick: {}",
            t.policy, t.calls, t.p99_us, t.worst_us, t.over
        )
        .ok();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_replay_table_has_free_and_contact_windows_and_small_errors() {
        let rows = replay_table(1, 1500);
        assert_eq!(rows.len(), LENGTHS.len());
        for row in &rows {
            assert!(row.free.windows > 100, "{row:?}");
            assert!(row.contact.windows > 20, "{row:?}");
            // `tab-replay`: far below one tick of thrust, which is 0.17.
            assert!(row.free.velocity < 0.05, "{row:?}");
            assert!(row.contact.velocity < 0.5, "{row:?}");
        }
    }

    #[test]
    fn a_flight_has_the_same_digest_every_time() {
        assert_eq!(digest(3, 400), digest(3, 400));
        assert_ne!(digest(3, 400), digest(4, 400));
    }
}
