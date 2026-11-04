//! The online flight against a server machine, in virtual time over a wire with a delay (and
//! losses, when asked).

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use nabla_core::solutions::SOLUTIONS;
use nabla_net::wire::ConnId;
use nabla_net::{Kind, ServerMachine};
use nabla_sim::rng::Rng;
use nabla_sim::steering::Input;
use nabla_sim::world::Preset;

use super::*;

struct Pipe {
    inbox: Rc<RefCell<VecDeque<Incoming>>>,
    outbox: Rc<RefCell<Vec<Kind>>>,
}

impl Transport for Pipe {
    fn send(&mut self, kind: Kind) {
        self.outbox.borrow_mut().push(kind);
    }

    fn recv(&mut self) -> Option<Incoming> {
        self.inbox.borrow_mut().pop_front()
    }

    fn rtt(&self) -> Option<f64> {
        None
    }
}

struct Wire {
    inbox: Rc<RefCell<VecDeque<Incoming>>>,
    outbox: Rc<RefCell<Vec<Kind>>>,
    up: VecDeque<(f64, Kind)>,
    down: VecDeque<(f64, Incoming)>,
    /// What came while the client was stalled, which the browser hands over a message at a time,
    /// among the frames that run again.
    stash: VecDeque<Incoming>,
}

/// A server, and clients on wires to it that take `owd` seconds each way.
struct Rig {
    now: f64,
    owd: f64,
    loss: f64,
    rng: Rng,
    server: ServerMachine,
    wires: Vec<Wire>,
    clients: Vec<Online>,
    /// Clients whose frames do not run, as in a hidden tab or a window being dragged: what the
    /// network brings piles up unread.
    stalled: Vec<bool>,
}

impl Rig {
    fn new(world: World, owd: f64, loss: f64, seed: u64) -> Self {
        Self {
            now: 0.0,
            owd,
            loss,
            rng: Rng::new(seed),
            server: ServerMachine::new(world, Config::default(), 0.0).expect("a server world"),
            wires: Vec::new(),
            clients: Vec::new(),
            stalled: Vec::new(),
        }
    }

    fn add(&mut self, request: ShipRequest) -> usize {
        let inbox = Rc::default();
        let outbox = Rc::default();
        let pipe = Pipe {
            inbox: Rc::clone(&inbox),
            outbox: Rc::clone(&outbox),
        };
        let i = self.clients.len();
        self.server.connect(ConnId(i as u32), self.now);
        self.clients.push(Online::new(Box::new(pipe), request));
        self.stalled.push(false);
        self.wires.push(Wire {
            inbox,
            outbox,
            up: VecDeque::new(),
            down: VecDeque::new(),
            stash: VecDeque::new(),
        });
        i
    }

    /// Advance 1 ms.
    fn step(&mut self, hands: &dyn Fn(usize, f64) -> Hands) {
        self.now += 0.001;
        let now = self.now;
        for i in 0..self.wires.len() {
            let sent: Vec<Kind> = self.wires[i].outbox.borrow_mut().drain(..).collect();
            for kind in sent {
                let lost = matches!(kind, Kind::Datagram(_)) && self.rng.chance(self.loss);
                if !lost {
                    self.wires[i].up.push_back((now + self.owd, kind));
                }
            }
            while self.wires[i].up.front().is_some_and(|(due, _)| *due <= now) {
                let (_, kind) = self.wires[i].up.pop_front().expect("front");
                let conn = ConnId(i as u32);
                match kind {
                    Kind::Stream(bytes) => {
                        let out = self.server.on_stream(conn, &bytes, now);
                        route(
                            &mut self.wires,
                            out,
                            now + self.owd,
                            self.loss,
                            &mut self.rng,
                        );
                    }
                    Kind::Datagram(bytes) => self.server.on_datagram(conn, &bytes, now),
                    Kind::Close => {}
                }
            }
        }
        while self.server.due(self.server.tick_number()) <= now {
            let due = self.server.due(self.server.tick_number());
            let out = self.server.tick(due);
            route(
                &mut self.wires,
                out,
                now + self.owd,
                self.loss,
                &mut self.rng,
            );
        }
        for i in 0..self.wires.len() {
            while self.wires[i]
                .down
                .front()
                .is_some_and(|(due, _)| *due <= now)
            {
                let (_, incoming) = self.wires[i].down.pop_front().expect("front");
                if self.stalled[i] || !self.wires[i].stash.is_empty() {
                    self.wires[i].stash.push_back(incoming);
                } else {
                    self.wires[i].inbox.borrow_mut().push_back(incoming);
                }
            }
            if !self.stalled[i]
                && let Some(next) = self.wires[i].stash.pop_front()
            {
                self.wires[i].inbox.borrow_mut().push_back(next);
            }
            if !self.stalled[i] {
                self.clients[i].frame_at(now, hands(i, now));
            }
        }
    }

