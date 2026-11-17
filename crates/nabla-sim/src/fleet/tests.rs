//! The properties `specs/multiplayer.typ` asks of a world whose ships collide ("Ship
//! collisions"), each over worlds, ships and commands drawn from a seed, so that a failure names
//! the seed that reproduces it.

use nabla_core::{Control, Params, State};

use super::*;
use crate::rng::Rng;
use crate::ship::Ship;
use crate::steering::{Aim, Input, Steering};
use crate::world::{Preset, ShipRequest};

const S: f64 = 1000.0;

fn arena(preset: Preset) -> World {
    preset.world().multiplayer(60, S, 4).expect("a valid world")
}

/// A ship the world allows, of any mass and strength.
fn any_ship(rng: &mut Rng, world: &World) -> Params {
    if rng.chance(0.4) {
        return world.default_ship();
    }
    world.grant(&ShipRequest {
        mass: rng.range(0.1, 10.0),
        inertia: rng.range(0.1, 10.0),
        f_max: rng.range(0.5, 20.0),
        tau_max: rng.range(0.2, 20.0),
    })
}

/// A place for a group of ships that is clear of the bundled worlds' bodies.
fn open_place(rng: &mut Rng) -> (f64, f64) {
    (rng.range(100.0, 800.0), rng.range(100.0, 800.0))
}

/// `n` ships within `spread` of `center`, moving at up to `speed` and spinning at up to `spin`.
fn group(
    rng: &mut Rng,
    world: &World,
    n: usize,
    center: (f64, f64),
    spread: f64,
    speed: f64,
    spin: f64,
) -> Vec<Member> {
    (0..n)
        .map(|_| {
            let (v, course) = (rng.range(0.0, speed), rng.range(-3.2, 3.2));
            Member::new(
                any_ship(rng, world),
                State {
                    x: center.0 + rng.range(-spread, spread),
                    y: center.1 + rng.range(-spread, spread),
                    vx: v * f64::cos(course),
                    vy: v * f64::sin(course),
                    theta: rng.range(-10.0, 10.0),
                    omega: rng.range(-spin, spin),
                },
            )
        })
        .collect()
}

/// A world's worth of ships: a few groups, some crowded and some not, and some ships alone.
fn fleet(rng: &mut Rng, world: &World) -> Vec<Member> {
    let mut ships = Vec::new();
    for _ in 0..1 + rng.below(4) {
        let n = 1 + rng.below(5) as usize;
        let spread = [1.0, 3.0, 8.0, 30.0][rng.below(4) as usize];
        let speed = [0.0, 5.0, 40.0, 150.0][rng.below(4) as usize];
        let spin = [0.0, 3.0, 30.0][rng.below(3) as usize];
        let center = open_place(rng);
        ships.extend(group(rng, world, n, center, spread, speed, spin));
    }
    ships.truncate(16);
    ships
}

/// What a pilot might hold, or send to break things.
fn any_command(rng: &mut Rng, params: &Params) -> Command {
    match rng.below(6) {
        0 => Command::default(),
        1 => Command::Direct {
            thrust: params.f_max,
            torque: rng.range(-1.0, 1.0) * params.tau_max,
        },
        2 => Command::Manual {
            input: Input {
                thrust: 1.0,
                turn: [-1.0, 0.0, 1.0][rng.below(3) as usize],
                fine: rng.chance(0.2),
                aim: None,
            },
            steering: Steering::Sas,
        },
        3 => Command::Manual {
            input: Input {
                aim: Some(Aim {
                    heading: rng.range(-3.1, 3.1),
                    cone: 0.5,
                }),
                ..Input::default()
            },
            steering: Steering::Raw,
        },
        4 => Command::Direct {
            thrust: rng.wild(),
            torque: rng.wild(),
        },
        _ => Command::Manual {
            input: Input {
                thrust: rng.unit(),
                turn: rng.range(-1.0, 1.0),
                fine: false,
                aim: None,
            },
            steering: Steering::Raw,
        },
    }
}

fn commands_for(rng: &mut Rng, ships: &[Member]) -> Vec<Command> {
    ships
        .iter()
        .map(|ship| any_command(rng, &ship.params))
        .collect()
}

