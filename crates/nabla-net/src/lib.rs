//! The multiplayer protocol of `docs/multiplayer.typ`, as sans-IO state machines.
//!
//! A [`ServerMachine`] holds the world and runs its ticks. A [`ClientMachine`] holds one pilot's
//! view: the commands issued, the newest snapshot, and the prediction folded forward from it.
//! Neither does I/O, reads a clock or spawns a task. They are handed bytes, the time and the
//! pilot's input, and return bytes to send. A call is one action of the specification, and
//! `&mut self` is what makes it atomic. So the same machines run under `tokio` in production and
//! under [`sim`], a seeded simulated network, in test, as `quinn-proto` does.
//!
//! The crate builds for the web.

#![warn(clippy::all, rust_2018_idioms)]

pub mod client;
pub mod frame;
pub mod link;
pub mod margin;
#[cfg(feature = "native")]
pub mod native;
pub mod server;
pub mod sim;
pub mod wire;

pub use client::ClientMachine;
pub use server::ServerMachine;
use wire::ConnId;

/// The constants of the protocol (`docs/multiplayer.typ`, "Plan"), which are to be tuned.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Config {
    /// `K`: how many commands a datagram carries, the newest and `K - 1` before it.
    pub copies: usize,
    /// `N_d`: after this many ticks without a command, a ship's last one goes neutral.
    pub silence_ticks: u32,
    /// `N_b`: the width, in ticks, of the window of stamps the server buffers.
    pub window: u32,
    /// `T_idle`: seconds of silence after which a ship leaves, and frees its place.
    pub idle: f64,
    /// Seconds without a command that does anything, after which a pilot is shown as inactive.
    pub input_idle: f64,
    /// `mu*`: the margin, in seconds, a client steers for.
    pub target_margin: f64,
    /// `B`: the largest datagram, in bytes.
    pub max_datagram: usize,
    /// `max_pilots`: at most [`nabla_sim::world::MAX_PILOTS`].
    pub max_pilots: usize,
    /// Keep every ship's log, which proofs are about and the code does not need: for tests.
    pub keep_log: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            copies: 5,
            silence_ticks: 60,
            window: 60,
            idle: 60.0,
            input_idle: 30.0,
            target_margin: 0.017,
            max_datagram: 1150,
            max_pilots: nabla_sim::world::MAX_PILOTS,
            keep_log: false,
        }
    }
}

/// Something a machine asks its driver to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outgoing {
    pub conn: ConnId,
    pub kind: Kind,
}

/// Over which channel, or the end of the connection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A message on the connection's reliable, ordered stream: one frame.
    Stream(Vec<u8>),
    /// An unreliable, unordered datagram.
    Datagram(Vec<u8>),
    /// Close the connection.
    Close,
}
