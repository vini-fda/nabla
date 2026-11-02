//! The multiplayer server, and what it is tested with.
//!
//! [`server`] runs `nabla-net`'s [`nabla_net::ServerMachine`] behind WebTransport.
//! [`bot`] is a headless client that flies a ship by random input, [`impair`] a UDP proxy that
//! delays, loses, duplicates and reorders what passes through it, and the `nabla-soak` binary
//! runs sixteen bots against a server for as long as asked.

#![warn(clippy::all, rust_2018_idioms)]

pub mod bot;
pub mod cli;
pub mod impair;
pub mod metrics;
pub mod server;
pub mod soak;

pub use metrics::Metrics;
pub use server::{Server, Settings};
/// The server's certificate and key, which [`Settings`] holds.
pub use wtransport::Identity;
