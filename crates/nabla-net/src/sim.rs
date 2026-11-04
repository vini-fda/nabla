//! A simulated network, seeded and deterministic, for running the machines against each other.
//!
//! A [`Scenario`] says who connects when, over what links, flying how, and which of them are
//! hostile. [`Sim::run`] plays it out in simulated time, with one event queue, and checks the
//! theorems of `docs/multiplayer.typ` as assertions along the way: that the log holds exactly one
//! command per tick and never changes (`thm-once`), that a ship depends only on its own
//! connection (`thm-contain`, which the tests check by comparing two runs), and that a
//! prediction made with the commands that were applied is exact (`thm-exact`).
//!
//! The same seed gives the same run on every machine. Each peer's links draw from their own
//! random stream, so what one peer does cannot change the luck of another's packets.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap};

use nabla_core::State;
use nabla_sim::command::Command;
use nabla_sim::rng::Rng;
use nabla_sim::steering::{Aim, Input, Steering};
use nabla_sim::world::{ShipRequest, World};

use crate::client::ClientMachine;
use crate::server::ServerMachine;
use crate::wire::{ClientStream, Commands, ConnId, ShipId, Tick, Username, encode};
use crate::{Config, Kind, Outgoing};

/// What a one-way path does to datagrams. Streams ride the same delay and jitter but are
/// reliable and in order, as QUIC's are.
#[derive(Clone, Debug, PartialEq)]
pub struct Link {
    /// Seconds every datagram takes at least.
    pub delay: f64,
    /// Seconds more, uniformly in `[0, jitter)`. Jitter beyond a tick reorders datagrams.
    pub jitter: f64,
    /// The chance a datagram is lost.
    pub loss: f64,
    /// The chance a datagram arrives twice.
    pub duplicate: f64,
    /// Stretches of time, `[start, end)` in seconds, when every datagram is lost: a burst.
    pub outages: Vec<(f64, f64)>,
    /// From `time` on, every datagram takes `extra` seconds more: a change in the path.
    pub steps: Vec<(f64, f64)>,
}

impl Link {
    /// A path with a constant delay, and nothing else wrong with it.
    pub fn clean(delay: f64) -> Self {
        Self {
            delay,
            jitter: 0.0,
            loss: 0.0,
            duplicate: 0.0,
            outages: Vec::new(),
            steps: Vec::new(),
        }
    }

    /// The delay of a datagram sent at `now`, before jitter.
    fn delay_at(&self, now: f64) -> f64 {
        self.delay
            + self
                .steps
                .iter()
                .filter(|&&(t, _)| t <= now)
                .map(|&(_, extra)| extra)
                .sum::<f64>()
    }

    /// A bad path: jitter, loss, duplication and reordering.
    pub fn bad(delay: f64) -> Self {
        Self {
            delay,
            jitter: 0.05,
            loss: 0.1,
            duplicate: 0.05,
            outages: Vec::new(),
            steps: Vec::new(),
        }
    }

    /// When the copies of a datagram sent at `now` arrive: none, if it is lost.
    fn arrivals(&self, rng: &mut Rng, now: f64) -> Vec<f64> {
        let in_outage = self.outages.iter().any(|&(a, b)| a <= now && now < b);
        if in_outage || rng.chance(self.loss) {
            return Vec::new();
        }
        let copies = 1 + usize::from(rng.chance(self.duplicate));
        (0..copies)
            .map(|_| now + self.delay_at(now) + self.jitter * rng.unit())
            .collect()
    }
}

/// The name peer `peer` flies under, which no other peer has.
fn name_of(peer: usize) -> Username {
    Username::parse(&format!("pilot{peer}")).expect("a valid name")
}

