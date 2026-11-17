//! The world's map from tick to tick, `F` of `specs/multiplayer.typ`, for ships that collide.
//!
//! A tick is a function of the world before it and of the commands, and of nothing else:
//! [`Dynamics`] holds the world's constants and keeps nothing from one tick to the next. So a
//! state says all there is to say of a ship, and a client that starts from a snapshot computes
//! what the server computed.
//!
//! Ships are first sorted into *blocks* ([`Dynamics::blocks`]): two ships share a block when they
//! are near enough to touch within the tick, at the speeds they could have by then, and so do
//! two blocks. Each block is then stepped alone. A ship that is alone in its block flies as it
//! would with no other ship in the world: by the exact model in a vacuum, and by Rapier among
//! bodies. A block of several is one Rapier world, built for this tick from the states of its
//! ships and dropped after it, in which their hulls collide. The arena's wall comes last.
//!
//! That a block is stepped by itself is what makes the map local by construction: the ships of a
//! part of the world that is a union of blocks get the same states whether the rest of the world
//! is there or not.

use nabla_core::{Control, Params, State, step};
use rapier2d::prelude::{
    ColliderBuilder, ColliderHandle, MassProperties, PhysicsWorld, RigidBodyBuilder,
    RigidBodyHandle, SharedShape, Vector,
};

use crate::command::Command;
use crate::hull;
use crate::physics::{gravity_acceleration, substeps};
use crate::ship::wall;
use crate::steering::wrap;
use crate::world::{Arena, CelestialBody, World};

/// The most substeps a block of several ships takes in a tick. Up to it, no two points of two
/// hulls close by more than [`STEP`] of the hull radius in a substep.
pub const MAX_SUBSTEPS: usize = 64;

/// `delta`, as a fraction of the hull radius: how far two points of two hulls may close in one
/// substep.
///
/// It is under half the hull's width, so two hulls on a collision course overlap at the end of
/// some substep, and cannot pass through each other between two of them.
pub const STEP: f64 = 0.25;

/// How many rings of places around each spawn template are tried for a ship that joins.
const SPAWN_RINGS: usize = 8;

/// One ship of a world: all that the world's map reads of it and writes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Member {
    /// The ship's own parameters, with the world's `h`.
    pub params: Params,
    pub state: State,
    /// The heading the stability assist holds: the part of the extended state that is not
    /// physics.
    pub hold: Option<f64>,
    /// The control of the tick before. The map writes it, and reads it nowhere.
    pub control: Control,
}

impl Member {
    /// A ship with `params` in `state`, with the assist holding nothing.
    pub fn new(params: Params, state: State) -> Self {
        Self {
            params,
            state,
            hold: None,
            control: Control::default(),
        }
    }
}

/// What a tick found, with ships named by their place in the slice it was given.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// The blocks the ships were stepped in: each in rising order, and the blocks by their
    /// first ship.
    pub blocks: Vec<Vec<usize>>,
    /// The pairs of ships whose hulls pushed on each other in the tick, the lower first.
    pub touches: Vec<(usize, usize)>,
    /// The ships whose hull pushed on a body in the tick.
    pub grounded: Vec<usize>,
}

/// The constants of a world's map from tick to tick. It has no state of its own.
#[derive(Clone)]
pub struct Dynamics {
    h: f64,
    /// The hull radius `rho`, the same for every ship.
    radius: f64,
    arena: Option<Arena>,
    bodies: Vec<CelestialBody>,
    gravity_constant: f64,
    /// A bound on the gravitational acceleration anywhere: every body's surface gravity, summed.
    g_max: f64,
    collide: bool,
    hull: SharedShape,
}

impl std::fmt::Debug for Dynamics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Dynamics")
            .field("h", &self.h)
            .field("radius", &self.radius)
            .field("bodies", &self.bodies.len())
            .field("collide", &self.collide)
            .finish_non_exhaustive()
    }
}

/// `x` as the nearest single-precision number that is no larger in size: what a snapshot can
/// say exactly, and still admissible if `x` was.
fn narrow(x: f64) -> f64 {
    let y = x as f32;
    if f64::from(y).abs() > x.abs() {
        // The bit pattern below a finite, nonzero `f32` is the next one toward zero.
        f64::from(f32::from_bits(y.to_bits() - 1))
    } else {
        f64::from(y)
    }
}