    fn run(&mut self, seconds: f64, hands: &dyn Fn(usize, f64) -> Hands) {
        let until = self.now + seconds;
        while self.now < until {
            self.step(hands);
        }
    }

    /// Run until every client is in the lobby, and join them all.
    fn join_all(&mut self, requests: &[ShipRequest]) {
        for _ in 0..2000 {
            self.step(&|_, _| Hands::default());
            if self.clients.iter().all(|c| c.stage() == Stage::Lobby) {
                break;
            }
        }
        for (i, request) in requests.iter().enumerate() {
            let now = self.now;
            let username = Username::parse(&format!("pilot{i}")).expect("a valid name");
            self.clients[i].join_at(now, *request, username);
        }
    }
}

fn route(wires: &mut [Wire], out: Vec<nabla_net::Outgoing>, due: f64, loss: f64, rng: &mut Rng) {
    for message in out {
        let wire = &mut wires[message.conn.0 as usize];
        match message.kind {
            Kind::Stream(bytes) => wire.down.push_back((due, Incoming::Stream(bytes))),
            Kind::Datagram(bytes) => {
                if !rng.chance(loss) {
                    wire.down.push_back((due, Incoming::Datagram(bytes)));
                }
            }
            Kind::Close => wire
                .down
                .push_back((due, Incoming::Closed("closed".to_owned()))),
        }
    }
}

fn vacuum() -> World {
    Preset::Vacuum
        .world()
        .multiplayer(60, 1000.0, 16)
        .expect("a server world")
}

fn bodies() -> World {
    Preset::PlanetAndMoon
        .world()
        .multiplayer(60, 1000.0, 16)
        .expect("a server world")
}

/// A pilot who thrusts, and turns one way and the other every 1.3 s or so.
fn scripted(who: usize, now: f64) -> Hands {
    let phase = ((now + who as f64 * 0.7) / 1.3).floor() as i64;
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
    }
}

fn stronger_ship() -> ShipRequest {
    ShipRequest {
        mass: 2.0,
        inertia: 1.0,
        f_max: 12.0,
        tau_max: 8.0,
    }
}

