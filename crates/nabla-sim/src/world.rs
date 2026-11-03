//! Portable, versioned TOML saves. Runtime physics resources are rebuilt on load.

use log::Level;
use nabla_core::problem::Case;
use nabla_core::{Params, State, Task};
use serde::{Deserialize, Serialize};

use crate::span;

pub const MAX_SAVE_BYTES: usize = 1024 * 1024;
pub const MAX_GRAVITY_CONSTANT: f64 = 1e4;
/// The most pilots a world can hold at once (`N_max`): a whole snapshot then fits one datagram.
pub const MAX_PILOTS: usize = 16;
/// The largest half side `S` of a world's arena. Beyond it `f32` positions are too coarse for
/// Rapier to move a slow ship (`docs/multiplayer.typ`, "The arena").
pub const MAX_ARENA_HALF_SIDE: f64 = 2048.0;
/// The slowest and the fastest tick a version 2 world may have, in ticks per second: `h` is
/// then in `[0.01, 0.1]`, as in version 1.
pub const TICK_RATES: std::ops::RangeInclusive<u32> = 10..=100;

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

/// The square a version 2 world is bounded by: ships stay within `[-S, S]^2` (`S` is
/// `half_side`), and a wall stops them there ([`crate::ship::wall`]).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Arena {
    /// The half side `S`, in world units, at most [`MAX_ARENA_HALF_SIDE`].
    pub half_side: f64,
}

/// The ships a version 2 world allows, `Pset`.
///
/// An interval for each parameter, and caps on the largest linear and angular accelerations
/// `a = F_max / m` and `beta = tau_max / I`, which every bound of the multiplayer design is
/// stated in.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ships {
    pub mass: [f64; 2],
    pub inertia: [f64; 2],
    pub f_max: [f64; 2],
    pub tau_max: [f64; 2],
    /// `a_cap`: `F_max <= a_cap m`.
    pub acceleration_cap: f64,
    /// `beta_cap`: `tau_max <= beta_cap I`.
    pub angular_cap: f64,
}

/// The ship a client asks for, as it arrives off the wire: any numbers at all.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ShipRequest {
    pub mass: f64,
    pub inertia: f64,
    pub f_max: f64,
    pub tau_max: f64,
}

impl From<Params> for ShipRequest {
    fn from(p: Params) -> Self {
        Self {
            mass: p.mass,
            inertia: p.inertia,
            f_max: p.f_max,
            tau_max: p.tau_max,
        }
    }
}

impl Default for Ships {
    /// The defaults of the multiplayer design: they contain every ship of the scoring suite, and
    /// the caps are twice the bundled ship's.
    fn default() -> Self {
        Self {
            mass: [0.1, 10.0],
            inertia: [0.1, 10.0],
            f_max: [0.5, 20.0],
            tau_max: [0.2, 20.0],
            acceleration_cap: 20.0,
            angular_cap: 50.0,
        }
    }
}

impl Ships {
    /// Whether `p` is in `Pset`: each parameter in its interval, with `F_max <= a_cap m` and
    /// `tau_max <= beta_cap I`. (The caps are written as products, as [`Self::grant`] applies
    /// them, so that what it returns is always allowed despite rounding.)
    pub fn allows(&self, p: &Params) -> bool {
        let within = |x: f64, [lo, hi]: [f64; 2]| x.is_finite() && lo <= x && x <= hi;
        within(p.mass, self.mass)
            && within(p.inertia, self.inertia)
            && within(p.f_max, self.f_max)
            && within(p.tau_max, self.tau_max)
            && p.f_max <= self.acceleration_cap * p.mass
            && p.tau_max <= self.angular_cap * p.inertia
    }