/// What a hostile peer does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hostile {
    /// Random bytes as datagrams, of every length.
    Garbage,
    /// Joins, then sends commands made of any bit patterns at all, for stamps near the present.
    NanCommands,
    /// Joins, then sends stamps at the edges of the type, in the past and the far future, with
    /// the same stamp twice with different commands, and datagrams with too many commands.
    StampAbuse,
    /// Joins, then sends fifty valid datagrams a tick.
    Flood,
    /// Joins and leaves, hellos twice, joins before hello, leaves when it has no ship.
    Churn,
}

/// Who a peer is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Behavior {
    /// A client machine flying with a pilot who holds keys for a while and then changes them.
    Honest,
    Hostile(Hostile),
}

/// One peer: a connection with its links and its clock.
#[derive(Clone, Debug, PartialEq)]
pub struct Peer {
    pub behavior: Behavior,
    pub request: ShipRequest,
    /// Seconds into the run that it connects.
    pub join_at: f64,
    /// Seconds into the run that it asks to leave.
    pub leave_at: Option<f64>,
    /// Seconds into the run that it falls silent, with the connection still open: a hidden tab,
    /// which stops running frames but not the network.
    pub silent_at: Option<f64>,
    /// Seconds into the run that it wakes again.
    pub resume_at: Option<f64>,
    pub up: Link,
    pub down: Link,
    /// The fraction its clock runs fast by.
    pub drift: f64,
    /// Seconds its clock is ahead of the true time at the start.
    pub offset: f64,
}

impl Peer {
    /// An honest peer for `world`'s default ship on a path with `delay` seconds each way.
    pub fn honest(world: &World, delay: f64) -> Self {
        Self {
            behavior: Behavior::Honest,
            request: ShipRequest::from(world.default_ship()),
            join_at: 0.0,
            leave_at: None,
            silent_at: None,
            resume_at: None,
            up: Link::clean(delay),
            down: Link::clean(delay),
            drift: 0.0,
            offset: 0.0,
        }
    }
}

/// Which theorems to check as the run goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Checks {
    /// Assert `thm-once` and the bounds on memory and size. Needs `keep_log`.
    pub invariants: bool,
    /// Assert `thm-exact`: right in a vacuum, where client and server run the same exact map.
    /// Where there are bodies, the error is measured instead (see [`Report`]).
    pub exact: bool,
}

pub struct Scenario {
    pub seed: u64,
    pub world: World,
    pub protocol: Config,
    /// How many ticks the server runs.
    pub ticks: Tick,
    pub peers: Vec<Peer>,
    pub checks: Checks,
}

/// What a run measured.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Report {
    pub ticks: Tick,
    pub datagrams_up: u64,
    pub datagrams_down: u64,
    pub lost: u64,
    pub absorbed: u64,
    /// Predictions compared with the log and found exact.
    pub exact_checks: u64,
    pub max_buffered: usize,
    pub max_snapshot_bytes: usize,
    /// Per peer, `(time, e)` at each snapshot absorbed: the margin error in seconds.
    pub margin_errors: Vec<Vec<(f64, f64)>>,
    /// Over the predictions compared where they are not exact: the largest error in position and
    /// in velocity.
    pub max_error: (f64, f64),
}

enum What {
    ServerTick,
    ServerStream(ConnId, Vec<u8>),
    ServerDatagram(ConnId, Vec<u8>),
    ClientStream(usize, Vec<u8>),
    ClientDatagram(usize, Vec<u8>),
    Start(usize),
    Leave(usize),
    Silence(usize),
    Resume(usize),
    Poll(usize, u64),
    Act(usize),
}

/// An event, ordered by when it happens, then by what it is (deliveries before the tick that
/// would miss them by nothing, then ticks, then clients' own timers), then by whose it is, then
/// by the order its owner made it in. No global counter: another peer's activity cannot reorder
/// this peer's events.
struct Event {
    key: (u64, u8, u32, u64),
    what: What,
}

impl PartialEq for Event {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}

impl Eq for Event {}

impl PartialOrd for Event {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Event {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.key.cmp(&other.key)
    }
}

