//! The client's state machine: `Issue` and `Absorb` of the specification, the lobby, and the
//! prediction `Pred` that is folded forward from the newest snapshot.
//!
//! The pilot, or an autopilot, is a function from a stamp to a [`Command`], handed to
//! [`ClientMachine::poll`], which issues the commands due by the client's own clock. Nothing
//! here reads a clock: every call is told the time.
//!
//! Where ships collide, the prediction is of more than the own ship. A snapshot holds every
//! ship, and the client folds forward the ones that could come to share a block with its own
//! by the tick it issues next, or within [`COMPANY_LEAD`] ticks after it (its *company*), with
//! the world's own map: its own ship under the commands it issued, and each of the others under
//! the control it last flew, held. The ships beyond that cannot touch the own ship in that time,
//! short of a contact of their own, and are left out.

use std::collections::{BTreeMap, VecDeque};

use nabla_core::{Control, Params, State};
use nabla_sim::command::Command;
use nabla_sim::fleet::{Dynamics, Member};
use nabla_sim::world::{ShipRequest, World};

use crate::margin::MarginController;
use crate::wire::{
    ClientStream, Commands, MAX_STREAM_FRAME, PROTOCOL, Password, RosterEntry, ServerStream,
    ShipId, ShipRecord, Snapshot, Tick, Username, decode, encode,
};
use crate::{Config, Kind};

/// All that a server of protocol 2 or earlier says to a client of another version. Later servers
/// say more, so a server that says this is older than any client that reads this.
const OLD_SERVER_REFUSAL: &str = "protocol version";

/// A correction past this many world units, or [`SNAP_HEADING`] radians, is too large to hide:
/// a screen draws it as a jump, and not eased.
pub const SNAP_POSITION: f64 = 2.0;
pub const SNAP_HEADING: f64 = 1.0;
/// A correction past this many world units or radians can be seen.
pub const VISIBLE: f64 = 0.1;

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

/// How many snapshots of a remote ship are kept: enough to interpolate across a jittery path.
pub const REMOTE_KEPT: usize = 32;

/// The longest replay the client folds forward to measure a correction. A longer one is a stall,
/// which is a jump and not a correction.
const MAX_REPLAY: Tick = 1200;

/// How many ticks further than the next tick to issue the prediction looks for the ships to
/// carry: half a second at 60 Hz.
///
/// The ships carried are drawn in the own ship's frame, and the others in the past. With no
/// lead, a ship would change frames just as it came to touch the own ship, and with this one
/// it changes half a second before it can.
pub const COMPANY_LEAD: Tick = 30;

/// How far the own ship's predicted state moved when a snapshot was absorbed.
///
/// It is the new prediction less the old, at the stamp the client issues next. It is zero when
/// the commands issued were the ones applied (`thm-exact`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Correction {
    pub delta: State,
    /// The replay was too long to measure: the prediction was not corrected but restarted.
    pub jumped: bool,
}

impl Correction {
    /// The largest of `|dp|`, `|dv|`, `|dtheta|`, `|domega|`.
    pub fn size(&self) -> f64 {
        let d = &self.delta;
        d.x.hypot(d.y)
            .max(d.vx.hypot(d.vy))
            .max(d.theta.abs())
            .max(d.omega.abs())
    }
}

/// What the client has seen of its corrections.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CorrectionStats {
    pub snapshots: u64,
    pub nonzero: u64,
    /// Corrections past [`VISIBLE`], and past [`SNAP_POSITION`] or [`SNAP_HEADING`].
    pub visible: u64,
    pub snapped: u64,
    pub jumps: u64,
    pub max_position: f64,
    pub max_velocity: f64,
    pub max_heading: f64,
}

/// A remote ship's record in a snapshot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Remote {
    pub tick: Tick,
    pub record: ShipRecord,
}

/// Where a client is, for a screen to show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// Nothing is sent yet, or `Hello` is, and the lobby has not come.
    Connecting,
    /// In the lobby: the world is known, and the client has no ship.
    Lobby,
    /// `Join` is sent, and the ship has not come.
    Joining,
    Playing,
    /// There was no room.
    Full,
    /// The server turned the client away.
    Rejected,
    /// The server asks for a password this client does not have. It takes a new connection, with
    /// the password, to go on.
    NeedsPassword,
    /// The password was wrong, and the server ended the connection.
    WrongPassword,
}

/// What the lobby said about who is flying.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LobbyInfo {
    pub pilots: usize,
    pub max_pilots: usize,
}

/// One tick of the own ship's predicted flight: its state at the start, and the control it flew
/// the tick with. The last tick of a trajectory is the next to be issued, and has no control.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Step {
    pub tick: Tick,
    pub state: State,
    pub control: Option<Control>,
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
    /// The server asked for a password, and there is none to give it.
    NeedsPassword,
    /// The server refused the password.
    WrongPassword,
}

