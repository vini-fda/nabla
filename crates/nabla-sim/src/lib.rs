//! The simulation of a Nabla world, without any display, sound or network: the [`world`] file
//! format, the ship's [`physics`] among bodies, and how a pilot's input becomes a control
//! ([`steering`]).
//!
//! The client flies one ship with it, and a server flies many. It builds for the web, and
//! depends on neither a window system nor an audio device.

#![warn(clippy::all, rust_2018_idioms)]

pub mod command;
pub mod physics;
pub mod rng;
pub mod ship;
pub mod span;
pub mod steering;
pub mod world;
