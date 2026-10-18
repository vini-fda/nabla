//! Development tasks for this repository, in Rust rather than shell scripts (the `cargo xtask`
//! pattern: <https://github.com/matklad/cargo-xtask>). Run `cargo xtask help`.

mod bundle;
mod solutions;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context as _, Result, bail};
use xshell::{Shell, cmd};

const HELP: &str = "\
Usage: cargo xtask <task> [options]

Solutions (see TASK.md):
  eval <solution.rs>...
      Compile solutions and evaluate them on both suites. One file: a table of every case,
      then the score. Several: they compile in parallel, then get one line each, best first.
  submit <name> <solution.rs>
      Evaluate a solution and, if it solves every case, store it as `solutions/<name>.rs`.
  leaderboard
      The stored solutions, best first.
  verify
      Re-evaluate every stored solution and check its entry in `solutions/index.tsv`.

Viewer:
  run [--profile <name>] [--target <triple>] [-- <viewer arguments>...]
      Build and run the viewer. On macOS it runs inside the app bundle, so the Dock shows the
      Liquid Glass icon; an unchanged bundle is only refreshed with the new binary.
      `cargo viewer` is `cargo xtask run --profile fast`.
  bundle [--profile <name>] [--target <triple>] [--flat-icon] [--sign <identity>]
      Build the viewer and wrap it in a macOS app bundle, `Nabla.app`, next to the binary
      (`target/release/Nabla.app` by default). macOS only; needs Xcode, for `actool`.
      --flat-icon skips the Liquid Glass icon (Assets.car) and uses only the flat .icns.
      --sign signs with a keychain identity (name or SHA-1), with the hardened runtime and a
      secure timestamp, as notarization requires. Without it, the signature is ad-hoc.
  The profile defaults to `release`.

Repository:
  check
      The checks CI runs (.github/workflows/ci.yml) for this platform, plus the web build,
      which needs `trunk`.
";

/// The options shared by every task: which build of the viewer to use.
struct Build {
    profile: String,
    target: Option<String>,
}

impl Build {
    /// `cargo build` the viewer, and return the path of its binary.
    fn run(&self, sh: &Shell) -> Result<PathBuf> {
        let cargo = cargo();
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

fn cargo() -> String {
    std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned())
}

fn main() -> Result<ExitCode> {
    let mut args = std::env::args().skip(1);
    let task = args.next().unwrap_or_default();
    let args: Vec<String> = args.collect();

    let sh = Shell::new()?;
    sh.change_dir(project_root());
    match (task.as_str(), args.as_slice()) {
        ("eval", paths) => solutions::eval(paths)?,
        ("submit", [name, path]) => solutions::submit(name, path)?,
        ("leaderboard", []) => solutions::leaderboard()?,
        ("verify", []) => solutions::verify()?,
        ("check", []) => check(&sh)?,
        ("bundle" | "run", _) => return viewer(&sh, &task, args),
        ("help" | "--help" | "-h", _) => print_help(),
        ("submit" | "leaderboard" | "verify" | "check", _) => {
            bail!("wrong arguments for `{task}`\n\n{HELP}")
        }
        _ => bail!("unknown task `{task}`\n\n{HELP}"),
    }
    Ok(ExitCode::SUCCESS)
}

/// The `bundle` and `run` tasks.
fn viewer(sh: &Shell, task: &str, args: Vec<String>) -> Result<ExitCode> {
    let mut build = Build {
        profile: "release".to_owned(),
        target: None,
    };
    let mut flat_icon = false;
    let mut identity = None;
    let mut passthrough = Vec::new();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--profile" => build.profile = args.next().context("--profile needs a name")?,
            "--target" => build.target = Some(args.next().context("--target needs a triple")?),
            "--flat-icon" if task == "bundle" => flat_icon = true,
            "--sign" if task == "bundle" => {
                identity = Some(args.next().context("--sign needs an identity")?);
            }
            "--" if task == "run" => passthrough.extend(args.by_ref()),
            _ => bail!("unexpected argument `{arg}`\n\n{HELP}"),
        }
    }

    if task == "bundle" && !cfg!(target_os = "macos") {
        bail!("app bundles are macOS only");
    }
    let mut bin = build.run(sh)?;
    if task == "bundle" {
        bundle::create(sh, &bin, flat_icon, identity.as_deref())?;
        return Ok(ExitCode::SUCCESS);
    }
    if cfg!(target_os = "macos") {
        bin = bundle::refresh(sh, &bin)?;
    }
    run(&bin, &passthrough)
}

/// The checks of `.github/workflows/ci.yml` for this platform, plus the web build.
fn check(sh: &Shell) -> Result<()> {
    let cargo = cargo();
    let _rustflags = sh.push_env("RUSTFLAGS", "-D warnings");
    let _rustdocflags = sh.push_env("RUSTDOCFLAGS", "-D warnings");
    cmd!(sh, "{cargo} fmt --all -- --check").run()?;
    cmd!(
        sh,
        "{cargo} doc --locked --quiet --workspace --no-deps --document-private-items"
    )
    .run()?;
    cmd!(
        sh,
        "{cargo} clippy --locked --quiet --workspace --all-targets"
    )
    .run()?;
    cmd!(
        sh,
        "{cargo} clippy --locked --quiet -p ship2d_graphics --lib --target wasm32-unknown-unknown"
    )
    .run()?;
    // As in CI: release mode, with debug assertions and overflow checks on for this
    // workspace's crates.
    let checks = ["ship2d_optimization", "ship2d_graphics"]
        .iter()
        .flat_map(|package| {
            ["debug-assertions", "overflow-checks"].map(|setting| {
                [
                    "--config".to_owned(),
                    format!("profile.release.package.{package}.{setting}=true"),
                ]
            })
        })
        .flatten();
    cmd!(
        sh,
        "{cargo} test --locked --quiet --workspace --release {checks...}"
    )
    .run()?;
    let _viewer = sh.push_dir("viewer");
    cmd!(sh, "trunk build --release").run()?;
    Ok(())
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
