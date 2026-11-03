//! A UDP proxy that does to a connection what a bad network does: it delays what passes through
//! it, with jitter that reorders, loses some, and sends some twice.
//!
//! Clients connect to the proxy's address and the proxy forwards to the server, one socket for
//! each client, so that the server sees each as a peer of its own. QUIC runs on top, so what it
//! loses it retransmits on the stream and does not retransmit for datagrams, as in the wild.

use std::collections::HashMap;
use std::io;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use nabla_sim::rng::Rng;
use tokio::net::UdpSocket;
use tokio::task::JoinHandle;

/// What one direction of the path does to each packet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Impairment {
    /// Time every packet takes at least.
    pub delay: Duration,
    /// Time more, uniformly up to this: beyond a tick it reorders packets.
    pub jitter: Duration,
    /// The chance a packet is lost.
    pub loss: f64,
    /// The chance a packet is sent twice.
    pub duplicate: f64,
}

impl Impairment {
    pub const NONE: Self = Self {
        delay: Duration::ZERO,
        jitter: Duration::ZERO,
        loss: 0.0,
        duplicate: 0.0,
    };
}

/// A proxy that is running.
pub struct Proxy {
    addr: SocketAddr,
    task: JoinHandle<()>,
}

impl Proxy {
    /// Listen on a loopback port and forward to `upstream`, impairing each direction by
    /// `impairment`, with randomness from `seed`.
    ///
    /// # Errors
    /// If the socket cannot be bound.
    pub async fn start(
        upstream: SocketAddr,
        impairment: Impairment,
        seed: u64,
    ) -> io::Result<Self> {
        let front = Arc::new(UdpSocket::bind("127.0.0.1:0").await?);
        let addr = front.local_addr()?;
        let rng = Arc::new(Mutex::new(Rng::new(seed)));
        let task = tokio::spawn(async move {
            let mut clients: HashMap<SocketAddr, Arc<UdpSocket>> = HashMap::new();
            let mut buf = vec![0_u8; 65536];
            loop {
                let Ok((n, from)) = front.recv_from(&mut buf).await else {
                    return;
                };
                let socket = if let Some(socket) = clients.get(&from) {
                    Arc::clone(socket)
                } else {
                    let Ok(socket) = open(upstream).await else {
                        continue;
                    };
                    let socket = Arc::new(socket);
                    clients.insert(from, Arc::clone(&socket));
                    tokio::spawn(back(
                        Arc::clone(&socket),
                        Arc::clone(&front),
                        from,
                        impairment,
                        Arc::clone(&rng),
                    ));
                    socket
                };
                let packet = buf[..n].to_vec();
                let copies = copies(&impairment, &rng);
                for delay in copies {
                    let (socket, packet) = (Arc::clone(&socket), packet.clone());
                    tokio::spawn(async move {
                        tokio::time::sleep(delay).await;
                        socket.send(&packet).await.ok();
                    });
                }
            }
        });
        Ok(Self { addr, task })
    }

    /// The address clients connect to.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }
}

impl Drop for Proxy {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn open(upstream: SocketAddr) -> io::Result<UdpSocket> {
    let socket = UdpSocket::bind("127.0.0.1:0").await?;
    socket.connect(upstream).await?;
    Ok(socket)
}

/// The server's packets, back to one client.
async fn back(
    from_server: Arc<UdpSocket>,
    front: Arc<UdpSocket>,
    client: SocketAddr,
    impairment: Impairment,
    rng: Arc<Mutex<Rng>>,
) {
    let mut buf = vec![0_u8; 65536];
    while let Ok(n) = from_server.recv(&mut buf).await {
        let packet = buf[..n].to_vec();
        for delay in copies(&impairment, &rng) {
            let (front, packet) = (Arc::clone(&front), packet.clone());
            tokio::spawn(async move {
                tokio::time::sleep(delay).await;
                front.send_to(&packet, client).await.ok();
            });
        }
    }
}

/// How long each copy of a packet is held: none if it is lost, two if it is duplicated.
fn copies(impairment: &Impairment, rng: &Mutex<Rng>) -> Vec<Duration> {
    let mut rng = rng.lock().unwrap_or_else(PoisonError::into_inner);
    if rng.chance(impairment.loss) {
        return Vec::new();
    }
    let count = if rng.chance(impairment.duplicate) {
        2
    } else {
        1
    };
    (0..count)
        .map(|_| impairment.delay + impairment.jitter.mul_f64(rng.unit()))
        .collect()
}
