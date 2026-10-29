//! Compile a solution given as Rust source and evaluate it in a separate process.
//!
//! The source must define `pub struct Solution` implementing [`nabla_core::Controller`]. It is
//! written into a cargo project of its own, `target/candidates/<hash>/`, that depends on
//! `nabla-core` and this crate, built in release mode, and run under a time limit. The candidate
//! prints its report, or the trajectory of one case, on stdout.
//!
//! Every candidate project has its own target directory, so builds of different sources never
//! wait for each other's cargo lock: any number of evaluations can run at once, in one process
//! ([`build_sources`]) or in many. The price is that each new source also compiles both crates,
//! about a second. An unchanged source is not rebuilt.
//!
//! The candidate runs as ordinary native code with no sandbox: only evaluate sources you trust.

use crate::eval::{Report, parse_trajectory};
use crate::parallel;
use nabla_core::problem::Trajectory;
use std::fmt;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// Wall-clock limit for running a compiled candidate on both suites.
pub const TIME_LIMIT: Duration = Duration::from_secs(300);

#[derive(Debug)]
pub enum HarnessError {
    Io(io::Error),
    /// The source does not compile; carries the compiler output.
    Compile(String),
    /// The candidate process failed; carries its stderr.
    Crash(String),
    Timeout,
    /// The candidate's output is not a valid report.
    Report(String),
}

impl fmt::Display for HarnessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HarnessError::Io(e) => write!(f, "I/O error: {e}"),
            HarnessError::Compile(log) => write!(f, "compilation failed:\n{log}"),
            HarnessError::Crash(log) => write!(f, "candidate crashed:\n{log}"),
            HarnessError::Timeout => write!(
                f,
                "candidate exceeded the {}s time limit",
                TIME_LIMIT.as_secs()
            ),
            HarnessError::Report(e) => write!(f, "invalid report: {e}"),
        }
    }
}

impl From<io::Error> for HarnessError {
    fn from(e: io::Error) -> Self {
        HarnessError::Io(e)
    }
}

/// Root of the workspace's source tree (`crates/nabla-optimization/../..`), where it was when
/// this crate was compiled.
pub fn source_dir() -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest
        .ancestors()
        .nth(2)
        .unwrap_or(manifest)
        .to_path_buf()
}

/// Where the workspace is published; released binaries build candidates against its version
/// tag.
pub const REPOSITORY: &str = "https://github.com/vini-fda/nabla";

/// The source tree is still where it was compiled (development), as opposed to a released
/// binary running on another machine.
fn has_source_tree(dir: &Path) -> bool {
    dir.join("Cargo.toml").is_file()
}

/// How a candidate depends on the workspace crate `name`: the local source tree if `dir` holds
/// it, else the git tag of this exact version.
fn dependency(dir: &Path, name: &str) -> String {
    if has_source_tree(dir) {
        format!(
            "{{ path = {} }}",
            toml_string(&dir.join("crates").join(name))
        )
    } else {
        format!(
            "{{ git = \"{REPOSITORY}\", tag = \"v{}\" }}",
            env!("CARGO_PKG_VERSION")
        )
    }
}

/// Where candidate projects and builds go: the workspace's `target/` in development, else a
/// per-user cache directory.
fn work_dir(dir: &Path) -> PathBuf {
    if has_source_tree(dir) {
        return dir.join("target");
    }
    let var = |key: &str| std::env::var_os(key).map(PathBuf::from);
    let cache = if cfg!(windows) {
        var("LOCALAPPDATA")
    } else if cfg!(target_os = "macos") {
        var("HOME").map(|home| home.join("Library/Caches"))
    } else {
        var("XDG_CACHE_HOME").or_else(|| var("HOME").map(|home| home.join(".cache")))
    };
    cache.unwrap_or_else(std::env::temp_dir).join("nabla")
}

/// FNV-1a hash of a source text, as 16 hex digits. Carriage returns are skipped, so a file
/// checked out with CRLF line endings (Windows) has the same hash as its LF original.
pub fn source_hash(src: &str) -> String {
    let hash = src
        .bytes()
        .filter(|&b| b != b'\r')
        .fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
        });
    format!("{hash:016x}")
}

fn toml_string(path: &Path) -> String {
    format!(
        "\"{}\"",
        path.display()
            .to_string()
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
    )
}

/// A compiled solution: a native binary that evaluates or traces it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub binary: PathBuf,
}

/// Write `contents` to `path` unless it already holds exactly that, so that an unchanged file
/// keeps its modification time (no rebuild) and is never rewritten under a concurrent build.
fn write_if_changed(path: &Path, contents: &str) -> io::Result<()> {
    if std::fs::read(path).is_ok_and(|old| old == contents.as_bytes()) {
        return Ok(());
    }
    std::fs::write(path, contents)
}