const DELIVERY: u8 = 0;
const TICK: u8 = 1;
const TIMER: u8 = 2;

struct PeerState {
    spec: Peer,
    client: Option<ClientMachine>,
    up_rng: Rng,
    down_rng: Rng,
    stream_rng: Rng,
    hostile_rng: Rng,
    /// When the last frame sent on each stream arrives, so that frames keep their order.
    up_stream_last: f64,
    down_stream_last: f64,
    poll_generation: u64,
    /// The local time the pending poll was set for.
    poll_due: f64,
    alive: bool,
    silent: bool,
    /// What an honest peer issued, by stamp, for the checks.
    issued: BTreeMap<Tick, Command>,
    last_base: Option<Tick>,
    seq: u64,
}

pub struct Sim {
    seed: u64,
    world: World,
    protocol: Config,
    ticks: Tick,
    checks: Checks,
    server: ServerMachine,
    peers: Vec<PeerState>,
    queue: BinaryHeap<Reverse<Event>>,
    now: f64,
    server_seq: u64,
    report: Report,
    /// What each ship has been applied, as `thm-once` says it never changes.
    shadow: BTreeMap<ShipId, Vec<Command>>,
    honest_ships: BTreeMap<ShipId, usize>,
}

/// Real time of tick 0: the server is already running when the first peer connects.
const T0: f64 = 0.0;

impl Sim {
    /// # Panics
    /// If the scenario's world or config does not make a server.
    pub fn new(scenario: Scenario) -> Self {
        let Scenario {
            seed,
            world,
            mut protocol,
            ticks,
            peers,
            checks,
        } = scenario;
        protocol.keep_log |= checks.invariants || checks.exact;
        let server = ServerMachine::new(world.clone(), protocol, T0).expect("a valid scenario");
        let report = Report {
            margin_errors: vec![Vec::new(); peers.len()],
            ..Report::default()
        };
        let peers = peers
            .into_iter()
            .enumerate()
            .map(|(i, spec)| {
                let stream = |n: u64| Rng::fork(seed, (i as u64) * 8 + n);
                let client = (spec.behavior == Behavior::Honest)
                    .then(|| ClientMachine::new(protocol, spec.request, name_of(i), "sim"));
                PeerState {
                    spec,
                    client,
                    up_rng: stream(0),
                    down_rng: stream(1),
                    stream_rng: stream(2),
                    hostile_rng: stream(3),
                    up_stream_last: 0.0,
                    down_stream_last: 0.0,
                    poll_generation: 0,
                    poll_due: 0.0,
                    alive: true,
                    silent: false,
                    issued: BTreeMap::new(),
                    last_base: None,
                    seq: 0,
                }
            })
            .collect();
        Self {
            seed,
            world,
            protocol,
            ticks,
            checks,
            server,
            peers,
            queue: BinaryHeap::new(),
            now: 0.0,
            server_seq: 0,
            report,
            shadow: BTreeMap::new(),
            honest_ships: BTreeMap::new(),
        }
    }

    pub fn server(&self) -> &ServerMachine {
        &self.server
    }

    pub fn client(&self, peer: usize) -> Option<&ClientMachine> {
        self.peers.get(peer)?.client.as_ref()
    }

    pub fn report(&self) -> &Report {
        &self.report
    }

    fn schedule(&mut self, time: f64, priority: u8, owner: usize, what: What) {
        let seq = if owner == self.peers.len() {
            &mut self.server_seq
        } else {
            &mut self.peers[owner].seq
        };
        *seq += 1;
        let key = (time.max(0.0).to_bits(), priority, owner as u32, *seq);
        self.queue.push(Reverse(Event { key, what }));
    }