fn apart(a: &Member, b: &Member) -> f64 {
    (a.state.x - b.state.x).hypot(a.state.y - b.state.y)
}

fn momentum(ships: &[Member]) -> (f64, f64) {
    ships.iter().fold((0.0, 0.0), |p, ship| {
        (
            p.0 + ship.params.mass * ship.state.vx,
            p.1 + ship.params.mass * ship.state.vy,
        )
    })
}

/// The angular momentum about the origin: of the ships' motion, and of their spin.
fn angular_momentum(ships: &[Member]) -> f64 {
    ships
        .iter()
        .map(|ship| {
            let (s, p) = (&ship.state, &ship.params);
            p.mass * (s.x * s.vy - s.y * s.vx) + p.inertia * s.omega
        })
        .sum()
}

fn energy(ships: &[Member]) -> f64 {
    ships
        .iter()
        .map(|ship| {
            let (s, p) = (&ship.state, &ship.params);
            0.5 * p.mass * (s.vx * s.vx + s.vy * s.vy) + 0.5 * p.inertia * s.omega * s.omega
        })
        .sum()
}

fn finite(ship: &Member) -> bool {
    let s = &ship.state;
    [s.x, s.y, s.vx, s.vy, s.theta, s.omega]
        .iter()
        .all(|x| x.is_finite())
        && ship.control.thrust.is_finite()
        && ship.control.torque.is_finite()
        && ship.hold.is_none_or(f64::is_finite)
}

/// B1. A tick is a function of the world and the commands: the same again gives the same, to the
/// last bit, with a Rapier world made anew each time.
#[test]
fn a_tick_is_a_function_of_the_world_and_the_commands() {
    for preset in Preset::ALL {
        let world = arena(preset);
        let dynamics = Dynamics::new(&world);
        for seed in 0..60 {
            let mut rng = Rng::fork(0xB1, seed);
            let mut ships = fleet(&mut rng, &world);
            for tick in 0..120 {
                let commands = commands_for(&mut rng, &ships);
                let mut again = ships.clone();
                let report = dynamics.tick(&mut ships, &commands);
                let report_again = Dynamics::new(&world).tick(&mut again, &commands);
                assert_eq!(ships, again, "{preset:?} seed {seed} tick {tick}");
                assert_eq!(report, report_again, "{preset:?} seed {seed} tick {tick}");
            }
        }
    }
}

/// B2. Whatever the commands, and however the ships are piled up, a tick gives finite states
/// inside the arena.
#[test]
fn any_commands_and_any_pile_of_ships_give_finite_states_inside_the_arena() {
    for preset in Preset::ALL {
        let world = arena(preset);
        let dynamics = Dynamics::new(&world);
        for seed in 0..40 {
            let mut rng = Rng::fork(0xB2, seed);
            let n = 2 + rng.below(15) as usize;
            // In the open, on a wall, or in a corner; spread out a little, or all on one point.
            let center = match rng.below(3) {
                0 => open_place(&mut rng),
                1 => (S - 1.0, rng.range(-500.0, 500.0)),
                _ => (-(S - 1.0), S - 1.0),
            };
            let spread = if rng.chance(0.4) { 0.0 } else { 1.5 };
            let mut ships = group(&mut rng, &world, n, center, spread, 60.0, 20.0);
            if rng.chance(0.3) {
                // The same place, heading and all.
                let first = ships[0].state;
                for ship in &mut ships {
                    ship.state = first;
                }
            }
            for tick in 0..400 {
                let commands: Vec<Command> = ships
                    .iter()
                    .map(|ship| {
                        if rng.chance(0.5) {
                            // Push outward, into the wall and each other.
                            Command::Direct {
                                thrust: ship.params.f_max,
                                torque: 0.0,
                            }
                        } else {
                            any_command(&mut rng, &ship.params)
                        }
                    })
                    .collect();
                dynamics.tick(&mut ships, &commands);
                for ship in &ships {
                    assert!(finite(ship), "{preset:?} seed {seed} tick {tick}: {ship:?}");
                    let s = &ship.state;
                    assert!(
                        s.x.abs() <= S - 1.0 && s.y.abs() <= S - 1.0,
                        "{preset:?} seed {seed} tick {tick}: left the arena: {s:?}"
                    );
                    assert!(ship.control.is_admissible(&ship.params), "{ship:?}");
                }
            }
        }
    }
}

