//! The client's state machine: `Issue` and `Absorb` of the specification, the lobby, and the
//! prediction `Pred` that is folded forward from the newest snapshot.
//!
//! The pilot, or an autopilot, is a function from a stamp to a [`Command`], handed to
//! [`ClientMachine::poll`], which issues the commands due by the client's own clock. Nothing
//! here reads a clock: every call is told the time.

use std::collections::BTreeMap;

use nabla_core::{Params, State};
use nabla_sim::command::Command;
use nabla_sim::ship::Ship;
use nabla_sim::world::{ShipRequest, World};

use crate::margin::MarginController;
use crate::wire::{
    ClientStream, Commands, MAX_STREAM_FRAME, PROTOCOL, RosterEntry, ServerStream, ShipId,
    Snapshot, Tick, decode, encode,
};
use crate::{Config, Kind};

/// A stall longer than this, in seconds (a hidden tab, say), is not made up with a burst of
/// commands: the client's tick jumps ahead by the ticks missed, keeping its lead.
pub const MAX_CATCH_UP: f64 = 0.25;

/// What a client holds of the world: the newest snapshot it has absorbed, `(T_b, x_b)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Base {
    pub tick: Tick,
    pub state: State,
    pub hold: Option<f64>,
}

/// Where a connection is.
enum Phase {
    /// Nothing has been sent.
    Start,
    /// `Hello` is sent, and the lobby has not come.
    Hello,
    /// `Join` is sent at this local time, and the ship has not come.
    Joining {
        sent: f64,
    },
    Playing(Box<Play>),
    /// There was no room.
    Full,
    /// Refused.
    Rejected(String),
    /// The ship is gone (it left, or was idle too long), and the client is in the lobby.
    Lobby,
}

/// A ship being flown.
struct Play {
    ship: ShipId,
    sim: Ship,
    /// `k`: the stamp the client issues next.
    k: Tick,
    /// When the client issues it, on its own clock.
    next_issue: f64,
    /// `cmd`: what the client has issued, for the stamps it still needs.
    cmds: BTreeMap<Tick, Command>,
    base: Base,
    margin: MarginController,
}

pub struct ClientMachine {
    config: Config,
    request: ShipRequest,
    build: String,
    /// Join as soon as the first lobby arrives. A client that has left waits to be told.
    auto_join: bool,
    phase: Phase,
    world: Option<World>,
    roster: BTreeMap<ShipId, RosterEntry>,
}

impl ClientMachine {
    /// A client that will ask for the ship `request`, as soon as the lobby says it may.
    pub fn new(config: Config, request: ShipRequest, build: &str) -> Self {
        Self {
            config,
            request,
            build: build.to_owned(),
            auto_join: true,
            phase: Phase::Start,
            world: None,
            roster: BTreeMap::new(),
        }
    }

    /// The first message of the connection.
    pub fn start(&mut self) -> Vec<Kind> {
        self.phase = Phase::Hello;
        vec![Kind::Stream(encode(&ClientStream::Hello {
            protocol: PROTOCOL,
            build: self.build.clone(),
        }))]
    }

    /// Take a ship, as asked for in `request`, from the lobby, at local time `now`. A client that
    /// is not in the lobby, or has not heard what the world is, does nothing.
    pub fn join(&mut self, request: ShipRequest, now: f64) -> Vec<Kind> {
        if !matches!(self.phase, Phase::Lobby) || self.world.is_none() {
            return Vec::new();
        }
        self.request = request;
        self.phase = Phase::Joining { sent: now };
        vec![Kind::Stream(encode(&ClientStream::Join(request)))]
    }

    /// Give the ship up, and go back to the lobby, where this client stays until it joins again.
    pub fn leave(&mut self) -> Vec<Kind> {
        if matches!(self.phase, Phase::Playing(_)) {
            self.auto_join = false;
            self.phase = Phase::Lobby;
            return vec![Kind::Stream(encode(&ClientStream::Leave))];
        }
        Vec::new()
    }

