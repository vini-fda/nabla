//! A client's connection over `wtransport`, for the machines to be driven through.
//!
//! A [`Link`] is a WebTransport session to a server: [`Kind`]s go in with [`Link::send`], and
//! [`Incoming`]s come out of [`Link::try_recv`] or [`Link::recv`]. The first never waits, and the
//! second need not, so a frame loop can use it as it is, with a thread of its own for the
//! network ([`Link::spawn`]). The machine lives with whoever holds the link. Nothing here
//! touches game state.
//!
//! The stream is one bidirectional stream the client opens, with a frame per message
//! ([`frame`](crate::frame)).

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use tokio::runtime::Handle;
use tokio::sync::mpsc;
use wtransport::tls::Sha256Digest;
use wtransport::{ClientConfig, Endpoint};

use crate::Kind;
use crate::frame::{FrameReader, frame};
use crate::link::{Address, Incoming};
use crate::wire::MAX_STREAM_FRAME;

/// How often QUIC pings an idle connection, and how long silence ends one.
pub const KEEP_ALIVE: Duration = Duration::from_secs(4);
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(20);

/// How many datagrams QUIC queues to send, per connection. It evicts the oldest past that. A
/// full second of snapshots, which its default holds (1 MiB), would be a second of delay in each:
/// the newest supersedes the rest, so only a few are worth keeping.
const QUEUED_DATAGRAMS: usize = 4;

/// The transport settings of both ends: keep-alives and the idle timeout, and short datagram
/// queues, so that what congestion control holds back is replaced and not delayed.
pub fn transport() -> wtransport::quinn::TransportConfig {
    use wtransport::quinn::{IdleTimeout, TransportConfig};

    let queue = QUEUED_DATAGRAMS * crate::Config::default().max_datagram;
    let mut transport = TransportConfig::default();
    transport
        .keep_alive_interval(Some(KEEP_ALIVE))
        .max_idle_timeout(IdleTimeout::try_from(IDLE_TIMEOUT).ok())
        .datagram_send_buffer_size(queue)
        .datagram_receive_buffer_size(Some(queue));
    transport
}

/// A session with a server.
#[derive(Debug)]
pub struct Link {
    out: mpsc::UnboundedSender<Kind>,
    incoming: mpsc::UnboundedReceiver<Incoming>,
    rtt_us: Arc<AtomicU32>,
    /// Dropped with the link, to end the thread of [`Link::spawn`].
    _thread: Option<tokio::sync::oneshot::Sender<()>>,
}

impl Link {
    /// Connect from within the runtime `handle` belongs to. The connection is made in the
    /// background: a failure arrives as [`Incoming::Closed`].
    pub fn open(handle: &Handle, address: &Address) -> Self {
        let (out, out_rx) = mpsc::unbounded_channel();
        let (incoming_tx, incoming) = mpsc::unbounded_channel();
        let rtt_us = Arc::new(AtomicU32::new(0));
        let address = address.clone();
        let rtt = Arc::clone(&rtt_us);
        handle.spawn(async move {
            let reason = match run(&address, out_rx, &incoming_tx, &rtt).await {
                Ok(()) => "closed".to_owned(),
                Err(error) => error,
            };
            // The receiver is gone when the link was dropped, and nobody is left to tell.
            incoming_tx.send(Incoming::Closed(reason)).ok();
        });
        Self {
            out,
            incoming,
            rtt_us,
            _thread: None,
        }
    }