/// The partition is the finest one that keeps near ships together: no two near ships are in
/// different blocks, every block holds together by its own ships being near, the names of the
/// ships do not matter, and a longer horizon only merges blocks.
#[test]
fn the_partition_is_the_finest_that_keeps_near_ships_together() {
    let world = arena(Preset::PlanetAndMoon);
    let dynamics = Dynamics::new(&world);
    for seed in 0..2000 {
        let mut rng = Rng::fork(0xB3, seed);
        let ships = fleet(&mut rng, &world);
        let ticks = 1 + rng.below(12) as u32;
        let blocks = dynamics.blocks(&ships, ticks);
        let n = ships.len();
        let mut block_of = vec![usize::MAX; n];
        for (b, block) in blocks.iter().enumerate() {
            assert!(block.is_sorted(), "seed {seed}: {blocks:?}");
            for &i in block {
                assert_eq!(block_of[i], usize::MAX, "seed {seed}: {i} is in two blocks");
                block_of[i] = b;
            }
        }
        assert!(block_of.iter().all(|&b| b != usize::MAX), "seed {seed}");
        assert!(blocks.is_sorted_by_key(|block| block[0]), "seed {seed}");
        let reach: Vec<f64> = blocks
            .iter()
            .map(|block| dynamics.reach(&ships, block, ticks))
            .collect();
        let near = |i: usize, j: usize| {
            apart(&ships[i], &ships[j])
                <= 2.0 * dynamics.radius()
                    + dynamics.skin()
                    + reach[block_of[i]]
                    + reach[block_of[j]]
        };
        // Closed: near ships share a block.
        for i in 0..n {
            for j in 0..n {
                if block_of[i] != block_of[j] {
                    assert!(!near(i, j), "seed {seed}: {i} and {j} are near, and apart");
                }
            }
        }
        // The finest: each block is connected by its near pairs.
        for block in &blocks {
            let mut reached = vec![block[0]];
            let mut frontier = vec![block[0]];
            while let Some(i) = frontier.pop() {
                for &j in block {
                    if !reached.contains(&j) && near(i, j) {
                        reached.push(j);
                        frontier.push(j);
                    }
                }
            }
            assert_eq!(
                reached.len(),
                block.len(),
                "seed {seed}: {block:?} falls apart"
            );
        }
        // The names do not matter: the ships in another order fall into the same sets.
        let mut order: Vec<usize> = (0..n).collect();
        for i in (1..n).rev() {
            order.swap(i, rng.below(i as u64 + 1) as usize);
        }
        let shuffled: Vec<Member> = order.iter().map(|&i| ships[i]).collect();
        let mut renamed: Vec<Vec<usize>> = dynamics
            .blocks(&shuffled, ticks)
            .into_iter()
            .map(|block| {
                let mut block: Vec<usize> = block.into_iter().map(|i| order[i]).collect();
                block.sort_unstable();
                block
            })
            .collect();
        renamed.sort();
        assert_eq!(renamed, blocks, "seed {seed}");
        // A longer horizon only merges.
        let longer = dynamics.blocks(&ships, ticks + 1 + rng.below(6) as u32);
        for block in &blocks {
            assert!(
                longer
                    .iter()
                    .any(|wider| block.iter().all(|i| wider.contains(i))),
                "seed {seed}: {block:?} is split by a longer horizon"
            );
        }
    }
}