#[test]
fn two_pilots_with_different_ships_fly_in_one_world_at_100_ms_with_no_corrections() {
    let world = vacuum();
    let ours = ShipRequest::from(world.default_ship());
    let mut rig = Rig::new(world, 0.050, 0.0, 1);
    rig.add(ours);
    rig.add(stronger_ship());
    rig.join_all(&[ours, stronger_ship()]);
    rig.run(20.0, &scripted);

    let (a, b) = (&rig.clients[0], &rig.clients[1]);
    assert_eq!(a.stage(), Stage::Playing);
    assert_eq!(b.stage(), Stage::Playing);
    let (pa, pb) = (a.params().expect("playing"), b.params().expect("playing"));
    assert!(
        (pa.mass - 0.5).abs() < 1e-12 && (pb.mass - 2.0).abs() < 1e-12,
        "{pa:?} {pb:?}"
    );
    for client in [a, b] {
        let stats = client.info().corrections;
        assert!(stats.snapshots > 1000, "{stats:?}");
        // thm-exact: no command was lost, nor late, so nothing was ever corrected.
        assert_eq!((stats.nonzero, stats.jumps), (0, 0), "{stats:?}");
    }
    assert_eq!(a.info().ships, 2);

    // A sees B where B itself predicts it to be at that tick: both are the same exact states,
    // interpolated alike.
    let now = rig.now;
    let at = rig.clients[0].playout.tick.expect("playing");
    let seen = rig.clients[0].views(now);
    assert_eq!(seen.len(), 2);
    let other = &seen[1];
    assert!(!other.own && seen[0].own);
    let b = &rig.clients[1];
    let (lo, hi) = (at.floor() as Tick, at.floor() as Tick + 1);
    let (from, to) = (&b.trail[&lo], &b.trail[&hi]);
    let theirs = hermite(&from.state, &to.state, at - at.floor(), 1.0 / 60.0);
    assert!(
        (other.state.x - theirs.x).abs() < 1e-9,
        "{:?} {theirs:?}",
        other.state
    );
    assert!((other.state.y - theirs.y).abs() < 1e-9);
    assert!((other.state.theta - theirs.theta).abs() < 1e-9);
    // And a few ticks behind the newest snapshot, as `D_r` says.
    let lag = f64::from(rig.clients[0].machine.base().expect("playing").tick) - at;
    assert!((2.4..=4.6).contains(&lag), "{lag}");
}

#[test]
fn among_bodies_with_loss_the_corrections_stay_small() {
    let mut rig = Rig::new(bodies(), 0.050, 0.03, 5);
    let ours = ShipRequest::from(rig.server.world().default_ship());
    rig.add(ours);
    rig.add(stronger_ship());
    rig.join_all(&[ours, stronger_ship()]);
    rig.run(20.0, &scripted);
    for (i, client) in rig.clients.iter().enumerate() {
        let stats = client.info().corrections;
        assert!(stats.snapshots > 800, "{i}: {stats:?}");
        assert_eq!(stats.jumps, 0, "{i}: {stats:?}");
        // The replay is Rapier restarted from a snapshot (`tab-replay`), and losing every copy
        // of a changed command is rare: so corrections are about the size the table gives.
        assert!(stats.max_position < 0.05, "{i}: {stats:?}");
        assert!(stats.max_velocity < 0.3, "{i}: {stats:?}");
    }
}

#[test]
fn an_autopilot_lands_while_another_pilot_flies_by_hand() {
    let policy = SOLUTIONS
        .iter()
        .find(|s| s.name == "flip_and_burn_tuned")
        .expect("a policy");
    let world = vacuum();
    let ours = ShipRequest::from(world.default_ship());
    let mut rig = Rig::new(world, 0.050, 0.0, 9);
    rig.add(ours);
    rig.add(ours);
    rig.join_all(&[ours, ours]);
    rig.run(3.0, &scripted);
    rig.clients[1]
        .engage(Objective::ReachB, policy)
        .expect("a ship to fly");
    assert!(rig.clients[1].assisted());
    let mut landed_at = None;
    for _ in 0..120_000 {
        rig.step(&scripted);
        if rig.clients[1].take_landed() {
            landed_at = Some(rig.now);
            break;
        }
    }
    let landed_at = landed_at.expect("the autopilot lands within two minutes");
    assert!(rig.clients[1].take_fault().is_none());
    assert!(landed_at < 100.0, "{landed_at}");
    // The pilot by hand was never disturbed, and neither was the autopilot's prediction: its
    // commands changed every tick, and all arrived.
    for client in &rig.clients {
        let stats = client.info().corrections;
        assert_eq!((stats.nonzero, stats.jumps), (0, 0), "{stats:?}");
    }
    assert_eq!(rig.clients[0].stage(), Stage::Playing);
}

