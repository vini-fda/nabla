//! Fixed-substep Rapier collisions and Newtonian gravity acting only on the player.

use log::Level;
use nabla_core::{Control, State};
use rapier2d::prelude::{
    ColliderBuilder, MassProperties, PhysicsWorld, RigidBodyBuilder, RigidBodyHandle, Vector,
};

use crate::span::span;
use crate::world::{CelestialBody, World};

/// Sum G M (body - player) / r^3. Inside a body, use a uniform solid sphere's field,
/// which is continuous at the surface and zero at the center, avoiding a singularity.
/// Acceleration is independent of the player's mass; bodies never attract one another.
pub fn gravity_acceleration(
    bodies: &[CelestialBody],
    gravity_constant: f64,
    position: [f64; 2],
) -> [f64; 2] {
    let mut acceleration = [0.0; 2];
    if gravity_constant == 0.0 {
        return acceleration;
    }
    for body in bodies {
        let dx = f64::from(body.position[0]) - position[0];
        let dy = f64::from(body.position[1]) - position[1];
        let distance = dx.hypot(dy).max(f64::from(body.radius));
        let factor = gravity_constant * body.mass / distance.powi(3);
        acceleration[0] += factor * dx;
        acceleration[1] += factor * dy;
    }
    acceleration
}

pub struct Physics {
    world: PhysicsWorld,
    player: RigidBodyHandle,
    sources: Vec<CelestialBody>,
    gravity_constant: f64,
    /// Preserve the model's unwrapped heading across Rapier's wrapped rotations.
    theta: f64,
}

impl Physics {
    pub fn new(save: &World) -> Self {
        let _span = span!(Level::Debug, "physics of {} bodies", save.bodies.len());
        let mut world = PhysicsWorld {
            gravity: Vector::ZERO,
            ..PhysicsWorld::default()
        };
        world.integration_parameters.max_ccd_substeps = 8;
        for body in &save.bodies {
            world.insert(
                RigidBodyBuilder::fixed().translation(Vector::from_array(body.position)),
                ColliderBuilder::ball(body.radius).friction(0.5),
            );
        }
        let p = &save.players[0];
        let s = p.state;
        let params = p.task.params;
        let (player, _) = world.insert(
            RigidBodyBuilder::dynamic()
                .translation(Vector::new(s.x as f32, s.y as f32))
                .rotation(crate::game::wrap(s.theta) as f32)
                .linvel(Vector::new(s.vx as f32, s.vy as f32))
                .angvel(s.omega as f32)
                .additional_mass_properties(MassProperties::new(
                    Vector::ZERO,
                    params.mass as f32,
                    params.inertia as f32,
                ))
                .ccd_enabled(true)
                .can_sleep(false),
            ColliderBuilder::ball(p.radius)
                .density(0.0)
                .friction(0.5)
                .restitution(0.0),
        );
        // Rapier normally updates mass during its first step; gravity needs it beforehand.
        world.bodies[player].recompute_mass_properties_from_colliders(&world.colliders);
        Self {
            world,
            player,
            sources: save.bodies.clone(),
            gravity_constant: save.gravity_constant,
            theta: s.theta,
        }
    }