    /// Play the scenario out: until the server has run its ticks.
    pub fn run(&mut self) -> &Report {
        let server = self.peers.len();
        self.schedule(T0, TICK, server, What::ServerTick);
        for i in 0..self.peers.len() {
            let spec = self.peers[i].spec.clone();
            self.schedule(spec.join_at, DELIVERY, i, What::Start(i));
            if let Some(t) = spec.leave_at {
                self.schedule(t, DELIVERY, i, What::Leave(i));
            }
            if let Some(t) = spec.silent_at {
                self.schedule(t, DELIVERY, i, What::Silence(i));
            }
            if let Some(t) = spec.resume_at {
                self.schedule(t, DELIVERY, i, What::Resume(i));
            }
        }
        while let Some(Reverse(event)) = self.queue.pop() {
            self.now = f64::from_bits(event.key.0);
            self.handle(event.what);
            if self.server.tick_number() >= self.ticks {
                break;
            }
        }
        self.finish();
        &self.report
    }

    fn handle(&mut self, what: What) {
        match what {
            What::ServerTick => {
                let outgoing = self.server.tick(self.now);
                self.after_tick(&outgoing);
                self.dispatch(outgoing);
                let next = self.server.tick_number();
                if next < self.ticks {
                    let due = self.server.due(next);
                    let owner = self.peers.len();
                    self.schedule(due, TICK, owner, What::ServerTick);
                }
            }
            What::ServerStream(conn, bytes) => {
                let outgoing = self.server.on_stream(conn, &bytes, self.now);
                self.dispatch(outgoing);
            }
            What::ServerDatagram(conn, bytes) => {
                self.server.on_datagram(conn, &bytes, self.now);
            }
            What::ClientStream(i, bytes) => {
                let local = self.local(i, self.now);
                if let Some(client) = &mut self.peers[i].client {
                    let kinds = client.on_stream(&bytes, local);
                    self.send_up(i, kinds);
                    self.reschedule_poll(i);
                }
            }
            What::ClientDatagram(i, bytes) => self.client_datagram(i, &bytes),
            What::Start(i) => self.start(i),
            What::Leave(i) => {
                if let Some(client) = &mut self.peers[i].client {
                    let kinds = client.leave();
                    self.send_up(i, kinds);
                }
            }
            What::Silence(i) => self.peers[i].silent = true,
            What::Resume(i) => {
                self.peers[i].silent = false;
                self.reschedule_poll(i);
            }
            What::Poll(i, generation) => self.poll(i, generation),
            What::Act(i) => self.act(i),
        }
    }

    fn local(&self, peer: usize, real: f64) -> f64 {
        let spec = &self.peers[peer].spec;
        spec.offset + (1.0 + spec.drift) * real
    }

    fn real(&self, peer: usize, local: f64) -> f64 {
        let spec = &self.peers[peer].spec;
        (local - spec.offset) / (1.0 + spec.drift)
    }

    fn start(&mut self, i: usize) {
        self.server.connect(ConnId(i as u32), self.now);
        let hostile = match self.peers[i].spec.behavior {
            Behavior::Honest => None,
            Behavior::Hostile(kind) => Some(kind),
        };
        match hostile {
            None => {
                if let Some(client) = &mut self.peers[i].client {
                    let kinds = client.start();
                    self.send_up(i, kinds);
                }
            }
            Some(kind) => {
                // Hostile peers that fly first say hello and join, politely.
                if !matches!(kind, Hostile::Garbage | Hostile::Churn) {
                    let request = self.peers[i].spec.request;
                    let hello = ClientStream::Hello {
                        protocol: crate::wire::PROTOCOL,
                        build: "hostile".to_owned(),
                    };
                    let frames = vec![
                        Kind::Stream(encode(&hello)),
                        Kind::Stream(encode(&ClientStream::Join {
                            request,
                            username: name_of(i),
                        })),
                    ];
                    self.send_up(i, frames);
                }
                self.schedule(self.now, TIMER, i, What::Act(i));
            }
        }
    }

