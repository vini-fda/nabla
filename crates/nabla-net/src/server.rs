//! The server's state machine: `Deliver`, `Tick`, `Join` and `Leave` of the specification.
//!
//! It is a function of what it is handed, and a call is atomic. `Tick` waits for nobody: it has
//! no precondition, and no client or network can stop the world.
//!
//! The world is the ships' states and nothing more. A tick is the world's map
//! ([`nabla_sim::fleet::Dynamics::tick`]) applied to them and to the commands chosen, so what a
//! snapshot carries is all a client needs to compute the next tick as the server will.

use std::collections::BTreeMap;

use nabla_core::Params;
use nabla_sim::command::Command;
use nabla_sim::fleet::{Dynamics, Member};
use nabla_sim::world::{MAX_PILOTS, World};

use crate::wire::{
    ClientStream, Commands, ConnId, Lobby, MAX_COMMANDS, MAX_STREAM_FRAME, PROTOCOL, Password,
    RosterEntry, ServerStream, ShipId, ShipRecord, Snapshot, Tick, Username, Welcome, decode,
    encode,
};
use crate::{Config, Kind, Outgoing};

/// What a ship's state was at a tick: all that a snapshot says of the world.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    pub state: nabla_core::State,
    pub hold: Option<f64>,
}

/// A ship's life on the server, the history variable of the proofs: when it joined, and for each
/// tick what was applied and the state after it.
#[derive(Clone, Debug, PartialEq)]
pub struct ShipLog {
    /// The tick the ship joined at, `T_0`.
    pub since: Tick,
    /// `samples[n]` is the ship at tick `since + n`, so there is one more than `commands`.
    pub samples: Vec<Sample>,
    /// `commands[n]` is the command that tick `since + n` applied.
    pub commands: Vec<Command>,
    /// `controls[n]`: the control tick `since + n` flew, which the snapshot after it carries.
    pub controls: Vec<nabla_core::Control>,
    /// `delivered[n]`: a command stamped `since + n` had arrived, and was the one applied.
    pub delivered: Vec<bool>,
    /// `company[n]`: the other ships of this ship's block at tick `since + n`, which are the only
    /// ships that tick could let act on it. Empty when it flew the tick alone.
    pub company: Vec<Vec<ShipId>>,
    /// `touched[n]`: at tick `since + n` the ship's hull pushed on another ship's, or on a body.
    pub touched: Vec<bool>,
}

impl ShipLog {
    /// The ship at `tick`, if it was in the world then.
    pub fn at(&self, tick: Tick) -> Option<&Sample> {
        self.samples
            .get(usize::try_from(tick.checked_sub(self.since)?).ok()?)
    }

    /// The command applied at `tick`.
    pub fn applied(&self, tick: Tick) -> Option<&Command> {
        self.commands
            .get(usize::try_from(tick.checked_sub(self.since)?).ok()?)
    }

    /// Whether the command applied at `tick` was one that had arrived for it.
    pub fn was_delivered(&self, tick: Tick) -> Option<bool> {
        self.delivered
            .get(usize::try_from(tick.checked_sub(self.since)?).ok()?)
            .copied()
    }

    /// The control `tick` flew.
    pub fn control_at(&self, tick: Tick) -> Option<nabla_core::Control> {
        self.controls
            .get(usize::try_from(tick.checked_sub(self.since)?).ok()?)
            .copied()
    }

    /// The other ships of the ship's block at `tick`.
    pub fn company_at(&self, tick: Tick) -> Option<&[ShipId]> {
        self.company
            .get(usize::try_from(tick.checked_sub(self.since)?).ok()?)
            .map(Vec::as_slice)
    }

    /// Whether the ship's hull pushed on anything at `tick`.
    pub fn touched_at(&self, tick: Tick) -> Option<bool> {
        self.touched
            .get(usize::try_from(tick.checked_sub(self.since)?).ok()?)
            .copied()
    }
}

/// What a connection is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    /// Connected, and `Hello` not yet heard.
    Fresh,
    /// `Hello` is heard, and the server waits for the password.
    Locked,
    /// In the lobby: no ship.
    Lobby,
    /// Flying this ship.
    Piloting(ShipId),
    /// Told to go, and on its way out: nothing it says counts any more.
    TurnedAway,
}