    /// One control period, recording substeps for rendering through collisions.
    pub fn step(&mut self, h: f64, u: Control) -> Vec<State> {
        let _span = span!(Level::Trace, "physics step");
        let count = (h * 120.0).ceil() as usize;
        let dt = (h / count as f64) as f32;
        self.world.integration_parameters.dt = dt;
        let mut samples = Vec::with_capacity(count + 1);
        samples.push(self.state());
        for _ in 0..count {
            let body = &mut self.world.bodies[self.player];
            let old_angle = body.rotation().angle();
            let old_omega = body.angvel();
            let heading = old_angle + body.angvel() * dt * 0.5;
            let position = body.translation();
            let [ax, ay] = gravity_acceleration(
                &self.sources,
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
            self.world.step();
            let angle = self.world.bodies[self.player].rotation().angle();
            let expected =
                f64::from(f32::midpoint(old_omega, self.world.bodies[self.player].angvel()) * dt);
            self.theta += expected + crate::game::wrap(f64::from(angle - old_angle) - expected);
            samples.push(self.state());
        }
        samples
    }

    fn state(&self) -> State {
        let body = &self.world.bodies[self.player];
        State {
            x: f64::from(body.translation().x),
            y: f64::from(body.translation().y),
            vx: f64::from(body.linvel().x),
            vy: f64::from(body.linvel().y),
            theta: self.theta,
            omega: f64::from(body.angvel()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::Preset;

    #[test]
    fn thrust_and_torque_use_saved_mass_and_inertia() {
        let mut save = Preset::Planet.world();
        save.gravity_constant = 0.0;
        save.players[0].state = State {
            x: -100.0,
            ..State::default()
        };
        let p = save.players[0].task.params;
        let mut thrust = Physics::new(&save);
        let mut torque = Physics::new(&save);
        for _ in 0..50 {
            thrust.step(
                0.02,
                Control {
                    thrust: p.f_max,
                    torque: 0.0,
                },
            );
            torque.step(
                0.02,
                Control {
                    thrust: 0.0,
                    torque: p.tau_max,
                },
            );
        }
        assert!(
            (thrust.state().vx - p.f_max / p.mass).abs() < 1e-4,
            "configured mass"
        );
        assert!(
            (torque.state().omega - p.tau_max / p.inertia).abs() < 1e-4,
            "configured inertia"
        );
        assert!(
            torque.state().theta > std::f64::consts::TAU,
            "heading stays unwrapped"
        );
    }

    #[test]
    fn collision_stops_a_fast_ship_and_leaves_the_planet_fixed() {
        let mut save = Preset::Planet.world();
        save.bodies[0].position = [0.0, 0.0];
        save.players[0].state = State {
            x: -20.0,
            vx: 2000.0,
            ..State::default()
        };
        let mut physics = Physics::new(&save);
        for _ in 0..100 {
            physics.step(0.02, Control::default());
        }
        let state = physics.state();
        assert!(state.x <= -7.95, "CCD prevents tunneling: {state:?}");
        assert!(state.vx.abs() < 0.1, "collision stops inward motion");
        let (_, planet) = physics
            .world
            .bodies
            .iter()
            .find(|(_, b)| b.is_fixed())
            .expect("fixed planet");
        assert_eq!(planet.translation(), Vector::ZERO, "planet stays static");
    }

    #[test]
    fn zero_gravity_disables_attraction_and_coasting_is_undamped() {
        let mut save = Preset::Planet.world();
        save.gravity_constant = 0.0;
        save.players[0].state = State {
            x: -30.0,
            y: 20.0,
            vx: 2.0,
            ..State::default()
        };
        let mut physics = Physics::new(&save);
        for _ in 0..100 {
            physics.step(0.02, Control::default());
        }
        let s = physics.state();
        assert!(
            (s.x + 26.0).abs() < 0.001 && (s.y - 20.0).abs() < 0.001,
            "coasts in a line: {s:?}"
        );
        assert!(
            (s.vx - 2.0).abs() < 1e-5 && s.vy.abs() < 1e-5,
            "no gravity or damping"
        );
    }

    fn source(position: [f32; 2], radius: f32, mass: f64) -> CelestialBody {
        CelestialBody {
            name: "Body".to_owned(),
            position,
            radius,
            mass,
        }
    }

    #[test]
    fn gravity_obeys_inverse_square_mass_and_constant_scaling() {
        let mut bodies = [source([0.0, 0.0], 1.0, 10.0)];
        let near = gravity_acceleration(&bodies, 2.0, [3.0, 4.0]);
        assert!(
            (near[0] + 0.48).abs() < 1e-12 && (near[1] + 0.64).abs() < 1e-12,
            "G M / r^2 points toward the body: {near:?}"
        );
        let far = gravity_acceleration(&bodies, 2.0, [6.0, 8.0]);
        for axis in 0..2 {
            assert!(
                (far[axis] * 4.0 - near[axis]).abs() < 1e-12,
                "inverse-square falloff"
            );
        }
        let stronger = gravity_acceleration(&bodies, 6.0, [3.0, 4.0]);
        bodies[0].mass *= 3.0;
        let heavier = gravity_acceleration(&bodies, 2.0, [3.0, 4.0]);
        for axis in 0..2 {
            assert!(
                (stronger[axis] - near[axis] * 3.0).abs() < 1e-12,
                "linear in G"
            );
            assert!(
                (heavier[axis] - stronger[axis]).abs() < 1e-12,
                "linear in body mass"
            );
        }
    }

    #[test]
    fn gravity_superposes_bodies_and_vanishes_without_sources() {
        let bodies = [source([6.0, 0.0], 1.0, 18.0), source([0.0, 4.0], 1.0, 8.0)];
        assert_eq!(gravity_acceleration(&bodies, 2.0, [0.0, 0.0]), [1.0, 1.0]);
        assert_eq!(gravity_acceleration(&bodies, 0.0, [0.0, 0.0]), [0.0; 2]);
        assert_eq!(gravity_acceleration(&[], 2.0, [0.0, 0.0]), [0.0; 2]);
        assert_eq!(
            gravity_acceleration(&[source([0.0; 2], 1.0, 0.0)], 2.0, [3.0, 4.0]),
            [0.0; 2]
        );
    }

    #[test]
    fn gravity_is_finite_inside_bodies_and_continuous_at_the_surface() {
        let bodies = [source([0.0, 0.0], 2.0, 8.0)];
        assert_eq!(gravity_acceleration(&bodies, 1.0, [0.0, 0.0]), [0.0; 2]);
        assert_eq!(gravity_acceleration(&bodies, 1.0, [1.0, 0.0]), [-1.0, 0.0]);
        for x in [2.0 - 1e-8, 2.0, 2.0 + 1e-8] {
            let a = gravity_acceleration(&bodies, 1.0, [x, 0.0]);
            assert!(
                (a[0] + 2.0).abs() < 1e-7 && a[1] == 0.0,
                "continuous surface field"
            );
        }
    }

    #[test]
    fn gravity_accelerates_players_equally_and_keeps_bodies_static() {
        let mut save = Preset::Planet.world();
        save.bodies = vec![source([0.0, 0.0], 1.0, 100.0)];
        save.gravity_constant = 2.0;
        save.players[0].state = State {
            x: 10.0,
            theta: 0.4,
            ..State::default()
        };
        let mut light = Physics::new(&save);
        save.players[0].task.params.mass *= 10.0;
        let mut heavy = Physics::new(&save);
        let dt = 1.0 / 120.0;
        light.step(dt, Control::default());
        heavy.step(dt, Control::default());
        assert!(
            (light.state().vx / dt + 2.0).abs() < 1e-4,
            "integrates G M / r^2"
        );
        for _ in 0..100 {
            light.step(0.02, Control::default());
            heavy.step(0.02, Control::default());
        }
        let (a, b) = (light.state(), heavy.state());
        assert!(
            a.x < 9.0 && a.vx < -2.0 && a.y == 0.0,
            "attracts the player: {a:?}"
        );
        assert!(
            (a.x - b.x).abs() < 1e-5 && (a.vx - b.vx).abs() < 1e-5,
            "player mass cancels: {a:?}, {b:?}"
        );
        assert!(
            (a.theta - 0.4).abs() < 1e-6 && a.omega == 0.0,
            "gravity adds no torque"
        );
        for (_, body) in light
            .world
            .bodies
            .iter()
            .filter(|(_, body)| body.is_fixed())
        {
            assert_eq!(body.translation(), Vector::ZERO, "static source position");
            assert_eq!(body.linvel(), Vector::ZERO, "static source velocity");
        }
    }

    #[test]
    fn thrust_can_balance_gravity() {
        let mut save = Preset::Planet.world();
        save.bodies = vec![source([0.0, 0.0], 1.0, 100.0)];
        save.gravity_constant = 2.0;
        save.players[0].state = State {
            x: 10.0,
            ..State::default()
        };
        let mut physics = Physics::new(&save);
        for _ in 0..100 {
            physics.step(
                0.02,
                Control {
                    thrust: 2.0 * save.players[0].task.params.mass,
                    torque: 0.0,
                },
            );
        }
        assert!(
            (physics.state().x - 10.0).abs() < 1e-6 && physics.state().vx.abs() < 1e-6,
            "thrust and gravity are added each substep"
        );
    }
}
