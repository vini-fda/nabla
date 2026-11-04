//! Two pilots fly through a real server, over QUIC and WebTransport on loopback, with a proxy
//! that makes the path 100 ms there and back (M5 of `docs/multiplayer.typ`).

#![cfg(not(target_arch = "wasm32"))]

use std::time::{Duration, Instant};

use nabla_client::net;
use nabla_client::online::{Hands, Online};
use nabla_core::solutions::SOLUTIONS;
use nabla_net::Config;
use nabla_net::client::{SNAP_POSITION, Stage};
use nabla_net::wire::Username;
use nabla_server::impair::{Impairment, Proxy};
use nabla_server::{Identity, Metrics, Server, Settings};
use nabla_sim::steering::{Input, Steering};
use nabla_sim::world::{Preset, ShipRequest};

fn name(text: &str) -> Username {
    Username::parse(text).expect("a valid name")
}

fn hands(who: usize, seconds: f64) -> Hands {
    let phase = ((seconds + who as f64 * 0.7) / 1.3).floor() as i64;
    Hands {
        input: Input {
            thrust: if phase % 3 == 0 { 0.0 } else { 1.0 },
            turn: f64::from(i8::try_from(phase.rem_euclid(3)).expect("small") - 1),
            ..Input::default()
        },
        steering: if who == 0 {
            Steering::Raw
        } else {
            Steering::Sas
        },
        ..Hands::default()
    }
}

/// Fly two clients for `seconds`, pilot 0 with the default ship and pilot 1 with a stronger one,
/// and pilot 1's autopilot, if `autopilot_after` says when to engage it (from the start of the
/// flight), flying until it lands or the time is up.
fn two_pilots(
    preset: Preset,
    seconds: f64,
    autopilot_after: Option<f64>,
) -> ([Online; 2], bool, Metrics) {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("a runtime");
    let world = preset.world().multiplayer(60, 1000.0, 16).expect("a world");
    let ours = ShipRequest::from(world.default_ship());
    let stronger = ShipRequest {
        mass: 2.0,
        inertia: 1.0,
        f_max: 12.0,
        tau_max: 8.0,
    };
    let (address, metrics, _proxy) = runtime.block_on(async {
        let server = Server::bind(Settings {
            bind: "127.0.0.1:0".parse().expect("an address"),
            world,
            config: Config::default(),
            identity: Identity::self_signed(["localhost", "127.0.0.1"]).expect("a certificate"),
        })
        .expect("a server");
        let target = server.local_addr().expect("an address");
        let mut address = server.address("127.0.0.1").expect("an address");
        // The path: 50 ms each way, through the proxy.
        let proxy = Proxy::start(
            target,
            Impairment {
                delay: Duration::from_millis(50),
                ..Impairment::NONE
            },
            1,
        )
        .await
        .expect("a proxy");
        address.url = format!("https://127.0.0.1:{}", proxy.addr().port());
        let metrics = server.metrics();
        tokio::spawn(server.serve());
        (address, metrics, proxy)
    });

    let mut clients = [
        Online::new(net::connect(&address).expect("a link"), ours),
        Online::new(net::connect(&address).expect("a link"), stronger),
    ];
    let started = Instant::now();
    let mut joined = false;
    while started.elapsed().as_secs_f64() < seconds + 5.0 {
        let t = started.elapsed().as_secs_f64();
        for (who, client) in clients.iter_mut().enumerate() {
            client.frame(hands(who, t));
        }
        if !joined && clients.iter().all(|c| c.stage() == Stage::Lobby) {
            clients[0].join(ours, name("ours"));
            clients[1].join(stronger, name("stronger"));
            joined = true;
        }
        if joined && clients.iter().all(|c| c.stage() == Stage::Playing) {
            break;
        }
        std::thread::sleep(Duration::from_millis(4));
    }
    assert!(joined, "the lobby came");
    let flying = Instant::now();
    let mut landed = false;
    while flying.elapsed().as_secs_f64() < seconds && !landed {
        let t = flying.elapsed().as_secs_f64();
        for (who, client) in clients.iter_mut().enumerate() {
            client.frame(hands(who, t));
        }
        if let Some(after) = autopilot_after
            && t >= after
            && !clients[1].assisted()
        {
            let policy = SOLUTIONS
                .iter()
                .find(|s| s.name == "flip_and_burn_tuned")
                .expect("a policy");
            clients[1]
                .engage(nabla_client::Objective::ReachB, policy)
                .expect("a ship to fly");
        }
        landed = clients[1].take_landed();
        std::thread::sleep(Duration::from_millis(4));
    }
    let metrics = metrics.lock().expect("the metrics").clone();
    drop(runtime);
    (clients, landed, metrics)
}

