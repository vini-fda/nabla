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
#[cfg(not(target_arch = "wasm32"))]
pub mod dev_log;
mod formula;
mod game;
mod gamepad;
mod gizmo;
pub mod grid;
mod highlight;
mod keys;
#[cfg(target_arch = "wasm32")]
mod measure_web;
pub mod net;
mod notifications;
pub mod online;
mod orbit;
mod planets;
mod plots;
mod plume;
pub mod replay;
mod ship3d;
mod sound;
mod sprites;
pub mod theme;
mod version;
mod world_io;
pub use app::ViewerApp;
/// What a flight aims for, and what an orbit assist is made of: in the signatures of `online`.
pub use game::Objective;
use nabla_sim::physics;
/// The world format, shared with the server (`nabla-sim`).
pub use nabla_sim::world;
pub use orbit::{Pilot, Prediction, Target};

/// Multisampling (MSAA samples) the native app asks eframe for. The web painter doesn't
/// multisample. The 3D ship's pipeline must match egui's render pass, which eframe sets
/// up from this and [`DEPTH_BUFFER`].
pub const MULTISAMPLING: u16 = 4;
/// Depth buffer bits the app asks eframe for, for the 3D ship view.
pub const DEPTH_BUFFER: u8 = 24;
