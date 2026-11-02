//! `nabla-bot`: one headless pilot, flying a ship by random input on a server.
//!
//! ```text
//! nabla-bot --server https://host:4433#sha256=... [--seconds 30] [--seed 1]
//! ```

#![expect(clippy::print_stdout, reason = "the report is this program's output")]

use std::time::Duration;

use nabla_net::link::Address;
use nabla_server::bot::{self, Outcome};
use nabla_server::cli::{flag, parsed};
use nabla_sim::world::Preset;

#[tokio::main]
async fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    if let Err(why) = go().await {
        log::error!("{why}\nusage: nabla-bot --server ADDRESS [--seconds N] [--seed N]");
        std::process::exit(2);
    }
}

async fn go() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let address = Address::parse(&flag(&args, "--server").ok_or("--server is needed")?)?;
    let seconds: f64 = parsed(&args, "--seconds", 30.0)?;
    let seed: u64 = parsed(&args, "--seed", 1)?;
    // The bundled ship, which every server grants.
    let request = Preset::default().world().players[0].task.params.into();
    let report = bot::fly(&address, request, seed, Duration::from_secs_f64(seconds)).await;
    println!(
        "{:?}: {} commands, {} snapshots absorbed, {} corrected (max {:.2e} position, {:.2e} velocity), \
         farthest {:.1}, margin error {:.1} ms",
        report.outcome,
        report.issued,
        report.corrections.snapshots,
        report.corrections.nonzero,
        report.corrections.max_position,
        report.corrections.max_velocity,
        report.farthest,
        report.final_margin_error * 1e3
    );
    if report.outcome == Outcome::Flew {
        Ok(())
    } else {
        Err(format!("the flight ended: {:?}", report.outcome))
    }
}