/// B3. Locality: a part of the world that is a union of blocks gets the same states, to the last
/// bit, whether the rest of the world is there or not. A ship alone in its block is the least
/// such part, and flies as [`Ship`] does.
#[test]
fn a_union_of_blocks_steps_as_the_whole_world_does() {
    let mut several = 0;
    for preset in Preset::ALL {
        let world = arena(preset);
        let dynamics = Dynamics::new(&world);
        for seed in 0..80 {
            let mut rng = Rng::fork(0xB4, seed);
            let mut ships = fleet(&mut rng, &world);
            for tick in 0..90 {
                let commands = commands_for(&mut rng, &ships);
                let blocks = dynamics.blocks(&ships, 1);
                let mut part: Vec<usize> = blocks
                    .iter()
                    .filter(|_| rng.chance(0.5))
                    .flatten()
                    .copied()
                    .collect();
                part.sort_unstable();
                several += usize::from(blocks.iter().any(|block| block.len() > 1));
                let mut alone: Vec<Member> = part.iter().map(|&i| ships[i]).collect();
                let commands_alone: Vec<Command> = part.iter().map(|&i| commands[i]).collect();
                let lone = blocks.iter().find(|block| block.len() == 1).map(|b| b[0]);
                let flown_alone = lone.map(|i| {
                    let mut ship = Ship::new(&world, ships[i].params, ships[i].state);
                    ship.restore(ships[i].state, ships[i].hold);
                    ship.tick(&commands[i]);
                    (ship.state(), ship.hold(), ship.control())
                });
                dynamics.tick(&mut alone, &commands_alone);
                dynamics.tick(&mut ships, &commands);
                for (member, &i) in alone.iter().zip(&part) {
                    assert_eq!(*member, ships[i], "{preset:?} seed {seed} tick {tick}");
                }
                if let (Some(i), Some(flown)) = (lone, flown_alone) {
                    assert_eq!(
                        flown,
                        (ships[i].state, ships[i].hold, ships[i].control),
                        "{preset:?} seed {seed} tick {tick}: a ship alone is a ship of Phase A"
                    );
                }
            }
        }
    }
    assert!(several > 1000, "blocks of several were stepped: {several}");
}

/// Where ships do not collide, the world's map is each ship's own: Phase A.
#[test]
fn a_world_without_ship_collisions_is_a_product_of_ships() {
    for preset in Preset::ALL {
        let world = World {
            ship_collisions: Some(false),
            ..arena(preset)
        };
        let dynamics = Dynamics::new(&world);
        assert!(!dynamics.collide());
        for seed in 0..20 {
            let mut rng = Rng::fork(0xA, seed);
            let center = open_place(&mut rng);
            let mut ships = group(&mut rng, &world, 6, center, 1.0, 20.0, 5.0);
            let mut each: Vec<Ship> = ships
                .iter()
                .map(|ship| Ship::new(&world, ship.params, ship.state))
                .collect();
            for tick in 0..200 {
                let commands = commands_for(&mut rng, &ships);
                let report = dynamics.tick(&mut ships, &commands);
                assert!(report.blocks.iter().all(|block| block.len() == 1));
                assert!(report.touches.is_empty());
                for ((ship, flown), command) in each.iter_mut().zip(&ships).zip(&commands) {
                    ship.tick(command);
                    assert_eq!(
                        ship.state(),
                        flown.state,
                        "{preset:?} seed {seed} tick {tick}"
                    );
                }
            }
        }
    }
}

/// B4. Ships of different blocks do not touch in the tick: they end it more than two hull radii
/// apart, crowded and hard-flown as they may be. This is proved only for ships that nothing
/// touches, so here it is measured.
#[test]
fn ships_of_different_blocks_end_the_tick_apart() {
    let mut pairs = 0_u64;
    let mut closest = f64::INFINITY;
    for preset in Preset::ALL {
        let world = arena(preset);
        let dynamics = Dynamics::new(&world);
        for seed in 0..120 {
            let mut rng = Rng::fork(0xB5, seed);
            // Several groups thrown at one place, so that blocks form, break and meet.
            let target = open_place(&mut rng);
            let mut ships = Vec::new();
            for _ in 0..2 + rng.below(4) {
                let n = 1 + rng.below(4) as usize;
                let from = (
                    target.0 + rng.range(-25.0, 25.0),
                    target.1 + rng.range(-25.0, 25.0),
                );
                let speed = rng.range(0.0, 120.0);
                let mut some = group(&mut rng, &world, n, from, 2.5, 3.0, 40.0);
                let away = (target.0 - from.0).hypot(target.1 - from.1).max(1e-9);
                for ship in &mut some {
                    ship.state.vx += speed * (target.0 - from.0) / away;
                    ship.state.vy += speed * (target.1 - from.1) / away;
                }
                ships.extend(some);
            }
            ships.truncate(16);
            for tick in 0..150 {
                let commands = commands_for(&mut rng, &ships);
                let report = dynamics.tick(&mut ships, &commands);
                for (a, first) in report.blocks.iter().enumerate() {
                    for second in &report.blocks[a + 1..] {
                        for &i in first {
                            for &j in second {
                                let gap = apart(&ships[i], &ships[j]) - 2.0 * dynamics.radius();
                                pairs += 1;
                                closest = closest.min(gap);
                                assert!(
                                    gap > 0.0,
                                    "{preset:?} seed {seed} tick {tick}: {i} and {j} of different blocks are {gap} apart"
                                );
                            }
                        }
                    }
                }
            }
        }
    }
    assert!(pairs > 100_000, "{pairs} pairs");
    assert!(closest < 1.0, "the ships were not crowded: {closest}");
}

