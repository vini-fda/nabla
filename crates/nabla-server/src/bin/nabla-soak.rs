//! `nabla-soak`: bots against a server, in this process, over an impaired path.
//!
//! ```text
//! nabla-soak [--bots 16] [--seconds 600] [--world planet-moon] [--delay-ms 40]
//!            [--jitter-ms 20] [--loss 0.03] [--duplicate 0.01] [--seed 1] [--ceiling 0.6]
//! ```
//!
//! It prints what the server and the bots measured, and exits with 1 if the expectations of
//! M4 are not met.

#![expect(clippy::print_stdout, reason = "the report is this program's output")]

use std::time::Duration;

use nabla_server::cli::{parsed, world};
use nabla_server::impair::Impairment;
use nabla_server::soak::{Soak, run};

#[tokio::main]
async fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    match go().await {
        Ok(true) => {}
        Ok(false) => std::process::exit(1),
        Err(why) => {
            log::error!("{why}");
            std::process::exit(2);
        }
    }
}

async fn go() -> Result<bool, String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let bots: usize = parsed(&args, "--bots", 16)?;
    let seconds: f64 = parsed(&args, "--seconds", 600.0)?;
    let name: String = parsed(&args, "--world", "planet-moon".to_owned())?;
    let ms = |name, default| -> Result<Duration, String> {
        Ok(Duration::from_secs_f64(
            parsed::<f64>(&args, name, default)? / 1000.0,
        ))
    };
    let impairment = Impairment {
        delay: ms("--delay-ms", 40.0)?,
        jitter: ms("--jitter-ms", 20.0)?,
        loss: parsed(&args, "--loss", 0.03)?,
        duplicate: parsed(&args, "--duplicate", 0.01)?,
    };
    let ceiling: f64 = parsed(&args, "--ceiling", 0.6)?;
    let seed: u64 = parsed(&args, "--seed", 1)?;
    let world = world(&name, 60, 1000.0, nabla_sim::world::MAX_PILOTS)?;
    println!("{bots} bots for {seconds} s in {name}, each way {impairment:?}, seed {seed}");
    let outcomes = run(Soak {
        world,
        bots,
        duration: Duration::from_secs_f64(seconds),
        impairment,
        seed,
    })
    .await?;
    println!("server: {}", outcomes.metrics);
    println!(
        "server UDP: {} B out, {} B in; {:.3} Mbit/s out per client",
        outcomes.metrics.udp_out,
        outcomes.metrics.udp_in,
        outcomes.downstream_mbit()
    );
    let (mut nonzero, mut snapshots, mut jumps, mut visible, mut snapped) = (0, 0, 0, 0, 0);
    let (mut max_position, mut max_velocity, mut max_heading) = (0.0_f64, 0.0_f64, 0.0_f64);
    let mut late = 0.0_f64;
    for bot in &outcomes.bots {
        let c = &bot.corrections;
        nonzero += c.nonzero;
        visible += c.visible;
        snapped += c.snapped;
        snapshots += c.snapshots;
        jumps += c.jumps;
        max_position = max_position.max(c.max_position);
        max_velocity = max_velocity.max(c.max_velocity);
        max_heading = max_heading.max(c.max_heading);
        late = late.max(bot.late_margin_error);
    }
    let issued: u64 = outcomes.bots.iter().map(|b| b.issued).sum();
    println!(
        "bots: {issued} commands, {snapshots} snapshots absorbed, {nonzero} corrected \
         (max {max_position:.4} position, {max_velocity:.4} velocity, {max_heading:.4} heading), \
         {jumps} jumps; late margin error up to {:.1} ms",
        late * 1e3
    );
    let percent = |n: u64| 100.0 * n as f64 / snapshots.max(1) as f64;
    println!(
        "corrections: {:.2}% of snapshots moved the prediction, {:.2}% by a visible 0.1 or more, \
         {:.3}% by 2 or more (drawn as a jump)",
        percent(nonzero),
        percent(visible),
        percent(snapped)
    );
    println!("the extra pilot: {:?}", outcomes.extra.outcome);
    let failures = outcomes.failures(ceiling);
    for failure in &failures {
        println!("FAILED: {failure}");
    }
    if failures.is_empty() {
        println!("ok");
    }
    Ok(failures.is_empty())
}