    /// An honest client's turn: issue what is due, send it, and set the next alarm.
    fn poll(&mut self, i: usize, generation: u64) {
        let peer = &self.peers[i];
        if generation != peer.poll_generation || peer.silent || !peer.alive {
            return;
        }
        // Float rounding between the two clocks must not make an alarm fire a hair early.
        let local = self.local(i, self.now).max(peer.poll_due);
        let seed = self.seed;
        let peer = &mut self.peers[i];
        let Some(client) = &mut peer.client else {
            return;
        };
        let issued = &mut peer.issued;
        let kinds = client.poll(local, &mut |stamp, _predicted| {
            let command = honest_pilot(seed, i, stamp);
            issued.insert(stamp, command);
            command
        });
        // Only the checks look back, and not far.
        if let Some(&newest) = issued.keys().next_back() {
            *issued = issued.split_off(&newest.saturating_sub(512));
        }
        self.send_up(i, kinds);
        self.reschedule_poll(i);
    }

    fn reschedule_poll(&mut self, i: usize) {
        let Some(deadline) = self.peers[i]
            .client
            .as_ref()
            .and_then(ClientMachine::next_deadline)
        else {
            return;
        };
        let at = self.real(i, deadline).max(self.now);
        self.peers[i].poll_generation += 1;
        self.peers[i].poll_due = deadline;
        let generation = self.peers[i].poll_generation;
        self.schedule(at, TIMER, i, What::Poll(i, generation));
    }

    /// Frames and datagrams from peer `i`, over its up link.
    fn send_up(&mut self, i: usize, kinds: Vec<Kind>) {
        for kind in kinds {
            let conn = ConnId(i as u32);
            match kind {
                Kind::Stream(bytes) => {
                    let peer = &mut self.peers[i];
                    let wire = peer.spec.up.delay_at(self.now)
                        + peer.spec.up.jitter * peer.stream_rng.unit();
                    let arrival = (self.now + wire).max(peer.up_stream_last + 1e-9);
                    peer.up_stream_last = arrival;
                    self.schedule(arrival, DELIVERY, i, What::ServerStream(conn, bytes));
                }
                Kind::Datagram(bytes) => {
                    self.report.datagrams_up += 1;
                    let peer = &mut self.peers[i];
                    let arrivals = peer.spec.up.arrivals(&mut peer.up_rng, self.now);
                    self.report.lost += u64::from(arrivals.is_empty());
                    for at in arrivals {
                        self.schedule(at, DELIVERY, i, What::ServerDatagram(conn, bytes.clone()));
                    }
                }
                Kind::Close => {}
            }
        }
    }

    /// What the server sent, over each peer's down link.
    fn dispatch(&mut self, outgoing: Vec<Outgoing>) {
        for Outgoing { conn, kind } in outgoing {
            let i = conn.0 as usize;
            if !self.peers[i].alive {
                continue;
            }
            match kind {
                Kind::Stream(bytes) => {
                    let peer = &mut self.peers[i];
                    let wire = peer.spec.down.delay_at(self.now)
                        + peer.spec.down.jitter * peer.stream_rng.unit();
                    let arrival = (self.now + wire).max(peer.down_stream_last + 1e-9);
                    peer.down_stream_last = arrival;
                    self.schedule(arrival, DELIVERY, i, What::ClientStream(i, bytes));
                }
                Kind::Datagram(bytes) => {
                    self.report.datagrams_down += 1;
                    let peer = &mut self.peers[i];
                    let arrivals = peer.spec.down.arrivals(&mut peer.down_rng, self.now);
                    self.report.lost += u64::from(arrivals.is_empty());
                    for at in arrivals {
                        self.schedule(at, DELIVERY, i, What::ClientDatagram(i, bytes.clone()));
                    }
                }
                Kind::Close => {
                    self.peers[i].alive = false;
                    let more = self.server.on_disconnect(conn);
                    self.dispatch(more);
                }
            }
        }
    }

