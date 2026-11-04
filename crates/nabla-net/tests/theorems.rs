//! The theorems of `docs/multiplayer.typ` as assertions on a simulated network: `thm-once`,
//! `thm-contain`, `thm-exact` and `thm-margin`, and what holds against hostile clients.

use nabla_net::Config;
use nabla_net::sim::{Behavior, Checks, Hostile, Link, Peer, Report, Scenario, Sim};
use nabla_net::wire::{
    ClientStream, Commands, ConnId, ServerStream, ShipId, Username, decode, encode,
};
use nabla_net::{Kind, ServerMachine};
use nabla_sim::command::Command;
use nabla_sim::rng::Rng;
use nabla_sim::world::{Preset, ShipRequest, World};

const ALL: Checks = Checks {
    invariants: true,
    exact: true,
};

fn vacuum() -> World {
    Preset::Vacuum
        .world()
        .multiplayer(60, 1000.0, 4)
        .expect("a valid world")
}

fn scenario(seed: u64, world: World, ticks: u32, peers: Vec<Peer>, checks: Checks) -> Scenario {
    Scenario {
        seed,
        world,
        protocol: Config::default(),
        ticks,
        peers,
        checks,
    }
}

fn run(scenario: Scenario) -> (Sim, Report) {
    let mut sim = Sim::new(scenario);
    let report = sim.run().clone();
    (sim, report)
}

#[test]
fn one_pilot_on_a_clean_link_is_predicted_exactly() {
    let world = vacuum();
    let peers = vec![Peer::honest(&world, 0.05)];
    let (sim, report) = run(scenario(1, world, 1200, peers, ALL));
    assert_eq!(report.ticks, 1200);
    // Exact prediction means no snapshot ever corrects it, to the last bit.
    let stats = sim
        .client(0)
        .expect("honest")
        .correction_stats()
        .expect("playing");
    assert!(stats.snapshots > 1000, "{stats:?}");
    assert_eq!(stats.nonzero, 0, "{stats:?}");
    assert!(report.absorbed > 1000, "{}", report.absorbed);
    assert!(report.exact_checks > 3000, "{}", report.exact_checks);
    assert!(report.max_buffered <= 60);
    let errors = &report.margin_errors[0];
    let (first, last) = (
        errors.first().expect("reports"),
        errors.last().expect("reports"),
    );
    assert!(
        first.1.abs() < 0.05 && last.1.abs() < 0.002,
        "{first:?} {last:?}"
    );
}

/// The error `e = mu - mu*` of the margin after time `from`, in milliseconds, at each report.
fn errors_ms(report: &Report, from: f64) -> Vec<(f64, f64)> {
    report.margin_errors[0]
        .iter()
        .filter(|&&(t, _)| t >= from)
        .map(|&(t, e)| (t, e * 1e3))
        .collect()
}

/// An honest peer 100 ms from the server, 50 ms each way, as in the paper's example.
fn peer_at_100_ms(world: &World) -> Peer {
    Peer::honest(world, 0.05)
}

#[test]
fn the_margin_settles_to_its_target() {
    let world = vacuum();
    let (_, report) = run(scenario(
        2,
        world.clone(),
        1800,
        vec![peer_at_100_ms(&world)],
        ALL,
    ));
    let e = errors_ms(&report, 0.0);
    assert!(
        e[0].1.abs() < 20.0,
        "it starts within a tick or so: {:?}",
        e[0]
    );
    let settled = errors_ms(&report, 2.5);
    assert!(
        settled.iter().all(|&(_, e)| e.abs() < 0.1),
        "settled by 2.5 s"
    );
}