/// Compile `src` into a [`Candidate`]. Unchanged sources reuse the previous build.
///
/// Needs `cargo`. Without the source tree (a released binary), the first build downloads this
/// version of `nabla-core` and `nabla-optimization` from [`REPOSITORY`].
pub fn build_source(src: &str) -> Result<Candidate, HarnessError> {
    let dir = source_dir();
    let project = work_dir(&dir).join("candidates").join(source_hash(src));
    std::fs::create_dir_all(project.join("src"))?;
    write_if_changed(
        &project.join("Cargo.toml"),
        &format!(
            "[package]\nname = \"candidate\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n\
             [dependencies]\nnabla-core = {}\nnabla-optimization = {}\n\n[workspace]\n",
            dependency(&dir, "nabla-core"),
            dependency(&dir, "nabla-optimization"),
        ),
    )?;
    write_if_changed(
        &project.join("src/main.rs"),
        "mod solution;\n\nfn main() {\n    \
         nabla_optimization::eval::candidate_main::<solution::Solution>();\n}\n",
    )?;
    write_if_changed(&project.join("src/solution.rs"), src)?;

    // The project's own target directory: see the module documentation.
    let target = project.join("target");
    let build = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .args(["build", "--release", "--quiet", "--manifest-path"])
        .arg(project.join("Cargo.toml"))
        .arg("--target-dir")
        .arg(&target)
        .output()
        .map_err(|e| match e.kind() {
            io::ErrorKind::NotFound => HarnessError::Compile(
                "Compiling a solution needs Rust's `cargo`, which was not found. \
                 Install Rust from https://rustup.rs and restart."
                    .to_string(),
            ),
            _ => HarnessError::Io(e),
        })?;
    if !build.status.success() {
        return Err(HarnessError::Compile(
            String::from_utf8_lossy(&build.stderr).into_owned(),
        ));
    }
    Ok(Candidate {
        binary: target
            .join("release")
            .join(format!("candidate{}", std::env::consts::EXE_SUFFIX)),
    })
}

/// [`build_source`] for every source, in parallel. The results are in the order of `sources`.
pub fn build_sources(sources: &[&str]) -> Vec<Result<Candidate, HarnessError>> {
    parallel::map(sources, |src| build_source(src))
}

impl Candidate {
    /// Evaluate on both suites.
    pub fn evaluate(&self) -> Result<Report, HarnessError> {
        Report::parse(&self.run(&[])?).map_err(HarnessError::Report)
    }

    /// Simulate the case of either suite named `case` with sampling period `h`, recording the
    /// trajectory.
    pub fn trace(&self, case: &str, h: f64) -> Result<Trajectory, HarnessError> {
        let h = h.to_string();
        parse_trajectory(&self.run(&["trace", case, &h])?).map_err(HarnessError::Report)
    }

    /// Run the binary under [`TIME_LIMIT`] and return its stdout.
    fn run(&self, args: &[&str]) -> Result<String, HarnessError> {
        let mut child = Command::new(&self.binary)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let drain = |mut pipe: Box<dyn Read + Send>| {
            thread::spawn(move || {
                let mut buf = Vec::new();
                pipe.read_to_end(&mut buf)
                    .map(|_| String::from_utf8_lossy(&buf).into_owned())
            })
        };
        let stdout = drain(Box::new(child.stdout.take().expect("piped stdout")));
        let stderr = drain(Box::new(child.stderr.take().expect("piped stderr")));

        let start = Instant::now();
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            if start.elapsed() > TIME_LIMIT {
                child.kill()?;
                child.wait()?;
                return Err(HarnessError::Timeout);
            }
            thread::sleep(Duration::from_millis(20));
        };
        let stdout = stdout.join().expect("stdout reader panicked")?;
        let stderr = stderr.join().expect("stderr reader panicked")?;
        if !status.success() {
            return Err(HarnessError::Crash(stderr));
        }
        Ok(stdout)
    }
}

/// Compile `src` and evaluate it on both suites.
pub fn evaluate_source(src: &str) -> Result<Report, HarnessError> {
    build_source(src)?.evaluate()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_hash_ignores_line_ending_style() {
        assert_eq!(source_hash("a\nb\n"), source_hash("a\r\nb\r\n"));
        assert_ne!(source_hash("a\nb\n"), source_hash("a\nc\n"));
    }

    #[test]
    fn candidates_use_the_source_tree_when_present() {
        let dir = source_dir();
        assert!(dependency(&dir, "nabla-core").starts_with("{ path = "));
        assert_eq!(work_dir(&dir), dir.join("target"));
    }

    #[test]
    fn released_binaries_use_the_version_tag_and_a_cache() {
        let missing = Path::new("/nonexistent/nabla");
        let tag = format!("tag = \"v{}\"", env!("CARGO_PKG_VERSION"));
        let dependency = dependency(missing, "nabla-core");
        assert!(
            dependency.contains(REPOSITORY) && dependency.contains(&tag),
            "{dependency}"
        );
        let work = work_dir(missing);
        assert!(
            work.ends_with("nabla") && !work.starts_with(missing),
            "{work:?}"
        );
    }
}