    fn client_datagram(&mut self, i: usize, bytes: &[u8]) {
        let local = self.local(i, self.now);
        let Some(client) = &mut self.peers[i].client else {
            return;
        };
        if !client.on_datagram(bytes, local) {
            return;
        }
        self.report.absorbed += 1;
        if let Some(e) = client.margin_error() {
            self.report.margin_errors[i].push((self.now, e));
        }
        self.reschedule_poll(i);
        self.after_absorb(i);
    }

    /// Hostile peers' turns.
    fn act(&mut self, i: usize) {
        let Behavior::Hostile(kind) = self.peers[i].spec.behavior else {
            return;
        };
        if !self.peers[i].alive {
            return;
        }
        let tick = self.server.tick_number();
        let mut frames = Vec::new();
        let rng = &mut self.peers[i].hostile_rng;
        match kind {
            Hostile::Garbage => {
                let len = rng.below(1500) as usize;
                frames.push(Kind::Datagram(
                    (0..len).map(|_| rng.next_u64() as u8).collect(),
                ));
            }
            Hostile::NanCommands => {
                let stamp = tick + rng.below(12) as Tick;
                let cmds = (0..5).map(|_| Some(wild_command(rng))).collect();
                frames.push(Kind::Datagram(encode(&Commands { stamp, cmds })));
            }
            Hostile::StampAbuse => {
                let stamp = match rng.below(6) {
                    0 => Tick::MAX,
                    1 => 0,
                    2 => tick.saturating_add(1_000_000_000),
                    3 => tick.saturating_sub(5),
                    4 => tick + 1000,
                    _ => tick + 2,
                };
                let count = if rng.chance(0.2) { 40 } else { 5 };
                let cmds = (0..count).map(|_| Some(wild_command(rng))).collect();
                frames.push(Kind::Datagram(encode(&Commands {
                    stamp,
                    cmds: Vec::clone(&Vec::new()),
                })));
                frames.push(Kind::Datagram(encode(&Commands { stamp, cmds })));
                // The same stamp twice, with two different commands.
                for _ in 0..2 {
                    frames.push(Kind::Datagram(encode(&Commands {
                        stamp: tick + 3,
                        cmds: vec![Some(wild_command(rng))],
                    })));
                }
            }
            Hostile::Flood => {
                for _ in 0..50 {
                    let stamp = tick + rng.below(8) as Tick;
                    let cmds = vec![Some(honest_pilot(self.seed, i, stamp)); 3];
                    frames.push(Kind::Datagram(encode(&Commands { stamp, cmds })));
                }
            }
            Hostile::Churn => {
                for _ in 0..3 {
                    let request = ShipRequest {
                        mass: rng.wild(),
                        inertia: rng.wild(),
                        f_max: rng.wild(),
                        tau_max: rng.wild(),
                    };
                    let message = match rng.below(4) {
                        0 => ClientStream::Hello {
                            protocol: crate::wire::PROTOCOL,
                            build: String::new(),
                        },
                        1 | 2 => ClientStream::Join {
                            request,
                            username: name_of(i),
                        },
                        _ => ClientStream::Leave,
                    };
                    frames.push(Kind::Stream(encode(&message)));
                }
            }
        }
        self.send_up(i, frames);
        let at = self.now + self.world.h();
        self.schedule(at, TIMER, i, What::Act(i));
    }

