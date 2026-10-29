//! Evaluating control policies for the problem of `nabla-core`, and the machinery to search
//! for better ones.
//!
//! * [`eval`]: evaluation on both suites and the report format.
//! * [`parallel`]: the lock-free parallel map that evaluations and batch builds run on.
//! * [`harness`]: compile and evaluate a solution given as Rust source.

pub mod eval;
pub mod harness;
pub mod parallel;