fn is_finite(s: &State) -> bool {
    [s.x, s.y, s.vx, s.vy, s.theta, s.omega]
        .iter()
        .all(|x| x.is_finite())
}

/// What the active contacts of a step of `world` say, with ships named by their place in `block`:
/// which pairs of hulls pushed on each other, and which hulls pushed on a body.
fn note_contacts(
    world: &PhysicsWorld,
    handles: &[(RigidBodyHandle, ColliderHandle)],
    block: &[usize],
    touches: &mut Vec<(usize, usize)>,
    grounded: &mut Vec<usize>,
) {
    for pair in world.contact_pairs() {
        if !pair.has_any_active_contact() {
            continue;
        }
        let ship_of = |collider: ColliderHandle| {
            handles
                .iter()
                .position(|&(_, c)| c == collider)
                .map(|n| block[n])
        };
        match (ship_of(pair.collider1), ship_of(pair.collider2)) {
            (Some(a), Some(b)) => {
                let touch = (a.min(b), a.max(b));
                if !touches.contains(&touch) {
                    touches.push(touch);
                }
            }
            (Some(a), None) | (None, Some(a)) => {
                if !grounded.contains(&a) {
                    grounded.push(a);
                }
            }
            (None, None) => {}
        }
    }
}

impl Dynamics {
    /// The map of `world`, whose ships collide unless the world says they do not.
    pub fn new(world: &World) -> Self {
        let radius = world.hull_radius();
        let points: Vec<Vector> = hull::points(radius)
            .into_iter()
            .map(|[x, y]| Vector::new(x, y))
            .collect();
        let g_max = world
            .bodies
            .iter()
            .map(|b| world.gravity_constant * b.mass / f64::from(b.radius).powi(2))
            .sum();
        Self {
            h: world.h(),
            radius: f64::from(radius),
            arena: world.arena,
            bodies: world.bodies.clone(),
            gravity_constant: world.gravity_constant,
            g_max,
            collide: world.ship_collisions(),
            hull: SharedShape::convex_hull(&points).expect("the hull is a convex polygon"),
        }
    }

    /// The tick `h`, in seconds.
    pub fn h(&self) -> f64 {
        self.h
    }

    /// The hull radius `rho`.
    pub fn radius(&self) -> f64 {
        self.radius
    }

    /// Whether ships collide with each other. Where they do not, every block is one ship.
    pub fn collide(&self) -> bool {
        self.collide
    }

    /// `sigma`: what is added to the distance at which two ships count as near. It covers the
    /// distance at which Rapier begins to see a contact, and the rounding of positions to single
    /// precision.
    pub fn skin(&self) -> f64 {
        self.radius / 8.0 + 0.05
    }

    /// `D_B(L)`: the reach of the block `block` of `ships` over the next `ticks` ticks, which is
    /// a bound on how far any point of its hulls gets from where the hull's center is now, for
    /// as long as nothing touches a ship of it.
    ///
    /// It has three terms, each from the ship of the block that is worst for it. The fastest
    /// ship's speed, for the ticks. The largest acceleration (thrust and gravity) over the
    /// ticks: a step of the integrator moves a ship by its velocity after the step, so `L` ticks
    /// of acceleration `A` move it `A h^2 L (L + 1) / 2` at most. And the sweep of the fastest
    /// point of a hull about its center in one tick, at the fastest spin torque can have made
    /// by then.
    pub fn reach(&self, ships: &[Member], block: &[usize], ticks: u32) -> f64 {
        let l = f64::from(ticks);
        let (mut speed, mut gain, mut spin) = (0.0_f64, 0.0_f64, 0.0_f64);
        for &i in block {
            let (s, p) = (&ships[i].state, &ships[i].params);
            speed = speed.max(s.vx.hypot(s.vy));
            gain = gain.max(p.f_max / p.mass + self.g_max);
            spin = spin.max(s.omega.abs() + p.tau_max / p.inertia * l * self.h);
        }
        speed * l * self.h
            + gain * self.h * self.h * l * (l + 1.0) / 2.0
            + self.radius * spin * self.h
    }