/// B5 and B6. Away from bodies and walls, and coasting, a collision keeps the momentum of the
/// ships, linear and angular, and makes no energy.
#[test]
fn a_collision_keeps_the_momentum_of_its_ships_and_makes_no_energy() {
    let world = arena(Preset::Vacuum);
    let dynamics = Dynamics::new(&world);
    let mut collisions = 0;
    let mut worst = (0.0_f64, 0.0_f64, 0.0_f64);
    for seed in 0..400 {
        let mut rng = Rng::fork(0xB6, seed);
        let n = 2 + rng.below(4) as usize;
        let speed = [3.0, 20.0, 100.0][rng.below(3) as usize];
        let center = (rng.range(-200.0, 200.0), rng.range(-200.0, 200.0));
        let mut ships = group(&mut rng, &world, n, center, 6.0, 0.0, 6.0);
        // Thrown at the middle.
        for ship in &mut ships {
            let (dx, dy) = (center.0 - ship.state.x, center.1 - ship.state.y);
            let away = dx.hypot(dy).max(1e-9);
            let v = rng.range(0.3, 1.0) * speed;
            ship.state.vx = v * dx / away;
            ship.state.vy = v * dy / away;
        }
        let coast = vec![Command::default(); n];
        let scale = ships
            .iter()
            .map(|ship| ship.params.mass * ship.state.vx.hypot(ship.state.vy))
            .sum::<f64>()
            .max(1.0);
        let reach = ships
            .iter()
            .map(|ship| ship.state.x.hypot(ship.state.y))
            .fold(0.0, f64::max)
            + 50.0;
        let (p0, l0) = (momentum(&ships), angular_momentum(&ships));
        let mut e = energy(&ships);
        let e0 = e.max(1e-9);
        let mut touched = false;
        for tick in 0..240 {
            let report = dynamics.tick(&mut ships, &coast);
            touched |= !report.touches.is_empty();
            let now = energy(&ships);
            assert!(
                now <= e + 1e-4 * e0,
                "seed {seed} tick {tick}: energy rose from {e} to {now}"
            );
            worst.2 = worst.2.max((now - e) / e0);
            e = now;
        }
        collisions += usize::from(touched);
        let (p1, l1) = (momentum(&ships), angular_momentum(&ships));
        let dp = (p1.0 - p0.0).hypot(p1.1 - p0.1) / scale;
        let dl = (l1 - l0).abs() / (scale * reach);
        worst = (worst.0.max(dp), worst.1.max(dl), worst.2);
        assert!(dp < 1e-4, "seed {seed}: momentum moved by {dp} of its size");
        assert!(dl < 1e-3, "seed {seed}: angular momentum moved by {dl}");
    }
    assert!(collisions > 200, "{collisions} flights with a collision");
    assert!(worst.0 > 0.0, "single precision shows: {worst:?}");
}

