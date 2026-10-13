//! Viewer for `ship2d_optimization` solutions: compiles a solution's Rust source, runs it on
//! a test case, and replays the run in continuous time on a pannable, zoomable canvas.

#![warn(clippy::all, rust_2018_idioms)]

mod app;
pub mod camera;
pub mod grid;
pub mod replay;
pub mod source;
pub mod timeline;
pub use app::ViewerApp;