    /// `Pi_L`: the coupling partition of `ships` over the next `ticks` ticks, each block in
    /// rising order and the blocks by their first ship.
    ///
    /// Two ships of different blocks are *near* when their centers are within two hull radii,
    /// the skin, and the [`reach`](Self::reach) of both blocks. The partition is the finest in
    /// which no two near ships are in different blocks. It is found by merging blocks until
    /// none are near, and since a merge only widens a reach, the order of merging does not
    /// matter.
    ///
    /// With one tick it is the partition a tick steps by. With more it says which ships can
    /// come to share a block within that many ticks, short of a contact.
    pub fn blocks(&self, ships: &[Member], ticks: u32) -> Vec<Vec<usize>> {
        let n = ships.len();
        let mut block: Vec<usize> = (0..n).collect();
        if self.collide && n > 1 {
            let touching = 2.0 * self.radius + self.skin();
            loop {
                let reach: Vec<f64> = (0..n)
                    .map(|first| {
                        let members: Vec<usize> = (0..n).filter(|&i| block[i] == first).collect();
                        self.reach(ships, &members, ticks)
                    })
                    .collect();
                let mut merged = false;
                for i in 0..n {
                    for j in i + 1..n {
                        let (a, b) = (block[i], block[j]);
                        if a == b {
                            continue;
                        }
                        let (p, q) = (&ships[i].state, &ships[j].state);
                        let apart = (p.x - q.x).hypot(p.y - q.y);
                        // Written so that a place that is not a number is near every ship: such
                        // a ship is never stepped as if it were alone.
                        let is_apart = apart > touching + reach[a] + reach[b];
                        if !is_apart {
                            let (keep, drop) = (a.min(b), a.max(b));
                            for label in &mut block {
                                if *label == drop {
                                    *label = keep;
                                }
                            }
                            merged = true;
                        }
                    }
                }
                if !merged {
                    break;
                }
            }
        }
        (0..n)
            .filter(|&first| block[first] == first)
            .map(|first| (first..n).filter(|&i| block[i] == first).collect())
            .collect()
    }

    /// How many substeps a block takes in a tick.
    ///
    /// A ship alone takes the `ceil(120 h)` of [`substeps`], and Rapier's continuous detection
    /// keeps it out of the bodies. That detection does nothing between two moving ships, so a
    /// block of several takes as many substeps as keep any two points of two hulls from closing
    /// by more than [`STEP`] of the hull radius in one, up to [`MAX_SUBSTEPS`].
    pub fn substeps_of(&self, ships: &[Member], block: &[usize]) -> usize {
        let (base, _) = substeps(self.h);
        if block.len() < 2 {
            return base;
        }
        let closing = 2.0 * self.reach(ships, block, 1);
        let needed = (closing / (STEP * self.radius)).ceil();
        if needed.is_finite() {
            (needed as usize).clamp(base, MAX_SUBSTEPS.max(base))
        } else {
            MAX_SUBSTEPS.max(base)
        }
    }

    /// One tick of the world: `F(W, c)`.
    ///
    /// `ships` are the world's ships in the order of their names, which is the order they enter
    /// a block's Rapier world in, and `commands[i]` is the command of `ships[i]`. Each command is
    /// sanitized and becomes a control, rounded toward zero to single precision (which is what
    /// a snapshot can carry, and what Rapier computes with). Then each block is stepped, and the
    /// wall stops what it has to.
    ///
    /// It is total: any commands at all, and any finite states, give finite states inside the
    /// arena.
    ///
    /// # Panics
    /// If there is not one command for each ship.
    pub fn tick(&self, ships: &mut [Member], commands: &[Command]) -> Report {
        assert_eq!(ships.len(), commands.len(), "a command for each ship");
        let mut controls = Vec::with_capacity(ships.len());
        for (ship, command) in ships.iter_mut().zip(commands) {
            let command = command.sanitize(&ship.params);
            let (mut u, hold) = command.actuate(&ship.params, &ship.state, ship.hold);
            if !u.is_admissible(&ship.params) {
                u = Control::default();
            }
            ship.hold = hold;
            controls.push(Control {
                thrust: narrow(u.thrust),
                torque: narrow(u.torque),
            });
        }
        let mut report = Report {
            blocks: self.blocks(ships, 1),
            ..Report::default()
        };
        let before: Vec<State> = ships.iter().map(|ship| ship.state).collect();
        for block in &report.blocks {
            if let ([i], true) = (block.as_slice(), self.bodies.is_empty()) {
                ships[*i].state = step(&ships[*i].params, &ships[*i].state, &controls[*i]);
            } else {
                self.step_block(
                    ships,
                    &controls,
                    block,
                    &mut report.touches,
                    &mut report.grounded,
                );
            }
        }
        for ((ship, u), old) in ships.iter_mut().zip(controls).zip(before) {
            if !is_finite(&ship.state) {
                // A second line of defense, as the check on the control is: a step that did not
                // come out as numbers leaves the ship where it was, at rest.
                ship.state = State {
                    vx: 0.0,
                    vy: 0.0,
                    omega: 0.0,
                    ..old
                };
            }
            if let Some(arena) = self.arena {
                ship.state = wall(ship.state, arena.half_side, self.radius);
            }
            ship.control = u;
        }
        report.touches.sort_unstable();
        report.grounded.sort_unstable();
        report
    }