/// B7. Two ships on a collision course never pass through each other, however fast they close:
/// a block takes the substeps its speed calls for.
#[test]
fn two_ships_on_a_collision_course_never_pass_through_each_other() {
    let world = arena(Preset::Vacuum);
    let dynamics = Dynamics::new(&world);
    let mut most_substeps = 0;
    for seed in 0..3000 {
        let mut rng = Rng::fork(0xB7, seed);
        let (params_a, params_b) = (any_ship(&mut rng, &world), any_ship(&mut rng, &world));
        // The closing speed is at most 520, which the substeps cover with room for the spin.
        let (speed_a, speed_b) = (rng.range(0.0, 260.0), rng.range(0.5, 260.0));
        let course = rng.range(-3.2, 3.2);
        let (ux, uy) = (f64::cos(course), f64::sin(course));
        // Both hulls hold a disc of a quarter of the hull radius about the center, so centers
        // that would pass within less than half of it must hit.
        let miss = rng.range(-0.4, 0.4);
        let gap = rng.range(3.0, 12.0);
        let spin = [0.0, 2.0, 12.0][rng.below(3) as usize];
        let mut ships = vec![
            Member::new(
                params_a,
                State {
                    x: -gap * ux,
                    y: -gap * uy,
                    vx: speed_a * ux,
                    vy: speed_a * uy,
                    theta: rng.range(-3.2, 3.2),
                    omega: rng.range(-spin, spin),
                },
            ),
            Member::new(
                params_b,
                State {
                    x: gap * ux - miss * uy,
                    y: gap * uy + miss * ux,
                    vx: -speed_b * ux,
                    vy: -speed_b * uy,
                    theta: rng.range(-3.2, 3.2),
                    omega: rng.range(-spin, spin),
                },
            ),
        ];
        let before = (ships[0].state, ships[1].state);
        let coast = [Command::default(), Command::default()];
        let mut touched = false;
        let ticks = (4.0 * gap / ((speed_a + speed_b) * dynamics.h())).ceil() as usize + 5;
        for _ in 0..ticks.min(4000) {
            let block: Vec<usize> = vec![0, 1];
            if dynamics.blocks(&ships, 1).len() == 1 {
                most_substeps = most_substeps.max(dynamics.substeps_of(&ships, &block));
            }
            touched |= !dynamics.tick(&mut ships, &coast).touches.is_empty();
        }
        let changed = |was: &State, is: &State| (is.vx - was.vx).hypot(is.vy - was.vy);
        assert!(
            touched,
            "seed {seed}: closing at {} they never touched",
            speed_a + speed_b
        );
        assert!(
            changed(&before.0, &ships[0].state) + changed(&before.1, &ships[1].state) > 0.0,
            "seed {seed}: they passed through each other untouched"
        );
    }
    assert!(
        most_substeps > 20,
        "fast blocks were stepped finely: {most_substeps}"
    );
}

/// The shove bound: a ship that another runs into, in the open and coasting, ends up moving
/// against the pair's center of mass no faster than the energy of the encounter allows, which
/// for a light ship hitting a heavy one is little. It follows from B5 and B6.
#[test]
fn no_ship_is_shoved_harder_than_the_energy_of_the_encounter_allows() {
    let world = arena(Preset::Vacuum);
    let dynamics = Dynamics::new(&world);
    let mut tightest = 0.0_f64;
    for seed in 0..1500 {
        let mut rng = Rng::fork(0xB8, seed);
        let (params_a, params_b) = (any_ship(&mut rng, &world), any_ship(&mut rng, &world));
        let closing = rng.range(1.0, 150.0);
        let course = rng.range(-3.2, 3.2);
        let (ux, uy) = (f64::cos(course), f64::sin(course));
        let miss = rng.range(-1.2, 1.2);
        let spin = [0.0, 3.0][rng.below(2) as usize];
        let mut ships = vec![
            Member::new(
                params_a,
                State {
                    x: -4.0 * ux,
                    y: -4.0 * uy,
                    vx: closing * ux,
                    vy: closing * uy,
                    theta: rng.range(-3.2, 3.2),
                    omega: rng.range(-spin, spin),
                },
            ),
            Member::new(
                params_b,
                State {
                    x: 4.0 * ux - miss * uy,
                    y: 4.0 * uy + miss * ux,
                    theta: rng.range(-3.2, 3.2),
                    omega: rng.range(-spin, spin),
                    ..State::default()
                },
            ),
        ];
        let total = params_a.mass + params_b.mass;
        let (px, py) = momentum(&ships);
        let (cx, cy) = (px / total, py / total);
        // The energy in the frame of the center of mass, which a collision can only lower.
        let inner = energy(&ships) - 0.5 * total * (cx * cx + cy * cy);
        let coast = [Command::default(), Command::default()];
        for _ in 0..(16.0 / (closing * dynamics.h())).ceil() as usize + 20 {
            dynamics.tick(&mut ships, &coast);
            for ship in &ships {
                let against = (ship.state.vx - cx).hypot(ship.state.vy - cy);
                let allowed = (2.0 * inner / ship.params.mass).sqrt();
                tightest = tightest.max(against / allowed);
                assert!(
                    against <= allowed * (1.0 + 1e-3) + 1e-6,
                    "seed {seed}: shoved to {against}, past {allowed}"
                );
            }
        }
    }
    assert!(tightest > 0.9, "the bound is met: {tightest}");
}