impl Phase {
    /// Whether the connection may hear of the world and who is in it.
    fn is_admitted(self) -> bool {
        matches!(self, Self::Lobby | Self::Piloting(_))
    }
}

struct Conn {
    phase: Phase,
    /// When it connected, by the server's clock.
    since: f64,
    /// When anything last came from it, by the server's clock.
    last_heard: f64,
}

/// A ship in the world and what the server keeps for it.
struct Slot {
    conn: ConnId,
    /// The ship itself: its granted parameters, its state, and the control of its last tick.
    member: Member,
    params: Params,
    /// `buf[i]`: commands received for the stamps in the window, `[T, T + N_b)`.
    buf: BTreeMap<Tick, Command>,
    /// `last[i]`: the command applied last, which the hold rule repeats.
    last: Command,
    /// `miss[i]`: ticks in a row with no command.
    miss: u32,
    /// Whether a command has been applied yet: the ticks before the first are not its pilot's.
    commanded: bool,
    /// How early the newest command came, for the next snapshot, in microseconds.
    margin_us: i32,
    newest_stamp: Option<Tick>,
    /// The tick of the last command that asked for something: not neutral, as a hand off is.
    last_input: Tick,
    /// The pilot said it has left the app.
    away: bool,
}

pub struct ServerMachine {
    world: World,
    /// The world's map from tick to tick, which keeps nothing between ticks.
    dynamics: Dynamics,
    world_file: String,
    config: Config,
    /// What a connection must say to be let in, if anything.
    password: Option<Password>,
    /// The real time of tick 0: `R(T) = t0 + T h`.
    t0: f64,
    /// The tick that runs next: `T`.
    tick: Tick,
    conns: BTreeMap<ConnId, Conn>,
    ships: BTreeMap<ShipId, Slot>,
    roster: BTreeMap<ShipId, RosterEntry>,
    next_ship: u32,
    /// How many ships have joined in all, to pick spawn places.
    joined: usize,
    logs: BTreeMap<ShipId, ShipLog>,
    /// Ticks of ships already under command that came with no command for them.
    held: u64,
    /// Ticks of pairs of ships whose hulls pushed on each other.
    touches: u64,
}

impl ServerMachine {
    /// A server for `world`, whose tick 0 is at real time `t0`.
    ///
    /// # Errors
    /// When the world is not a version 2 world, or allows more pilots than `config` does.
    pub fn new(world: World, config: Config, t0: f64) -> Result<Self, String> {
        if world.version != 2 {
            return Err("A server needs a world version 2".to_owned());
        }
        if config.max_pilots == 0 || config.max_pilots > MAX_PILOTS {
            return Err(format!("max_pilots must be 1 to {MAX_PILOTS}"));
        }
        world.validate()?;
        Ok(Self {
            world_file: world.to_toml()?,
            dynamics: Dynamics::new(&world),
            world,
            config,
            password: None,
            t0,
            tick: 0,
            conns: BTreeMap::new(),
            ships: BTreeMap::new(),
            roster: BTreeMap::new(),
            next_ship: 0,
            joined: 0,
            logs: BTreeMap::new(),
            held: 0,
            touches: 0,
        })
    }

    /// This server, letting in only the connections that say `password`.
    #[must_use]
    pub fn with_password(mut self, password: Password) -> Self {
        self.password = Some(password);
        self
    }

    /// The tick that runs next, `T`: so there have been `T` of them.
    pub fn tick_number(&self) -> Tick {
        self.tick
    }

    /// How many times a ship already under command got no command for its tick, which came late,
    /// was lost, or was never sent. Only then can a prediction be wrong (`thm-exact`).
    pub fn held(&self) -> u64 {
        self.held
    }

    /// How many times, over all ticks, two ships' hulls pushed on each other in a tick.
    pub fn touches(&self) -> u64 {
        self.touches
    }

    pub fn world(&self) -> &World {
        &self.world
    }