#[test]
fn a_correction_is_hidden_and_then_fades() {
    let mut smoothing = Smoothing::default();
    let drawn = |prediction: f64, smoothing: &Smoothing| prediction + smoothing.x;
    // The prediction was at 5.0, the snapshot moves it to 5.3.
    let before = drawn(5.0, &smoothing);
    smoothing.absorb(&Correction {
        delta: State {
            x: 0.3,
            ..State::default()
        },
        jumped: false,
    });
    assert!(
        (drawn(5.3, &smoothing) - before).abs() < 1e-15,
        "continuous at the correction"
    );
    // Then it fades: to 5% in three time constants.
    for _ in 0..(3.0 * SMOOTHING * 60.0) as usize {
        smoothing.decay(1.0 / 60.0);
    }
    assert!(smoothing.x.abs() < 0.3 * 0.0501, "{smoothing:?}");
    // Corrections every tick stay within `Delta_max / (1 - lambda)`.
    let mut smoothing = Smoothing::default();
    let lambda = (-1.0_f64 / 60.0 / SMOOTHING).exp();
    for n in 0..600 {
        let sign = if n % 2 == 0 { 1.0 } else { -1.0 };
        smoothing.absorb(&Correction {
            delta: State {
                x: 0.1 * sign,
                ..State::default()
            },
            jumped: false,
        });
        smoothing.decay(1.0 / 60.0);
        assert!(smoothing.x.abs() <= 0.1 / (1.0 - lambda), "{smoothing:?}");
    }
}

#[test]
fn a_large_correction_is_a_jump_and_not_eased() {
    let mut smoothing = Smoothing {
        x: 0.2,
        y: 0.1,
        theta: 0.1,
    };
    smoothing.absorb(&Correction {
        delta: State {
            x: SNAP_POSITION * 1.1,
            ..State::default()
        },
        jumped: false,
    });
    assert_eq!(smoothing, Smoothing::default());
    let mut smoothing = Smoothing {
        x: 0.2,
        ..Smoothing::default()
    };
    smoothing.absorb(&Correction {
        delta: State::default(),
        jumped: true,
    });
    assert_eq!(smoothing, Smoothing::default());
}

#[test]
fn hermite_interpolation_is_as_exact_as_the_paper_says() {
    use nabla_core::step as vacuum_step;
    let world = vacuum();
    let params = world.default_ship();
    let h = 1.0 / 60.0;
    let params = Params { h, ..params };
    let mut worst = 0.0_f64;
    let mut rng = Rng::new(3);
    for _ in 0..2000 {
        let s = State {
            x: rng.range(-100.0, 100.0),
            y: rng.range(-100.0, 100.0),
            vx: rng.range(-20.0, 20.0),
            vy: rng.range(-20.0, 20.0),
            theta: rng.range(-3.0, 3.0),
            omega: rng.range(-10.0, 10.0),
        };
        let u = Control {
            thrust: rng.range(0.0, params.f_max),
            torque: rng.range(-params.tau_max, params.tau_max),
        };
        let end = vacuum_step(&params, &s, &u);
        let fraction = rng.unit();
        let exact = flow(&params, &s, &u, fraction * h);
        let near = hermite(&s, &end, fraction, h);
        worst = worst.max((near.x - exact.x).hypot(near.y - exact.y));
        // The bound of prop-hermite for q = 1, with the spin of this sample.
        let a = params.f_max / params.mass;
        let beta = params.tau_max / params.inertia;
        let omega = s.omega.abs().max(end.omega.abs());
        let bound = h.powi(4) / 384.0 * a * (beta + omega * omega);
        assert!(
            (near.x - exact.x).hypot(near.y - exact.y) <= 2.0 * bound + 1e-12,
            "{} vs {bound}",
            (near.x - exact.x).hypot(near.y - exact.y)
        );
        // The endpoints are exact.
        assert!((hermite(&s, &end, 0.0, h).x - s.x).abs() < 1e-12);
        assert!((hermite(&s, &end, 1.0, h).x - end.x).abs() < 1e-12);
    }
    assert!(worst < 1e-5, "{worst}");
}

