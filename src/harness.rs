//! Compile a solution given as Rust source and evaluate it in a separate process.
//!
//! The source must define `pub struct Solution` implementing [`crate::Controller`]. It is written
//! into a throwaway cargo project under `target/candidates/<hash>/` that depends on this crate,
//! built in release mode (all candidates share one target directory, so this crate is only
//! compiled once), and run under a time limit. The candidate prints its report, or the
//! trajectory of one case, on stdout.
//!
//! The candidate runs as ordinary native code with no sandbox: only evaluate sources you trust.

use crate::eval::{Report, parse_trajectory};
use crate::problem::Trajectory;
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

/// Root of this crate.
pub fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// FNV-1a hash of a source text, as 16 hex digits.
pub fn source_hash(src: &str) -> String {
    let hash = src.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
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

/// Compile `src` into a [`Candidate`]. Unchanged sources reuse the previous build.
pub fn build_source(src: &str) -> Result<Candidate, HarnessError> {
    let hash = source_hash(src);
    let target = crate_dir().join("target");
    let project = target.join("candidates").join(&hash);
    let package = format!("candidate_{hash}");
    std::fs::create_dir_all(project.join("src"))?;
    std::fs::write(
        project.join("Cargo.toml"),
        format!(
            "[package]\nname = \"{package}\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n\
             [dependencies]\nship2d_optimization = {{ path = {} }}\n\n[workspace]\n",
            toml_string(&crate_dir())
        ),
    )?;
    std::fs::write(
        project.join("src/main.rs"),
        "mod solution;\n\nfn main() {\n    \
         ship2d_optimization::eval::candidate_main::<solution::Solution>();\n}\n",
    )?;
    std::fs::write(project.join("src/solution.rs"), src)?;

    let shared_target = target.join("candidates-target");
    let build = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .args(["build", "--release", "--quiet", "--manifest-path"])
        .arg(project.join("Cargo.toml"))
        .env("CARGO_TARGET_DIR", &shared_target)
        .output()?;
    if !build.status.success() {
        return Err(HarnessError::Compile(
            String::from_utf8_lossy(&build.stderr).into_owned(),
        ));
    }
    Ok(Candidate {
        binary: shared_target.join("release").join(&package),
    })
}

impl Candidate {
    /// Evaluate on both suites.
    pub fn evaluate(&self) -> Result<Report, HarnessError> {
        Report::parse(&self.run(&[])?).map_err(HarnessError::Report)
    }

    /// Simulate the case of either suite named `case`, recording the trajectory.
    pub fn trace(&self, case: &str) -> Result<Trajectory, HarnessError> {
        parse_trajectory(&self.run(&["trace", case])?).map_err(HarnessError::Report)
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
