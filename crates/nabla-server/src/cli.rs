//! The little the two binaries share: flags, and the world a server runs.

use nabla_sim::world::{Preset, World};

/// The value after `--name`, if the flag is there.
pub fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

/// The value after `--name`, parsed, or `default`.
///
/// # Errors
/// If the flag is there and its value does not parse.
pub fn parsed<T: std::str::FromStr>(args: &[String], name: &str, default: T) -> Result<T, String> {
    match flag(args, name) {
        None => Ok(default),
        Some(text) => text
            .parse()
            .map_err(|_unreadable| format!("{name}: cannot read {text:?}")),
    }
}

/// A bundled world by name, or a world file, as a version 2 world for a server.
///
/// # Errors
/// If the file cannot be read, or the world would not be valid.
pub fn world(name: &str, tick_rate: u32, half_side: f64, spawns: usize) -> Result<World, String> {
    let world = match name {
        "vacuum" => Preset::Vacuum.world(),
        "planet" => Preset::Planet.world(),
        "planet-moon" => Preset::PlanetAndMoon.world(),
        path => {
            let source =
                std::fs::read_to_string(path).map_err(|e| format!("cannot read {path}: {e}"))?;
            World::from_toml(&source)?
        }
    };
    world.multiplayer(tick_rate, half_side, spawns)
}