/// `Join` moves no one: a ship spawns alone in its block, inside the arena and clear of the
/// bodies, and the ships already flying take their next tick as if it had not come.
#[test]
fn a_ship_joins_where_it_moves_no_one() {
    for preset in Preset::ALL {
        let world = arena(preset);
        let dynamics = Dynamics::new(&world);
        for seed in 0..60 {
            let mut rng = Rng::fork(0xB9, seed);
            let mut ships: Vec<Member> = Vec::new();
            for index in 0..16 {
                let params = any_ship(&mut rng, &world);
                let state = dynamics
                    .spawn(&world, &ships, index, params)
                    .expect("there is room for sixteen");
                assert!(dynamics.is_clear(&ships, params, state));
                for other in &ships {
                    let gap = (other.state.x - state.x).hypot(other.state.y - state.y);
                    assert!(
                        gap > 2.0 * dynamics.radius(),
                        "{preset:?} seed {seed}: ship {index} spawned {gap} from another"
                    );
                }
                let commands = commands_for(&mut rng, &ships);
                let mut without = ships.clone();
                dynamics.tick(&mut without, &commands);
                ships.push(Member::new(params, state));
                let mut with = ships.clone();
                let mut all = commands.clone();
                all.push(Command::default());
                dynamics.tick(&mut with, &all);
                assert_eq!(
                    with[..without.len()],
                    without[..],
                    "{preset:?} seed {seed}: ship {index} moved someone by joining"
                );
                // The ones flying move on before the next one comes, or sit still.
                if rng.chance(0.5) {
                    for _ in 0..rng.below(40) {
                        let commands = commands_for(&mut rng, &ships);
                        dynamics.tick(&mut ships, &commands);
                    }
                }
            }
        }
    }
}

/// With no room, a ship does not join: every place is taken.
#[test]
fn a_world_too_small_for_another_ship_has_no_spawn_for_it() {
    let mut alone = Preset::Vacuum.world();
    alone.players[0].state = State::default();
    let world = alone.multiplayer(60, 2.5, 1).expect("a valid world");
    let dynamics = Dynamics::new(&world);
    let params = world.default_ship();
    let first = dynamics
        .spawn(&world, &[], 0, params)
        .expect("an empty world has room");
    assert_eq!(
        first,
        world.spawn(0).state,
        "the template itself, when it is clear"
    );
    let ships = [Member::new(params, first)];
    assert_eq!(dynamics.spawn(&world, &ships, 1, params), None);
    // And where ships do not collide, they share the template.
    let apart = World {
        ship_collisions: Some(false),
        ..world.clone()
    };
    assert_eq!(
        Dynamics::new(&apart).spawn(&apart, &ships, 1, params),
        Some(first)
    );
}

/// The horizon lemma: for as long as nothing has touched anything, the blocks a tick steps by
/// lie within the blocks the partition foresaw for that many ticks.
#[test]
fn within_the_horizon_no_block_forms_that_the_partition_did_not_foresee() {
    let mut checked = 0;
    let mut merged = 0;
    for preset in Preset::ALL {
        let world = arena(preset);
        let dynamics = Dynamics::new(&world);
        for seed in 0..400 {
            let mut rng = Rng::fork(0xBA, seed);
            let center = open_place(&mut rng);
            let n = 2 + rng.below(6) as usize;
            let spread = rng.range(3.0, 40.0);
            let speed = rng.range(0.0, 60.0);
            let mut ships = group(&mut rng, &world, n, center, spread, speed, 10.0);
            let horizon = 1 + rng.below(30) as u32;
            let foreseen = dynamics.blocks(&ships, horizon);
            for tick in 0..horizon {
                let stepped = dynamics.blocks(&ships, 1);
                for block in &stepped {
                    checked += 1;
                    merged += usize::from(block.len() > 1);
                    assert!(
                        foreseen
                            .iter()
                            .any(|wider| block.iter().all(|i| wider.contains(i))),
                        "{preset:?} seed {seed}: at tick {tick} of {horizon}, {block:?} is not within {foreseen:?}"
                    );
                }
                let commands = commands_for(&mut rng, &ships);
                let report = dynamics.tick(&mut ships, &commands);
                if !report.touches.is_empty() || !report.grounded.is_empty() {
                    // The lemma says nothing past a contact.
                    break;
                }
            }
        }
    }
    assert!(
        checked > 10_000 && merged > 500,
        "{checked} blocks, {merged} of several"
    );
}