    /// The real time tick `tick` is due, `R(tick)`.
    pub fn due(&self, tick: Tick) -> f64 {
        self.t0 + f64::from(tick) * self.world.h()
    }

    /// How many ships are flying.
    pub fn pilots(&self) -> usize {
        self.ships.len()
    }

    /// The ship a connection flies, if it flies one.
    pub fn ship_of(&self, conn: ConnId) -> Option<ShipId> {
        match self.conns.get(&conn)?.phase {
            Phase::Piloting(ship) => Some(ship),
            _ => None,
        }
    }

    /// How many commands are buffered for a ship, which never exceeds `N_b`.
    pub fn buffered(&self, ship: ShipId) -> Option<usize> {
        self.ships.get(&ship).map(|slot| slot.buf.len())
    }

    /// The ship's state now, if it is flying.
    pub fn state_of(&self, ship: ShipId) -> Option<nabla_core::State> {
        self.ships.get(&ship).map(|slot| slot.member.state)
    }

    /// The world's map from tick to tick.
    pub fn dynamics(&self) -> &Dynamics {
        &self.dynamics
    }

    /// The ships flying, with their states now, in the order of their names: the world `W`.
    pub fn fleet(&self) -> Vec<(ShipId, Member)> {
        self.ships
            .iter()
            .map(|(&id, slot)| (id, slot.member))
            .collect()
    }

    /// The parameters a flying ship was granted.
    pub fn params_of(&self, ship: ShipId) -> Option<Params> {
        self.ships.get(&ship).map(|slot| slot.params)
    }

    /// The log of a ship, living or gone, if the config keeps logs.
    pub fn log(&self, ship: ShipId) -> Option<&ShipLog> {
        self.logs.get(&ship)
    }

    /// A connection arrived.
    pub fn connect(&mut self, conn: ConnId, now: f64) {
        self.conns.insert(
            conn,
            Conn {
                phase: Phase::Fresh,
                since: now,
                last_heard: now,
            },
        );
    }

    /// A connection is gone: its ship, if any, leaves.
    pub fn on_disconnect(&mut self, conn: ConnId) -> Vec<Outgoing> {
        let mut out = Vec::new();
        if let Some(Phase::Piloting(ship)) = self.conns.get(&conn).map(|c| c.phase) {
            self.leave(ship, &mut out);
        }
        self.conns.remove(&conn);
        out
    }

    /// A frame arrived on a connection's stream.
    pub fn on_stream(&mut self, conn: ConnId, bytes: &[u8], now: f64) -> Vec<Outgoing> {
        let mut out = Vec::new();
        let Some(c) = self.conns.get_mut(&conn) else {
            return out;
        };
        c.last_heard = now;
        let phase = c.phase;
        let Ok(message) = decode::<ClientStream>(bytes, MAX_STREAM_FRAME) else {
            // A client that speaks nonsense on a stream is not worth keeping.
            out.push(Outgoing {
                conn,
                kind: Kind::Close,
            });
            return out;
        };
        match (phase, message) {
            (Phase::Fresh, ClientStream::Hello { protocol, .. }) => {
                if protocol != PROTOCOL {
                    out.push(stream(
                        conn,
                        &ServerStream::Rejected(other_version(protocol)),
                    ));
                    out.push(Outgoing {
                        conn,
                        kind: Kind::Close,
                    });
                } else if self.password.is_some() {
                    self.set_phase(conn, Phase::Locked);
                    out.push(stream(conn, &ServerStream::PasswordRequired));
                } else {
                    self.set_phase(conn, Phase::Lobby);
                    out.push(self.lobby(conn));
                }
            }
            (Phase::Locked, ClientStream::Password(given)) => {
                if self.password.as_ref().is_some_and(|p| p.matches(&given)) {
                    self.set_phase(conn, Phase::Lobby);
                    out.push(self.lobby(conn));
                } else {
                    self.set_phase(conn, Phase::TurnedAway);
                    out.push(stream(conn, &ServerStream::WrongPassword));
                    out.push(Outgoing {
                        conn,
                        kind: Kind::Close,
                    });
                }
            }
            (Phase::Lobby, ClientStream::Join { request, username }) => {
                self.join(conn, &request, username, &mut out);
            }
            (Phase::Piloting(ship), ClientStream::Leave) => {
                self.leave(ship, &mut out);
                out.push(self.lobby(conn));
            }
            (Phase::Piloting(ship), ClientStream::Away(away)) => {
                if let Some(slot) = self.ships.get_mut(&ship) {
                    slot.away = away;
                }
            }
            // A message out of turn changes nothing.
            _ => {}
        }
        out
    }