/// The prediction from one base: the own ship and its company, tick by tick.
///
/// It is a function of the base, the roster and the commands issued, and is kept only so that
/// it is not computed again: a new base starts a new one.
struct Forecast {
    /// The tick of the base it starts from, `T_b`.
    from: Tick,
    /// How many ticks ahead its ships were chosen for: it is good for ticks up to
    /// `from + horizon`.
    horizon: Tick,
    /// The ships it carries, in the order of their names: the block of the own ship in the
    /// coupling partition of the base, over `horizon` ticks.
    ids: Vec<ShipId>,
    /// Where the own ship is among them.
    own: usize,
    /// What each ship is replayed with: for the others, the control of the base, held. The own
    /// ship's place is filled at each tick.
    held: Vec<Command>,
    /// `rows[m]`: the ships at tick `from + m`.
    rows: Vec<Vec<Member>>,
    /// `used[m]`: the command the own ship replayed at tick `from + m`.
    used: Vec<Command>,
    /// `blocks[m]`: the blocks tick `from + m` was stepped by, as places in `ids`.
    blocks: Vec<Vec<Vec<usize>>>,
}

impl Forecast {
    fn new(
        dynamics: &Dynamics,
        roster: &BTreeMap<ShipId, RosterEntry>,
        ship: ShipId,
        params: Params,
        base: &Base,
        around: &[ShipRecord],
        horizon: Tick,
    ) -> Self {
        let own = Member {
            params,
            state: base.state,
            hold: base.hold,
            control: Control::default(),
        };
        let mut all = vec![(ship, own, Command::default())];
        for record in around {
            // A ship the roster has not told of yet has no parameters to fly it with.
            let Some(entry) = roster.get(&record.ship) else {
                continue;
            };
            if record.ship == ship {
                continue;
            }
            let control = Control {
                thrust: f64::from(record.thrust),
                torque: f64::from(record.torque),
            };
            let member = Member {
                params: entry.params,
                state: record.state,
                hold: record.hold,
                control,
            };
            let held = Command::Direct {
                thrust: control.thrust,
                torque: control.torque,
            };
            all.push((record.ship, member, held));
        }
        all.sort_by_key(|&(id, ..)| id);
        all.dedup_by_key(|&mut (id, ..)| id);
        let members: Vec<Member> = all.iter().map(|&(_, member, _)| member).collect();
        let me = all
            .iter()
            .position(|&(id, ..)| id == ship)
            .expect("the own ship is among them");
        let block = dynamics
            .blocks(&members, horizon.max(1))
            .into_iter()
            .find(|block| block.contains(&me))
            .expect("every ship is in a block");
        Self {
            from: base.tick,
            horizon,
            ids: block.iter().map(|&i| all[i].0).collect(),
            own: block
                .iter()
                .position(|&i| i == me)
                .expect("the own ship is in its block"),
            held: block.iter().map(|&i| all[i].2).collect(),
            rows: vec![block.iter().map(|&i| members[i]).collect()],
            used: Vec::new(),
            blocks: Vec::new(),
        }
    }

    /// The last tick computed.
    fn end(&self) -> Tick {
        self.from + self.used.len() as Tick
    }

    /// Fold forward to tick `upto`, the own ship under the commands issued (a stamp with none
    /// replays as the one before it, as the hold rule would), the others under theirs, held.
    fn extend(&mut self, dynamics: &Dynamics, cmds: &BTreeMap<Tick, Command>, upto: Tick) {
        while self.end() < upto {
            let stamp = self.end();
            let previous = self.used.last().copied().unwrap_or_default();
            let command = cmds.get(&stamp).copied().unwrap_or(previous);
            let mut commands = self.held.clone();
            commands[self.own] = command;
            let mut next = self.rows[self.used.len()].clone();
            let report = dynamics.tick(&mut next, &commands);
            self.rows.push(next);
            self.used.push(command);
            self.blocks.push(report.blocks);
        }
    }

    /// Forget what was computed past tick `upto`, whose commands were not yet issued then.
    fn truncate(&mut self, upto: Tick) {
        let keep = upto.saturating_sub(self.from) as usize;
        self.used.truncate(keep);
        self.blocks.truncate(keep);
        self.rows.truncate(keep + 1);
    }

    /// The ships at `tick`, which must have been computed.
    fn row(&self, tick: Tick) -> &[Member] {
        &self.rows[(tick - self.from) as usize]
    }
}

