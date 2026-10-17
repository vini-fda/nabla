//! Viewer for `ship2d_optimization` solutions: compiles a solution's Rust source, runs it on
//! a test case, and replays the run in continuous time on a pannable, zoomable canvas.

#![warn(clippy::all, rust_2018_idioms)]

mod app;
pub mod camera;
mod formula;
mod gizmo;
pub mod grid;
mod highlight;
mod plume;
pub mod replay;
mod ship3d;
mod sound;
pub mod source;
mod sprites;
mod synth;
pub mod theme;
pub mod timeline;
pub use app::ViewerApp;

/// Multisampling (MSAA samples) the native app asks eframe for. The web painter doesn't
/// multisample. The 3D ship's pipeline must match egui's render pass, which eframe sets
/// up from this and [`DEPTH_BUFFER`].
pub const MULTISAMPLING: u16 = 4;
/// Depth buffer bits the app asks eframe for, for the 3D ship view.
pub const DEPTH_BUFFER: u8 = 24;
