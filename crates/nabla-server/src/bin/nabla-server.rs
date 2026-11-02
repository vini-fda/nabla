//! `nabla-server`: a multiplayer world, over WebTransport.
//!
//! ```text
//! nabla-server [--bind 0.0.0.0:4433] [--world vacuum|planet|planet-moon|FILE.toml]
//!              [--tick-rate 60] [--arena 1000] [--max-pilots 16]
//!              [--cert cert.pem --key key.pem] [--name localhost --name 192.168.0.5]
//! ```
//!
//! Without `--cert` it makes a self-signed certificate, good for two weeks, whose hash is printed
//! in the address clients connect to. With one, clients need no hash.

use std::net::SocketAddr;
use std::time::Duration;

use nabla_net::Config;
use nabla_server::cli::{flag, parsed, world};
use nabla_server::{Server, Settings};
use wtransport::Identity;

const HELP: &str = "nabla-server [--bind ADDR] [--world NAME|FILE] [--tick-rate N] [--arena S] \
[--max-pilots N] [--cert PEM --key PEM] [--name HOST]...";

#[tokio::main]
async fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    if let Err(why) = run().await {
        log::error!("{why}\nusage: {HELP}");
        std::process::exit(2);
    }
}

async fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        log::info!("usage: {HELP}");
        return Ok(());
    }
    let bind: SocketAddr = parsed(
        &args,
        "--bind",
        "0.0.0.0:4433".parse().map_err(|_bad| "bind")?,
    )?;
    let tick_rate: u32 = parsed(&args, "--tick-rate", 60)?;
    let arena: f64 = parsed(&args, "--arena", 1000.0)?;
    let max_pilots: usize = parsed(&args, "--max-pilots", nabla_sim::world::MAX_PILOTS)?;
    let name = flag(&args, "--world").unwrap_or_else(|| "planet-moon".to_owned());
    let world = world(&name, tick_rate, arena, nabla_sim::world::MAX_PILOTS)?;

    let identity = match (flag(&args, "--cert"), flag(&args, "--key")) {
        (Some(cert), Some(key)) => Identity::load_pemfiles(&cert, &key)
            .await
            .map_err(|e| format!("cannot load the certificate: {e}"))?,
        (None, None) => {
            let mut names: Vec<String> = args
                .windows(2)
                .filter(|w| w[0] == "--name")
                .map(|w| w[1].clone())
                .collect();
            if names.is_empty() {
                names = ["localhost", "127.0.0.1", "::1"]
                    .map(str::to_owned)
                    .to_vec();
            }
            log::warn!("self-signed certificate for {names:?}: it expires in 14 days");
            Identity::self_signed(&names).map_err(|e| format!("cannot make a certificate: {e}"))?
        }
        _ => return Err("--cert and --key go together".to_owned()),
    };
    let config = Config {
        max_pilots,
        ..Config::default()
    };
    let server = Server::bind(Settings {
        bind,
        world,
        config,
        identity,
    })?;
    let host = if bind.ip().is_unspecified() {
        "localhost".to_owned()
    } else {
        bind.ip().to_string()
    };
    let address = server.address(&host).map_err(|e| e.to_string())?;
    log::info!("world {name}, {tick_rate} Hz, arena {arena}, up to {max_pilots} pilots");
    log::info!("connect to {address}");
    if bind.ip().is_loopback() {
        log::info!(
            "in the web app of `cargo xtask web-dev`: http://localhost:8080/?server={}&connect#dev",
            percent_encode(&address.to_string())
        );
    }
    let metrics = server.metrics();
    let serving = tokio::spawn(server.serve());
    let mut report = tokio::time::interval(Duration::from_secs(10));
    loop {
        tokio::select! {
            _ = report.tick() => {
                if let Ok(m) = metrics.lock() {
                    log::info!("{m}");
                }
            }
            _ = tokio::signal::ctrl_c() => break,
        }
    }
    serving.abort();
    Ok(())
}

/// `text` as a URL query value: everything but letters, digits and `-._~` is escaped.
fn percent_encode(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'0'..=b'9' | b'a'..=b'z' | b'A'..=b'Z' | b'-' | b'.' | b'_' | b'~' => {
                char::from(b).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}