    /// A datagram arrived from a connection, at real time `now`: `Deliver` of the specification
    /// for each command in it.
    pub fn on_datagram(&mut self, conn: ConnId, bytes: &[u8], now: f64) {
        let Some(c) = self.conns.get_mut(&conn) else {
            return;
        };
        c.last_heard = now;
        let Phase::Piloting(id) = c.phase else {
            return;
        };
        let Ok(commands) = decode::<Commands>(bytes, self.config.max_datagram) else {
            return;
        };
        if commands.cmds.len() > MAX_COMMANDS {
            return;
        }
        let Some(slot) = self.ships.get_mut(&id) else {
            return;
        };
        // The margin of the newest command to arrive: how long before its tick was due, which is
        // negative for one that came too late to count.
        if slot
            .newest_stamp
            .is_none_or(|newest| commands.stamp > newest)
        {
            slot.newest_stamp = Some(commands.stamp);
            let due = self.t0 + f64::from(commands.stamp) * self.world.h();
            slot.margin_us =
                ((due - now) * 1e6).clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32;
        }
        for (offset, command) in commands.cmds.iter().enumerate() {
            let Some(command) = command else { continue };
            let Some(stamp) = u32::try_from(offset)
                .ok()
                .and_then(|offset| commands.stamp.checked_sub(offset))
            else {
                break;
            };
            // Discard below `T`, and bound memory above the window. First write wins.
            let in_window = stamp >= self.tick && stamp - self.tick < self.config.window;
            if in_window && !slot.buf.contains_key(&stamp) {
                slot.buf.insert(stamp, command.sanitize(&slot.params));
            }
        }
    }

    /// The tick due now, at real time `now`, which should be `R(T)`: every ship takes one step
    /// under the command it gets, and every pilot is sent the world.
    ///
    /// It has no precondition. A ship with no command for this tick gets the hold rule's.
    pub fn tick(&mut self, now: f64) -> Vec<Outgoing> {
        let mut out = Vec::new();
        self.retire_the_idle(now, &mut out);
        self.retire_the_unadmitted(now, &mut out);
        let t = self.tick;
        // `Chosen(i)` for every ship, and then the world's map: one step of all of them.
        let mut delivered = Vec::with_capacity(self.ships.len());
        let mut chosen = Vec::with_capacity(self.ships.len());
        let mut fleet = Vec::with_capacity(self.ships.len());
        for slot in self.ships.values_mut() {
            let arrived = slot.buf.remove(&t);
            chosen.push(match arrived {
                Some(command) => command,
                None if slot.miss.saturating_add(1) < self.config.silence_ticks => slot.last,
                None => slot.last.neutral(),
            });
            delivered.push(arrived.is_some());
            fleet.push(slot.member);
        }
        let report = self.dynamics.tick(&mut fleet, &chosen);
        let ids: Vec<ShipId> = self.ships.keys().copied().collect();
        let mut block_of = vec![0; ids.len()];
        for (b, block) in report.blocks.iter().enumerate() {
            for &i in block {
                block_of[i] = b;
            }
        }
        for (i, slot) in self.ships.values_mut().enumerate() {
            let (chosen, delivered) = (chosen[i], delivered[i]);
            slot.member = fleet[i];
            slot.last = chosen;
            slot.miss = if delivered {
                0
            } else {
                slot.miss.saturating_add(1)
            };
            self.held += u64::from(!delivered && slot.commanded);
            slot.commanded |= delivered;
            if delivered && chosen != chosen.neutral() {
                slot.last_input = t;
            }
            if self.config.keep_log
                && let Some(log) = self.logs.get_mut(&ids[i])
            {
                log.commands.push(chosen);
                log.controls.push(slot.member.control);
                log.delivered.push(delivered);
                log.samples.push(Sample {
                    state: slot.member.state,
                    hold: slot.member.hold,
                });
                log.company.push(
                    report.blocks[block_of[i]]
                        .iter()
                        .filter(|&&j| j != i)
                        .map(|&j| ids[j])
                        .collect(),
                );
                log.touched.push(
                    report.grounded.contains(&i)
                        || report.touches.iter().any(|&(a, b)| a == i || b == i),
                );
            }
        }
        self.touches += report.touches.len() as u64;
        self.tick += 1;
        self.report_presence(&mut out);
        let records: Vec<ShipRecord> = self
            .ships
            .iter()
            .map(|(&id, slot)| record(id, &slot.member))
            .collect();
        for slot in self.ships.values() {
            let snapshot = Snapshot {
                label: self.tick,
                margin_us: slot.margin_us,
                ships: records.clone(),
            };
            out.push(Outgoing {
                conn: slot.conn,
                kind: Kind::Datagram(encode(&snapshot)),
            });
        }
        out
    }