    /// Step the ships of `block` together, in a Rapier world made for this tick: the bodies
    /// within the block's reach, and the ships in order, each with its hull.
    fn step_block(
        &self,
        ships: &mut [Member],
        controls: &[Control],
        block: &[usize],
        touches: &mut Vec<(usize, usize)>,
        grounded: &mut Vec<usize>,
    ) {
        let mut world = PhysicsWorld {
            gravity: Vector::ZERO,
            ..PhysicsWorld::default()
        };
        world.integration_parameters.max_ccd_substeps = 8;
        self.insert_bodies(&mut world, ships, block);
        let handles: Vec<(RigidBodyHandle, ColliderHandle)> = block
            .iter()
            .map(|&i| {
                let (s, p) = (&ships[i].state, &ships[i].params);
                let (body, collider) = world.insert(
                    RigidBodyBuilder::dynamic()
                        .translation(Vector::new(s.x as f32, s.y as f32))
                        .rotation(wrap(s.theta) as f32)
                        .linvel(Vector::new(s.vx as f32, s.vy as f32))
                        .angvel(s.omega as f32)
                        .additional_mass_properties(MassProperties::new(
                            Vector::ZERO,
                            p.mass as f32,
                            p.inertia as f32,
                        ))
                        .ccd_enabled(true)
                        .can_sleep(false),
                    ColliderBuilder::new(self.hull.clone())
                        .density(0.0)
                        .friction(0.5)
                        .restitution(0.0),
                );
                // Rapier normally updates mass during its first step; gravity needs it before.
                world.bodies[body].recompute_mass_properties_from_colliders(&world.colliders);
                (body, collider)
            })
            .collect();
        let count = self.substeps_of(ships, block);
        let delta_t = (self.h / count as f64) as f32;
        world.integration_parameters.dt = delta_t;
        // The model's heading is not wrapped, and Rapier's rotation is.
        let mut thetas: Vec<f64> = block.iter().map(|&i| ships[i].state.theta).collect();
        let mut turned = vec![(0.0_f32, 0.0_f32); block.len()];
        for _ in 0..count {
            self.substep_forces(&mut world, controls, block, &handles, &mut turned, delta_t);
            world.step();
            for (n, theta) in thetas.iter_mut().enumerate() {
                let body = &world.bodies[handles[n].0];
                let (old_angle, old_omega) = turned[n];
                let angle = body.rotation().angle();
                let expected = f64::from(f32::midpoint(old_omega, body.angvel()) * delta_t);
                *theta += expected + wrap(f64::from(angle - old_angle) - expected);
            }
            note_contacts(&world, &handles, block, touches, grounded);
        }
        for (n, &i) in block.iter().enumerate() {
            let body = &world.bodies[handles[n].0];
            ships[i].state = State {
                x: f64::from(body.translation().x),
                y: f64::from(body.translation().y),
                vx: f64::from(body.linvel().x),
                vy: f64::from(body.linvel().y),
                theta: thetas[n],
                omega: f64::from(body.angvel()),
            };
        }
    }