#[test]
fn a_ship_that_leaves_goes_to_the_lobby_and_a_full_server_says_so() {
    let world = vacuum();
    let ours = ShipRequest::from(world.default_ship());
    let mut rig = Rig::new(world, 0.020, 0.0, 2);
    rig.add(ours);
    rig.join_all(&[ours]);
    rig.run(2.0, &scripted);
    assert_eq!(rig.clients[0].stage(), Stage::Playing);
    rig.clients[0].leave();
    rig.run(1.0, &scripted);
    assert_eq!(rig.clients[0].stage(), Stage::Lobby);
    assert_eq!(rig.clients[0].views(rig.now).len(), 0);
    assert_eq!(rig.server.pilots(), 0);
}

#[test]
fn a_stall_is_made_up_by_a_jump_and_not_by_an_estimate_of_the_path_gone_wild() {
    // A hidden tab, or a window held by the mouse, runs no frames for seconds, while the network
    // goes on bringing snapshots, which the browser hands over one at a time among the frames
    // that come back. Read as if they had just arrived, they look like a path with a delay of
    // seconds, and the client took a lead of seconds over the server: the page showed +750 ms of
    // margin.
    let world = vacuum();
    let ours = ShipRequest::from(world.default_ship());
    let mut rig = Rig::new(world, 0.050, 0.0, 4);
    rig.add(ours);
    rig.join_all(&[ours]);
    rig.run(5.0, &scripted);
    let before = rig.clients[0].info().margin_error.expect("playing");
    assert!(before.abs() < 0.005, "{before}");
    rig.stalled[0] = true;
    rig.run(4.0, &scripted);
    rig.stalled[0] = false;
    rig.run(2.0, &scripted);
    let client = &rig.clients[0];
    let error = client.info().margin_error.expect("playing");
    assert!(
        error.abs() < 0.030,
        "margin error {error} s after a stall of 4 s"
    );
    let ahead =
        client.machine.tick().expect("playing") - client.machine.base().expect("playing").tick;
    assert!(
        (3..=9).contains(&ahead),
        "{ahead} ticks ahead of the newest snapshot"
    );
    // And the ship was never taken from its place by it: nothing was ever corrected by more than
    // the stall itself, which the server flew as neutral.
    assert_eq!(client.stage(), Stage::Playing);
}

#[test]
fn the_orbit_assist_flies_a_ship_on_a_server_around_a_body() {
    use crate::orbit::Target;

    // One planet: with the moon in the world, an orbit at the ship's altitude crosses it.
    let world = Preset::Planet
        .world()
        .multiplayer(60, 1000.0, 16)
        .expect("a server world");
    let ours = ShipRequest::from(world.default_ship());
    let mut rig = Rig::new(world, 0.050, 0.0, 12);
    rig.add(ours);
    rig.join_all(&[ours]);
    rig.run(1.0, &scripted);
    let (state, planet) = {
        let client = &rig.clients[0];
        let world = client.world().expect("a world");
        let state = client.next_state().expect("playing");
        (state, world.bodies[0].clone())
    };
    let target = Target::at_current_altitude(rig.clients[0].world().expect("a world"), &state, 0)
        .expect("a target");
    let policy = &SOLUTIONS[0];
    let forecast = rig.clients[0]
        .engage(Objective::Orbit(target), policy)
        .expect("the orbit can be flown from here");
    assert!(forecast.is_some(), "an orbit has a forecast");
    assert!(rig.clients[0].orbit_pilot().is_some());
    rig.run(40.0, &scripted);
    let client = &mut rig.clients[0];
    assert!(
        client.take_fault().is_none(),
        "the assist never lost the ship"
    );
    assert!(client.orbit_pilot().is_some(), "still flying it");
    let end = client.next_state().expect("playing");
    let distance =
        (end.x - f64::from(planet.position[0])).hypot(end.y - f64::from(planet.position[1]));
    let radius = target.radius(client.world().expect("a world"));
    assert!(
        (distance - radius).abs() < 0.3 * radius,
        "{distance} from the planet's center, for an orbit of radius {radius}"
    );
    assert!(!client.forecast().is_empty(), "a forecast of the orbit");
}

