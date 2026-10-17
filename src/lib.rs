//! A finite, discretized version of the ship control problem (come to rest at a point), set
//! up as an optimization task: find a controller that solves every test case in the fewest
//! total steps. See `TASK.md`.
//!
//! * [`model`]: ship dynamics and the exact sampled-data step `Φ_h`.
//! * [`problem`]: target set, the [`Controller`] interface and the simulation loop.
//! * [`suite`]: the test cases.
//! * [`eval`]: evaluation on both suites and the report format.
//! * [`parallel`]: the lock-free parallel map that evaluations and batch builds run on.
//! * [`harness`]: compile and evaluate a solution given as Rust source.
//! * [`db`]: the database of solutions.
//! * `registry` (feature `registry`): every stored solution, compiled in, for tools such as the
//!   `ship2d` viewer.

// Stored solutions refer to this crate by name, also when compiled into it by `registry`.
extern crate self as ship2d_optimization;

pub mod db;
pub mod eval;
pub mod harness;
pub mod model;
pub mod parallel;
pub mod problem;
#[cfg(feature = "registry")]
pub mod registry;
pub mod suite;

pub use model::{Control, Params, State, step};
pub use problem::{Controller, Goal, Task, Tolerance};
