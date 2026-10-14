//! A solution given as Rust source text, and running it to get trajectories.
//!
//! A source is any `.rs` text defining `pub struct Solution` implementing
//! `ship2d_optimization::Controller`. To run it on a case:
//!
//! * if the text is byte-identical to a stored solution, use the copy compiled into the viewer
//!   (`ship2d_optimization`'s `registry`), which is instant and also works on the web;
//! * otherwise (native only) compile it with `ship2d_optimization::harness` on a background
//!   thread, and ask the compiled binary for the trajectory of the case.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::SystemTime;

use ship2d_optimization::db::{Entry, parse_index};
use ship2d_optimization::harness::source_hash;
use ship2d_optimization::problem::{Case, Trajectory};
use ship2d_optimization::registry::{self, StoredSolution};

/// Rust source text of a solution.
#[derive(Clone, Debug)]
pub struct SolutionSource {
    /// File name without `.rs`.
    pub name: String,
    /// Where it was read from, if it is a file on disk (enables reloading).
    pub path: Option<PathBuf>,
    pub text: String,
    pub hash: String,
    modified: Option<SystemTime>,
}

impl SolutionSource {
    /// Read a `.rs` file.
    ///
    /// # Errors
    /// If the file cannot be read as UTF-8 text.
    pub fn from_file(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let name = path.file_stem().map_or_else(
            || "solution".to_owned(),
            |s| s.to_string_lossy().into_owned(),
        );
        Ok(Self {
            name,
            path: Some(path.to_owned()),
            hash: source_hash(&text),
            modified: modified_time(path),
            text,
        })
    }

    /// Source text that does not come from a file (e.g. a file dropped on the web page).
    pub fn from_text(name: &str, text: String) -> Self {
        Self {
            name: name.trim_end_matches(".rs").to_owned(),
            path: None,
            hash: source_hash(&text),
            modified: None,
            text,
        }
    }

    /// The file was modified on disk since it was read.
    pub fn modified_on_disk(&self) -> bool {
        self.path
            .as_deref()
            .is_some_and(|p| modified_time(p) != self.modified)
    }

    /// The stored solution with exactly this text, if any.
    pub fn precompiled(&self) -> Option<&'static StoredSolution> {
        registry::SOLUTIONS
            .iter()
            .find(|s| source_hash(s.source) == self.hash)
    }

    /// The database index entry (score) of this exact text, if any.
    pub fn database_entry(&self) -> Option<Entry> {
        parse_index(registry::INDEX)
            .ok()?
            .into_iter()
            .find(|e| e.source_hash == self.hash)
    }
}

fn modified_time(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// What the runner is doing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Status {
    #[default]
    Idle,
    /// Compiling the source (native only).
    Compiling,
    /// Running the compiled binary on a case.
    Tracing,
    /// Running the copy compiled into the viewer.
    Precompiled,
    /// Running a freshly compiled native binary.
    Compiled,
    Failed(String),
}

/// A finished background job.
struct JobDone {
    generation: u64,
    case: String,
    #[cfg(not(target_arch = "wasm32"))]
    candidate: Result<ship2d_optimization::harness::Candidate, String>,
    trajectory: Result<Trajectory, String>,
}

/// Runs the current source on cases, compiling it in the background when needed.
#[derive(Default)]
pub struct Runner {
    ctx: Option<egui::Context>,
    precompiled: Option<&'static StoredSolution>,
    #[cfg(not(target_arch = "wasm32"))]
    candidate: Option<ship2d_optimization::harness::Candidate>,
    /// Incremented on every request, so results of superseded jobs are dropped.
    generation: u64,
    pending: Option<Receiver<JobDone>>,
    pub status: Status,
}

impl Runner {
    pub fn new(ctx: egui::Context) -> Self {
        Self {
            ctx: Some(ctx),
            ..Self::default()
        }
    }