    fn set_phase(&mut self, conn: ConnId, phase: Phase) {
        if let Some(c) = self.conns.get_mut(&conn) {
            c.phase = phase;
        }
    }

    fn lobby(&self, conn: ConnId) -> Outgoing {
        stream(
            conn,
            &ServerStream::Lobby(Lobby {
                world: self.world_file.clone(),
                pilots: self.ships.keys().copied().collect(),
                max_pilots: u8::try_from(self.config.max_pilots).unwrap_or(u8::MAX),
            }),
        )
    }

    /// `Join`: a ship, as granted, at a spawn place where it is alone in its block, and in the
    /// world between ticks. With no such place, as with no place left, there is no room.
    fn join(
        &mut self,
        conn: ConnId,
        request: &nabla_sim::world::ShipRequest,
        username: Username,
        out: &mut Vec<Outgoing>,
    ) {
        if self.ships.len() >= self.config.max_pilots {
            out.push(stream(conn, &ServerStream::Full));
            return;
        }
        if self.roster.values().any(|entry| entry.username == username) {
            out.push(stream(conn, &ServerStream::NameTaken));
            return;
        }
        let params = self.world.grant(request);
        let flying: Vec<Member> = self.ships.values().map(|slot| slot.member).collect();
        let Some(spawn) = self
            .dynamics
            .spawn(&self.world, &flying, self.joined, params)
        else {
            out.push(stream(conn, &ServerStream::Full));
            return;
        };
        let id = ShipId(self.next_ship);
        self.next_ship += 1;
        self.joined += 1;
        let member = Member::new(params, spawn);
        let sample = Sample {
            state: member.state,
            hold: member.hold,
        };
        if self.config.keep_log {
            self.logs.insert(
                id,
                ShipLog {
                    since: self.tick,
                    samples: vec![sample],
                    commands: Vec::new(),
                    controls: Vec::new(),
                    delivered: Vec::new(),
                    company: Vec::new(),
                    touched: Vec::new(),
                },
            );
        }
        self.ships.insert(
            id,
            Slot {
                conn,
                member,
                params,
                buf: BTreeMap::new(),
                last: Command::default(),
                miss: 0,
                commanded: false,
                margin_us: 0,
                newest_stamp: None,
                last_input: self.tick,
                away: false,
            },
        );
        self.set_phase(conn, Phase::Piloting(id));
        let entry = RosterEntry {
            ship: id,
            params,
            since: self.tick,
            until: None,
            username,
            active: true,
        };
        self.roster.insert(id, entry.clone());
        out.push(stream(
            conn,
            &ServerStream::Welcome(Welcome {
                ship: id,
                params,
                tick: self.tick,
                ships: self
                    .ships
                    .iter()
                    .map(|(&i, s)| record(i, &s.member))
                    .collect(),
            }),
        ));
        for existing in self.roster.values() {
            out.push(stream(conn, &ServerStream::Roster(existing.clone())));
        }
        let others: Vec<ConnId> = self
            .conns
            .iter()
            .filter(|&(&c, state)| c != conn && state.phase.is_admitted())
            .map(|(&c, _)| c)
            .collect();
        let message = ServerStream::Roster(entry);
        for other in others {
            out.push(stream(other, &message));
        }
    }

