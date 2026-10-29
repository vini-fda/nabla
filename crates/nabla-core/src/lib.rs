//! The ship and the problem it is controlled to solve: a finite, discretized version of the
//! ship control problem (come to rest at a point). A control policy is a [`Controller`]; the
//! task is to find one that solves every test case in the fewest total steps. See `TASK.md`.
//!
//! * [`model`]: ship dynamics and the exact sampled-data step `Φ_h`.
//! * [`problem`]: target set, the [`Controller`] interface and the simulation loop.
//! * [`suite`]: the test cases.
//!
//! Evaluating, compiling and searching for policies is `nabla-optimization`; the stored
//! policies are `nabla-solutions`.

pub mod model;
pub mod problem;
pub mod suite;

pub use model::{Control, Params, State, step};
pub use problem::{Controller, Goal, Task, Tolerance};