    /// Connect from a thread of its own, with a runtime for it alone: for a program that is not
    /// itself asynchronous, such as a game with a frame loop.
    ///
    /// # Errors
    /// If the thread or its runtime cannot be made.
    pub fn spawn(address: &Address) -> std::io::Result<Self> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let mut link = Self::open(runtime.handle(), address);
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        link._thread = Some(stop);
        std::thread::Builder::new()
            .name("nabla-net".to_owned())
            .spawn(move || {
                // Until the link is dropped, and then a moment, for its task to say goodbye.
                runtime.block_on(stopped).ok();
                runtime.block_on(tokio::time::sleep(Duration::from_millis(200)));
            })?;
        Ok(link)
    }

    /// Hand a message to the connection. It never waits.
    pub fn send(&self, kind: Kind) {
        // A closed connection has nobody to hear it, and says so through `Incoming::Closed`.
        self.out.send(kind).ok();
    }

    /// The next thing the connection delivered, if one has come.
    pub fn try_recv(&mut self) -> Option<Incoming> {
        self.incoming.try_recv().ok()
    }

    /// The next thing the connection delivers.
    pub async fn recv(&mut self) -> Option<Incoming> {
        self.incoming.recv().await
    }

    /// The round trip QUIC has measured, in seconds, or `None` before it has one.
    pub fn rtt(&self) -> Option<f64> {
        match self.rtt_us.load(Ordering::Relaxed) {
            0 => None,
            us => Some(f64::from(us) * 1e-6),
        }
    }
}

/// The client's configuration for `address`: the hash it names, or else the system's roots.
fn config(address: &Address) -> ClientConfig {
    let builder = ClientConfig::builder().with_bind_default();
    let builder = match address.hash {
        Some(hash) => builder.with_server_certificate_hashes([Sha256Digest::new(hash)]),
        None => builder.with_native_certs(),
    };
    let mut config = builder.build();
    config
        .quic_config_mut()
        .transport_config(Arc::new(transport()));
    config
}

async fn run(
    address: &Address,
    mut out: mpsc::UnboundedReceiver<Kind>,
    incoming: &mpsc::UnboundedSender<Incoming>,
    rtt_us: &AtomicU32,
) -> Result<(), String> {
    let endpoint = Endpoint::client(config(address)).map_err(|e| format!("no socket: {e}"))?;
    let connection = endpoint
        .connect(&address.url)
        .await
        .map_err(|e| format!("could not connect: {e}"))?;
    let (mut send, mut recv) = connection
        .open_bi()
        .await
        .map_err(|e| format!("no stream: {e}"))?
        .await
        .map_err(|e| format!("no stream: {e}"))?;

    let frames = incoming.clone();
    tokio::spawn(async move {
        let mut reader = FrameReader::new(MAX_STREAM_FRAME);
        let mut buf = vec![0_u8; 16 * 1024];
        while let Ok(Some(n)) = recv.read(&mut buf).await {
            reader.push(&buf[..n]);
            loop {
                match reader.next_frame() {
                    Ok(Some(bytes)) => {
                        if frames.send(Incoming::Stream(bytes)).is_err() {
                            return;
                        }
                    }
                    Ok(None) => break,
                    Err(_) => return,
                }
            }
        }
    });
    let datagrams = incoming.clone();
    let reader = connection.clone();
    tokio::spawn(async move {
        while let Ok(datagram) = reader.receive_datagram().await {
            if datagrams
                .send(Incoming::Datagram(datagram.payload().to_vec()))
                .is_err()
            {
                return;
            }
        }
    });

    loop {
        tokio::select! {
            kind = out.recv() => match kind {
                // The link was dropped.
                None | Some(Kind::Close) => {
                    connection.close(0_u8.into(), b"bye");
                    return Ok(());
                }
                Some(Kind::Stream(bytes)) => {
                    send.write_all(&frame(&bytes)).await.map_err(|e| format!("stream: {e}"))?;
                }
                Some(Kind::Datagram(bytes)) => {
                    // A datagram that does not fit is lost, which the protocol allows.
                    connection.send_datagram(bytes).ok();
                    let us = connection.rtt().as_micros();
                    rtt_us.store(u32::try_from(us).unwrap_or(u32::MAX), Ordering::Relaxed);
                }
            },
            reason = connection.closed() => return Err(format!("{reason}")),
        }
    }
}
