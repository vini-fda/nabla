//! The server's glue: `wtransport` sessions around one task that owns the
//! [`ServerMachine`].
//!
//! The owner task alternates between its tick timer and an inbox. Connection tasks decode
//! nothing and touch no game state: they turn what a session delivers into events for the
//! inbox, and what the owner sends into writes, so an `.await` is the only place another task may
//! run, and the owner holds nothing borrowed across one (`docs/multiplayer.typ`,
//! "Atomicity, and the owner task").

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use nabla_net::frame::{FrameReader, frame};
use nabla_net::link::{Address, hex};
use nabla_net::wire::{ConnId, MAX_CLIENT_FRAME};
use nabla_net::{Config, Kind, ServerMachine};
use nabla_sim::world::World;
use tokio::sync::mpsc;
use tokio::time::{MissedTickBehavior, interval, timeout};
use wtransport::endpoint::IncomingSession;
use wtransport::endpoint::endpoint_side::Server as ServerSide;
use wtransport::{Endpoint, Identity, ServerConfig};

use crate::metrics::Metrics;

/// Connections held at once, however many are flying: the rest are refused, so that a flood of
/// them cannot take the server's memory.
pub const MAX_CONNECTIONS: usize = 64;
/// How long a connection has to open its stream.
const STREAM_TIMEOUT: Duration = Duration::from_secs(10);
/// How many messages may wait for one connection, each way.
const QUEUE: usize = 256;
/// The inbox of the owner task: datagrams past it are dropped, as the network may drop them.
const INBOX: usize = 4096;

/// What a server needs.
pub struct Settings {
    pub bind: SocketAddr,
    pub world: World,
    pub config: Config,
    pub identity: Identity,
}

/// What a connection task tells the owner.
enum Event {
    Open {
        conn: ConnId,
        outbox: mpsc::Sender<Kind>,
    },
    Stream {
        conn: ConnId,
        bytes: Vec<u8>,
    },
    Datagram {
        conn: ConnId,
        bytes: Vec<u8>,
    },
    Closed {
        conn: ConnId,
    },
}

/// A server that is listening.
pub struct Server {
    endpoint: Endpoint<ServerSide>,
    machine: ServerMachine,
    h: f64,
    max_datagram: usize,
    hash: [u8; 32],
    metrics: Arc<Mutex<Metrics>>,
}

impl Server {
    /// Bind the socket and make the machine.
    ///
    /// # Errors
    /// If the world is not one a server can run, or the socket cannot be bound.
    pub fn bind(settings: Settings) -> Result<Self, String> {
        let Settings {
            bind,
            world,
            config,
            identity,
        } = settings;
        let hash = identity
            .certificate_chain()
            .as_slice()
            .first()
            .ok_or("no certificate")?
            .hash();
        let h = world.h();
        let max_datagram = config.max_datagram;
        let machine = ServerMachine::new(world, config, 0.0)?;
        let mut server = ServerConfig::builder()
            .with_bind_address(bind)
            .with_identity(identity)
            .build();
        server
            .quic_config_mut()
            .transport_config(Arc::new(nabla_net::native::transport()));
        let endpoint = Endpoint::server(server).map_err(|e| format!("cannot bind {bind}: {e}"))?;
        Ok(Self {
            endpoint,
            machine,
            h,
            max_datagram,
            hash: *hash.as_ref(),
            metrics: Arc::default(),
        })
    }

