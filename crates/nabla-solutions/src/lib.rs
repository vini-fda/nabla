//! The stored solutions: `solutions/<name>.rs` at the workspace root, each a control policy for
//! the problem of `nabla-core`, scored by `nabla-optimization`.
//!
//! * [`db`]: the database of solutions (their files and the index).
//! * `registry` (feature `registry`): every stored solution, compiled in, for tools such as the
//!   `nabla` app.

pub mod db;
#[cfg(feature = "registry")]
pub mod registry;