/// A ship being flown.
struct Play {
    ship: ShipId,
    /// The ship as granted, with the world's `h`.
    params: Params,
    /// The world's map from tick to tick, the same as the server's.
    dynamics: Dynamics,
    /// `k`: the stamp the client issues next.
    k: Tick,
    /// When the client issues it, on its own clock.
    next_issue: f64,
    /// `cmd`: what the client has issued, for the stamps it still needs.
    cmds: BTreeMap<Tick, Command>,
    base: Base,
    /// The other ships of the base's snapshot: with the base, the world at `T_b`.
    around: Vec<ShipRecord>,
    /// The prediction from the base, as far as it has been asked for.
    forecast: Option<Forecast>,
    margin: MarginController,
    /// The corrections of the snapshots absorbed since they were last taken, summed.
    pending: Option<Correction>,
    /// The same for each ship of the company that was in it before the snapshot too.
    company_pending: BTreeMap<ShipId, Correction>,
    stats: CorrectionStats,
    /// The round trip of the exchange that took the ship, in seconds: `Join` to `Welcome`.
    round_trip: f64,
    /// The snapshots of the other ships, oldest first.
    remote: BTreeMap<ShipId, VecDeque<Remote>>,
}

fn difference(after: &State, before: &State) -> State {
    State {
        x: after.x - before.x,
        y: after.y - before.y,
        vx: after.vx - before.vx,
        vy: after.vy - before.vy,
        theta: after.theta - before.theta,
        omega: after.omega - before.omega,
    }
}

fn sum(a: Correction, b: Correction) -> Correction {
    Correction {
        delta: State {
            x: a.delta.x + b.delta.x,
            y: a.delta.y + b.delta.y,
            vx: a.delta.vx + b.delta.vx,
            vy: a.delta.vy + b.delta.vy,
            theta: a.delta.theta + b.delta.theta,
            omega: a.delta.omega + b.delta.omega,
        },
        jumped: a.jumped || b.jumped,
    }
}

impl Play {
    /// The prediction from the base, folded forward to tick `upto` at least.
    fn forecast(&mut self, roster: &BTreeMap<ShipId, RosterEntry>, upto: Tick) -> &Forecast {
        let needed = upto.saturating_sub(self.base.tick);
        let fresh = self
            .forecast
            .take()
            .filter(|forecast| forecast.from == self.base.tick && forecast.horizon >= needed);
        let mut forecast = fresh.unwrap_or_else(|| {
            Forecast::new(
                &self.dynamics,
                roster,
                self.ship,
                self.params,
                &self.base,
                &self.around,
                needed.saturating_add(COMPANY_LEAD),
            )
        });
        forecast.extend(&self.dynamics, &self.cmds, upto);
        self.forecast.insert(forecast)
    }

    /// `Pred(n)` from the base: the own ship's state folded forward with the commands issued.
    fn fold(&mut self, roster: &BTreeMap<ShipId, RosterEntry>, n: Tick) -> State {
        let forecast = self.forecast(roster, n);
        forecast.row(n)[forecast.own].state
    }

    /// Take `base` and the ships `around` it as the newest snapshot, and say how far the
    /// prediction at `k` moved with it: of the own ship, and of each ship carried both before
    /// and after.
    fn rebase(
        &mut self,
        roster: &BTreeMap<ShipId, RosterEntry>,
        base: Base,
        around: Vec<ShipRecord>,
    ) {
        let k = self.k;
        let jumped = k.saturating_sub(self.base.tick) > MAX_REPLAY;
        let before: Vec<(ShipId, State)> = if jumped {
            Vec::new()
        } else {
            let forecast = self.forecast(roster, k);
            let row = forecast.row(k);
            forecast
                .ids
                .iter()
                .zip(row)
                .map(|(&id, member)| (id, member.state))
                .collect()
        };
        self.base = base;
        self.around = around;
        self.forecast = None;
        self.stats.snapshots += 1;
        if jumped {
            self.stats.jumps += 1;
            self.company_pending.clear();
            let jump = Correction {
                delta: State::default(),
                jumped: true,
            };
            self.pending = Some(
                self.pending
                    .map_or(jump, |sum_so_far| sum(sum_so_far, jump)),
            );
            return;
        }
        let forecast = self.forecast(roster, k);
        let after: Vec<(ShipId, State)> = forecast
            .ids
            .iter()
            .zip(forecast.row(k))
            .map(|(&id, member)| (id, member.state))
            .collect();
        let own = self.ship;
        let mut carried = BTreeMap::new();
        for (id, state) in &after {
            let Some((_, was)) = before.iter().find(|(other, _)| other == id) else {
                continue;
            };
            let correction = Correction {
                delta: difference(state, was),
                jumped: false,
            };
            if *id != own {
                let summed = self
                    .company_pending
                    .get(id)
                    .map_or(correction, |&so_far| sum(so_far, correction));
                carried.insert(*id, summed);
                continue;
            }
            if correction.size() > 0.0 {
                let d = &correction.delta;
                self.stats.nonzero += 1;
                let (position, heading) = (d.x.hypot(d.y), d.theta.abs());
                self.stats.visible += u64::from(position > VISIBLE || heading > VISIBLE);
                self.stats.snapped += u64::from(position > SNAP_POSITION || heading > SNAP_HEADING);
                self.stats.max_position = self.stats.max_position.max(d.x.hypot(d.y));
                self.stats.max_velocity = self.stats.max_velocity.max(d.vx.hypot(d.vy));
                self.stats.max_heading = self.stats.max_heading.max(d.theta.abs());
            }
            self.pending = Some(
                self.pending
                    .map_or(correction, |so_far| sum(so_far, correction)),
            );
        }
        // A ship that has left the company has no prediction left to correct.
        self.company_pending = carried;
    }
}