    /// A frame arrived on the stream, at local time `now`.
    pub fn on_stream(&mut self, bytes: &[u8], now: f64) -> Vec<Kind> {
        let Ok(message) = decode::<ServerStream>(bytes, MAX_STREAM_FRAME) else {
            return Vec::new();
        };
        match message {
            ServerStream::Lobby(lobby) => {
                let Ok(world) = World::from_toml(&lobby.world) else {
                    self.phase = Phase::Rejected("the server's world is not valid".to_owned());
                    return Vec::new();
                };
                if world.version != 2 {
                    self.phase =
                        Phase::Rejected("the server's world is not a version 2".to_owned());
                    return Vec::new();
                }
                self.world = Some(world);
                if self.auto_join && matches!(self.phase, Phase::Hello) {
                    self.phase = Phase::Joining { sent: now };
                    return vec![Kind::Stream(encode(&ClientStream::Join(self.request)))];
                }
            }
            ServerStream::Welcome(welcome) => {
                self.auto_join = false;
                self.welcome(&welcome, now);
            }
            ServerStream::Full => {
                if matches!(self.phase, Phase::Joining { .. }) {
                    self.phase = Phase::Full;
                }
            }
            ServerStream::Rejected(why) => self.phase = Phase::Rejected(why),
            ServerStream::Roster(entry) => self.roster_entry(entry),
        }
        Vec::new()
    }

    /// The ship is ours: start from the world at the tick it joined, `k` ahead of it by the
    /// round trip of the exchange and the margin.
    fn welcome(&mut self, welcome: &crate::wire::Welcome, now: f64) {
        let (Phase::Joining { sent }, Some(world)) = (&self.phase, &self.world) else {
            return;
        };
        let Some(own) = welcome.ships.iter().find(|r| r.ship == welcome.ship) else {
            return;
        };
        let h = world.h();
        let round_trip = (now - sent).max(0.0);
        // By the lemma on lead, the snapshot labeled `T` arrives when the client is at
        // `T - 1 + (RTT + mu) / h`.
        let lead = ((round_trip + self.config.target_margin) / h)
            .round()
            .max(1.0);
        let mut ship = Ship::new(world, welcome.params, own.state);
        ship.restore(world, own.state, own.hold);
        self.phase = Phase::Playing(Box::new(Play {
            ship: welcome.ship,
            sim: ship,
            k: welcome.tick + lead as Tick,
            next_issue: now,
            cmds: BTreeMap::new(),
            base: Base {
                tick: welcome.tick,
                state: own.state,
                hold: own.hold,
            },
            margin: MarginController::new(h, self.config.target_margin, round_trip / h),
        }));
    }

    fn roster_entry(&mut self, entry: RosterEntry) {
        if let Phase::Playing(play) = &self.phase
            && play.ship == entry.ship
            && entry.until.is_some()
        {
            self.phase = Phase::Lobby;
        }
        if entry.until.is_some() {
            self.roster.remove(&entry.ship);
        } else {
            self.roster.insert(entry.ship, entry);
        }
    }

    /// A datagram arrived, at local time `now`: `Absorb` of the specification, for a snapshot
    /// newer than the one held. Says whether it was.
    pub fn on_datagram(&mut self, bytes: &[u8], now: f64) -> bool {
        let (Phase::Playing(play), Some(world)) = (&mut self.phase, &self.world) else {
            return false;
        };
        let Ok(snapshot) = decode::<Snapshot>(bytes, self.config.max_datagram) else {
            return false;
        };
        // The guard keeps the base monotone under reordering.
        if snapshot.label <= play.base.tick {
            return false;
        }
        let Some(own) = snapshot.ships.iter().find(|r| r.ship == play.ship) else {
            return false;
        };
        let h = world.h();
        // Where the client's tick is now, in ticks, a fraction included.
        let tick_now = f64::from(play.k) - (play.next_issue - now).max(0.0) / h;
        let ticks_ahead = tick_now - f64::from(snapshot.label - 1);
        play.margin
            .observe(f64::from(snapshot.margin_us) * 1e-6, ticks_ahead);
        play.base = Base {
            tick: snapshot.label,
            state: own.state,
            hold: own.hold,
        };
        // Advance: the client's tick never trails the world's.
        play.k = play.k.max(snapshot.label);
        // Far too late, as after a stall: jump to the lead the lemma gives, `k := T_b + L`.
        if play.margin.far_behind() {
            play.k = play.k.max(snapshot.label - 1 + play.margin.lead_ticks());
        }
        let keep_from = play
            .base
            .tick
            .min(play.k.saturating_sub(self.config.copies as Tick));
        play.cmds = play.cmds.split_off(&keep_from);
        true
    }