    /// # Errors
    /// If the socket's address cannot be read.
    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.endpoint.local_addr()
    }

    /// The address a client connects to, with the certificate's hash in it.
    ///
    /// # Errors
    /// If the socket's address cannot be read.
    pub fn address(&self, host: &str) -> std::io::Result<Address> {
        let port = self.local_addr()?.port();
        let host = if host.contains(':') {
            format!("[{host}]")
        } else {
            host.to_owned()
        };
        Ok(Address {
            url: format!("https://{host}:{port}"),
            hash: Some(self.hash),
        })
    }

    /// The measurements, shared with the running server.
    pub fn metrics(&self) -> Arc<Mutex<Metrics>> {
        Arc::clone(&self.metrics)
    }

    /// Accept connections and run ticks until the task is dropped.
    pub async fn serve(self) {
        let Self {
            endpoint,
            machine,
            h,
            max_datagram,
            hash,
            metrics,
        } = self;
        log::info!("certificate sha256={}", hex(&hash));
        let (inbox, events) = mpsc::channel(INBOX);
        let open = Arc::new(AtomicUsize::new(0));
        let owner = tokio::spawn(own(machine, h, max_datagram, events, Arc::clone(&metrics)));
        let mut next = 0_u32;
        loop {
            let session = endpoint.accept().await;
            if open.load(Ordering::Relaxed) >= MAX_CONNECTIONS {
                session.refuse();
                if let Ok(mut m) = metrics.lock() {
                    m.refused += 1;
                }
                continue;
            }
            let conn = ConnId(next);
            next = next.wrapping_add(1);
            open.fetch_add(1, Ordering::Relaxed);
            if let Ok(mut m) = metrics.lock() {
                m.accepted += 1;
            }
            let (inbox, open, metrics) = (inbox.clone(), Arc::clone(&open), Arc::clone(&metrics));
            tokio::spawn(async move {
                let result = connection(session, conn, &inbox, &metrics).await;
                if let Err(why) = result {
                    log::debug!("connection {}: {why}", conn.0);
                }
                // The owner learns of every end, even of a connection that never opened.
                inbox.send(Event::Closed { conn }).await.ok();
                open.fetch_sub(1, Ordering::Relaxed);
            });
            if owner.is_finished() {
                log::error!("the owner task ended");
                return;
            }
        }
    }
}

/// The owner task: the machine, the tick timer, and the inbox.
async fn own(
    mut machine: ServerMachine,
    h: f64,
    max_datagram: usize,
    mut events: mpsc::Receiver<Event>,
    metrics: Arc<Mutex<Metrics>>,
) {
    let start = Instant::now();
    let now = || start.elapsed().as_secs_f64();
    let mut outboxes: BTreeMap<ConnId, mpsc::Sender<Kind>> = BTreeMap::new();
    // The timer tracks deadlines and not the time since the last wakeup, so it does not drift,
    // and bursts to catch up if the server falls behind.
    let mut ticker = interval(Duration::from_secs_f64(h));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Burst);
    loop {
        tokio::select! {
            _ = ticker.tick() => {
                let began = Instant::now();
                let due = machine.due(machine.tick_number());
                let late = Duration::from_secs_f64((now() - due).max(0.0));
                let out = machine.tick(now());
                let mut sent = Sent::default();
                for message in out {
                    sent.dispatch(&mut outboxes, message.conn, message.kind, max_datagram);
                }
                with(&metrics, |m| {
                    m.tick(began.elapsed(), late);
                    m.pilots = machine.pilots();
                    sent.add_to(m);
                });
            }
            event = events.recv() => {
                let Some(event) = event else { return };
                let mut sent = Sent::default();
                let mut received = (0_u64, 0_u64);
                match event {
                    Event::Open { conn, outbox } => {
                        machine.connect(conn, now());
                        outboxes.insert(conn, outbox);
                    }
                    Event::Stream { conn, bytes } => {
                        for message in machine.on_stream(conn, &bytes, now()) {
                            sent.dispatch(&mut outboxes, message.conn, message.kind, max_datagram);
                        }
                    }
                    Event::Datagram { conn, bytes } => {
                        received = (1, bytes.len() as u64);
                        machine.on_datagram(conn, &bytes, now());
                    }
                    Event::Closed { conn } => {
                        outboxes.remove(&conn);
                        for message in machine.on_disconnect(conn) {
                            sent.dispatch(&mut outboxes, message.conn, message.kind, max_datagram);
                        }
                    }
                }
                with(&metrics, |m| {
                    m.datagrams_in += received.0;
                    m.payload_in += received.1;
                    m.pilots = machine.pilots();
                    sent.add_to(m);
                });
            }
        }
    }
}

fn with(metrics: &Mutex<Metrics>, f: impl FnOnce(&mut Metrics)) {
    f(&mut metrics.lock().unwrap_or_else(PoisonError::into_inner));
}

/// What one handling of an event sent.
#[derive(Default)]
struct Sent {
    datagrams: u64,
    payload: u64,
    largest: usize,
    dropped: u64,
    oversize: u64,
}

impl Sent {
    fn add_to(&self, m: &mut Metrics) {
        m.datagrams_out += self.datagrams;
        m.payload_out += self.payload;
        m.largest_datagram = m.largest_datagram.max(self.largest);
        m.dropped_out += self.dropped;
        m.oversize += self.oversize;
    }