#[test]
fn two_pilots_in_a_vacuum_at_100_ms_are_never_corrected() {
    let (clients, _, metrics) = two_pilots(Preset::Vacuum, 5.0, None);
    for (i, client) in clients.iter().enumerate() {
        assert_eq!(client.stage(), Stage::Playing, "{i}");
        let info = client.info();
        assert!(info.corrections.snapshots > 200, "{i}: {info:?}");
        assert_eq!(info.ships, 2, "{i}");
        let rtt = info.rtt.expect("QUIC measured a round trip");
        assert!((0.09..0.2).contains(&rtt), "{rtt}");
    }
    // thm-exact, over a real transport: a prediction is wrong only for a command the server did
    // not apply. A machine busy enough to stall a client makes some late, so it is the ticks
    // held that bound the corrections, and with none held there are none.
    let corrected: u64 = clients.iter().map(|c| c.info().corrections.nonzero).sum();
    assert!(
        corrected <= metrics.held,
        "{corrected} corrections for {} ticks held",
        metrics.held
    );
    let (pa, pb) = (
        clients[0].params().expect("playing"),
        clients[1].params().expect("playing"),
    );
    assert!(
        pa.mass < 1.0 && pb.mass > 1.9,
        "different ships: {pa:?} {pb:?}"
    );
}

#[test]
fn among_bodies_the_corrections_match_the_replay_table() {
    let (clients, _, metrics) = two_pilots(Preset::PlanetAndMoon, 5.0, None);
    let held = metrics.held;
    for (i, client) in clients.iter().enumerate() {
        let c = client.info().corrections;
        assert!(c.snapshots > 200, "{i}: {c:?}");
        assert!(c.max_position < SNAP_POSITION, "{i}, {held} held: {c:?}");
        // tab-replay: positions to 1e-3 and velocities to 3e-2, even in contact. That is for
        // commands that all came in time, as they do in virtual time in
        // among_bodies_with_loss_the_corrections_stay_small. A machine busy enough to stall a
        // client makes some late, and what the pilot did differs from what was applied.
        if held == 0 {
            assert!(c.max_position < 0.05 && c.max_velocity < 0.3, "{i}: {c:?}");
        }
    }
}

#[test]
fn an_autopilot_lands_through_a_real_server_while_another_pilot_flies_by_hand() {
    let (clients, landed, metrics) = two_pilots(Preset::Vacuum, 40.0, Some(0.5));
    let held = metrics.held;
    assert!(landed, "the autopilot lands within 40 s, {held} held");
    for (i, client) in clients.iter().enumerate() {
        assert_eq!(client.stage(), Stage::Playing, "{i}");
        let c = client.info().corrections;
        // Its commands change every tick, and each tick's are sent five times over: if one was
        // late or lost, the prediction was corrected, and only then. So how far depends on how
        // many were late, which on a real clock is up to the machine.
        assert_eq!(c.jumps, 0, "{i}, {held} held: {c:?}");
        assert!(c.max_position < SNAP_POSITION, "{i}, {held} held: {c:?}");
        if held == 0 {
            assert!(c.max_position < 0.05, "{i}: {c:?}");
        }
    }
}

