//! Nabla, a game of flying the ship to its goal, on the test cases of the ship control problem,
//! by hand or with a control policy as autopilot, on a pannable, zoomable canvas.

#![warn(clippy::all, rust_2018_idioms)]

mod app;
#[cfg(test)]
mod ascii_only;
pub mod camera;
mod canvas_buttons;
mod controller_guide;
mod controller_shape;
mod controller_view;
mod formula;
mod game;
mod gamepad;
mod gizmo;
pub mod grid;
mod highlight;
mod keys;
mod notifications;
mod orbit;
mod physics;
mod planets;
mod plume;
pub mod replay;
mod ship3d;
mod sound;
mod span;
mod sprites;
pub mod theme;
mod version;
pub mod world;
mod world_io;
pub use app::ViewerApp;

/// Multisampling (MSAA samples) the native app asks eframe for. The web painter doesn't
/// multisample. The 3D ship's pipeline must match egui's render pass, which eframe sets
/// up from this and [`DEPTH_BUFFER`].
pub const MULTISAMPLING: u16 = 4;
/// Depth buffer bits the app asks eframe for, for the 3D ship view.
pub const DEPTH_BUFFER: u8 = 24;
