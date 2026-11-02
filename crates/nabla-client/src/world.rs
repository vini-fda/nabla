//! Portable, versioned TOML saves. Runtime physics resources are rebuilt on load.

use log::Level;
use nabla_core::problem::Case;
use nabla_core::{State, Task};
use serde::{Deserialize, Serialize};

use crate::span::span;

pub const MAX_SAVE_BYTES: usize = 1024 * 1024;
pub const MAX_GRAVITY_CONSTANT: f64 = 1e4;

fn default_body_mass() -> f64 {
    1.0
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Player {
    pub state: State,
    pub task: Task,
    /// Circular collision envelope, in world units, also used to size the ship.
    pub radius: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CelestialBody {
    pub name: String,
    pub position: [f32; 2],
    pub radius: f32,
    /// Gravitational mass; the body's collision geometry remains fixed.
    #[serde(default = "default_body_mass")]
    pub mass: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct World {
    pub version: u32,
    pub name: String,
    /// Newton's G in world units. Missing in legacy saves, which retain zero gravity.
    #[serde(default)]
    pub gravity_constant: f64,
    pub players: Vec<Player>,
    #[serde(default)]
    pub bodies: Vec<CelestialBody>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Preset {
    Vacuum,
    Planet,
    #[default]
    PlanetAndMoon,
}

impl Preset {
    pub const ALL: [Self; 3] = [Self::Vacuum, Self::Planet, Self::PlanetAndMoon];

    pub fn name(self) -> &'static str {
        match self {
            Self::Vacuum => "Pure vacuum",
            Self::Planet => "Planet",
            Self::PlanetAndMoon => "Planet + moon",
        }
    }

    pub fn world(self) -> World {
        let source = match self {
            Self::Vacuum => include_str!("../assets/worlds/vacuum.toml"),
            Self::Planet => include_str!("../assets/worlds/planet.toml"),
            Self::PlanetAndMoon => include_str!("../assets/worlds/planet_moon.toml"),
        };
        World::from_toml(source).expect("bundled world is valid")
    }
}

impl World {
    pub fn vacuum(case: &Case, radius: f32) -> Self {
        Self {
            version: 1,
            name: "Pure vacuum".to_owned(),
            gravity_constant: 0.0,
            players: vec![Player {
                state: case.start,
                task: case.task,
                radius,
            }],
            bodies: Vec::new(),
        }
    }

    pub fn case(&self) -> Case {
        Case {
            name: self.name.clone(),
            start: self.players[0].state,
            task: self.players[0].task,
        }
    }

    /// Parse and validate before allocating any simulation resources.
    ///
    /// # Errors
    /// Rejects malformed, unsupported, oversized or physically invalid saves.
    pub fn from_toml(source: &str) -> Result<Self, String> {
        let _span = span!(Level::Debug, "parsing a world, {} bytes", source.len());
        if source.len() > MAX_SAVE_BYTES {
            return Err("World saves must be at most 1 MiB".to_owned());
        }
        let world: Self = toml::from_str(source).map_err(|e| e.to_string())?;
        world.validate()?;
        Ok(world)
    }

    /// # Errors
    /// Returns an error for invalid worlds or a serialization failure.
    pub fn to_toml(&self) -> Result<String, String> {
        self.validate()?;
        toml::to_string_pretty(self).map_err(|e| e.to_string())
    }

    /// # Errors
    /// Rejects unsupported versions/player counts, nonfinite numbers and unsafe ranges.
    pub fn validate(&self) -> Result<(), String> {
        let valid_name = |name: &str| !name.trim().is_empty() && name.len() <= 128;
        let bounded = |x: f64, max: f64| x.is_finite() && x.abs() <= max;
        if self.version != 1 || self.players.len() != 1 {
            return Err("Expected world version 1 with exactly one player".to_owned());
        }
        if !valid_name(&self.name) || self.bodies.len() > 128 {
            return Err("Invalid world name or more than 128 celestial bodies".to_owned());
        }
        if !self.gravity_constant.is_finite()
            || !(0.0..=MAX_GRAVITY_CONSTANT).contains(&self.gravity_constant)
        {
            return Err("Invalid gravitational constant (expected 0 through 10000)".to_owned());
        }
        let player = &self.players[0];
        let s = player.state;
        let t = player.task;
        let p = t.params;
        if ![s.x, s.y, t.goal.x, t.goal.y]
            .into_iter()
            .all(|x| bounded(x, 1e6))
            || ![s.vx, s.vy, s.omega].into_iter().all(|x| bounded(x, 1e4))
            || !bounded(s.theta, 1e9)
            || !(0.05..=10.0).contains(&player.radius)
            || ![p.mass, p.inertia, p.f_max, p.tau_max]
                .into_iter()
                .all(|x| (0.05..=20.0).contains(&x))
            || !(0.01..=0.1).contains(&p.h)
            || ![
                t.tolerance.position,
                t.tolerance.velocity,
                t.tolerance.omega,
            ]
            .into_iter()
            .all(|x| x.is_finite() && (0.0..=100.0).contains(&x))
            || !(1..=100_000).contains(&t.max_steps)
        {
            return Err("Invalid player state, ship parameters or goal".to_owned());
        }
        for body in &self.bodies {
            if !valid_name(&body.name)
                || !body
                    .position
                    .into_iter()
                    .all(|x| bounded(f64::from(x), 1e6))
                || !(0.05..=100_000.0).contains(&body.radius)
                || !body.mass.is_finite()
                || !(0.0..=1e12).contains(&body.mass)
            {
                return Err("Invalid celestial body name, position, radius or mass".to_owned());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_round_trip() {
        for (count, preset) in Preset::ALL.into_iter().enumerate() {
            let world = preset.world();
            assert_eq!(world.bodies.len(), count, "preset body count");
            assert_eq!(
                World::from_toml(&world.to_toml().expect("serialize")).expect("parse"),
                world
            );
        }
    }

    #[test]
    fn gravity_settings_round_trip_and_legacy_saves_remain_gravity_free() {
        let mut world = Preset::Planet.world();
        world.gravity_constant = 2.75;
        world.bodies[0].mass = 1234.0;
        let source = world.to_toml().expect("serialize");
        assert_eq!(World::from_toml(&source).expect("parse"), world);

        let legacy = include_str!("../assets/worlds/planet.toml")
            .replace("gravity_constant = 1.0\n", "")
            .replace("mass = 343.0\n", "");
        let world = World::from_toml(&legacy).expect("legacy version 1");
        assert_eq!(world.gravity_constant, 0.0, "preserve legacy physics");
        assert_eq!(world.bodies[0].mass, 1.0, "default gravitational mass");
    }

    #[test]
    fn rejects_invalid_gravity_settings() {
        let source = include_str!("../assets/worlds/planet.toml");
        for value in ["-1.0", "nan", "inf", "-inf", "10001.0"] {
            let invalid = source.replace(
                "gravity_constant = 1.0",
                &format!("gravity_constant = {value}"),
            );
            assert!(World::from_toml(&invalid).is_err(), "reject G = {value}");
        }
        for value in ["-1.0", "nan", "inf", "-inf", "1.1e12"] {
            let invalid = source.replace("mass = 343.0", &format!("mass = {value}"));
            assert!(
                World::from_toml(&invalid).is_err(),
                "reject body mass = {value}"
            );
        }
        let mut world = Preset::Planet.world();
        world.gravity_constant = 0.0;
        world.bodies[0].mass = 0.0;
        assert!(world.validate().is_ok(), "allow disabling attraction");
    }

    #[test]
    fn rejects_invalid_saves() {
        let mut world = Preset::Planet.world();
        world.players[0].state.vx = f64::NAN;
        assert!(world.to_toml().is_err(), "reject NaN");
        world = Preset::Planet.world();
        world.bodies[0].radius = -1.0;
        assert!(world.validate().is_err(), "reject negative radius");
        world = Preset::Vacuum.world();
        world.players.clear();
        assert!(world.validate().is_err(), "require a player");
        world = Preset::Vacuum.world();
        world.players.push(world.players[0].clone());
        assert!(world.validate().is_err(), "only one player is supported");
        assert!(
            World::from_toml(&" ".repeat(MAX_SAVE_BYTES + 1)).is_err(),
            "bounded size"
        );
        assert!(World::from_toml("version = 99").is_err(), "invalid schema");
    }
}
