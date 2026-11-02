//! Works out what this build is, for `version::build_version!`: `NABLA_BUILD_VERSION`.

use std::env;
use std::path::Path;
use std::process::Command;

/// The output of `git args`, if it ran and succeeded. Never takes git's optional locks: a mere
/// `git status` would otherwise rewrite the index, which would make this script run again.
fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("--no-optional-locks")
        .args(args)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// The commit's short hash, or `dev` with uncommitted changes or without git.
fn development_version() -> String {
    let clean = git(&["status", "--porcelain"]).is_some_and(|changes| changes.is_empty());
    clean
        .then(|| git(&["rev-parse", "--short", "HEAD"]))
        .flatten()
        .unwrap_or_else(|| "dev".to_owned())
}

/// Whether this is the `release` profile. Cargo's `PROFILE` can't tell: it says `release` for
/// the `fast` profile of `cargo nabla` too, which inherits from it. So this goes by the profile's
/// directory: `OUT_DIR` is `<profile>/build/<package>/out`.
fn is_release() -> bool {
    let out_dir = env::var("OUT_DIR").expect("set by Cargo");
    Path::new(&out_dir)
        .ancestors()
        .nth(3)
        .and_then(Path::file_name)
        .is_some_and(|profile| profile == "release")
}

/// Runs again when a commit is made or anything that's built changes, which is what can change
/// the answer. A path that doesn't exist would make it run every time.
fn rerun_when_this_changes() {
    let in_git = ["HEAD", "index", "logs/HEAD"]
        .into_iter()
        .filter_map(|file| git(&["rev-parse", "--git-path", file]));
    let sources = [
        "src",
        "assets",
        "build.rs",
        "Cargo.toml",
        "../nabla-core/src",
        "../nabla-core/Cargo.toml",
        "../nabla-synth/src",
        "../nabla-synth/Cargo.toml",
        "../../Cargo.toml",
        "../../Cargo.lock",
    ]
    .map(str::to_owned);
    for path in in_git.chain(sources) {
        if Path::new(&path).exists() {
            println!("cargo::rerun-if-changed={path}");
        }
    }
}

fn main() {
    rerun_when_this_changes();
    let version = if is_release() {
        format!("v{}", env::var("CARGO_PKG_VERSION").expect("set by Cargo"))
    } else {
        development_version()
    };
    println!("cargo::rustc-env=NABLA_BUILD_VERSION={version}");
}