#[test]
fn after_a_change_in_the_path_the_margin_returns_without_overshooting() {
    // The theorem on margin control: at RTT = 100 ms, n_f = 6, an error decays to 5% in about
    // 0.7 s without undershoot. Here the path changes at 5 s, and the error is first the size of
    // the change. Decay is a little slower than the linear law's, where the 5% clamp on a tick's
    // stretch binds.
    for extra in [0.02, -0.02, 0.1] {
        let world = vacuum();
        let mut peer = peer_at_100_ms(&world);
        peer.up.steps.push((5.0, extra));
        let (_, report) = run(scenario(3, world, 2400, vec![peer], ALL));
        let after = errors_ms(&report, 5.0);
        let worst = after.iter().map(|&(_, e)| e.abs()).fold(0.0, f64::max);
        assert!(
            worst >= 0.9 * extra.abs() * 1e3 * 0.4,
            "the change is felt: {worst} for {extra}"
        );
        // No undershoot: once the error is past its peak, it never crosses zero.
        let peak = after
            .iter()
            .position(|&(_, e)| e.abs() >= worst - 1e-9)
            .expect("a peak");
        let sign = after[peak].1.signum();
        assert!(
            after[peak..].iter().all(|&(_, e)| e * sign >= -0.01),
            "no undershoot for {extra}"
        );
        // And it decays to 5% of the peak, and then to nothing.
        let (t_peak, _) = after[peak];
        let t_5pct = after[peak..]
            .iter()
            .find(|&&(_, e)| e.abs() <= 0.05 * worst)
            .expect("decays to 5%")
            .0;
        let allowed = if extra.abs() > 0.05 { 6.0 } else { 1.5 };
        assert!(
            t_5pct - t_peak < allowed,
            "{} s to 5% for {extra}",
            t_5pct - t_peak
        );
        let end = after.last().expect("reports").1;
        assert!(end.abs() < 0.1, "settled: {end} ms for {extra}");
    }
}

#[test]
fn a_drifting_clock_leaves_the_steady_error_the_theorem_predicts() {
    // A constant drift w = zeta h leaves the steady error zeta h / gamma, gamma = 1 / (3 (n_f + 1)),
    // here n_f = 6.
    let world = vacuum();
    let zeta = 1e-3;
    let mut peer = peer_at_100_ms(&world);
    peer.drift = zeta;
    let (_, report) = run(scenario(4, world.clone(), 2400, vec![peer], ALL));
    let predicted = zeta * world.h() * 3.0 * (6.0 + 1.0) * 1e3;
    let steady = errors_ms(&report, 10.0);
    for &(_, e) in &steady {
        assert!(
            (e - predicted).abs() < 0.05 * predicted,
            "{e} ms against {predicted} ms"
        );
    }
}

/// A scenario of a few honest peers on bad paths, from a seed: a mix of delays, jitter, loss, bursts,
/// duplication, drifting clocks, ships asked for in any numbers, and pilots who come and go.
fn random_scenario(seed: u64, world: &World, ticks: u32, checks: Checks) -> Scenario {
    let mut rng = Rng::new(seed);
    let seconds = f64::from(ticks) * world.h();
    let link = |rng: &mut Rng| Link {
        delay: rng.range(0.005, 0.15),
        jitter: rng.range(0.0, 0.04),
        loss: rng.range(0.0, 0.2),
        duplicate: rng.range(0.0, 0.1),
        outages: if rng.chance(0.3) {
            let start = rng.range(1.0, seconds - 1.0);
            vec![(start, start + 0.08)]
        } else {
            Vec::new()
        },
        steps: Vec::new(),
    };
    let peers = (0..1 + rng.below(4) as usize)
        .map(|i| {
            let request = if rng.chance(0.5) {
                ShipRequest {
                    mass: rng.wild(),
                    inertia: rng.wild(),
                    f_max: rng.wild(),
                    tau_max: rng.wild(),
                }
            } else {
                ShipRequest::from(world.default_ship())
            };
            Peer {
                behavior: Behavior::Honest,
                request,
                join_at: if i == 0 {
                    0.0
                } else {
                    rng.range(0.0, seconds / 2.0)
                },
                leave_at: rng.chance(0.2).then(|| rng.range(seconds / 2.0, seconds)),
                silent_at: None,
                resume_at: None,
                up: link(&mut rng),
                down: link(&mut rng),
                drift: rng.range(-1e-4, 1e-4),
                offset: rng.range(-1000.0, 1000.0),
            }
        })
        .collect();
    scenario(seed, world.clone(), ticks, peers, checks)
}

