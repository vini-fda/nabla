//! Sixteen bots against a server over loopback, through a proxy that impairs the path, and what
//! to expect of them (`docs/multiplayer.typ`, M4).

use std::net::SocketAddr;
use std::time::Duration;

use crate::bot::{self, BotReport, Outcome};
use crate::impair::{Impairment, Proxy};
use crate::{Server, Settings};
use nabla_net::Config;
use nabla_sim::world::{MAX_PILOTS, World};
use wtransport::Identity;

/// What to run.
pub struct Soak {
    pub world: World,
    pub bots: usize,
    pub duration: Duration,
    pub impairment: Impairment,
    pub seed: u64,
}

/// What came of it.
pub struct Outcomes {
    pub bots: Vec<BotReport>,
    /// The bot that arrived when every place was taken.
    pub extra: BotReport,
    pub metrics: crate::Metrics,
    pub h: f64,
    pub half_side: f64,
    pub seconds: f64,
}

/// Run it.
///
/// # Errors
/// If the server cannot start.
pub async fn run(soak: Soak) -> Result<Outcomes, String> {
    let h = soak.world.h();
    let half_side = soak.world.arena.as_ref().map_or(0.0, |a| a.half_side);
    let request = soak.world.default_ship().into();
    let identity = Identity::self_signed(["localhost", "127.0.0.1", "::1"])
        .map_err(|e| format!("certificate: {e}"))?;
    let bind: SocketAddr = "127.0.0.1:0".parse().map_err(|_bad| "address")?;
    let server = Server::bind(Settings {
        bind,
        world: soak.world,
        config: Config::default(),
        identity,
    })?;
    let metrics = server.metrics();
    let target = server.local_addr().map_err(|e| e.to_string())?;
    let proxy = Proxy::start(target, soak.impairment, soak.seed)
        .await
        .map_err(|e| e.to_string())?;
    let mut address = server.address("127.0.0.1").map_err(|e| e.to_string())?;
    address.url = format!("https://127.0.0.1:{}", proxy.addr().port());
    let serving = tokio::spawn(server.serve());

    let started = tokio::time::Instant::now();
    let mut flights = Vec::new();
    for i in 0..soak.bots.min(MAX_PILOTS) {
        let address = address.clone();
        let (seed, duration) = (soak.seed + i as u64, soak.duration);
        flights.push(tokio::spawn(async move {
            // Not all at one instant, as pilots are not.
            tokio::time::sleep(Duration::from_millis(30 * i as u64)).await;
            bot::fly(&address, request, seed, duration).await
        }));
    }
    // Everyone is in by now (the places are taken), so this one finds no room.
    tokio::time::sleep(Duration::from_secs(3)).await;
    let extra = bot::fly(&address, request, soak.seed + 99, Duration::from_secs(2)).await;
    let mut bots = Vec::new();
    for flight in flights {
        bots.push(flight.await.map_err(|e| e.to_string())?);
    }
    let seconds = started.elapsed().as_secs_f64();
    // The server notes a connection's bytes when it ends: let the last ones end.
    tokio::time::sleep(Duration::from_millis(500)).await;
    serving.abort();
    let metrics = metrics
        .lock()
        .map_err(|_poisoned| "the metrics are poisoned".to_owned())?
        .clone();
    Ok(Outcomes {
        bots,
        extra,
        metrics,
        h,
        half_side,
        seconds,
    })
}

impl Outcomes {
    /// What the paper expects, as a list of what failed.
    pub fn failures(&self, ceiling_mbit: f64) -> Vec<String> {
        let mut bad = Vec::new();
        for (i, bot) in self.bots.iter().enumerate() {
            if bot.outcome != Outcome::Flew {
                bad.push(format!("bot {i} ended: {:?}", bot.outcome));
            }
            if bot.regressions > 0 {
                bad.push(format!(
                    "bot {i}: the base went backward {} times",
                    bot.regressions
                ));
            }
            if bot.farthest > self.half_side + 1e-6 {
                bad.push(format!("bot {i} left the arena: {}", bot.farthest));
            }
        }
        if self.extra.outcome != Outcome::Full {
            bad.push(format!(
                "the extra pilot got {:?}, not Full",
                self.extra.outcome
            ));
        }
        let p99 = f64::from(self.metrics.work_percentile(0.99)) * 1e-6;
        if p99 >= self.h {
            bad.push(format!(
                "p99 tick time {p99} s is not under h = {} s",
                self.h
            ));
        }
        if self.metrics.largest_datagram > Config::default().max_datagram {
            bad.push(format!(
                "a snapshot of {} bytes",
                self.metrics.largest_datagram
            ));
        }
        let mbit = self.downstream_mbit();
        if mbit > ceiling_mbit {
            bad.push(format!(
                "downstream {mbit:.3} Mbit/s per client, over {ceiling_mbit}"
            ));
        }
        bad
    }

    /// The server's UDP bytes out per client per second, in Mbit/s.
    pub fn downstream_mbit(&self) -> f64 {
        let clients = (self.bots.len() + 1) as f64;
        self.metrics.udp_out as f64 * 8.0 / 1e6 / self.seconds / clients
    }
}