/// The thrust fraction a screen would give the sound at `now`, for the own ship.
fn heard(rig: &mut Rig) -> f64 {
    let now = rig.now;
    let views = rig.clients[0].views(now);
    let own = views.first().filter(|view| view.own).expect("the own ship");
    own.replay.thrust_fraction(own.t)
}

#[test]
fn a_frame_that_runs_long_does_not_silence_the_thrust_after_it() {
    // On a short path the client leads the snapshots by a tick or two, so a frame of a few tens of
    // milliseconds (an orbit's forecast, say) lets one overtake the tick the client was about to
    // issue. That tick stays in the trail with no control, and the controls after it must stay
    // on their own ticks: they moved a tick early, which left none for now, and the sound heard
    // nothing while the pilot held full thrust, until that tick left the trail half a minute on.
    let world = vacuum();
    let ours = ShipRequest::from(world.default_ship());
    let mut rig = Rig::new(world, 0.0005, 0.0, 6);
    rig.add(ours);
    rig.join_all(&[ours]);
    let thrust = |level: f64| {
        move |_: usize, _: f64| Hands {
            input: Input {
                thrust: level,
                ..Input::default()
            },
            steering: Steering::Raw,
        }
    };
    rig.run(3.0, &thrust(1.0));
    // A few long frames, with all that came during each read at once.
    for _ in 0..20 {
        rig.stalled[0] = true;
        for _ in 0..40 {
            rig.step(&thrust(1.0));
        }
        rig.stalled[0] = false;
        let came: Vec<Incoming> = rig.wires[0].stash.drain(..).collect();
        rig.wires[0].inbox.borrow_mut().extend(came);
        rig.step(&thrust(1.0));
    }
    let k = rig.clients[0].machine.tick().expect("playing");
    assert!(
        rig.clients[0]
            .trail
            .range(..k)
            .any(|(_, step)| step.control.is_none()),
        "a tick with no control in the trail, or this tests nothing"
    );
    let (mut samples, mut silent) = (0, 0);
    for step in 0..2000 {
        rig.step(&thrust(1.0));
        if step % 16 == 0 && step > 500 {
            samples += 1;
            if heard(&mut rig) < 0.99 {
                silent += 1;
            }
        }
    }
    assert_eq!(silent, 0, "{silent} of {samples} frames heard no thrust");
    // Letting go is heard as well: the controls are on their own ticks, not merely present.
    rig.run(0.2, &thrust(0.0));
    assert!(heard(&mut rig) < 0.01, "thrust heard after letting go");
}

#[test]
fn thrust_is_heard_again_when_the_orbit_assist_hands_the_ship_back() {
    use crate::orbit::Target;

    let world = Preset::Planet
        .world()
        .multiplayer(60, 1000.0, 16)
        .expect("a server world");
    let ours = ShipRequest::from(world.default_ship());
    let mut rig = Rig::new(world, 0.050, 0.0, 12);
    rig.add(ours);
    rig.join_all(&[ours]);
    rig.run(1.0, &scripted);
    let state = rig.clients[0].next_state().expect("playing");
    let target = Target::at_current_altitude(rig.clients[0].world().expect("a world"), &state, 0)
        .expect("a target");
    rig.clients[0]
        .engage(Objective::Orbit(target), &SOLUTIONS[0])
        .expect("an orbit");
    rig.run(20.0, &scripted);
    rig.clients[0].disengage();
    let thrusting = |_: usize, _: f64| Hands {
        input: Input {
            thrust: 1.0,
            ..Input::default()
        },
        steering: Steering::Raw,
    };
    let (mut samples, mut silent) = (0, 0);
    for step in 0..10_000 {
        rig.step(&thrusting);
        if step % 50 == 0 && step > 500 {
            samples += 1;
            if heard(&mut rig) < 0.99 {
                silent += 1;
            }
        }
    }
    assert_eq!(silent, 0, "{silent} of {samples} frames heard no thrust");
}