/// Run `body` for each seed in `seeds`, on every core, and collect the reports in order.
fn over_seeds(seeds: std::ops::Range<u64>, body: impl Fn(u64) -> Report + Sync) -> Vec<Report> {
    let seeds: Vec<u64> = seeds.collect();
    let threads = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    let chunk = seeds.len().div_ceil(threads).max(1);
    std::thread::scope(|scope| {
        let handles: Vec<_> = seeds
            .chunks(chunk)
            .map(|part| scope.spawn(|| part.iter().map(|&seed| body(seed)).collect::<Vec<_>>()))
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().expect("no run panicked"))
            .collect()
    })
}

#[test]
fn a_million_ticks_of_seeds_keep_every_invariant_and_every_prediction_exact() {
    // thm-once and thm-exact, as assertions inside the simulator, over 10^6 ticks of seeds, on
    // paths that lose, reorder and duplicate, with clients whose clocks drift and who ask for
    // ships in any numbers.
    let world = vacuum();
    let ticks = 5_000;
    let reports = over_seeds(0..200, |seed| {
        let mut sim = Sim::new(random_scenario(seed, &world, ticks, ALL));
        sim.run().clone()
    });
    let total: u64 = reports.iter().map(|r| u64::from(r.ticks)).sum();
    assert!(total >= 1_000_000, "{total} ticks");
    let exact: u64 = reports.iter().map(|r| r.exact_checks).sum();
    let lost: u64 = reports.iter().map(|r| r.lost).sum();
    let sent: u64 = reports
        .iter()
        .map(|r| r.datagrams_up + r.datagrams_down)
        .sum();
    assert!(exact > 1_000_000, "{exact} predictions found exact");
    assert!(
        lost * 20 > sent,
        "the paths were bad: {lost} of {sent} lost"
    );
    assert!(reports.iter().all(|r| r.max_buffered <= 60));
}

/// Three honest peers, the first of which is the one whose ship we watch.
fn honest_three(world: &World) -> Vec<Peer> {
    let bad = |delay: f64| Peer {
        up: Link::bad(delay),
        down: Link::bad(delay),
        ..Peer::honest(world, delay)
    };
    let mut peers = vec![bad(0.03), bad(0.06), bad(0.02)];
    peers[1].join_at = 0.2;
    peers[2].join_at = 0.4;
    peers
}

/// The same first peer, with the others replaced by clients that send anything.
fn hostile_five(world: &World) -> Vec<Peer> {
    let mut peers = vec![honest_three(world).remove(0)];
    for (n, kind) in [
        Hostile::Flood,
        Hostile::NanCommands,
        Hostile::StampAbuse,
        Hostile::Churn,
        Hostile::Garbage,
    ]
    .into_iter()
    .enumerate()
    {
        let mut peer = Peer::honest(world, 0.04);
        peer.behavior = Behavior::Hostile(kind);
        peer.join_at = 0.2 + 0.1 * n as f64;
        peers.push(peer);
    }
    peers
}