/// A client that goes away says so: the server frees its ship at once, and not after the
/// connection's idle timeout.
#[test]
fn a_client_that_is_dropped_frees_its_ship_at_once() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("a runtime");
    let world = Preset::Vacuum
        .world()
        .multiplayer(60, 1000.0, 16)
        .expect("a world");
    let ship = ShipRequest::from(world.default_ship());
    let (address, metrics) = runtime.block_on(async {
        let server = Server::bind(Settings {
            bind: "127.0.0.1:0".parse().expect("an address"),
            world,
            config: Config::default(),
            identity: Identity::self_signed(["localhost", "127.0.0.1"]).expect("a certificate"),
        })
        .expect("a server");
        let address = server.address("127.0.0.1").expect("an address");
        let metrics = server.metrics();
        tokio::spawn(server.serve());
        (address, metrics)
    });
    let pilots = || metrics.lock().expect("the metrics").pilots;

    let mut client = Online::new(net::connect(&address).expect("a link"), ship);
    let started = Instant::now();
    let mut joined = false;
    while pilots() == 0 {
        assert!(started.elapsed() < Duration::from_secs(5), "the ship came");
        client.frame(hands(0, started.elapsed().as_secs_f64()));
        if !joined && client.stage() == Stage::Lobby {
            client.join(ship, name("solo"));
            joined = true;
        }
        std::thread::sleep(Duration::from_millis(4));
    }
    drop(client);

    let dropped = Instant::now();
    while pilots() > 0 {
        assert!(
            dropped.elapsed() < Duration::from_secs(2),
            "the ship was still held after {:?}",
            dropped.elapsed()
        );
        std::thread::sleep(Duration::from_millis(4));
    }
}

/// Names, a refusal and an absence, as they cross a real connection.
#[test]
fn names_and_absences_reach_the_other_pilot_over_a_real_connection() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("a runtime");
    let world = Preset::Vacuum
        .world()
        .multiplayer(60, 1000.0, 16)
        .expect("a world");
    let ship = ShipRequest::from(world.default_ship());
    let address = runtime.block_on(async {
        let server = Server::bind(Settings {
            bind: "127.0.0.1:0".parse().expect("an address"),
            world,
            config: Config::default(),
            identity: Identity::self_signed(["localhost", "127.0.0.1"]).expect("a certificate"),
        })
        .expect("a server");
        let address = server.address("127.0.0.1").expect("an address");
        tokio::spawn(server.serve());
        address
    });
    let mut clients = [
        Online::new(net::connect(&address).expect("a link"), ship),
        Online::new(net::connect(&address).expect("a link"), ship),
    ];
    let started = Instant::now();
    // Run frames until `done`, with pilot 1 at the controls or away.
    let until = |clients: &mut [Online; 2], away: bool, done: &dyn Fn(&[Online; 2]) -> bool| {
        while !done(clients) {
            assert!(started.elapsed() < Duration::from_secs(20), "it came");
            let t = started.elapsed().as_secs_f64();
            clients[0].frame(hands(0, t));
            clients[1].frame(Hands {
                away,
                ..hands(1, t)
            });
            std::thread::sleep(Duration::from_millis(4));
        }
    };
    until(&mut clients, false, &|c| {
        c.iter().all(|c| c.stage() == Stage::Lobby)
    });
    clients[0].join(ship, name("miriam"));
    until(&mut clients, false, &|c| c[0].stage() == Stage::Playing);
    clients[1].join(ship, name("miriam"));
    until(&mut clients, false, &|c| c[1].refused_name().is_some());
    assert_eq!(clients[1].stage(), Stage::Lobby);
    clients[1].join(ship, name("juan"));
    until(&mut clients, false, &|c| c[1].stage() == Stage::Playing);

    let own = clients[1].ship().expect("flying");
    let seen = |c: &[Online; 2]| c[0].pilot(own).map(|entry| entry.active);
    until(&mut clients, false, &|c| seen(c) == Some(true));
    assert_eq!(
        clients[0].pilot(own).expect("told").username.as_str(),
        "juan"
    );
    until(&mut clients, true, &|c| seen(c) == Some(false));
    until(&mut clients, false, &|c| seen(c) == Some(true));
}