pub struct ClientMachine {
    config: Config,
    request: ShipRequest,
    build: String,
    /// What to say if the server asks for a password, and only then.
    password: Option<Password>,
    /// Join under this name as soon as the first lobby arrives. A client that has left waits to
    /// be told.
    auto_join: Option<Username>,
    /// The name last asked for, and the one the server last refused as taken.
    asked: Option<Username>,
    refused: Option<Username>,
    /// Whether the pilot has left this app for another, as last told by the screen.
    away: bool,
    phase: Phase,
    world: Option<World>,
    lobby: Option<LobbyInfo>,
    roster: BTreeMap<ShipId, RosterEntry>,
}

impl ClientMachine {
    /// A client that will ask for the ship `request`, flown as `username`, as soon as the lobby
    /// says it may.
    pub fn new(config: Config, request: ShipRequest, username: Username, build: &str) -> Self {
        let mut client = Self::waiting_in_the_lobby(config, request, build);
        client.auto_join = Some(username);
        client
    }

    /// A client that waits in the lobby for [`Self::join`], instead of taking a ship as soon as
    /// the lobby comes: for a screen where the pilot picks the ship and the name.
    pub fn waiting_in_the_lobby(config: Config, request: ShipRequest, build: &str) -> Self {
        Self {
            config,
            request,
            build: build.to_owned(),
            password: None,
            auto_join: None,
            asked: None,
            refused: None,
            away: false,
            phase: Phase::Start,
            world: None,
            lobby: None,
            roster: BTreeMap::new(),
        }
    }

    /// This client, saying `password` if the server asks for one.
    #[must_use]
    pub fn with_password(mut self, password: Password) -> Self {
        self.password = Some(password);
        self
    }

    /// The first message of the connection.
    pub fn start(&mut self) -> Vec<Kind> {
        self.phase = Phase::Hello;
        vec![Kind::Stream(encode(&ClientStream::Hello {
            protocol: PROTOCOL,
            build: self.build.clone(),
        }))]
    }

    /// Take a ship, as asked for in `request` and flown as `username`, from the lobby, at local
    /// time `now`. A client that is not in the lobby, or has not heard what the world is, does
    /// nothing.
    pub fn join(&mut self, request: ShipRequest, username: Username, now: f64) -> Vec<Kind> {
        if !matches!(self.phase, Phase::Lobby) || self.world.is_none() {
            return Vec::new();
        }
        self.request = request;
        self.phase = Phase::Joining { sent: now };
        self.asked = Some(username.clone());
        self.refused = None;
        vec![Kind::Stream(encode(&ClientStream::Join {
            request,
            username,
        }))]
    }

    /// The pilot has left this app for another, or come back. The server hears of it only while
    /// there is a ship to show it for: on a change, and when a ship comes, if the pilot is away.
    pub fn set_away(&mut self, away: bool) -> Vec<Kind> {
        if self.away == away {
            return Vec::new();
        }
        self.away = away;
        if self.is_playing() {
            vec![Kind::Stream(encode(&ClientStream::Away(away)))]
        } else {
            Vec::new()
        }
    }

    /// The name the server last refused because a ship is flying under it.
    pub fn refused_name(&self) -> Option<&Username> {
        self.refused.as_ref()
    }