    /// `Leave`: the ship goes, between ticks, and everyone is told.
    fn leave(&mut self, ship: ShipId, out: &mut Vec<Outgoing>) {
        let Some(slot) = self.ships.remove(&ship) else {
            return;
        };
        self.set_phase(slot.conn, Phase::Lobby);
        if let Some(mut entry) = self.roster.remove(&ship) {
            entry.until = Some(self.tick);
            let message = ServerStream::Roster(entry);
            for (&conn, state) in &self.conns {
                if state.phase.is_admitted() {
                    out.push(stream(conn, &message));
                }
            }
        }
    }

    /// Whether a pilot counts as active: it has not stepped away, its commands are arriving, and
    /// it has asked for something lately. It only reads the slot: the flight never depends on it.
    fn is_active(&self, slot: &Slot) -> bool {
        let idle = f64::from(self.tick.saturating_sub(slot.last_input)) * self.world.h();
        !slot.away && slot.miss < self.config.silence_ticks && idle <= self.config.input_idle
    }

    /// Tell everyone of each ship whose activity changed, once, as `leave` tells of a departure.
    fn report_presence(&mut self, out: &mut Vec<Outgoing>) {
        let changed: Vec<(ShipId, bool)> = self
            .ships
            .iter()
            .map(|(&id, slot)| (id, self.is_active(slot)))
            .filter(|&(id, active)| self.roster.get(&id).is_some_and(|e| e.active != active))
            .collect();
        for (id, active) in changed {
            let Some(entry) = self.roster.get_mut(&id) else {
                continue;
            };
            entry.active = active;
            let message = ServerStream::Roster(entry.clone());
            for (&conn, state) in &self.conns {
                if state.phase.is_admitted() {
                    out.push(stream(conn, &message));
                }
            }
        }
    }

    /// A ship silent for `T_idle` leaves, so a forgotten tab does not hold one of the places.
    fn retire_the_idle(&mut self, now: f64, out: &mut Vec<Outgoing>) {
        let idle: Vec<ShipId> = self
            .ships
            .iter()
            .filter(|(_, slot)| {
                self.conns
                    .get(&slot.conn)
                    .is_some_and(|c| now - c.last_heard > self.config.idle)
            })
            .map(|(&id, _)| id)
            .collect();
        for ship in idle {
            self.leave(ship, out);
        }
    }

    /// A connection that has not got as far as the lobby in time goes, so that one which never
    /// says anything, or does not know the password, does not hold a connection's place.
    fn retire_the_unadmitted(&mut self, now: f64, out: &mut Vec<Outgoing>) {
        for (&conn, c) in &mut self.conns {
            let waiting = matches!(c.phase, Phase::Fresh | Phase::Locked);
            if waiting && now - c.since > self.config.admission {
                c.phase = Phase::TurnedAway;
                out.push(Outgoing {
                    conn,
                    kind: Kind::Close,
                });
            }
        }
    }
}

/// What to tell a client that speaks another version of the protocol, and which end to update.
fn other_version(client: u16) -> String {
    let older = if client < PROTOCOL {
        "client"
    } else {
        "server"
    };
    format!("the client speaks protocol {client} and the server {PROTOCOL}: update the {older}")
}

/// A ship as a snapshot says it. The control is exactly the one its last tick flew, which the
/// world's map rounded to single precision.
fn record(ship: ShipId, member: &Member) -> ShipRecord {
    ShipRecord {
        ship,
        state: member.state,
        hold: member.hold,
        thrust: member.control.thrust as f32,
        torque: member.control.torque as f32,
    }
}

fn stream(conn: ConnId, message: &ServerStream) -> Outgoing {
    Outgoing {
        conn,
        kind: Kind::Stream(encode(message)),
    }
}