#[test]
fn a_ship_depends_on_nothing_but_its_own_connection() {
    // thm-contain: ship 0's trajectory is a function of its initial state, parameters, and the
    // commands delivered on its connection, with when. Whatever the other clients send, and
    // whenever they come and go, it is unchanged to the last bit.
    for preset in [Preset::Vacuum, Preset::PlanetAndMoon] {
        let world = preset
            .world()
            .multiplayer(60, 1000.0, 8)
            .expect("a valid world");
        for seed in 0..6 {
            let quiet = honest_three(&world);
            let (calm, _) = run(scenario(seed, world.clone(), 2400, quiet, ALL_NO_EXACT));
            let (stormy, _) = run(scenario(
                seed,
                world.clone(),
                2400,
                hostile_five(&world),
                ALL_NO_EXACT,
            ));
            let (a, b) = (
                calm.server()
                    .log(ShipId(0))
                    .expect("ship 0 in the calm run"),
                stormy
                    .server()
                    .log(ShipId(0))
                    .expect("ship 0 in the stormy run"),
            );
            assert_eq!(a.since, b.since, "{preset:?} seed {seed}: the same join");
            assert_eq!(a.samples.len(), (2400 - a.since + 1) as usize);
            assert_eq!(a.samples, b.samples, "{preset:?} seed {seed}");
            assert_eq!(a.commands, b.commands, "{preset:?} seed {seed}");
        }
    }
}

/// The checks that hold in any world: the invariants, but exactness only where the map is exact.
const ALL_NO_EXACT: Checks = Checks {
    invariants: true,
    exact: false,
};

#[test]
fn hostile_clients_change_nothing_for_an_honest_one_and_break_nothing() {
    let world = vacuum();
    for seed in 0..10 {
        let (sim, report) = run(scenario(
            seed,
            world.clone(),
            3000,
            hostile_five(&world),
            ALL,
        ));
        assert_eq!(report.ticks, 3000, "the world never stopped");
        assert!(
            report.exact_checks > 1000,
            "the honest pilot was predicted exactly: {}",
            report.exact_checks
        );
        assert!(report.max_snapshot_bytes <= 1150);
        for conn in 0..6 {
            if let Some(ship) = sim.server().ship_of(ConnId(conn)) {
                let s = sim.server().state_of(ship).expect("flying");
                let finite = [s.x, s.y, s.vx, s.vy, s.theta, s.omega]
                    .iter()
                    .all(|v| v.is_finite());
                assert!(finite, "seed {seed}: ship {ship:?} is {s:?}");
                assert!(
                    s.x.abs() <= 999.0 && s.y.abs() <= 999.0,
                    "inside the arena: {s:?}"
                );
            }
        }
    }
}

#[test]
fn among_bodies_the_invariants_hold_and_prediction_errors_are_small() {
    // Where there are bodies the client's replay is Rapier restarted from a snapshot, which is
    // not exact. The invariants are still assertions, and the error is measured.
    let world = Preset::PlanetAndMoon
        .world()
        .multiplayer(60, 1000.0, 4)
        .expect("a valid world");
    let reports = over_seeds(0..24, |seed| {
        let mut sim = Sim::new(random_scenario(seed, &world, 3000, ALL_NO_EXACT));
        sim.run().clone()
    });
    let worst = reports.iter().fold((0.0_f64, 0.0_f64), |acc, r| {
        (acc.0.max(r.max_error.0), acc.1.max(r.max_error.1))
    });
    assert!(
        worst.0 < 0.01 && worst.1 < 0.1,
        "prediction errors among bodies: {worst:?}"
    );
}

fn server(world: &World, config: Config) -> ServerMachine {
    ServerMachine::new(world.clone(), config, 0.0).expect("a valid server")
}

fn streams(out: Vec<nabla_net::Outgoing>) -> Vec<ServerStream> {
    out.into_iter()
        .filter_map(|o| match o.kind {
            Kind::Stream(bytes) => decode(&bytes, 1 << 21).ok(),
            _ => None,
        })
        .collect()
}

fn hello() -> Vec<u8> {
    encode(&ClientStream::Hello {
        protocol: nabla_net::wire::PROTOCOL,
        build: "t".to_owned(),
    })
}

fn join(world: &World) -> Vec<u8> {
    encode(&ClientStream::Join {
        request: ShipRequest::from(world.default_ship()),
        username: Username::parse("t").expect("a valid name"),
    })
}