    /// Give the ship up, and go back to the lobby, where this client stays until it joins again.
    pub fn leave(&mut self) -> Vec<Kind> {
        if matches!(self.phase, Phase::Playing(_)) {
            self.auto_join = None;
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
                self.lobby = Some(LobbyInfo {
                    pilots: lobby.pilots.len(),
                    max_pilots: usize::from(lobby.max_pilots),
                });
                if matches!(self.phase, Phase::Hello) {
                    let Some(username) = self.auto_join.clone() else {
                        self.phase = Phase::Lobby;
                        return Vec::new();
                    };
                    self.phase = Phase::Joining { sent: now };
                    return vec![Kind::Stream(encode(&ClientStream::Join {
                        request: self.request,
                        username,
                    }))];
                }
            }
            ServerStream::Welcome(welcome) => {
                self.auto_join = None;
                self.welcome(&welcome, now);
                if self.away && self.is_playing() {
                    return vec![Kind::Stream(encode(&ClientStream::Away(true)))];
                }
            }
            ServerStream::NameTaken => {
                if matches!(self.phase, Phase::Joining { .. }) {
                    // A client with no screen to pick another name on has nothing left to do.
                    self.phase = if self.auto_join.take().is_some() {
                        Phase::Rejected("the name is taken".to_owned())
                    } else {
                        self.refused = self.asked.clone();
                        Phase::Lobby
                    };
                }
            }
            ServerStream::Full => {
                if matches!(self.phase, Phase::Joining { .. }) {
                    self.phase = Phase::Full;
                }
            }
            ServerStream::Rejected(why) => {
                self.phase = Phase::Rejected(if why == OLD_SERVER_REFUSAL {
                    "this server speaks an older protocol than the client: update the server"
                        .to_owned()
                } else {
                    why
                });
            }
            ServerStream::Roster(entry) => self.roster_entry(entry),
            ServerStream::PasswordRequired => {
                if matches!(self.phase, Phase::Hello) {
                    let Some(password) = self.password.clone() else {
                        self.phase = Phase::NeedsPassword;
                        return Vec::new();
                    };
                    return vec![Kind::Stream(encode(&ClientStream::Password(password)))];
                }
            }
            ServerStream::WrongPassword => {
                if matches!(self.phase, Phase::Hello) {
                    self.phase = Phase::WrongPassword;
                }
            }
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
        self.phase = Phase::Playing(Box::new(Play {
            ship: welcome.ship,
            params: Params {
                h,
                ..welcome.params
            },
            dynamics: Dynamics::new(world),
            k: welcome.tick + lead as Tick,
            next_issue: now,
            cmds: BTreeMap::new(),
            base: Base {
                tick: welcome.tick,
                state: own.state,
                hold: own.hold,
            },
            around: welcome
                .ships
                .iter()
                .filter(|record| record.ship != welcome.ship)
                .copied()
                .collect(),
            forecast: None,
            margin: MarginController::new(h, self.config.target_margin, round_trip / h),
            round_trip,
            pending: None,
            company_pending: BTreeMap::new(),
            stats: CorrectionStats::default(),
            remote: BTreeMap::new(),
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
            if let Phase::Playing(play) = &mut self.phase {
                play.remote.remove(&entry.ship);
            }
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
        let margin = f64::from(snapshot.margin_us) * 1e-6;
        if snapshot.label > play.k {
            // The world has overtaken the client, so its tick says nothing of the path: only
            // Advance, below, has been moving it.
            play.margin.report(margin);
        } else {
            // Where the client's tick is now, in ticks, a fraction included, and past `k` in a
            // frame that came after the tick was due.
            let tick_now = f64::from(play.k) + (now - play.next_issue) / h;
            let ticks_ahead = tick_now - f64::from(snapshot.label - 1);
            play.margin.observe(margin, ticks_ahead);
        }
        // Advance: the client's tick never trails the world's.
        play.k = play.k.max(snapshot.label);
        // Far too late, as after a stall: jump to the lead the lemma gives, `k := T_b + L`.
        if play.margin.far_behind() {
            play.k = play.k.max(snapshot.label - 1 + play.margin.lead_ticks());
        }
        play.rebase(
            &self.roster,
            Base {
                tick: snapshot.label,
                state: own.state,
                hold: own.hold,
            },
            snapshot
                .ships
                .iter()
                .filter(|record| record.ship != play.ship)
                .copied()
                .collect(),
        );
        for record in &snapshot.ships {
            if record.ship != play.ship && self.roster.contains_key(&record.ship) {
                let seen = play.remote.entry(record.ship).or_default();
                if seen.back().is_none_or(|last| last.tick < snapshot.label) {
                    if seen.len() == REMOTE_KEPT {
                        seen.pop_front();
                    }
                    seen.push_back(Remote {
                        tick: snapshot.label,
                        record: *record,
                    });
                }
            }
        }
        let keep_from = play
            .base
            .tick
            .min(play.k.saturating_sub(self.config.copies as Tick));
        play.cmds = play.cmds.split_off(&keep_from);
        true
    }

    /// Issue every command that is due by `now`, on the client's own clock, asking `pilot` for
    /// the command of each stamp, given the own ship's predicted state at it, `Pred(k)`. A pilot
    /// at a keyboard ignores the state, and an autopilot flies by it. Returns the datagrams to
    /// send.
    pub fn poll(&mut self, now: f64, pilot: &mut dyn FnMut(Tick, &State) -> Command) -> Vec<Kind> {
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
            let predicted = play.fold(&self.roster, stamp);
            play.cmds.insert(stamp, pilot(stamp, &predicted));
            // Anything foreseen past this stamp held the command before it.
            if let Some(forecast) = &mut play.forecast {
                forecast.truncate(stamp);
            }
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

    /// Where the client is.
    pub fn stage(&self) -> Stage {
        match &self.phase {
            Phase::Start | Phase::Hello => Stage::Connecting,
            Phase::Lobby => Stage::Lobby,
            Phase::Joining { .. } => Stage::Joining,
            Phase::Playing(_) => Stage::Playing,
            Phase::Full => Stage::Full,
            Phase::Rejected(_) => Stage::Rejected,
            Phase::NeedsPassword => Stage::NeedsPassword,
            Phase::WrongPassword => Stage::WrongPassword,
        }
    }

    /// Who the lobby said was flying, when it was last heard.
    pub fn lobby(&self) -> Option<LobbyInfo> {
        self.lobby
    }

    pub fn is_playing(&self) -> bool {
        matches!(self.phase, Phase::Playing(_))
    }

    /// The reason the connection is over, if it is.
    pub fn refusal(&self) -> Option<&str> {
        match &self.phase {
            Phase::Rejected(why) => Some(why),
            Phase::Full => Some("the server is full"),
            Phase::NeedsPassword => Some("the server asks for a password"),
            Phase::WrongPassword => Some("the password is wrong"),
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

    /// The round trip of the exchange that took the ship, in seconds, at about a tick's accuracy:
    /// all the client knows of the path when the transport reports none.
    pub fn round_trip(&self) -> Option<f64> {
        self.play().map(|p| p.round_trip)
    }

    /// `e`: the last margin reported, less the target, in seconds.
    pub fn margin_error(&self) -> Option<f64> {
        self.play().map(|p| p.margin.error())
    }

    /// The ship the pilot flies, as granted.
    pub fn params(&self) -> Option<Params> {
        self.play().map(|p| p.params)
    }

    /// The command the client issued for `stamp`, if it still holds it.
    pub fn issued(&self, stamp: Tick) -> Option<Command> {
        self.play()?.cmds.get(&stamp).copied()
    }

    /// `Pred(n)`: the own ship's state at tick `n`, the newest snapshot folded forward with the
    /// commands issued. A stamp with no command replays as the one before it, as the hold rule
    /// would.
    pub fn predict(&mut self, n: Tick) -> Option<State> {
        self.predict_company(n)?
            .into_iter()
            .find(|&(ship, _)| Some(ship) == self.ship())
            .map(|(_, state)| state)
    }

    /// The prediction at tick `n` of every ship it carries, the own ship among them.
    pub fn predict_company(&mut self, n: Tick) -> Option<Vec<(ShipId, State)>> {
        let Phase::Playing(play) = &mut self.phase else {
            return None;
        };
        if n < play.base.tick {
            return None;
        }
        let forecast = play.forecast(&self.roster, n);
        Some(
            forecast
                .ids
                .iter()
                .zip(forecast.row(n))
                .map(|(&id, member)| (id, member.state))
                .collect(),
        )
    }

    /// The blocks the prediction stepped tick `n` by, which are those of its own ships as it has
    /// them at `n`, and not the server's where a ship it carries is not where the server has it.
    pub fn predicted_blocks(&mut self, n: Tick) -> Option<Vec<Vec<ShipId>>> {
        let Phase::Playing(play) = &mut self.phase else {
            return None;
        };
        if n < play.base.tick {
            return None;
        }
        let forecast = play.forecast(&self.roster, n + 1);
        let blocks = &forecast.blocks[(n - forecast.from) as usize];
        Some(
            blocks
                .iter()
                .map(|block| block.iter().map(|&i| forecast.ids[i]).collect())
                .collect(),
        )
    }

    /// The ships the prediction to the next tick to issue carries, in the order of their names:
    /// the own ship, and the ones that could come to share a block with it by then.
    pub fn company(&mut self) -> Vec<ShipId> {
        let Phase::Playing(play) = &mut self.phase else {
            return Vec::new();
        };
        let k = play.k;
        play.forecast(&self.roster, k).ids.clone()
    }

    /// The control the prediction holds for another ship: the one its last tick before the
    /// base flew.
    pub fn held(&self, ship: ShipId) -> Option<Control> {
        let record = self.play()?.around.iter().find(|r| r.ship == ship)?;
        Some(Control {
            thrust: f64::from(record.thrust),
            torque: f64::from(record.torque),
        })
    }

    /// The own ship's predicted flight from the newest snapshot to the next tick to issue, one
    /// [`Step`] a tick: for drawing the ship, and the plume it leaves.
    pub fn trajectory(&mut self) -> Vec<Step> {
        let own = self.ship();
        self.flights()
            .into_iter()
            .find(|(ship, _)| Some(*ship) == own)
            .map_or_else(Vec::new, |(_, steps)| steps)
    }

    /// The predicted flight of every other ship the prediction carries, as [`Self::trajectory`]
    /// is the own ship's: these are the ships drawn in the own ship's frame, where they may
    /// touch it.
    pub fn company_trajectories(&mut self) -> Vec<(ShipId, Vec<Step>)> {
        let own = self.ship();
        let mut flights = self.flights();
        flights.retain(|(ship, _)| Some(*ship) != own);
        flights
    }

    /// The flight of each ship of the prediction, from the base to the next tick to issue.
    fn flights(&mut self) -> Vec<(ShipId, Vec<Step>)> {
        let Phase::Playing(play) = &mut self.phase else {
            return Vec::new();
        };
        let (from, k) = (play.base.tick, play.k);
        let forecast = play.forecast(&self.roster, k);
        forecast
            .ids
            .iter()
            .enumerate()
            .map(|(n, &id)| {
                let steps = (from..=k)
                    .map(|tick| Step {
                        tick,
                        state: forecast.row(tick)[n].state,
                        // The control a tick flew is on the ship at the end of it.
                        control: (tick < k).then(|| forecast.row(tick + 1)[n].control),
                    })
                    .collect();
                (id, steps)
            })
            .collect()
    }

    /// The correction made by the snapshots absorbed since the last call, summed.
    pub fn take_correction(&mut self) -> Option<Correction> {
        match &mut self.phase {
            Phase::Playing(play) => play.pending.take(),
            _ => None,
        }
    }

    /// The corrections the snapshots absorbed since the last call made to the other ships the
    /// prediction carries, summed for each.
    pub fn take_company_corrections(&mut self) -> BTreeMap<ShipId, Correction> {
        match &mut self.phase {
            Phase::Playing(play) => std::mem::take(&mut play.company_pending),
            _ => BTreeMap::new(),
        }
    }

    /// What the corrections so far have been.
    pub fn correction_stats(&self) -> Option<CorrectionStats> {
        self.play().map(|p| p.stats)
    }

    /// The other ships' snapshots, oldest first, by ship.
    pub fn remotes(&self) -> Option<&BTreeMap<ShipId, VecDeque<Remote>>> {
        self.play().map(|p| &p.remote)
    }

    /// The tick map's `h`, if the world is known.
    pub fn h(&self) -> Option<f64> {
        self.world.as_ref().map(World::h)
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

#[cfg(test)]
mod tests {
    use nabla_sim::world::Preset;

    use super::*;
    use crate::wire::{Lobby, Welcome};

    fn world() -> World {
        Preset::Vacuum
            .world()
            .multiplayer(60, 1000.0, 4)
            .expect("a valid world")
    }

    fn name(text: &str) -> Username {
        Username::parse(text).expect("a valid name")
    }

    fn request() -> ShipRequest {
        ShipRequest::from(world().default_ship())
    }

    fn lobby() -> Vec<u8> {
        encode(&ServerStream::Lobby(Lobby {
            world: world().to_toml().expect("a world in TOML"),
            pilots: Vec::new(),
            max_pilots: 4,
        }))
    }

    /// Say hello, and hear the lobby: what the client answers.
    fn through_the_lobby(client: &mut ClientMachine) -> Vec<Kind> {
        client.start();
        client.on_stream(&lobby(), 0.0)
    }

    fn unconnected() -> ClientMachine {
        ClientMachine::waiting_in_the_lobby(Config::default(), request(), "test")
    }

    fn waiting() -> ClientMachine {
        let mut client = unconnected();
        assert!(through_the_lobby(&mut client).is_empty());
        client
    }

    fn welcome() -> Vec<u8> {
        let world = world();
        encode(&ServerStream::Welcome(Welcome {
            ship: ShipId(0),
            params: world.default_ship(),
            tick: 0,
            ships: vec![ShipRecord {
                ship: ShipId(0),
                state: world.spawn(0).state,
                hold: None,
                thrust: 0.0,
                torque: 0.0,
            }],
        }))
    }

    fn says(message: &ClientStream) -> Vec<Kind> {
        vec![Kind::Stream(encode(message))]
    }

    #[test]
    fn a_taken_name_leaves_the_client_in_the_lobby_to_pick_another() {
        let mut client = waiting();
        client.join(request(), name("miriam"), 1.0);
        assert_eq!(client.stage(), Stage::Joining);
        assert!(
            client
                .on_stream(&encode(&ServerStream::NameTaken), 1.1)
                .is_empty()
        );
        assert_eq!(client.stage(), Stage::Lobby);
        assert_eq!(client.refused_name(), Some(&name("miriam")));
        assert_eq!(
            client.join(request(), name("juan"), 2.0),
            says(&ClientStream::Join {
                request: request(),
                username: name("juan"),
            })
        );
        assert_eq!(client.stage(), Stage::Joining);
        assert_eq!(client.refused_name(), None, "asking again clears it");
    }

    #[test]
    fn a_client_that_joins_by_itself_stops_at_a_taken_name() {
        let mut client = ClientMachine::new(Config::default(), request(), name("bot1"), "test");
        assert_eq!(
            through_the_lobby(&mut client),
            says(&ClientStream::Join {
                request: request(),
                username: name("bot1"),
            })
        );
        client.on_stream(&encode(&ServerStream::NameTaken), 0.1);
        assert_eq!(client.stage(), Stage::Rejected);
        assert_eq!(client.refusal(), Some("the name is taken"));
    }

    #[test]
    fn a_client_with_a_password_says_it_when_asked_and_only_then() {
        let password = Password::parse("hunter2").expect("valid");
        let mut client = unconnected().with_password(password.clone());
        assert_eq!(
            client.start(),
            says(&ClientStream::Hello {
                protocol: PROTOCOL,
                build: "test".to_owned(),
            }),
            "the first message does not hold it"
        );
        assert_eq!(
            client.on_stream(&encode(&ServerStream::PasswordRequired), 0.1),
            says(&ClientStream::Password(password))
        );
        assert_eq!(client.stage(), Stage::Connecting);

        // From the lobby, a server has no more to ask for.
        client.on_stream(&lobby(), 0.2);
        assert_eq!(client.stage(), Stage::Lobby);
        assert!(
            client
                .on_stream(&encode(&ServerStream::PasswordRequired), 0.3)
                .is_empty()
        );
    }

    #[test]
    fn a_client_without_a_password_stops_and_says_so() {
        let mut client = unconnected();
        client.start();
        assert!(
            client
                .on_stream(&encode(&ServerStream::PasswordRequired), 0.1)
                .is_empty()
        );
        assert_eq!(client.stage(), Stage::NeedsPassword);
        assert_eq!(client.refusal(), Some("the server asks for a password"));
    }

    #[test]
    fn a_wrong_password_ends_the_attempt() {
        let password = Password::parse("hunter2").expect("valid");
        let mut client = unconnected().with_password(password);
        client.start();
        client.on_stream(&encode(&ServerStream::PasswordRequired), 0.1);
        client.on_stream(&encode(&ServerStream::WrongPassword), 0.2);
        assert_eq!(client.stage(), Stage::WrongPassword);
        assert_eq!(client.refusal(), Some("the password is wrong"));
    }

    #[test]
    fn a_refusal_from_an_older_server_says_to_update_it() {
        let mut client = unconnected();
        client.start();
        let refusal = ServerStream::Rejected(OLD_SERVER_REFUSAL.to_owned());
        client.on_stream(&encode(&refusal), 0.1);
        assert_eq!(client.stage(), Stage::Rejected);
        assert!(
            client
                .refusal()
                .is_some_and(|why| why.contains("update the server")),
            "{:?}",
            client.refusal()
        );
    }

    #[test]
    fn any_other_refusal_is_passed_on_as_it_came() {
        let mut client = unconnected();
        client.start();
        client.on_stream(&encode(&ServerStream::Rejected("why".to_owned())), 0.1);
        assert_eq!(client.refusal(), Some("why"));
    }

    #[test]
    fn a_taken_name_out_of_turn_changes_nothing() {
        let mut client = waiting();
        client.on_stream(&encode(&ServerStream::NameTaken), 0.1);
        assert_eq!(client.stage(), Stage::Lobby);
        assert_eq!(client.refused_name(), None);
        client.join(request(), name("miriam"), 1.0);
        client.on_stream(&welcome(), 1.1);
        client.on_stream(&encode(&ServerStream::NameTaken), 1.2);
        assert_eq!(client.stage(), Stage::Playing);
        assert_eq!(client.refused_name(), None);
    }

    #[test]
    fn the_server_hears_of_an_absence_only_on_a_change_and_with_a_ship() {
        let mut client = waiting();
        assert!(client.set_away(true).is_empty(), "no ship to show it for");
        client.join(request(), name("miriam"), 1.0);
        assert_eq!(
            client.on_stream(&welcome(), 1.1),
            says(&ClientStream::Away(true)),
            "away since before the ship came"
        );
        assert!(client.set_away(true).is_empty(), "no change");
        assert_eq!(client.set_away(false), says(&ClientStream::Away(false)));
        assert!(client.set_away(false).is_empty(), "no change");
        assert_eq!(client.set_away(true), says(&ClientStream::Away(true)));
    }

    #[test]
    fn a_pilot_at_the_controls_says_nothing_when_the_ship_comes() {
        let mut client = waiting();
        client.join(request(), name("miriam"), 1.0);
        assert!(client.on_stream(&welcome(), 1.1).is_empty());
    }
}