    /// Issue every command that is due by `now`, on the client's own clock, asking `pilot` for
    /// the command of each stamp. Returns the datagrams to send.
    pub fn poll(&mut self, now: f64, pilot: &mut dyn FnMut(Tick) -> Command) -> Vec<Kind> {
        let Phase::Playing(play) = &mut self.phase else {
            return Vec::new();
        };
        if now - play.next_issue > MAX_CATCH_UP {
            // A stall: the time lost is not made up. The client jumps to the lead it should have
            // over the newest snapshot, `k := T_b + L`, which Advance has been dragging along
            // behind the world.
            let lead = play.base.tick.saturating_sub(1) + play.margin.lead_ticks();
            play.k = play.k.max(lead);
            play.next_issue = now;
        }
        let mut out = Vec::new();
        while now >= play.next_issue {
            let stamp = play.k;
            play.cmds.insert(stamp, pilot(stamp));
            play.k += 1;
            play.next_issue += play.margin.tick_length();
            let cmds = (0..self.config.copies as Tick)
                .map(|back| {
                    let earlier = stamp.checked_sub(back)?;
                    play.cmds.get(&earlier).copied()
                })
                .collect();
            out.push(Kind::Datagram(encode(&Commands { stamp, cmds })));
        }
        out
    }

    /// When the client next issues a command, by its own clock.
    pub fn next_deadline(&self) -> Option<f64> {
        match &self.phase {
            Phase::Playing(play) => Some(play.next_issue),
            _ => None,
        }
    }

    pub fn is_playing(&self) -> bool {
        matches!(self.phase, Phase::Playing(_))
    }

    /// The reason the connection is over, if it is.
    pub fn refusal(&self) -> Option<&str> {
        match &self.phase {
            Phase::Rejected(why) => Some(why),
            Phase::Full => Some("the server is full"),
            _ => None,
        }
    }

    pub fn world(&self) -> Option<&World> {
        self.world.as_ref()
    }

    pub fn roster(&self) -> &BTreeMap<ShipId, RosterEntry> {
        &self.roster
    }

    /// The ship this client flies.
    pub fn ship(&self) -> Option<ShipId> {
        self.play().map(|p| p.ship)
    }

    fn play(&self) -> Option<&Play> {
        match &self.phase {
            Phase::Playing(play) => Some(play),
            _ => None,
        }
    }

    /// `k`: the stamp the client issues next.
    pub fn tick(&self) -> Option<Tick> {
        self.play().map(|p| p.k)
    }

    /// `(T_b, x_b)`: the newest snapshot absorbed.
    pub fn base(&self) -> Option<Base> {
        self.play().map(|p| p.base)
    }

    /// `e`: the last margin reported, less the target, in seconds.
    pub fn margin_error(&self) -> Option<f64> {
        self.play().map(|p| p.margin.error())
    }

    /// The ship the pilot flies, as granted.
    pub fn params(&self) -> Option<Params> {
        self.play().map(|p| *p.sim.params())
    }

    /// The command the client issued for `stamp`, if it still holds it.
    pub fn issued(&self, stamp: Tick) -> Option<Command> {
        self.play()?.cmds.get(&stamp).copied()
    }

    /// `Pred(n)`: the own ship's state at tick `n`, the newest snapshot folded forward with the
    /// commands issued. A stamp with no command replays as the one before it, as the hold rule
    /// would.
    pub fn predict(&mut self, n: Tick) -> Option<State> {
        let (Phase::Playing(play), Some(world)) = (&mut self.phase, &self.world) else {
            return None;
        };
        if n < play.base.tick {
            return None;
        }
        play.sim.restore(world, play.base.state, play.base.hold);
        let mut previous = Command::default();
        for stamp in play.base.tick..n {
            let command = play.cmds.get(&stamp).copied().unwrap_or(previous);
            play.sim.tick(world, &command);
            previous = command;
        }
        Some(play.sim.state())
    }

    /// The command that `Pred` replays at `stamp`: the one issued, or else the one before.
    pub fn replayed(&self, stamp: Tick) -> Option<Command> {
        let play = self.play()?;
        let mut previous = Command::default();
        for s in play.base.tick..=stamp {
            previous = play.cmds.get(&s).copied().unwrap_or(previous);
        }
        Some(previous)
    }
}
