//! A headless client that flies a ship by random input, as a person at a keyboard might: it
//! holds a key for a while, and lets go, and tries another.
//!
//! The bot is the real client machine on the real transport, so what it measures is what a
//! pilot would see: how far each snapshot moved its prediction, how its margin settled, whether
//! the ship stayed in the arena, and whether it was told the server was full.

use std::time::{Duration, Instant};

use nabla_net::client::CorrectionStats;
use nabla_net::link::{Address, Incoming};
use nabla_net::native::Link;
use nabla_net::wire::{Password, Username};
use nabla_net::{ClientMachine, Config, Kind};
use nabla_sim::command::Command;
use nabla_sim::rng::Rng;
use nabla_sim::steering::{Aim, Input, Steering};
use nabla_sim::world::ShipRequest;
use tokio::runtime::Handle;
use tokio::time::sleep;

/// How a bot's flight ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// It flew for as long as it was asked to.
    Flew,
    /// The server had no room.
    Full,
    /// The server refused it, and said why.
    Refused(String),
    /// The connection ended.
    Closed(String),
}

/// What a bot saw.
#[derive(Clone, Debug)]
pub struct BotReport {
    pub outcome: Outcome,
    /// Commands issued.
    pub issued: u64,
    /// Snapshots absorbed, and how far they moved the prediction.
    pub corrections: CorrectionStats,
    /// `|mu - mu*|` in seconds, at the end and the largest in the last half of the flight.
    pub final_margin_error: f64,
    pub late_margin_error: f64,
    /// The largest `|x|` or `|y|` the predicted ship reached, to hold against the arena.
    pub farthest: f64,
    /// Times a snapshot's tick was not greater than the base before it: must be zero.
    pub regressions: u64,
    /// The round trip QUIC measured at the end, in seconds.
    pub rtt: Option<f64>,
}

/// A pilot who changes their mind about four times a second.
struct Pilot {
    rng: Rng,
    command: Command,
    until: u32,
    steering: Steering,
}

impl Pilot {
    fn new(seed: u64) -> Self {
        let mut rng = Rng::new(seed);
        let steering = if rng.chance(0.5) {
            Steering::Sas
        } else {
            Steering::Raw
        };
        Self {
            rng,
            command: Command::default(),
            until: 0,
            steering,
        }
    }

    /// The command for `stamp`: the same as the last until the pilot changes their mind.
    fn command(&mut self, stamp: u32) -> Command {
        if stamp >= self.until {
            self.until = stamp + 5 + u32::try_from(self.rng.below(25)).unwrap_or(0);
            let thrust = if self.rng.chance(0.6) { 1.0 } else { 0.0 };
            let turn = f64::from(i8::try_from(self.rng.below(3)).unwrap_or(1) - 1);
            let aim = self.rng.chance(0.2).then(|| Aim {
                heading: self.rng.range(-3.0, 3.0),
                cone: self.rng.range(0.1, 1.0),
            });
            self.command = Command::Manual {
                input: Input {
                    thrust,
                    turn,
                    fine: self.rng.chance(0.2),
                    aim,
                },
                steering: self.steering,
            };
        }
        self.command
    }
}

/// Connect to `address`, ask for `request` under the name `username`, and fly for `duration`
/// (counted from the first moment it plays) on the runtime this is awaited in.
///
/// It says `password` if the server asks for one.
///
/// It flies by random input from `seed`, or with its hands off the controls if there is none: the
/// ship coasts, and its commands, which keep its place, ask for nothing.
pub async fn fly(
    address: &Address,
    request: ShipRequest,
    username: Username,
    password: Option<Password>,
    seed: Option<u64>,
    duration: Duration,
) -> BotReport {
    let mut link = Link::open(&Handle::current(), address);
    let mut machine = ClientMachine::new(Config::default(), request, username, "nabla-bot");
    if let Some(password) = password {
        machine = machine.with_password(password);
    }
    for kind in machine.start() {
        link.send(kind);
    }
    let started = Instant::now();
    let now = || started.elapsed().as_secs_f64();
    let mut pilot = seed.map(Pilot::new);
    let mut report = BotReport {
        outcome: Outcome::Flew,
        issued: 0,
        corrections: CorrectionStats::default(),
        final_margin_error: 0.0,
        late_margin_error: 0.0,
        farthest: 0.0,
        regressions: 0,
        rtt: None,
    };
    let mut playing_since: Option<f64> = None;
    let mut last_base = 0;
    loop {
        let wait = machine
            .next_deadline()
            .map_or(0.02, |due| (due - now()).clamp(0.0, 0.02));
        tokio::select! {
            incoming = link.recv() => match incoming {
                Some(Incoming::Stream(bytes)) => {
                    for kind in machine.on_stream(&bytes, now()) {
                        link.send(kind);
                    }
                }
                Some(Incoming::Datagram(bytes)) => {
                    let absorbed = machine.on_datagram(&bytes, now());
                    if absorbed && let Some(base) = machine.base() {
                        if base.tick <= last_base {
                            report.regressions += 1;
                        }
                        last_base = base.tick;
                    }
                }
                Some(Incoming::Closed(why)) => {
                    report.outcome = Outcome::Closed(why);
                    break;
                }
                None => break,
            },
            () = sleep(Duration::from_secs_f64(wait)) => {}
        }
        if let Some(why) = machine.refusal() {
            report.outcome = if why.contains("full") {
                Outcome::Full
            } else {
                Outcome::Refused(why.to_owned())
            };
            break;
        }
        if machine.is_playing() {
            let since = *playing_since.get_or_insert_with(now);
            let mut farthest = report.farthest;
            let issued = machine.poll(now(), &mut |stamp, predicted| {
                farthest = farthest.max(predicted.x.abs()).max(predicted.y.abs());
                pilot
                    .as_mut()
                    .map_or_else(Command::default, |pilot| pilot.command(stamp))
            });
            report.farthest = farthest;
            for kind in issued {
                if matches!(kind, Kind::Datagram(_)) {
                    report.issued += 1;
                }
                link.send(kind);
            }
            if let Some(error) = machine.margin_error() {
                report.final_margin_error = error.abs();
                if now() - since > duration.as_secs_f64() / 2.0 {
                    report.late_margin_error = report.late_margin_error.max(error.abs());
                }
            }
            if now() - since >= duration.as_secs_f64() {
                break;
            }
        }
    }
    report.corrections = machine.correction_stats().unwrap_or(report.corrections);
    report.rtt = link.rtt();
    link.send(Kind::Close);
    // Let the goodbye go out.
    sleep(Duration::from_millis(50)).await;
    report
}