    /// Hand a message to its connection's task, without waiting. A datagram that finds the queue
    /// full is dropped, as the network may drop it. A stream message that does is the end of the
    /// connection, for its client is not reading.
    fn dispatch(
        &mut self,
        outboxes: &mut BTreeMap<ConnId, mpsc::Sender<Kind>>,
        conn: ConnId,
        kind: Kind,
        max_datagram: usize,
    ) {
        let Some(outbox) = outboxes.get(&conn) else {
            return;
        };
        let (is_datagram, length) = match &kind {
            Kind::Datagram(bytes) => (true, bytes.len()),
            Kind::Stream(_) | Kind::Close => (false, 0),
        };
        if is_datagram && length > max_datagram {
            self.oversize += 1;
            return;
        }
        let close = matches!(kind, Kind::Close);
        match outbox.try_send(kind) {
            Ok(()) => {
                if is_datagram {
                    self.datagrams += 1;
                    self.payload += length as u64;
                    self.largest = self.largest.max(length);
                }
                if close {
                    outboxes.remove(&conn);
                }
            }
            Err(_) => {
                if is_datagram {
                    self.dropped += 1;
                } else {
                    // Dropping the sender ends the connection's task.
                    outboxes.remove(&conn);
                }
            }
        }
    }
}

/// One session, from the handshake to its end.
async fn connection(
    session: IncomingSession,
    conn: ConnId,
    inbox: &mpsc::Sender<Event>,
    metrics: &Mutex<Metrics>,
) -> Result<(), String> {
    let request = session.await.map_err(|e| format!("handshake: {e}"))?;
    let connection = request
        .accept()
        .await
        .map_err(|e| format!("session: {e}"))?;
    let (outbox, mut queue) = mpsc::channel::<Kind>(QUEUE);
    inbox
        .send(Event::Open { conn, outbox })
        .await
        .map_err(|_gone| "the owner is gone")?;
    let (mut send, mut recv) = timeout(STREAM_TIMEOUT, connection.accept_bi())
        .await
        .map_err(|_late| "no stream")?
        .map_err(|e| format!("stream: {e}"))?;

    let frames = inbox.clone();
    let session = connection.clone();
    tokio::spawn(async move {
        let mut reader = FrameReader::new(MAX_CLIENT_FRAME);
        let mut buf = vec![0_u8; 4096];
        'read: while let Ok(Some(n)) = recv.read(&mut buf).await {
            reader.push(&buf[..n]);
            loop {
                match reader.next_frame() {
                    Ok(Some(bytes)) => {
                        if frames.send(Event::Stream { conn, bytes }).await.is_err() {
                            break 'read;
                        }
                    }
                    Ok(None) => break,
                    Err(_) => break 'read,
                }
            }
        }
        // A stream that ends or breaks the framing ends the session.
        session.close(1_u8.into(), b"stream");
    });
    let datagrams = inbox.clone();
    let reader = connection.clone();
    let dropped = Arc::new(AtomicUsize::new(0));
    let dropped_in = Arc::clone(&dropped);
    tokio::spawn(async move {
        while let Ok(datagram) = reader.receive_datagram().await {
            let bytes = datagram.payload().to_vec();
            if datagrams.try_send(Event::Datagram { conn, bytes }).is_err() {
                dropped_in.fetch_add(1, Ordering::Relaxed);
            }
        }
    });

    let result = loop {
        tokio::select! {
            kind = queue.recv() => match kind {
                // The owner dropped the outbox, or said to close.
                None | Some(Kind::Close) => {
                    // Let what is queued reach the client first: a stream's last words.
                    send.finish().await.ok();
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    break Ok(());
                }
                Some(Kind::Stream(bytes)) => {
                    if let Err(e) = send.write_all(&frame(&bytes)).await {
                        break Err(format!("stream write: {e}"));
                    }
                }
                Some(Kind::Datagram(bytes)) => {
                    // Lost datagrams are allowed. QUIC evicts the oldest queued, and a newer
                    // snapshot supersedes it.
                    connection.send_datagram(bytes).ok();
                }
            },
            reason = connection.closed() => break Err(format!("{reason}")),
        }
    };
    let stats = connection.quic_connection().stats();
    with(metrics, |m| {
        m.udp_in += stats.udp_rx.bytes;
        m.udp_out += stats.udp_tx.bytes;
        m.dropped_in += dropped.load(Ordering::Relaxed) as u64;
    });
    connection.close(0_u8.into(), b"bye");
    result
}