    /// The bodies a substep of the tick could bring the ships of `block` to, in `world`.
    fn insert_bodies(&self, world: &mut PhysicsWorld, ships: &[Member], block: &[usize]) {
        let reach = self.reach(ships, block, 1);
        for body in &self.bodies {
            let within = f64::from(body.radius) + self.radius + self.skin() + reach;
            let in_reach = block.iter().any(|&i| {
                let s = &ships[i].state;
                let apart =
                    (s.x - f64::from(body.position[0])).hypot(s.y - f64::from(body.position[1]));
                apart <= within || apart.is_nan()
            });
            if in_reach {
                world.insert(
                    RigidBodyBuilder::fixed().translation(Vector::from_array(body.position)),
                    ColliderBuilder::ball(body.radius).friction(0.5),
                );
            }
        }
    }

    /// The force and torque each ship of `block` takes in one substep of `world`: thrust along
    /// its mid-substep heading, and gravity. It notes the angle and spin each had before, for
    /// the heading's wrap after the step.
    fn substep_forces(
        &self,
        world: &mut PhysicsWorld,
        controls: &[Control],
        block: &[usize],
        handles: &[(RigidBodyHandle, ColliderHandle)],
        turned: &mut [(f32, f32)],
        delta_t: f32,
    ) {
        for (n, &i) in block.iter().enumerate() {
            let u = controls[i];
            let body = &mut world.bodies[handles[n].0];
            let old_angle = body.rotation().angle();
            let old_omega = body.angvel();
            let heading = old_angle + old_omega * delta_t * 0.5;
            let position = body.translation();
            let [ax, ay] = gravity_acceleration(
                &self.bodies,
                self.gravity_constant,
                [f64::from(position.x), f64::from(position.y)],
            );
            body.reset_forces(false);
            body.reset_torques(false);
            body.add_force(
                Vector::new(heading.cos(), heading.sin()) * u.thrust as f32
                    + Vector::new(ax as f32, ay as f32) * body.mass(),
                true,
            );
            body.add_torque(u.torque as f32, true);
            turned[n] = (old_angle, old_omega);
        }
    }

    /// Whether a ship in `state` with `params` would be alone in its block among `ships`, inside
    /// the arena, and clear of every body: a place a ship can join at without moving anyone.
    pub fn is_clear(&self, ships: &[Member], params: Params, state: State) -> bool {
        if let Some(arena) = self.arena {
            let reach = arena.half_side - self.radius;
            if !(state.x.abs() <= reach && state.y.abs() <= reach) {
                return false;
            }
        }
        let in_a_body = self.bodies.iter().any(|body| {
            let apart = (f64::from(body.position[0]) - state.x)
                .hypot(f64::from(body.position[1]) - state.y);
            apart < f64::from(body.radius) + self.radius
        });
        if in_a_body {
            return false;
        }
        let mut with: Vec<Member> = ships.to_vec();
        with.push(Member::new(params, state));
        let newcomer = with.len() - 1;
        self.blocks(&with, 1)
            .iter()
            .any(|block| block.as_slice() == [newcomer])
    }

    /// `spawn`: where the `index`th ship to join `world` starts, with `params`, given the ships
    /// already flying: the first place that [`is_clear`](Self::is_clear), or none if there is
    /// no room.
    ///
    /// The places are the world's spawn templates, from the `index`th on, and then rings of
    /// places around each, four hull radii apart, out to eight rings. Where ships do
    /// not collide every place is clear of them, and the ship takes its template, as in Phase A.
    pub fn spawn(
        &self,
        world: &World,
        ships: &[Member],
        index: usize,
        params: Params,
    ) -> Option<State> {
        let templates = world.players.len();
        let gap = 4.0 * self.radius;
        for ring in 0..=SPAWN_RINGS {
            let places = (6 * ring).max(1);
            for t in 0..templates {
                let template = world.spawn(index + t).state;
                for place in 0..places {
                    let angle = std::f64::consts::TAU * place as f64 / places as f64;
                    let state = State {
                        x: template.x + gap * ring as f64 * angle.cos(),
                        y: template.y + gap * ring as f64 * angle.sin(),
                        ..template
                    };
                    if self.is_clear(ships, params, state) {
                        return Some(state);
                    }
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests;
