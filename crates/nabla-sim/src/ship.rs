//! One ship's map from tick to tick, `f_i` of `specs/multiplayer.typ`: the command is made safe,
//! becomes a control, the physics advances one tick, and the arena's wall stops the ship.
//!
//! This is the world's map ([`crate::fleet::Dynamics::tick`]) for a ship that is alone in its
//! block, which is every ship of a world whose ships do not collide, and any ship far enough
//! from the others in one whose ships do. It depends on this ship's state, parameters and
//! command, and on the world's constants, and on nothing else: a [`Ship`] keeps no physics from
//! one tick to the next.

use nabla_core::{Control, Params, State};

use crate::command::Command;
use crate::fleet::{Dynamics, Member};
use crate::world::World;

/// `Wall`: stop the ship at the edge of the arena, whose half side is `half_side`, for a hull of
/// radius `radius`.
///
/// Each axis is treated alone. A position past `S' = half_side - radius` goes back to `S'`, and
/// its velocity, if it points outward, to `0`: the wall is plastic and frictionless. The heading
/// and the spin are left alone. It maps every state into the arena, is idempotent, and in
/// position it is the projection on a convex set, so it never increases the distance between two
/// ships.
#[must_use]
pub fn wall(s: State, half_side: f64, radius: f64) -> State {
    let reach = half_side - radius;
    let axis = |p: f64, v: f64| {
        if p > reach {
            (reach, v.min(0.0))
        } else if p < -reach {
            (-reach, v.max(0.0))
        } else {
            (p, v)
        }
    };
    let (x, vx) = axis(s.x, s.vx);
    let (y, vy) = axis(s.y, s.vy);
    State { x, y, vx, vy, ..s }
}

/// One ship, flying alone, advancing a tick at a time under the world's rules.
#[derive(Clone, Debug)]
pub struct Ship {
    dynamics: Dynamics,
    member: Member,
}

impl Ship {
    /// A ship with `params`, which are the ship's own except for `h`, which is the world's, in
    /// `state`, with the assist holding nothing.
    pub fn new(world: &World, params: Params, state: State) -> Self {
        let params = Params {
            h: world.h(),
            ..params
        };
        Self {
            dynamics: Dynamics::new(world),
            member: Member::new(params, state),
        }
    }

    /// The ship put back in state `state`, holding `hold`, as a client does to replay from a
    /// snapshot. Nothing is lost by it: the state and the hold are all a tick reads.
    pub fn restore(&mut self, state: State, hold: Option<f64>) {
        self.member.state = state;
        self.member.hold = hold;
    }

    pub fn state(&self) -> State {
        self.member.state
    }

    pub fn hold(&self) -> Option<f64> {
        self.member.hold
    }

    /// The control of the tick before, to draw the plume.
    pub fn control(&self) -> Control {
        self.member.control
    }

    pub fn params(&self) -> &Params {
        &self.member.params
    }