/// The order of two ships' names is the order the solver takes them in, and the outcome of a
/// collision depends on it: both orders give a collision that keeps the laws above, and not the
/// same one. What it must not do is favor a name. Over many collisions, the ship named first is
/// shoved as hard as the same ship named second, to within a thousandth of the closing speed.
#[test]
fn the_order_of_names_gives_no_ship_an_advantage() {
    let world = arena(Preset::Vacuum);
    let dynamics = Dynamics::new(&world);
    let mut bias: Vec<f64> = Vec::new();
    let mut differed = 0;
    for seed in 0..2000 {
        let mut rng = Rng::fork(0xBB, seed);
        let closing = rng.range(1.0, 60.0);
        let course = rng.range(-3.2, 3.2);
        let (ux, uy) = (f64::cos(course), f64::sin(course));
        let miss = rng.range(-0.4, 0.4);
        let coming = (closing * ux, closing * uy);
        let mut ships = vec![
            Member::new(
                any_ship(&mut rng, &world),
                State {
                    x: 100.0 - 3.0 * ux,
                    y: 100.0 - 3.0 * uy,
                    vx: coming.0,
                    vy: coming.1,
                    theta: rng.range(-3.2, 3.2),
                    omega: rng.range(-2.0, 2.0),
                },
            ),
            Member::new(
                any_ship(&mut rng, &world),
                State {
                    x: 100.0 + 3.0 * ux - miss * uy,
                    y: 100.0 + 3.0 * uy + miss * ux,
                    theta: rng.range(-3.2, 3.2),
                    omega: rng.range(-2.0, 2.0),
                    ..State::default()
                },
            ),
        ];
        let mut renamed = vec![ships[1], ships[0]];
        let coast = [Command::default(), Command::default()];
        // To the end of the first tick in which they touch: a collision is not Lipschitz (the
        // proposition on grazing), so the two orders drift apart after it.
        for _ in 0..2000 {
            let report = dynamics.tick(&mut ships, &coast);
            dynamics.tick(&mut renamed, &coast);
            if !report.touches.is_empty() {
                break;
            }
        }
        let shoved =
            |ship: &Member| (ship.state.vx - coming.0).hypot(ship.state.vy - coming.1) / closing;
        bias.push(shoved(&ships[0]) - shoved(&renamed[1]));
        differed += usize::from(ships[0].state != renamed[1].state);
    }
    let count = bias.len() as f64;
    let mean = bias.iter().sum::<f64>() / count;
    let deviation = (bias.iter().map(|b| (b - mean).powi(2)).sum::<f64>() / count).sqrt();
    assert!(
        differed > 1000,
        "the order matters to the outcome: {differed}"
    );
    assert!(
        mean.abs() < 1e-3 && mean.abs() < 4.0 * deviation / count.sqrt(),
        "the name first is shoved {mean} of the closing speed more, with a deviation of {deviation}"
    );
}

/// The control a tick flew is what its snapshot will say: single precision, and admissible.
#[test]
fn the_control_of_a_tick_is_admissible_and_single_precision() {
    let world = arena(Preset::PlanetAndMoon);
    let dynamics = Dynamics::new(&world);
    let mut rng = Rng::new(0xBC);
    for _ in 0..300 {
        let mut ships = fleet(&mut rng, &world);
        for _ in 0..20 {
            let commands = commands_for(&mut rng, &ships);
            dynamics.tick(&mut ships, &commands);
            for ship in &ships {
                let Control { thrust, torque } = ship.control;
                assert!(ship.control.is_admissible(&ship.params), "{ship:?}");
                assert_eq!(f64::from(thrust as f32), thrust);
                assert_eq!(f64::from(torque as f32), torque);
            }
        }
    }
}