    /// `Grant`: the ship a request gets, with the tick `h` of the world. A total function of any
    /// numbers whatever, whose image is in `Pset`, and which fixes every point of it.
    ///
    /// Each number that is not finite is replaced by the default ship's, each is clamped into its
    /// interval, and then `F_max` and `tau_max` are held under the caps.
    #[must_use]
    pub fn grant(&self, default: &Params, request: &ShipRequest) -> Params {
        let pick = |x: f64, fallback: f64, [lo, hi]: [f64; 2]| {
            (if x.is_finite() { x } else { fallback }).max(lo).min(hi)
        };
        let mass = pick(request.mass, default.mass, self.mass);
        let inertia = pick(request.inertia, default.inertia, self.inertia);
        let f_max = pick(request.f_max, default.f_max, self.f_max);
        let tau_max = pick(request.tau_max, default.tau_max, self.tau_max);
        Params {
            mass,
            inertia,
            f_max: f_max.min(self.acceleration_cap * mass),
            tau_max: tau_max.min(self.angular_cap * inertia),
            h: default.h,
        }
    }

    /// Whether the intervals and caps make sense, and leave room for a ship.
    fn valid(&self) -> bool {
        let interval = |[lo, hi]: [f64; 2]| {
            lo.is_finite() && hi.is_finite() && 0.05 <= lo && lo <= hi && hi <= 20.0
        };
        let cap = |x: f64| x.is_finite() && 0.0 < x && x <= 1e4;
        interval(self.mass)
            && interval(self.inertia)
            && interval(self.f_max)
            && interval(self.tau_max)
            && cap(self.acceleration_cap)
            && cap(self.angular_cap)
            // The caps leave each interval of forces non-empty.
            && self.f_max[0] <= self.acceleration_cap * self.mass[0]
            && self.tau_max[0] <= self.angular_cap * self.inertia[0]
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct World {
    /// 1: one ship, flown alone, with its goal. 2: a world that a server runs for many ships, with
    /// a tick rate, an arena and the allowed ships.
    pub version: u32,
    pub name: String,
    /// Newton's G in world units. Missing in legacy saves, which retain zero gravity.
    #[serde(default)]
    pub gravity_constant: f64,
    /// Version 2: ticks per second, an integer so that every replica takes the same number of
    /// Rapier substeps per tick. The tick is `h = 1 / tick_rate`, which every player's `h` equals.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tick_rate: Option<u32>,
    /// Version 2: where the world ends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arena: Option<Arena>,
    /// Version 2: which ships a pilot may have.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ships: Option<Ships>,
    /// Version 1: the one player. Version 2: the spawn templates, of which the first gives the
    /// default ship, and all share one hull radius.
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
            tick_rate: None,
            arena: None,
            ships: None,
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

    /// In a version 2 world the tick belongs to the world: every player's `h` is made exactly
    /// `1 / tick_rate`, whatever the file wrote (it must still be there, as a version 1 world
    /// has it).
    fn set_tick(&mut self) {
        if let (2, Some(rate @ 1..)) = (self.version, self.tick_rate) {
            for player in &mut self.players {
                player.task.params.h = 1.0 / f64::from(rate);
            }
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
        let mut world: Self = toml::from_str(source).map_err(|e| e.to_string())?;
        world.set_tick();
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
        match self.version {
            1 if self.players.len() == 1
                && self.tick_rate.is_none()
                && self.arena.is_none()
                && self.ships.is_none() => {}
            1 => {
                return Err(
                    "Expected world version 1 with exactly one player, and no tick rate, arena or ships"
                        .to_owned(),
                );
            }
            2 if (1..=MAX_PILOTS).contains(&self.players.len()) => {}
            2 => {
                return Err(format!(
                    "Expected 1 to {MAX_PILOTS} players in a world version 2"
                ));
            }
            _ => return Err("Expected world version 1 or 2".to_owned()),
        }
        if !valid_name(&self.name) || self.bodies.len() > 128 {
            return Err("Invalid world name or more than 128 celestial bodies".to_owned());
        }
        if !self.gravity_constant.is_finite()
            || !(0.0..=MAX_GRAVITY_CONSTANT).contains(&self.gravity_constant)
        {
            return Err("Invalid gravitational constant (expected 0 through 10000)".to_owned());
        }
        for player in &self.players {
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
        if self.version == 2 {
            self.validate_multiplayer()?;
        }
        Ok(())
    }

    /// What a version 2 world adds: the tick, the arena, the allowed ships, and spawn templates
    /// that are inside the arena, clear of the bodies, and ships the world allows.
    fn validate_multiplayer(&self) -> Result<(), String> {
        let (Some(tick_rate), Some(arena), Some(ships)) = (self.tick_rate, self.arena, self.ships)
        else {
            return Err("A world version 2 needs a tick rate, an arena and ships".to_owned());
        };
        if !TICK_RATES.contains(&tick_rate) {
            return Err(format!(
                "Invalid tick rate (expected {TICK_RATES:?} per second)"
            ));
        }
        let half_side = arena.half_side;
        if !half_side.is_finite() || !(2.0..=MAX_ARENA_HALF_SIDE).contains(&half_side) {
            return Err(format!(
                "Invalid arena (expected a half side of 2 through {MAX_ARENA_HALF_SIDE})"
            ));
        }
        if !ships.valid() {
            return Err(
                "Invalid ships: intervals within 0.05 through 20, and positive caps".to_owned(),
            );
        }
        let radius = self.players[0].radius;
        let h = 1.0 / f64::from(tick_rate);
        for player in &self.players {
            let s = player.state;
            let reach = half_side - f64::from(radius);
            if player.radius != radius {
                return Err("All ships of a world share one hull radius".to_owned());
            }
            if player.task.params.h != h {
                return Err("A player's h must be 1 / tick_rate".to_owned());
            }
            if !ships.allows(&player.task.params) {
                return Err("A spawn template's ship is not among the allowed ships".to_owned());
            }
            if s.x.abs() > reach || s.y.abs() > reach {
                return Err("A spawn template is outside the arena".to_owned());
            }
            let inside_a_body = self.bodies.iter().any(|body| {
                let distance =
                    (f64::from(body.position[0]) - s.x).hypot(f64::from(body.position[1]) - s.y);
                distance < f64::from(body.radius) + f64::from(radius)
            });
            if inside_a_body {
                return Err("A spawn template is inside a body".to_owned());
            }
        }
        Ok(())
    }

    /// The tick `h`, in seconds: `1 / tick_rate` in a version 2 world, the player's own in a
    /// version 1 one.
    pub fn h(&self) -> f64 {
        self.tick_rate
            .map_or(self.players[0].task.params.h, |rate| 1.0 / f64::from(rate))
    }

    /// The hull radius `rho`, the same for every ship.
    pub fn hull_radius(&self) -> f32 {
        self.players[0].radius
    }

    /// The ship a pilot gets without choosing: the first spawn template's.
    pub fn default_ship(&self) -> Params {
        self.players[0].task.params
    }

    /// Whether `p` is a ship this world allows. In a version 1 world, only its own.
    pub fn allows(&self, p: &Params) -> bool {
        self.ships
            .map_or_else(|| *p == self.default_ship(), |ships| ships.allows(p))
    }

    /// `Grant`: the ship a pilot who asks for `request` gets, at this world's tick. See
    /// [`Ships::grant`]. A version 1 world allows no choice, and gives its own ship.
    #[must_use]
    pub fn grant(&self, request: &ShipRequest) -> Params {
        let default = self.default_ship();
        self.ships
            .map_or(default, |ships| ships.grant(&default, request))
    }

    /// The `index`th ship to join takes the spawn template `index mod n`. Ships can share a place,
    /// as ships do not touch in Phase A.
    pub fn spawn(&self, index: usize) -> &Player {
        &self.players[index % self.players.len()]
    }

    /// This world as a version 2 world for a server: at the tick `tick_rate`, in an arena of
    /// half side `half_side`, with the default ships, and `spawns` templates of the one player
    /// in a column, 4 units apart. Version 2 worlds are returned as they are.
    ///
    /// # Errors
    /// Fails when the result would not be valid, such as a body at the spawns.
    pub fn multiplayer(
        &self,
        tick_rate: u32,
        half_side: f64,
        spawns: usize,
    ) -> Result<Self, String> {
        if self.version == 2 {
            return Ok(self.clone());
        }
        let h = 1.0 / f64::from(tick_rate);
        let players = (0..spawns)
            .map(|k| {
                let mut player = self.players[0].clone();
                player.state.y += 4.0 * k as f64;
                player.task.params.h = h;
                player
            })
            .collect();
        let world = Self {
            version: 2,
            tick_rate: Some(tick_rate),
            arena: Some(Arena { half_side }),
            ships: Some(Ships::default()),
            players,
            ..self.clone()
        };
        world.validate()?;
        Ok(world)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::Rng;

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

    fn arena_world() -> World {
        Preset::PlanetAndMoon
            .world()
            .multiplayer(60, 1000.0, 4)
            .expect("a valid world")
    }

    #[test]
    fn a_version_2_world_round_trips_and_keeps_its_tick_exact() {
        let world = arena_world();
        assert_eq!(world.version, 2);
        assert_eq!(world.h(), 1.0 / 60.0);
        let source = world.to_toml().expect("serialize");
        assert!(source.contains("tick_rate = 60"), "{source}");
        let parsed = World::from_toml(&source).expect("parse");
        assert_eq!(parsed, world);
        // The file's own `h` is not trusted: the tick belongs to the world.
        let wrong = source.replace("h = 0.016666666666666666", "h = 0.0167");
        assert_ne!(wrong, source, "the file writes its h");
        let parsed = World::from_toml(&wrong).expect("parse");
        assert!(parsed.players.iter().all(|p| p.task.params.h == 1.0 / 60.0));
        assert_eq!(
            parsed.spawn(5).state,
            parsed.players[1].state,
            "templates repeat"
        );
    }

    #[test]
    fn version_1_worlds_keep_loading_as_they_were() {
        for preset in Preset::ALL {
            let world = preset.world();
            assert_eq!(world.version, 1);
            assert_eq!(
                (world.tick_rate, world.arena, world.ships),
                (None, None, None)
            );
            assert_eq!(world.h(), 0.02);
            let params = world.default_ship();
            assert_eq!(
                world.grant(&ShipRequest::from(Params {
                    mass: 9.0,
                    ..params
                })),
                params
            );
            assert!(world.allows(&params), "its own ship");
        }
    }

    #[test]
    fn rejects_invalid_version_2_worlds() {
        let good = arena_world();
        let bad = |edit: &dyn Fn(&mut World)| {
            let mut world = good.clone();
            edit(&mut world);
            world.validate().expect_err("should be rejected")
        };
        bad(&|w| w.tick_rate = None);
        bad(&|w| w.arena = None);
        bad(&|w| w.ships = None);
        bad(&|w| w.tick_rate = Some(5));
        bad(&|w| w.tick_rate = Some(101));
        bad(&|w| w.arena = Some(Arena { half_side: 2049.0 }));
        let nan_arena = Arena {
            half_side: f64::NAN,
        };
        bad(&|w| w.arena = Some(nan_arena));
        bad(&|w| w.arena = Some(Arena { half_side: 0.5 }));
        bad(&|w| w.players.clear());
        bad(&|w| w.players = vec![w.players[0].clone(); MAX_PILOTS + 1]);
        bad(&|w| w.players[1].task.params.h = 0.02);
        bad(&|w| w.players[1].radius = 2.0);
        bad(&|w| w.players[0].state.x = 1000.0);
        bad(&|w| {
            w.players[0].state = State {
                x: 0.0,
                y: -12.0,
                ..State::default()
            }
        });
        bad(&|w| w.players[0].task.params.mass = 50.0);
        bad(&|w| w.players[0].task.params.f_max = 20.0);
        bad(&|w| w.ships.as_mut().expect("ships").mass = [10.0, 0.1]);
        bad(&|w| w.ships.as_mut().expect("ships").acceleration_cap = 0.0);
        bad(&|w| w.ships.as_mut().expect("ships").angular_cap = f64::INFINITY);
        // Caps that leave no room for the smallest force.
        bad(&|w| w.ships.as_mut().expect("ships").acceleration_cap = 1.0);
        // A version 1 world has none of these.
        let mut v1 = Preset::Planet.world();
        v1.tick_rate = Some(60);
        v1.validate().expect_err("version 1 has no tick rate");
        let mut v3 = good;
        v3.version = 3;
        v3.validate().expect_err("unknown version");
    }

    /// Any numbers at all, as a hostile client sends.
    fn wild_request(rng: &mut Rng) -> ShipRequest {
        ShipRequest {
            mass: rng.wild(),
            inertia: rng.wild(),
            f_max: rng.wild(),
            tau_max: rng.wild(),
        }
    }

    #[test]
    fn grant_is_total_lands_in_the_allowed_ships_and_is_the_identity_on_them() {
        // Lemma "Grant".
        let world = arena_world();
        let ships = world.ships.expect("ships");
        let mut rng = Rng::new(0x6BA27);
        for _ in 0..200_000 {
            let request = wild_request(&mut rng);
            let granted = world.grant(&request);
            assert!(
                ships.allows(&granted),
                "{request:?} was granted {granted:?}"
            );
            assert_eq!(granted.h, world.h());
            assert_eq!(
                world.grant(&ShipRequest::from(granted)),
                granted,
                "a fixed point"
            );
        }
        // Every point of Pset, the corners and the capped edge included.
        for _ in 0..200_000 {
            let mass = rng.range(ships.mass[0], ships.mass[1]);
            let inertia = rng.range(ships.inertia[0], ships.inertia[1]);
            let f_top = ships.f_max[1].min(ships.acceleration_cap * mass);
            let tau_top = ships.tau_max[1].min(ships.angular_cap * inertia);
            let pick = |rng: &mut Rng, lo: f64, hi: f64| match rng.below(4) {
                0 => lo,
                1 => hi,
                _ => rng.range(lo, hi),
            };
            let p = Params {
                mass,
                inertia,
                f_max: pick(&mut rng, ships.f_max[0], f_top),
                tau_max: pick(&mut rng, ships.tau_max[0], tau_top),
                h: world.h(),
            };
            assert!(ships.allows(&p), "{p:?}");
            assert_eq!(
                world.grant(&ShipRequest::from(p)),
                p,
                "fixes every allowed ship"
            );
        }
    }

    #[test]
    fn grant_replaces_what_is_not_finite_with_the_default_and_clamps_the_rest() {
        let world = arena_world();
        let default = world.default_ship();
        let nan = f64::NAN;
        let granted = world.grant(&ShipRequest {
            mass: nan,
            inertia: nan,
            f_max: nan,
            tau_max: nan,
        });
        assert_eq!(granted, default);
        let granted = world.grant(&ShipRequest {
            mass: 1e9,
            inertia: -1.0,
            f_max: 1e9,
            tau_max: 1e9,
        });
        let ships = world.ships.expect("ships");
        assert_eq!(
            (granted.mass, granted.inertia),
            (ships.mass[1], ships.inertia[0])
        );
        assert_eq!(
            granted.f_max, ships.f_max[1],
            "within the cap at the largest mass"
        );
        // The cap binds for a light, strong ship: 20 / 0.1 would be an acceleration of 200.
        let light = world.grant(&ShipRequest {
            mass: 0.1,
            inertia: 10.0,
            f_max: 20.0,
            tau_max: 0.2,
        });
        assert_eq!(light.f_max, ships.acceleration_cap * 0.1);
        assert!(light.f_max / light.mass <= ships.acceleration_cap * (1.0 + 1e-15));
    }
}