#[test]
fn a_late_frame_still_hears_the_thrust_of_the_tick_it_is_in() {
    // A frame that comes after the next tick is due (it spent its time on a forecast, say) has
    // run past the end of the own ship's flight, where there is no control: it heard nothing.
    let world = vacuum();
    let ours = ShipRequest::from(world.default_ship());
    let mut rig = Rig::new(world, 0.050, 0.0, 8);
    rig.add(ours);
    rig.join_all(&[ours]);
    let thrusting = |_: usize, _: f64| Hands {
        input: Input {
            thrust: 1.0,
            ..Input::default()
        },
        steering: Steering::Raw,
    };
    rig.run(3.0, &thrusting);
    for late in [0.0, 0.005, 0.015, 0.02, 0.03, 0.1] {
        let now = rig.now + late;
        let views = rig.clients[0].views(now);
        let own = views.first().filter(|view| view.own).expect("the own ship");
        assert!(
            own.replay.thrust_fraction(own.t) > 0.99,
            "{late} s late heard {}",
            own.replay.thrust_fraction(own.t)
        );
    }
}

#[test]
fn the_own_plots_show_the_recent_flight_in_the_same_buffers_each_frame() {
    let world = vacuum();
    let ours = ShipRequest::from(world.default_ship());
    let mut rig = Rig::new(world, 0.050, 0.0, 8);
    rig.add(ours);
    rig.join_all(&[ours]);
    let hands = |_: usize, _: f64| Hands::default();
    let mut allocations = None;
    for _ in 0..3 {
        rig.run(1.0, &hands);
        let now = rig.now;
        let views = rig.clients[0].views(now);
        let own = views.first().filter(|view| view.own).expect("the own ship");
        let plots = rig.clients[0].plots_of_own(own);
        let last = own.replay.last_frame();
        assert_eq!(plots.x.points().len(), last + 1);
        assert_eq!(plots.thrust.points().len(), 2 * last);
        let newest = plots.x.points().last().expect("a sample");
        assert_eq!(newest.y, own.replay.state(last).x);
        assert!(newest.x >= own.t, "{} is before {}", newest.x, own.t);
        let now = plots.allocations();
        assert_eq!(*allocations.get_or_insert(now), now);
    }
}

#[test]
fn a_prediction_is_corrected_only_for_a_command_the_server_held() {
    // `thm-exact`: a correction needs an applied command other than the one predicted with, and
    // the only way to that is a tick with no command. So a client whose frames stall, which
    // real-time tests cannot rule out on a busy machine, sees at most one for each such tick.
    let flight = |stalls: bool| {
        let world = vacuum();
        let ours = ShipRequest::from(world.default_ship());
        let mut rig = Rig::new(world, 0.050, 0.0, 21);
        rig.add(ours);
        rig.join_all(&[ours]);
        rig.run(1.0, &scripted);
        let start = rig.server.held();
        let base = rig.clients[0].info().corrections.nonzero;
        for _ in 0..20_000 {
            // 80 ms without frames around each change of the pilot's input, every 1.3 s or so.
            let phase = (rig.now + 0.08) / 1.3;
            rig.stalled[0] = stalls && phase.fract() < 0.08 / 1.3;
            rig.step(&scripted);
        }
        rig.stalled[0] = false;
        rig.run(0.5, &scripted);
        (
            rig.server.held() - start,
            rig.clients[0].info().corrections.nonzero - base,
        )
    };
    assert_eq!(flight(false), (0, 0), "(held, corrected) with no stall");
    let (held, corrected) = flight(true);
    assert!(corrected > 0, "the stalls changed nothing: {held} held");
    assert!(
        corrected <= held,
        "{corrected} corrections for {held} ticks held"
    );
}