    /// After a server tick: what the specification says holds of the server, holds.
    fn after_tick(&mut self, outgoing: &[Outgoing]) {
        let seed = self.seed;
        for out in outgoing {
            if let Kind::Datagram(bytes) = &out.kind {
                self.report.max_snapshot_bytes = self.report.max_snapshot_bytes.max(bytes.len());
                assert!(
                    bytes.len() <= self.protocol.max_datagram,
                    "seed {seed}: a snapshot of {} bytes",
                    bytes.len()
                );
            }
        }
        let tick = self.server.tick_number();
        self.report.ticks = tick;
        for (i, peer) in self.peers.iter().enumerate() {
            if let Some(ship) = self.server.ship_of(ConnId(i as u32)) {
                if peer.spec.behavior == Behavior::Honest {
                    self.honest_ships.insert(ship, i);
                }
                if let Some(buffered) = self.server.buffered(ship) {
                    self.report.max_buffered = self.report.max_buffered.max(buffered);
                    assert!(
                        buffered <= self.protocol.window as usize,
                        "seed {seed}: ship {ship:?} buffers {buffered} commands (thm-once 4)"
                    );
                }
            }
        }
        if !self.checks.invariants {
            return;
        }
        let flying: Vec<ShipId> = (0..self.peers.len())
            .filter_map(|i| self.server.ship_of(ConnId(i as u32)))
            .collect();
        for ship in flying {
            let log = self.server.log(ship).expect("the log is kept");
            // thm-once (1): exactly one command for each tick since the ship joined.
            let ticks_flown = (tick - log.since) as usize;
            assert_eq!(
                log.commands.len(),
                ticks_flown,
                "seed {seed}: one command per tick"
            );
            assert_eq!(
                log.samples.len(),
                ticks_flown + 1,
                "seed {seed}: one state per tick"
            );
            let shadow = self.shadow.entry(ship).or_default();
            if let Some(&applied) = log.commands.last() {
                shadow.push(applied);
                self.check_provenance(ship, tick - 1);
            }
        }
    }

    /// `thm-once` (2) and (3) for the command applied at `tick`, if the ship's pilot is honest:
    /// it is the one issued for the stamp, or the one before it, or that one's neutral form; and
    /// if one had arrived in time, it is exactly the one issued.
    fn check_provenance(&self, ship: ShipId, tick: Tick) {
        let Some(&i) = self.honest_ships.get(&ship) else {
            return;
        };
        let seed = self.seed;
        let log = self.server.log(ship).expect("the log is kept");
        let params = self
            .server
            .params_of(ship)
            .expect("a flying ship has parameters");
        let applied = *log.applied(tick).expect("a tick that ran");
        let previous = tick
            .checked_sub(1)
            .and_then(|t| log.applied(t).copied())
            .unwrap_or_default();
        let issued = self.peers[i].issued.get(&tick).map(|c| c.sanitize(&params));
        let delivered = log.was_delivered(tick).expect("a tick that ran");
        let is_provenance =
            Some(applied) == issued || applied == previous || applied == previous.neutral();
        assert!(
            is_provenance,
            "seed {seed}: tick {tick} applied {applied:?}, not from {issued:?} or {previous:?}"
        );
        if delivered {
            assert_eq!(
                Some(applied),
                issued,
                "seed {seed}: tick {tick} (thm-once 3)"
            );
        }
    }

    /// After a client absorbs a snapshot: the base is true (`thm-once` 5), and the prediction
    /// from it is exact where the commands it replays are the ones that were applied (`thm-exact`).
    fn after_absorb(&mut self, i: usize) {
        if !self.checks.invariants && !self.checks.exact {
            return;
        }
        let seed = self.seed;
        let peer = &mut self.peers[i];
        let Some(client) = peer.client.as_mut() else {
            return;
        };
        let (Some(ship), Some(base), Some(k), Some(params)) =
            (client.ship(), client.base(), client.tick(), client.params())
        else {
            return;
        };
        let log = self.server.log(ship).expect("the log is kept");
        assert_eq!(
            log.at(base.tick).map(|s| (s.state, s.hold)),
            Some((base.state, base.hold)),
            "seed {seed}: the base at {} is not the server's (thm-once 5)",
            base.tick
        );
        // The Advance rule: the client's tick never trails the world's.
        assert!(
            base.tick <= k,
            "seed {seed}: the base {} is past the client's tick {k} (Lead)",
            base.tick
        );
        if let Some(last) = peer.last_base {
            assert!(
                base.tick > last,
                "seed {seed}: the base moved back (thm-once 5)"
            );
        }
        peer.last_base = Some(base.tick);
        // The longest stretch from the base on over which the replay used what was applied.
        let have = log.since + u32::try_from(log.commands.len()).expect("fewer ticks than u32");
        let end = k.min(have);
        let mut agree = base.tick;
        while agree < end {
            let applied = *log.applied(agree).expect("within the log");
            let replayed = client.replayed(agree).expect("playing").sanitize(&params);
            if applied != replayed {
                break;
            }
            agree += 1;
        }
        let mut worst = (0.0_f64, 0.0_f64);
        for n in base.tick..=agree {
            let predicted: State = client.predict(n).expect("not before the base");
            let truth = log.at(n).expect("within the log").state;
            if self.checks.exact {
                assert_eq!(
                    predicted, truth,
                    "seed {seed}: the prediction at {n} from {} (thm-exact)",
                    base.tick
                );
                self.report.exact_checks += 1;
            } else {
                worst.0 = worst
                    .0
                    .max((predicted.x - truth.x).hypot(predicted.y - truth.y));
                worst.1 = worst
                    .1
                    .max((predicted.vx - truth.vx).hypot(predicted.vy - truth.vy));
            }
        }
        self.report.max_error.0 = self.report.max_error.0.max(worst.0);
        self.report.max_error.1 = self.report.max_error.1.max(worst.1);
    }

