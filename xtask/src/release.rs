//! The steps of a release (see the README) up to the last one, pushing the tag: `bump` sets the
//! workspace version, and `tag` checks that the checkout is ready and tags it.

use std::fmt;
use std::ops::Range;
use std::str::FromStr;

use anyhow::{Context as _, Result, bail};
use xshell::{Shell, cmd};

use crate::cargo;

const MANIFEST: &str = "Cargo.toml";

/// A release version, `X.Y.Z`. Releases have no pre-release or build parts, and the order is
/// that of the fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Version {
    major: u64,
    minor: u64,
    patch: u64,
}

impl Version {
    /// The next version, for the `part` to increase: `major`, `minor` or `patch`.
    fn bumped(self, part: &str) -> Result<Self> {
        let Self {
            major,
            minor,
            patch,
        } = self;
        let (major, minor, patch) = match part {
            "major" => (major + 1, 0, 0),
            "minor" => (major, minor + 1, 0),
            "patch" => (major, minor, patch + 1),
            _ => bail!("cannot bump `{part}`: expected `major`, `minor` or `patch`"),
        };
        Ok(Self {
            major,
            minor,
            patch,
        })
    }
}

impl FromStr for Version {
    type Err = anyhow::Error;

    fn from_str(version: &str) -> Result<Self> {
        let parsed = (|| {
            let mut parts = version.split('.').map(|part| part.parse().ok());
            let parsed = Self {
                major: parts.next()??,
                minor: parts.next()??,
                patch: parts.next()??,
            };
            parts.next().is_none().then_some(parsed)
        })();
        parsed.with_context(|| format!("version `{version}` is not of the form X.Y.Z"))
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// Where the workspace version is in the text of the root manifest: the range between the quotes.
fn version_span(manifest: &str) -> Result<Range<usize>> {
    const KEY: &str = "version = \"";
    let mut section = "";
    let mut line_start = 0;
    for line in manifest.split_inclusive('\n') {
        if line.starts_with('[') {
            section = line.trim_end();
        } else if section == "[workspace.package]"
            && let Some(value) = line.strip_prefix(KEY)
            && let Some((value, _)) = value.split_once('"')
        {
            let start = line_start + KEY.len();
            return Ok(start..start + value.len());
        }
        line_start += line.len();
    }
    bail!("no `version = \"...\"` line in [workspace.package] of {MANIFEST}")
}

/// The `bump` task: increase the workspace version, here and in `Cargo.lock`. The commit is left
/// to the caller, as the change may go with others.
#[expect(clippy::print_stdout, reason = "the new version is the task's output")]
pub fn bump(sh: &Shell, part: &str) -> Result<()> {
    let mut manifest = sh.read_file(MANIFEST)?;
    let span = version_span(&manifest)?;
    let old: Version = manifest[span.clone()].parse()?;
    let new = old.bumped(part)?;
    manifest.replace_range(span, &new.to_string());
    sh.write_file(MANIFEST, manifest)?;

    // Only the workspace's own packages: this leaves the dependencies' versions alone.
    let cargo = cargo();
    cmd!(sh, "{cargo} update --quiet --workspace").run()?;

    println!("{old} -> {new}");
    Ok(())
}

/// The `tag` task: check that the checkout is what should be released, tag it with the workspace
/// version, and leave only the push. Running it again after a failed push is fine.
#[expect(clippy::print_stdout, reason = "the push command is the task's output")]
pub fn tag(sh: &Shell) -> Result<()> {
    let branch = cmd!(sh, "git branch --show-current").read()?;
    if branch != "main" {
        let branch = if branch.is_empty() {
            "a detached HEAD".to_owned()
        } else {
            format!("`{branch}`")
        };
        bail!("releases are tagged on `main`, but the checkout is on {branch}");
    }
    // This also covers a stale `Cargo.lock`, which CI's `--locked` builds would reject: the
    // `cargo run` behind this task has already updated it.
    let changes = cmd!(sh, "git status --porcelain").read()?;
    if !changes.is_empty() {
        bail!("commit or remove these changes first, they would not be released:\n{changes}");
    }

    let manifest = sh.read_file(MANIFEST)?;
    let version: Version = manifest[version_span(&manifest)?].parse()?;
    let tag = format!("v{version}");
    let tag_ref = format!("refs/tags/{tag}");

    let remote = cmd!(sh, "git ls-remote origin refs/heads/main {tag_ref}")
        .read()
        .context("reading `main` and the tag from origin")?;
    let on_remote = |reference: &str| {
        remote.lines().find_map(|line| {
            let (commit, name) = line.split_once('\t')?;
            (name == reference).then_some(commit)
        })
    };
    if on_remote(&tag_ref).is_some() {
        bail!("{tag} is already on origin: bump the version first, with `cargo bump`");
    }
    let head = cmd!(sh, "git rev-parse HEAD").read()?;
    if on_remote("refs/heads/main") != Some(head.as_str()) {
        bail!("local `main` and `origin/main` differ: push or pull first");
    }

    // Empty if there is no such tag.
    let tagged_commit = format!("{tag_ref}^{{commit}}");
    let tagged = cmd!(sh, "git rev-parse --quiet --verify {tagged_commit}")
        .ignore_status()
        .read()?;
    if tagged.is_empty() {
        let tags = cmd!(sh, "git tag --list v*.*.*").read()?;
        let newest = tags
            .lines()
            .filter_map(|tag| tag.strip_prefix('v')?.parse::<Version>().ok())
            .max();
        if let Some(newest) = newest.filter(|newest| *newest >= version) {
            bail!("the version in {MANIFEST} is {version}, not newer than the tag v{newest}");
        }
        cmd!(sh, "git tag -a {tag} -m {tag}").run()?;
    } else if tagged != head {
        bail!(
            "{tag} is on another commit, {tagged}. Bump the version, or if that tag is a \
             mistake, delete it with `git tag -d {tag}`"
        );
    }

    println!("{tag} is ready to publish:\n\n    git push origin {tag}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(major: u64, minor: u64, patch: u64) -> Version {
        Version {
            major,
            minor,
            patch,
        }
    }

    #[test]
    fn parses_only_release_versions() {
        assert_eq!("1.20.3".parse::<Version>().ok(), Some(version(1, 20, 3)));
        for bad in ["", "1", "1.2", "1.2.3.4", "1.2.x", "1.2.3-rc.1", "v1.2.3"] {
            assert!(bad.parse::<Version>().is_err(), "accepted `{bad}`");
        }
    }

    #[test]
    fn bumps_reset_the_lower_parts() {
        let old = version(1, 2, 3);
        assert_eq!(old.bumped("major").ok(), Some(version(2, 0, 0)));
        assert_eq!(old.bumped("minor").ok(), Some(version(1, 3, 0)));
        assert_eq!(old.bumped("patch").ok(), Some(version(1, 2, 4)));
        assert!(old.bumped("huge").is_err(), "accepted an unknown part");
    }

    #[test]
    fn versions_compare_numerically() {
        assert!(version(0, 10, 0) > version(0, 9, 9), "compared as text");
    }

    #[test]
    fn finds_the_workspace_version_only() {
        let manifest = "\
[package]
version = \"9.9.9\"

[workspace.package]
edition = \"2024\"
version = \"0.18.0\"

[dependencies]
version = \"8.8.8\"
";
        let span = version_span(manifest).ok();
        assert_eq!(span.map(|span| &manifest[span]), Some("0.18.0"));
        assert!(
            version_span("[package]\nversion = \"9.9.9\"\n").is_err(),
            "found a version outside [workspace.package]"
        );
    }
}