#[test]
fn the_server_answers_each_message_only_in_its_turn() {
    let world = vacuum();
    let mut server = server(&world, Config::default());
    let conn = ConnId(7);
    server.connect(conn, 0.0);
    assert!(
        streams(server.on_stream(conn, &join(&world), 0.0)).is_empty(),
        "no join before hello"
    );
    let lobby = streams(server.on_stream(conn, &hello(), 0.0));
    assert!(matches!(lobby.as_slice(), [ServerStream::Lobby(l)] if l.max_pilots == 16));
    assert!(
        streams(server.on_stream(conn, &hello(), 0.0)).is_empty(),
        "no second hello"
    );
    let welcome = streams(server.on_stream(conn, &join(&world), 0.0));
    assert!(matches!(
        welcome.as_slice(),
        [ServerStream::Welcome(_), ServerStream::Roster(_)]
    ));
    assert!(
        streams(server.on_stream(conn, &join(&world), 0.0)).is_empty(),
        "no second ship"
    );
    assert_eq!(server.pilots(), 1);
    // A wrong version is turned away, and nonsense on a stream ends the connection.
    server.connect(ConnId(8), 0.0);
    let wrong = encode(&ClientStream::Hello {
        protocol: 999,
        build: String::new(),
    });
    let out = server.on_stream(ConnId(8), &wrong, 0.0);
    assert!(out.iter().any(|o| o.kind == Kind::Close));
    server.connect(ConnId(9), 0.0);
    let out = server.on_stream(ConnId(9), &[0xFF; 20], 0.0);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].kind, Kind::Close);
}

#[test]
fn a_seventeenth_pilot_is_told_the_server_is_full() {
    let world = Preset::Vacuum
        .world()
        .multiplayer(60, 1000.0, 4)
        .expect("a valid world");
    let mut server = server(&world, Config::default());
    for i in 0..17 {
        server.connect(ConnId(i), 0.0);
        server.on_stream(ConnId(i), &hello(), 0.0);
        let out = streams(server.on_stream(ConnId(i), &join(&world), 0.0));
        if i < 16 {
            assert!(
                matches!(out.first(), Some(ServerStream::Welcome(_))),
                "pilot {i}"
            );
        } else {
            assert_eq!(out, [ServerStream::Full]);
        }
    }
    assert_eq!(server.pilots(), 16);
    // A place that frees up can be taken.
    server.on_disconnect(ConnId(3));
    server.on_stream(ConnId(16), &join(&world), 0.0);
    assert_eq!(server.pilots(), 16);
    assert!(server.ship_of(ConnId(16)).is_some());
}