    /// At the end: the logs are what they were when they were written.
    fn finish(&self) {
        if !self.checks.invariants {
            return;
        }
        for (&ship, shadow) in &self.shadow {
            let log = self.server.log(ship).expect("the log is kept");
            assert_eq!(
                &log.commands[..shadow.len()],
                shadow.as_slice(),
                "seed {}: the log of {ship:?} changed (thm-once 1)",
                self.seed
            );
        }
    }
}

/// What an honest pilot holds at a stamp.
///
/// A function of the stamp alone, so that the one command a client issues for a stamp is the same
/// however often it is asked. It changes every 24 ticks, between letting go, keys, fine control, a
/// pointer and a control of its own.
pub fn honest_pilot(seed: u64, peer: usize, stamp: Tick) -> Command {
    let mut rng = Rng::fork(seed ^ 0x0D17, ((peer as u64) << 32) | u64::from(stamp / 24));
    let steering = if rng.chance(0.5) {
        Steering::Sas
    } else {
        Steering::Raw
    };
    match rng.below(10) {
        0..=3 => Command::Manual {
            input: Input::default(),
            steering,
        },
        4..=6 => Command::Manual {
            input: Input {
                thrust: 1.0,
                turn: [-1.0, 0.0, 1.0][rng.below(3) as usize],
                fine: rng.chance(0.2),
                aim: None,
            },
            steering,
        },
        7 => Command::Manual {
            input: Input {
                thrust: rng.unit(),
                turn: rng.range(-1.0, 1.0),
                fine: true,
                aim: None,
            },
            steering,
        },
        8 => Command::Manual {
            input: Input {
                aim: Some(Aim {
                    heading: rng.range(-std::f64::consts::PI, std::f64::consts::PI),
                    cone: 0.4,
                }),
                ..Input::default()
            },
            steering,
        },
        _ => Command::Direct {
            thrust: rng.range(0.0, 5.0),
            torque: rng.range(-5.0, 5.0),
        },
    }
}

/// A command from a client that has made it of anything.
pub fn wild_command(rng: &mut Rng) -> Command {
    if rng.chance(0.3) {
        return Command::Direct {
            thrust: rng.wild(),
            torque: rng.wild(),
        };
    }
    let aim = rng.chance(0.5).then(|| Aim {
        heading: rng.wild(),
        cone: rng.wild(),
    });
    Command::Manual {
        input: Input {
            thrust: rng.wild(),
            turn: rng.wild(),
            fine: rng.chance(0.5),
            aim,
        },
        steering: if rng.chance(0.5) {
            Steering::Sas
        } else {
            Steering::Raw
        },
    }
}
