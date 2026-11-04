//! The little the two binaries share: flags, and the world a server runs.

use nabla_net::wire::Password;
use nabla_sim::world::{Preset, World};

/// The password in the file after `--password-file`, if the flag is there. The line ending of the
/// file's last line is not part of it.
///
/// It comes from a file so that it is not in the process list. Anything wrong with the file is an
/// error, so that a server meant to be private never starts open.
///
/// # Errors
/// If the flag has no value, the file cannot be read, or what it holds is not a password.
pub fn password_file(args: &[String]) -> Result<Option<Password>, String> {
    const FLAG: &str = "--password-file";
    if !args.iter().any(|a| a == FLAG) {
        return Ok(None);
    }
    let path = flag(args, FLAG).ok_or_else(|| format!("{FLAG} needs a file"))?;
    warn_if_others_can_read(&path);
    let text = std::fs::read_to_string(&path).map_err(|e| format!("cannot read {path}: {e}"))?;
    Password::parse(text.trim_end_matches(['\r', '\n']))
        .map(Some)
        .map_err(|why| format!("{path}: {why}"))
}

#[cfg(unix)]
fn warn_if_others_can_read(path: &str) {
    use std::os::unix::fs::PermissionsExt as _;
    if let Ok(metadata) = std::fs::metadata(path)
        && metadata.permissions().mode() & 0o077 != 0
    {
        log::warn!("{path} can be read by other users: `chmod 600` it");
    }
}

#[cfg(not(unix))]
fn warn_if_others_can_read(_path: &str) {}

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

#[cfg(test)]
mod tests {
    use super::*;

    /// The arguments that name a new file holding `contents`, which is removed when `check` ends.
    fn with_file(contents: &str, test: &str, check: impl FnOnce(&[String])) {
        let path =
            std::env::temp_dir().join(format!("nabla-password-{}-{test}", std::process::id()));
        std::fs::write(&path, contents).expect("a temporary file");
        check(&["--password-file".to_owned(), path.display().to_string()]);
        std::fs::remove_file(&path).expect("the temporary file is there");
    }

    #[test]
    fn no_flag_means_no_password() {
        assert_eq!(password_file(&["--world".to_owned()]), Ok(None));
    }

    #[test]
    fn the_last_line_ending_is_not_part_of_the_password() {
        let expected = Password::parse("hunter2").expect("valid");
        for (test, contents) in [
            ("bare", "hunter2"),
            ("lf", "hunter2\n"),
            ("crlf", "hunter2\r\n"),
        ] {
            with_file(contents, test, |args| {
                assert_eq!(password_file(args), Ok(Some(expected.clone())), "{test}");
            });
        }
    }

    #[test]
    fn a_server_meant_to_be_private_never_starts_open() {
        assert!(password_file(&["--password-file".to_owned()]).is_err());
        assert!(
            password_file(&[
                "--password-file".to_owned(),
                "/nonexistent/nabla-password".to_owned()
            ])
            .is_err()
        );
        for (test, contents) in [("empty", ""), ("blank", "\n"), ("spaced", " hunter2\n")] {
            with_file(contents, test, |args| {
                assert!(password_file(args).is_err(), "{test}");
            });
        }
    }
}