#[test]
fn a_missing_command_is_held_and_then_goes_neutral() {
    // The hold rule, with N_d = 5: the last command is repeated until five ticks in a row have
    // had none, and then hands go off.
    let world = vacuum();
    let config = Config {
        silence_ticks: 5,
        window: 10,
        keep_log: true,
        ..Config::default()
    };
    let mut server = server(&world, config);
    let conn = ConnId(0);
    server.connect(conn, 0.0);
    server.on_stream(conn, &hello(), 0.0);
    server.on_stream(conn, &join(&world), 0.0);
    let ship = server.ship_of(conn).expect("flying");
    let thrust = Command::Direct {
        thrust: 2.0,
        torque: 0.5,
    };
    let send = |server: &mut ServerMachine, stamp: u32| {
        let bytes = encode(&Commands {
            stamp,
            cmds: vec![Some(thrust)],
        });
        server.on_datagram(conn, &bytes, 0.0);
    };
    // The window is [T, T + N_b): 9 is in, 10 is out.
    send(&mut server, 10);
    assert_eq!(server.buffered(ship), Some(0));
    send(&mut server, 9);
    assert_eq!(server.buffered(ship), Some(1));
    // A second command for a stamp does not replace the first.
    let other = Command::Direct {
        thrust: 1.0,
        torque: 0.0,
    };
    server.on_datagram(
        conn,
        &encode(&Commands {
            stamp: 1,
            cmds: vec![Some(thrust)],
        }),
        0.0,
    );
    server.on_datagram(
        conn,
        &encode(&Commands {
            stamp: 1,
            cmds: vec![Some(other)],
        }),
        0.0,
    );
    for t in 0..9 {
        server.tick(f64::from(t) * world.h());
    }
    // A stamp below T is discarded.
    server.on_datagram(
        conn,
        &encode(&Commands {
            stamp: 2,
            cmds: vec![Some(other)],
        }),
        0.0,
    );
    assert_eq!(server.buffered(ship), Some(1), "only stamp 9 is waiting");
    let log = server.log(ship).expect("log");
    let neutral = Command::default();
    let off = Command::Direct {
        thrust: 0.0,
        torque: 0.0,
    };
    let applied: Vec<Command> = (0..9).map(|t| *log.applied(t).expect("ran")).collect();
    // Nothing before stamp 1, the thrust at 1 and held through 5, and then neutral from 6.
    assert_eq!(applied[0], neutral);
    assert!(applied[1..=5].iter().all(|c| *c == thrust), "{applied:?}");
    assert_eq!(applied[6], off, "N_d ticks of silence: hands off");
    assert_eq!(applied[7], off);
    assert_eq!(applied[8], off.neutral());
    assert_eq!(log.was_delivered(1), Some(true));
    assert_eq!(log.was_delivered(2), Some(false));
}

#[test]
fn a_ship_that_falls_silent_leaves_and_so_does_one_that_asks() {
    let world = vacuum();
    let mut quiet = Peer::honest(&world, 0.03);
    quiet.silent_at = Some(2.0);
    let mut polite = Peer::honest(&world, 0.03);
    polite.leave_at = Some(4.0);
    let mut stays = Peer::honest(&world, 0.03);
    stays.join_at = 0.1;
    let config = Config {
        idle: 3.0,
        ..Config::default()
    };
    let mut sc = scenario(5, world, 900, vec![quiet, polite, stays], ALL);
    sc.protocol = config;
    let (sim, _) = run(sc);
    assert_eq!(sim.server().pilots(), 1, "only the one who stayed");
    assert!(sim.server().ship_of(ConnId(2)).is_some());
    assert!(
        !sim.client(0).expect("honest").is_playing(),
        "the idle pilot was told"
    );
    assert!(!sim.client(1).expect("honest").is_playing());
    assert!(sim.client(2).expect("honest").is_playing());
    let roster = sim.client(2).expect("honest").roster();
    assert_eq!(roster.len(), 1, "the roster shows who is left: {roster:?}");
}

#[test]
fn a_hidden_tab_coasts_and_then_catches_up_without_breaking_anything() {
    // The tab stops running frames at 3 s and wakes at 5 s. Its ship goes neutral after N_d
    // ticks and coasts. The snapshots go on arriving, so the base passes the client's tick, and
    // the Advance rule pulls the tick along. On waking, the client skips the ticks it missed, to
    // keep its lead, and the margin settles again.
    let world = vacuum();
    let mut peer = Peer::honest(&world, 0.05);
    peer.silent_at = Some(3.0);
    peer.resume_at = Some(5.0);
    let (sim, report) = run(scenario(6, world, 2400, vec![peer], ALL));
    assert!(sim.client(0).expect("honest").is_playing());
    let log = sim.server().log(ShipId(0)).expect("log");
    let neutral = |t: u32| *log.applied(t).expect("ran") == log.applied(t).expect("ran").neutral();
    assert!(
        neutral(3 * 60 + 80),
        "the hidden ship coasts (hands off) after N_d ticks"
    );
    let after = errors_ms(&report, 9.0);
    assert!(
        after.iter().all(|&(_, e)| e.abs() < 1.0),
        "the margin settled again"
    );
    assert!(
        report.exact_checks > 3000,
        "predictions stayed exact throughout"
    );
}
