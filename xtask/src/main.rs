//! Development tasks for this repository, in Rust rather than shell scripts (the `cargo xtask`
//! pattern: <https://github.com/matklad/cargo-xtask>). Run `cargo xtask help`.

mod bundle;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context as _, Result, bail};
use xshell::{Shell, cmd};

const HELP: &str = "\
Usage: cargo xtask <task> [options]

Tasks:
  bundle [--profile <name>] [--target <triple>] [--flat-icon]
      Build the viewer and wrap it in a macOS app bundle, `Nabla.app`, next to the binary
      (`target/release/Nabla.app` by default). macOS only; needs Xcode, for `actool`.
      --flat-icon skips the Liquid Glass icon (Assets.car) and uses only the flat .icns.

  run [--profile <name>] [--target <triple>] [-- <viewer arguments>...]
      Build and run the viewer. On macOS it runs inside the app bundle, so the Dock shows the
      Liquid Glass icon; an unchanged bundle is only refreshed with the new binary.
      `cargo viewer` is `cargo xtask run --profile fast`.

The profile defaults to `release`.
";

/// The options shared by every task: which build of the viewer to use.
struct Build {
    profile: String,
    target: Option<String>,
}

impl Build {
    /// `cargo build` the viewer, and return the path of its binary.
    fn run(&self, sh: &Shell) -> Result<PathBuf> {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
        let profile = &self.profile;
        let target = self.target.as_ref().map(|t| ["--target", t]);
        let target = target.iter().flatten();
        cmd!(
            sh,
            "{cargo} build --profile {profile} {target...} -p ship2d_graphics"
        )
        .run()?;

        let mut dir = target_dir();
        if let Some(target) = &self.target {
            dir.push(target);
        }
        // Cargo puts the `dev` and `test` profiles' output in `debug`.
        dir.push(match profile.as_str() {
            "dev" | "test" => "debug",
            "bench" => "release",
            other => other,
        });
        Ok(dir.join(format!("ship2d_graphics{}", std::env::consts::EXE_SUFFIX)))
    }
}

/// The repository root.
fn project_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask is one level below the root")
}

fn target_dir() -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR")
        .map_or_else(|| project_root().join("target"), PathBuf::from)
}

fn main() -> Result<ExitCode> {
    let mut args = std::env::args().skip(1);
    let task = args.next().unwrap_or_default();

    let mut build = Build {
        profile: "release".to_owned(),
        target: None,
    };
    let mut flat_icon = false;
    let mut passthrough = Vec::new();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--profile" => build.profile = args.next().context("--profile needs a name")?,
            "--target" => build.target = Some(args.next().context("--target needs a triple")?),
            "--flat-icon" if task == "bundle" => flat_icon = true,
            "--" if task == "run" => passthrough.extend(args.by_ref()),
            _ => bail!("unexpected argument `{arg}`\n\n{HELP}"),
        }
    }

    let sh = Shell::new()?;
    sh.change_dir(project_root());
    match task.as_str() {
        "bundle" => {
            if !cfg!(target_os = "macos") {
                bail!("app bundles are macOS only");
            }
            let bin = build.run(&sh)?;
            bundle::create(&sh, &bin, flat_icon)?;
            Ok(ExitCode::SUCCESS)
        }
        "run" => {
            let mut bin = build.run(&sh)?;
            if cfg!(target_os = "macos") {
                bin = bundle::refresh(&sh, &bin)?;
            }
            run(&bin, &passthrough)
        }
        "help" | "--help" | "-h" => {
            print_help();
            Ok(ExitCode::SUCCESS)
        }
        _ => bail!("unknown task `{task}`\n\n{HELP}"),
    }
}

#[expect(clippy::print_stdout, reason = "the help text is the task's output")]
fn print_help() {
    print!("{HELP}");
}

/// Run `bin` in place of this process where the platform allows, so the viewer gets the
/// terminal's signals (Ctrl+C) directly.
fn run(bin: &Path, args: &[String]) -> Result<ExitCode> {
    let mut command = std::process::Command::new(bin);
    command.args(args);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        let error = command.exec();
        Err(error).with_context(|| format!("running {}", bin.display()))
    }
    #[cfg(not(unix))]
    {
        let status = command
            .status()
            .with_context(|| format!("running {}", bin.display()))?;
        Ok(status
            .code()
            .and_then(|code| u8::try_from(code).ok())
            .map_or(ExitCode::FAILURE, ExitCode::from))
    }
}
