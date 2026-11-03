//! One ship's map from tick to tick, `f_i` of `docs/multiplayer.typ`: the command is made safe,
//! becomes a control, the physics advances one tick, and the arena's wall stops the ship.
//!
//! The map depends on this ship's state, parameters and command, and on the world's constants,
//! and on nothing about the other ships. That is the whole content of Phase A: a server runs one
//! [`Ship`] for each pilot, and a client runs a copy of its own to predict.

use nabla_core::{Control, Params, State, step};

use crate::command::Command;
use crate::physics::Physics;
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

/// One ship, advancing a tick at a time under the world's rules.
pub struct Ship {
    params: Params,
    state: State,
    /// The heading the stability assist holds: the part of the ship's extended state that is not
    /// physics.
    hold: Option<f64>,
    /// The control of the tick before, to draw the plume.
    control: Control,
    /// Rapier, in a world with bodies. In a vacuum the exact model steps the ship instead.
    physics: Option<Physics>,
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
            params,
            state,
            hold: None,
            control: Control::default(),
            physics: Self::physics_in(world, &params, state),
        }
    }

    fn physics_in(world: &World, params: &Params, state: State) -> Option<Physics> {
        (!world.bodies.is_empty())
            .then(|| Physics::for_ship(world, state, params, world.hull_radius()))
    }

    /// The ship put back in state `state`, holding `hold`, as a client does to replay from a
    /// snapshot. Rapier's contact cache, which a state does not have, starts afresh.
    pub fn restore(&mut self, world: &World, state: State, hold: Option<f64>) {
        self.state = state;
        self.hold = hold;
        self.physics = Self::physics_in(world, &self.params, state);
    }

    pub fn state(&self) -> State {
        self.state
    }

    pub fn hold(&self) -> Option<f64> {
        self.hold
    }

    pub fn control(&self) -> Control {
        self.control
    }

    pub fn params(&self) -> &Params {
        &self.params
    }

    /// One tick under `command`: `f(x, c) = (Wall(phi(s, u)), eta')` where `(u, eta')` is the
    /// actuation of the command. The command is sanitized first, so any value at all is safe,
    /// and should its control still not be admissible (it cannot be, for a finite state) the ship
    /// coasts.
    pub fn tick(&mut self, world: &World, command: &Command) {
        let command = command.sanitize(&self.params);
        let (mut u, hold) = command.actuate(&self.params, &self.state, self.hold);
        if !u.is_admissible(&self.params) {
            u = Control::default();
        }
        let moved = match &mut self.physics {
            Some(physics) => physics
                .step(self.params.h, u)
                .pop()
                .expect("a step has samples"),
            None => step(&self.params, &self.state, &u),
        };
        let walled = world.arena.map_or(moved, |arena| {
            wall(moved, arena.half_side, f64::from(world.hull_radius()))
        });
        if walled != moved
            && let Some(physics) = &mut self.physics
        {
            physics.set_state(&walled);
        }
        self.state = walled;
        self.hold = hold;
        self.control = u;
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
            ship.tick(&world, &full);
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
                ship.tick(&world, &mode);
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
            ship.tick(&world, &full);
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
                ship.tick(&world, c);
            }
            ship.state()
        };
        assert_eq!(run(), run(), "the same inputs, the same ship, bit for bit");
    }

    #[test]
    fn restoring_a_vacuum_ship_resumes_it_exactly() {
        let world = arena(Preset::Vacuum);
        let params = world.default_ship();
        let mut rng = Rng::new(5);
        let mut live = Ship::new(&world, params, world.spawn(0).state);
        let mut replay = Ship::new(&world, params, world.spawn(0).state);
        for tick in 0..300 {
            let c = adversary(&mut rng, &params, &live);
            live.tick(&world, &c);
            if tick == 100 {
                replay.restore(&world, live.state(), live.hold());
            } else if tick > 100 {
                replay.tick(&world, &c);
            }
        }
        assert_eq!(replay.state(), live.state());
        assert_eq!(replay.hold(), live.hold());
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
                    ship.tick(&world, &mode);
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
                ship.tick(&world, &full);
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

    #[test]
    fn a_tick_of_60_hz_is_two_substeps() {
        assert_eq!((1.0 / 60.0 * 120.0_f64).ceil(), 2.0);
        assert_eq!(
            (120.0 * (1.0 / 60.0_f64)).ceil(),
            2.0,
            "as the paper computes it"
        );
    }
}