    /// Switch to a new source text; previous builds no longer apply.
    pub fn set_source(&mut self, source: &SolutionSource) {
        self.precompiled = source.precompiled();
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.candidate = None;
        }
        self.generation += 1;
        self.pending = None;
    }

    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }

    /// The trajectory of `source` on `case`: returned at once when precompiled, otherwise
    /// computed in the background and delivered later by [`Runner::poll`].
    pub fn request(&mut self, source: &SolutionSource, case: &Case) -> Option<Trajectory> {
        self.generation += 1;
        self.pending = None;
        if let Some(solution) = self.precompiled {
            self.status = Status::Precompiled;
            return Some((solution.simulate)(case));
        }
        self.spawn(source, case);
        None
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn spawn(&mut self, source: &SolutionSource, case: &Case) {
        use ship2d_optimization::harness::build_source;

        let (tx, rx) = std::sync::mpsc::channel();
        let generation = self.generation;
        let candidate = self.candidate.clone();
        self.status = if candidate.is_some() {
            Status::Tracing
        } else {
            Status::Compiling
        };
        let (text, file_name, case, h) = (
            source.text.clone(),
            format!("{}.rs", source.name),
            case.name.clone(),
            case.task.params.h,
        );
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let candidate = candidate.map_or_else(
                || {
                    build_source(&text)
                        .map_err(|e| e.to_string().replace("src/solution.rs", &file_name))
                },
                Ok,
            );
            let trajectory = match &candidate {
                Ok(c) => c.trace(&case, h).map_err(|e| e.to_string()),
                Err(e) => Err(e.clone()),
            };
            // The receiver may be gone if the request was superseded.
            let _ignored = tx.send(JobDone {
                generation,
                case,
                candidate,
                trajectory,
            });
            if let Some(ctx) = ctx {
                ctx.request_repaint();
            }
        });
        self.pending = Some(rx);
    }

    #[cfg(target_arch = "wasm32")]
    fn spawn(&mut self, _source: &SolutionSource, _case: &Case) {
        self.status = Status::Failed(
            "This source is not in the database, and compiling Rust needs the native app."
                .to_owned(),
        );
        if let Some(ctx) = &self.ctx {
            ctx.request_repaint();
        }
    }

    /// A trajectory finished by the background job: `(case name, trajectory)`.
    pub fn poll(&mut self) -> Option<(String, Trajectory)> {
        let done = match self.pending.as_ref()?.try_recv() {
            Ok(done) => done,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => {
                self.pending = None;
                self.status = Status::Failed("the background job stopped unexpectedly".to_owned());
                return None;
            }
        };
        self.pending = None;
        if done.generation != self.generation {
            return None;
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.candidate = done.candidate.ok();
        }
        match done.trajectory {
            Ok(trajectory) => {
                self.status = Status::Compiled;
                Some((done.case, trajectory))
            }
            Err(e) => {
                self.status = Status::Failed(e);
                None
            }
        }
    }
}

/// Stored solutions, best score first, with their index entry and source file.
pub fn database() -> Vec<(Entry, &'static StoredSolution, PathBuf)> {
    registry::valid()
        .into_iter()
        .map(|(solution, entry)| {
            let path = ship2d_optimization::db::source_path(solution.name);
            (entry, solution, path)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ship2d_optimization::suite;

    #[test]
    fn database_sources_are_precompiled() {
        for (entry, solution, path) in database() {
            let from_disk = SolutionSource::from_file(&path).expect("stored file");
            assert_eq!(from_disk.name, solution.name, "name from file stem");
            assert!(
                from_disk.precompiled().is_some(),
                "{} found by hash",
                solution.name
            );
            assert_eq!(
                from_disk.database_entry(),
                Some(entry),
                "index entry by hash"
            );
            assert!(!from_disk.modified_on_disk(), "freshly read");
        }
    }

    #[test]
    fn edited_source_is_not_precompiled() {
        let (_, solution, _) = database().into_iter().next().expect("a stored solution");
        let edited =
            SolutionSource::from_text("edited.rs", format!("{}\n// edit\n", solution.source));
        assert_eq!(edited.name, "edited", "extension stripped");
        assert!(
            edited.precompiled().is_none(),
            "different text, different hash"
        );
        assert!(edited.database_entry().is_none(), "not in the index");
    }

    /// Compiles a real candidate with cargo: takes a few seconds.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn edited_source_compiles_and_traces_in_the_background() {
        let (_, solution, _) = database().into_iter().next().expect("a stored solution");
        let source =
            SolutionSource::from_text("edited", format!("{}\n// edited\n", solution.source));
        // A sampling period other than the suite's, which the candidate must be told.
        let mut case = suite::find("rest_facing_goal").expect("case");
        case.task.params.h = 0.05;
        let mut runner = Runner::default();
        runner.set_source(&source);
        assert!(runner.request(&source, &case).is_none(), "not precompiled");
        assert_eq!(runner.status, Status::Compiling, "compiling");
        let start = std::time::Instant::now();
        let (name, trajectory) = loop {
            if let Some(done) = runner.poll() {
                break done;
            }
            assert!(
                !matches!(runner.status, Status::Failed(_)),
                "{:?}",
                runner.status
            );
            assert!(start.elapsed().as_secs() < 600, "build timed out");
            std::thread::sleep(std::time::Duration::from_millis(50));
        };
        assert_eq!(name, case.name, "trajectory of the requested case");
        assert_eq!(runner.status, Status::Compiled, "status");
        assert_eq!(
            trajectory,
            (solution.simulate)(&case),
            "same as the precompiled copy"
        );

        // A second case reuses the build.
        let other = suite::find("spinning").expect("case");
        assert!(runner.request(&source, &other).is_none(), "background");
        assert_eq!(runner.status, Status::Tracing, "no rebuild");
    }

    #[test]
    fn precompiled_requests_answer_immediately() {
        let (_, solution, _) = database().into_iter().next().expect("a stored solution");
        let source = SolutionSource::from_text(solution.name, solution.source.to_owned());
        let mut runner = Runner::default();
        runner.set_source(&source);
        let case = suite::find("rest_facing_goal").expect("case");
        let trajectory = runner.request(&source, &case).expect("immediate");
        assert!(trajectory.outcome.steps().is_some(), "solves the case");
        assert_eq!(runner.status, Status::Precompiled, "status");
        assert!(!runner.busy(), "no background job");
    }
}