    /// One tick under `command`: `f(x, c) = (Wall(phi(s, u)), eta')` where `(u, eta')` is the
    /// actuation of the command. The command is sanitized first, so any value at all is safe,
    /// and should its control still not be admissible (it cannot be, for a finite state) the ship
    /// coasts.
    pub fn tick(&mut self, command: &Command) {
        self.dynamics.tick(
            std::slice::from_mut(&mut self.member),
            std::slice::from_ref(command),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::Rng;
    use crate::steering::{Aim, Input, Steering};
    use crate::world::Preset;

    const S: f64 = 1000.0;

    fn arena(preset: Preset) -> World {
        preset.world().multiplayer(60, S, 4).expect("a valid world")
    }

    fn at(x: f64, y: f64) -> State {
        State {
            x,
            y,
            ..State::default()
        }
    }

    #[test]
    fn the_wall_holds_is_idempotent_and_never_pulls_ships_apart() {
        let mut rng = Rng::new(3);
        let state = |rng: &mut Rng| State {
            x: rng.range(-1500.0, 1500.0),
            y: rng.range(-1500.0, 1500.0),
            vx: rng.range(-50.0, 50.0),
            vy: rng.range(-50.0, 50.0),
            theta: rng.range(-9.0, 9.0),
            omega: rng.range(-9.0, 9.0),
        };
        for _ in 0..20_000 {
            let (a, b) = (state(&mut rng), state(&mut rng));
            let (wa, wb) = (wall(a, S, 1.0), wall(b, S, 1.0));
            assert!(wa.x.abs() <= S - 1.0 && wa.y.abs() <= S - 1.0, "{wa:?}");
            assert_eq!(wall(wa, S, 1.0), wa, "idempotent");
            assert_eq!((wa.theta, wa.omega), (a.theta, a.omega), "only moves");
            let apart = |p: &State, q: &State| (p.x - q.x).hypot(p.y - q.y);
            assert!(apart(&wa, &wb) <= apart(&a, &b) + 1e-12, "a projection");
            assert!(
                a.vx * wa.vx >= 0.0 && wa.vx.abs() <= a.vx.abs(),
                "no speed added"
            );
        }
        // Inward velocity is kept at the wall, outward velocity is lost.
        let out = wall(
            State {
                x: 2000.0,
                vx: 5.0,
                ..State::default()
            },
            S,
            1.0,
        );
        assert_eq!((out.x, out.vx), (S - 1.0, 0.0));
        let back = wall(
            State {
                x: 2000.0,
                vx: -5.0,
                ..State::default()
            },
            S,
            1.0,
        );
        assert_eq!((back.x, back.vx), (S - 1.0, -5.0));
    }

    /// The bound of the lemma "The arena holds", for a ship of acceleration `a` that spawned with
    /// speed `v0`, on each axis.
    fn speed_bound(a: f64, h: f64, v0: f64) -> f64 {
        (v0 * v0 + 4.0 * a * (S - 1.0) + a * a * h * h).sqrt()
    }

    #[test]
    fn a_pilot_who_thrusts_across_the_arena_gets_within_a_hair_of_the_speed_bound_and_no_further() {
        let world = arena(Preset::Vacuum);
        let params = world.default_ship();
        let a = params.f_max / params.mass;
        let mut ship = Ship::new(&world, params, at(-(S - 1.0), 0.0));
        let full = Command::Direct {
            thrust: params.f_max,
            torque: 0.0,
        };
        let mut fastest = 0.0_f64;
        for _ in 0..4000 {
            ship.tick(&full);
            fastest = fastest.max(ship.state().vx.abs());
        }
        let bound = speed_bound(a, world.h(), 0.0);
        assert!(fastest <= bound, "{fastest} over {bound}");
        assert!(
            fastest >= 0.99 * bound,
            "{fastest} is too far below {bound}"
        );
    }

    /// A pilot that sends whatever it likes: now holding keys, now garbage, now an aim.
    fn adversary(rng: &mut Rng, params: &Params, ship: &Ship) -> Command {
        let s = ship.state();
        match rng.below(5) {
            0 => Command::Direct {
                thrust: params.f_max,
                torque: rng.range(-1.0, 1.0) * params.tau_max,
            },
            // Point where the ship is going and burn: the fastest way to build up speed.
            1 => Command::Manual {
                input: Input {
                    aim: Some(Aim {
                        heading: s.vy.atan2(s.vx),
                        cone: 0.3,
                    }),
                    ..Input::default()
                },
                steering: Steering::Raw,
            },
            2 => Command::Manual {
                input: Input {
                    thrust: 1.0,
                    turn: rng.range(-1.0, 1.0),
                    ..Input::default()
                },
                steering: Steering::Sas,
            },
            3 => Command::Direct {
                thrust: rng.wild(),
                torque: rng.wild(),
            },
            _ => Command::Manual {
                input: Input {
                    thrust: rng.unit(),
                    turn: rng.range(-1.0, 1.0),
                    fine: rng.chance(0.3),
                    aim: None,
                },
                steering: Steering::Raw,
            },
        }
    }

    #[test]
    fn adversarial_pilots_never_pass_the_speed_bound_of_the_arena_in_a_vacuum() {
        let world = arena(Preset::Vacuum);
        let params = world.default_ship();
        let a = params.f_max / params.mass;
        let mut rng = Rng::new(0xA2E4A);
        let mut closest = 0.0_f64;
        for _ in 0..40 {
            let speed = rng.range(0.0, 20.0);
            let heading = rng.range(-3.0, 3.0);
            let state = State {
                x: rng.range(-(S - 1.0), S - 1.0),
                y: rng.range(-(S - 1.0), S - 1.0),
                vx: speed * heading.cos(),
                vy: speed * heading.sin(),
                theta: heading,
                ..State::default()
            };
            let bound = speed_bound(a, world.h(), speed);
            let mut ship = Ship::new(&world, params, state);
            let mut mode = adversary(&mut rng, &params, &ship);
            for tick in 0..6000 {
                if tick % 40 == 0 {
                    mode = adversary(&mut rng, &params, &ship);
                }
                ship.tick(&mode);
                let s = ship.state();
                assert!(
                    s.x.abs() <= S - 1.0 && s.y.abs() <= S - 1.0,
                    "left the arena: {s:?}"
                );
                for v in [s.vx.abs(), s.vy.abs()] {
                    assert!(v <= bound, "{v} over {bound} at tick {tick}");
                    closest = closest.max(v / bound);
                }
            }
        }
        assert!(closest > 0.3, "the adversaries were not trying: {closest}");
    }

    #[test]
    fn a_ship_among_bodies_still_stops_at_the_wall() {
        let world = arena(Preset::PlanetAndMoon);
        let params = world.default_ship();
        let mut ship = Ship::new(&world, params, at(-15.0, 40.0));
        // The ship's heading is +x, so full thrust sends it out of the arena at the far side.
        let full = Command::Direct {
            thrust: params.f_max,
            torque: 0.0,
        };
        for tick in 0..3000 {
            ship.tick(&full);
            let s = ship.state();
            assert!(
                s.x.abs() <= S - 1.0 && s.y.abs() <= S - 1.0,
                "tick {tick}: {s:?}"
            );
        }
        let s = ship.state();
        assert!((s.x - (S - 1.0)).abs() < 1e-3, "rests at the wall: {s:?}");
        assert!(s.vx.abs() < 1.0, "the wall took the speed: {s:?}");
    }

    #[test]
    fn one_ships_run_depends_on_its_inputs_alone() {
        let world = arena(Preset::PlanetAndMoon);
        let params = world.default_ship();
        let start = world.spawn(0).state;
        let commands: Vec<Command> = {
            let mut rng = Rng::new(11);
            let probe = Ship::new(&world, params, start);
            (0..500)
                .map(|_| adversary(&mut rng, &params, &probe))
                .collect()
        };
        let run = || {
            let mut ship = Ship::new(&world, params, start);
            for c in &commands {
                ship.tick(c);
            }
            ship.state()
        };
        assert_eq!(run(), run(), "the same inputs, the same ship, bit for bit");
    }

    #[test]
    fn restoring_a_ship_resumes_it_exactly_in_a_vacuum_and_among_bodies() {
        // A state and a hold are all there is to a ship, so a replay from any tick of a flight is
        // that flight, to the last bit: on the planet's surface too, where the ship is in contact.
        for (preset, start) in [
            (Preset::Vacuum, None),
            (Preset::PlanetAndMoon, None),
            (Preset::Planet, Some(at(0.0, -3.9))),
        ] {
            let world = arena(preset);
            let params = world.default_ship();
            let start = start.unwrap_or_else(|| world.spawn(0).state);
            let mut rng = Rng::new(5);
            let mut live = Ship::new(&world, params, start);
            let mut replay = Ship::new(&world, params, start);
            let mut command = Command::default();
            for tick in 0..900 {
                if tick % 15 == 0 {
                    command = adversary(&mut rng, &params, &live);
                }
                live.tick(&command);
                if tick % 100 == 0 {
                    replay.restore(live.state(), live.hold());
                } else {
                    replay.tick(&command);
                    assert_eq!(replay.state(), live.state(), "{preset:?} tick {tick}");
                    assert_eq!(replay.hold(), live.hold(), "{preset:?} tick {tick}");
                }
            }
        }
    }

    #[test]
    fn the_control_a_ship_flew_is_admissible_and_fits_a_snapshot() {
        // The control is rounded toward zero to single precision, so that what a snapshot says
        // of it is all of it.
        let world = arena(Preset::Vacuum);
        let params = Params {
            f_max: 0.7,
            tau_max: 0.3,
            ..world.default_ship()
        };
        let mut rng = Rng::new(21);
        let mut ship = Ship::new(&world, params, world.spawn(0).state);
        for _ in 0..20_000 {
            let command = adversary(&mut rng, &params, &ship);
            ship.tick(&command);
            let u = ship.control();
            assert!(u.is_admissible(ship.params()), "{u:?}");
            assert_eq!(f64::from(u.thrust as f32), u.thrust, "{u:?}");
            assert_eq!(f64::from(u.torque as f32), u.torque, "{u:?}");
        }
    }

    #[test]
    #[expect(
        clippy::print_stderr,
        reason = "the measurement is the result: run with --nocapture"
    )]
    fn speeds_among_bodies_are_measured_where_the_arena_lemma_gives_no_bound() {
        // With bodies, a contact can turn motion along one axis into motion along the other, so
        // the lemma's bound has no proof. Adversarial pilots measure what they get.
        for preset in [Preset::Planet, Preset::PlanetAndMoon] {
            let world = arena(preset);
            let params = world.default_ship();
            let a = params.f_max / params.mass;
            let g_max = 9.5;
            let mut rng = Rng::new(0xB0D1E5);
            let mut fastest = 0.0_f64;
            for _ in 0..30 {
                let mut ship = Ship::new(&world, params, world.spawn(0).state);
                let mut mode = adversary(&mut rng, &params, &ship);
                for tick in 0..6000 {
                    if tick % 40 == 0 {
                        mode = adversary(&mut rng, &params, &ship);
                    }
                    ship.tick(&mode);
                    let s = ship.state();
                    fastest = fastest.max(s.vx.abs()).max(s.vy.abs());
                }
            }
            // And the run that is fastest in a vacuum, along a line that no body is on.
            let mut ship = Ship::new(&world, params, at(-(S - 1.0), 40.0));
            let full = Command::Direct {
                thrust: params.f_max,
                torque: 0.0,
            };
            for _ in 0..4000 {
                ship.tick(&full);
                fastest = fastest.max(ship.state().vx.abs());
            }
            let vacuum_bound = speed_bound(a, world.h(), 6.0);
            let with_gravity = speed_bound(a + g_max, world.h(), 6.0);
            eprintln!(
                "{preset:?}: fastest axis speed {fastest:.1}; the vacuum bound is {vacuum_bound:.1}, with gravity {with_gravity:.1}"
            );
            assert!(fastest <= with_gravity, "{fastest} over {with_gravity}");
        }
    }
}