#[test]
fn a_stall_past_the_catch_up_is_recovered_from_at_once() {
    // The snapshots that came during a stall say nothing of the path. Taken for reports, they
    // said the round trip was a single tick, so the client woke with too little lead, and its
    // commands stayed late, too little to jump and too much to stretch away, for a second or two.
    let world = vacuum();
    let ours = ShipRequest::from(world.default_ship());
    let mut rig = Rig::new(world, 0.050, 0.0, 33);
    rig.add(ours);
    rig.join_all(&[ours]);
    rig.run(3.0, &scripted);
    rig.stalled[0] = true;
    for _ in 0..400 {
        rig.step(&scripted);
    }
    rig.stalled[0] = false;
    // All that came is there to read at once, as on a native client.
    let came: Vec<Incoming> = rig.wires[0].stash.drain(..).collect();
    rig.wires[0].inbox.borrow_mut().extend(came);
    rig.run(0.5, &scripted);
    let held = rig.server.held();
    rig.run(2.0, &scripted);
    let more = rig.server.held() - held;
    let error = rig.clients[0].machine.margin_error().expect("playing");
    assert_eq!(more, 0, "ticks held from 0.5 s to 2.5 s after waking");
    assert!(error.abs() < 0.03, "margin error {error} s");
}

/// How many commands the server held, and how many ticks went without one, in 25 s of frames at
/// 60 Hz, where once a second `slow` frames in a row each come `gap` seconds after the last.
fn lost_to_slow_frames(owd: f64, offset: f64, gap: f64, slow: u32) -> (u64, usize) {
    let hands = |_: usize, _: f64| Hands {
        input: Input {
            thrust: 1.0,
            ..Input::default()
        },
        steering: Steering::Raw,
    };
    let frames = |rig: &mut Rig, seconds: f64, gap: f64, slow: u32| {
        let until = rig.now + seconds;
        let mut next = rig.now;
        let mut count = 0;
        while rig.now < until {
            let frame = rig.now + 0.0005 >= next;
            rig.stalled[0] = !frame;
            if frame {
                let brought: Vec<_> = rig.wires[0].stash.drain(..).collect();
                rig.wires[0].inbox.borrow_mut().extend(brought);
            }
            rig.step(&hands);
            if frame {
                count += 1;
                next += if (1..=slow).contains(&(count % 60)) {
                    gap
                } else {
                    1.0 / 60.0
                };
            }
        }
        rig.stalled[0] = false;
    };
    let world = vacuum();
    let ours = ShipRequest::from(world.default_ship());
    let mut rig = Rig::new(world, owd, 0.0, 7);
    rig.add(ours);
    rig.join_all(&[ours]);
    rig.run(4.0 + offset, &hands);
    frames(&mut rig, 3.0, 1.0 / 60.0, 0);
    let held = rig.server.held();
    let from = rig.clients[0].machine.tick().expect("playing");
    frames(&mut rig, 25.0, gap, slow);
    let to = rig.clients[0].machine.tick().expect("playing");
    let skipped = rig.clients[0]
        .trail
        .range(from..to)
        .filter(|(_, step)| step.control.is_none())
        .count();
    (rig.server.held() - held, skipped)
}

#[test]
fn slicing_a_capture_check_all_but_stops_the_loss_of_commands() {
    // The sums of `lost_to_slow_frames` over paths from 1 to 50 ms each way, and over where tick
    // starts happen to sit in the frame, which decides whether a late frame misses a tick.
    let lost = |gap: f64, slow: u32| {
        let (mut held, mut skipped) = (0, 0);
        for owd in [
            0.001, 0.002, 0.003, 0.005, 0.007, 0.010, 0.015, 0.025, 0.040, 0.050,
        ] {
            for offset in [0.0, 0.0031, 0.0067, 0.0113] {
                let (h, s) = lost_to_slow_frames(owd, offset, gap, slow);
                held += h;
                skipped += s;
            }
        }
        (held, skipped)
    };
    // Slices that fit in what the frame has spare leave the frames on time. Those that don't make
    // every frame of the check late by `CHECK_BUDGET`, for the 13 frames it takes.
    let (held, skipped) = lost(1.0 / 60.0 + 0.002, 13);
    // A whole check at once: one frame that misses the display's next chance to show.
    let (whole_held, whole_skipped) = lost(0.0334, 1);
    assert_eq!(skipped, 0, "slices made a snapshot overtake the client");
    assert!(whole_skipped > 0, "a whole check does skip commands");
    assert!(
        held * 50 < whole_held,
        "held {held} in slices, against {whole_held} at once"
    );
}
