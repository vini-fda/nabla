//! The connection to a server, as the online flight sees it, on every platform.
//!
//! Natively that is `nabla-net`'s `Link`, a `wtransport` session on a thread of its own. On the
//! web, it is a `web_sys::WebTransport` session driven by tasks on the one thread there is.
//! Either way, a frame never waits on the network: messages go out without waiting, and what
//! came in is taken from a queue.

use nabla_net::Kind;
#[cfg(not(target_arch = "wasm32"))]
use nabla_net::link::Address;
use nabla_net::link::Incoming;

/// A session with a server.
pub trait Transport {
    /// Hand a message to the connection. It never waits.
    fn send(&mut self, kind: Kind);
    /// The next thing the connection delivered, if one has come.
    fn recv(&mut self) -> Option<Incoming>;
    /// The round trip the transport has measured, in seconds, if it has.
    fn rtt(&self) -> Option<f64>;
}

/// Connect to `address`. The connection is made in the background: a failure arrives as
/// [`Incoming::Closed`].
///
/// # Errors
/// If the network cannot be started at all.
#[cfg(not(target_arch = "wasm32"))]
pub fn connect(address: &Address) -> Result<Box<dyn Transport>, String> {
    nabla_net::native::Link::spawn(address)
        .map(|link| Box::new(link) as Box<dyn Transport>)
        .map_err(|e| format!("cannot start the network: {e}"))
}

#[cfg(not(target_arch = "wasm32"))]
impl Transport for nabla_net::native::Link {
    fn send(&mut self, kind: Kind) {
        Self::send(self, kind);
    }

    fn recv(&mut self) -> Option<Incoming> {
        self.try_recv()
    }

    fn rtt(&self) -> Option<f64> {
        Self::rtt(self)
    }
}

#[cfg(target_arch = "wasm32")]
pub use web::connect;

/// WebTransport in the browser, through `web_sys` (which needs `--cfg=web_sys_unstable_apis`).
///
/// There is one thread, so the tasks that read and write share a queue by `Rc`, and a message
/// sent is written at once if the session is up, or else when it comes up.
#[cfg(target_arch = "wasm32")]
mod web {
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::rc::Rc;

    use js_sys::{Promise, Reflect, Uint8Array};
    use nabla_net::Kind;
    use nabla_net::frame::{FrameReader, frame};
    use nabla_net::link::{Address, Incoming};
    use nabla_net::wire::MAX_STREAM_FRAME;
    use wasm_bindgen::{JsCast as _, JsValue};
    use wasm_bindgen_futures::{JsFuture, spawn_local};
    use web_sys::{
        ReadableStream, ReadableStreamDefaultReader, WebTransport, WebTransportHash,
        WebTransportOptions, WritableStream, WritableStreamDefaultWriter,
    };

    use super::Transport;

    /// Snapshots that pile up while no frame runs (a hidden tab) are superseded by the newest, so
    /// only this many datagrams are kept.
    const MAX_QUEUED_DATAGRAMS: usize = 256;

    struct Writers {
        stream: WritableStreamDefaultWriter,
        datagrams: WritableStreamDefaultWriter,
    }

    #[derive(Default)]
    struct Shared {
        inbox: VecDeque<Incoming>,
        /// Messages sent before the session was up.
        waiting: VecDeque<Kind>,
        writers: Option<Writers>,
        closed: bool,
        transport: Option<WebTransport>,
    }

    /// A WebTransport session.
    pub struct WebLink {
        shared: Rc<RefCell<Shared>>,
    }

    /// # Errors
    /// If the address is not one a browser can open.
    pub fn connect(address: &Address) -> Result<Box<dyn Transport>, String> {
        let shared = Rc::new(RefCell::new(Shared::default()));
        let session = open(address)?;
        shared.borrow_mut().transport = Some(session.clone());
        let task = Rc::clone(&shared);
        spawn_local(async move {
            let reason = match run(&session, &task).await {
                Ok(()) => "closed".to_owned(),
                Err(why) => why,
            };
            let mut shared = task.borrow_mut();
            if !shared.closed {
                shared.closed = true;
                shared.inbox.push_back(Incoming::Closed(reason));
            }
        });
        Ok(Box::new(WebLink { shared }))
    }

    fn describe(value: &JsValue) -> String {
        let message = Reflect::get(value, &"message".into())
            .ok()
            .and_then(|m| m.as_string());
        value
            .as_string()
            .or(message)
            .unwrap_or_else(|| "a WebTransport error".to_owned())
    }

    /// The session, with the certificate's hash if the address has one. Nothing else is passed:
    /// the other options are missing from some engines.
    fn open(address: &Address) -> Result<WebTransport, String> {
        let options = WebTransportOptions::new();
        if let Some(hash) = address.hash {
            let digest = WebTransportHash::new();
            digest.set_algorithm("sha-256");
            digest.set_value(&Uint8Array::from(&hash[..]));
            options.set_server_certificate_hashes(&[digest]);
        }
        WebTransport::new_with_options(&address.url, &options).map_err(|e| describe(&e))
    }

    async fn run(session: &WebTransport, shared: &Rc<RefCell<Shared>>) -> Result<(), String> {
        JsFuture::from(session.ready())
            .await
            .map_err(|e| format!("could not connect: {}", describe(&e)))?;
        let bidi = JsFuture::from(session.create_bidirectional_stream())
            .await
            .map_err(|e| format!("no stream: {}", describe(&e)))?;
        let datagrams = session.datagrams();
        let writers = Writers {
            stream: property::<WritableStream>(&bidi, "writable")?
                .get_writer()
                .map_err(|e| describe(&e))?,
            datagrams: datagrams
                .writable()
                .get_writer()
                .map_err(|e| describe(&e))?,
        };
        {
            let mut shared = shared.borrow_mut();
            shared.writers = Some(writers);
            while let Some(kind) = shared.waiting.pop_front() {
                write(&mut shared, kind);
            }
        }
        let stream_reader: ReadableStreamDefaultReader =
            property::<ReadableStream>(&bidi, "readable")?
                .get_reader()
                .unchecked_into();
        let datagram_reader: ReadableStreamDefaultReader =
            datagrams.readable().get_reader().unchecked_into();
        let frames = Rc::clone(shared);
        spawn_local(async move {
            let mut reader = FrameReader::new(MAX_STREAM_FRAME);
            while let Some(chunk) = read(&stream_reader).await {
                reader.push(&chunk);
                while let Ok(Some(bytes)) = reader.next_frame() {
                    frames.borrow_mut().inbox.push_back(Incoming::Stream(bytes));
                }
            }
        });
        let queue = Rc::clone(shared);
        spawn_local(async move {
            while let Some(bytes) = read(&datagram_reader).await {
                let mut shared = queue.borrow_mut();
                if shared.inbox.len() >= MAX_QUEUED_DATAGRAMS
                    && let Some(old) = shared
                        .inbox
                        .iter()
                        .position(|i| matches!(i, Incoming::Datagram(_)))
                {
                    shared.inbox.remove(old);
                }
                shared.inbox.push_back(Incoming::Datagram(bytes));
            }
        });
        let closed: Promise = property(session, "closed")?;
        match JsFuture::from(closed).await {
            Ok(_) => Ok(()),
            Err(e) => Err(describe(&e)),
        }
    }

    /// The property `name` of `object`, as a `T`: for what `web_sys` does not wrap.
    fn property<T: wasm_bindgen::JsCast>(
        object: &(impl AsRef<JsValue> + ?Sized),
        name: &str,
    ) -> Result<T, String> {
        Reflect::get(object.as_ref(), &name.into())
            .map_err(|e| describe(&e))?
            .dyn_into::<T>()
            .map_err(|_wrong| format!("WebTransport has no {name}"))
    }

    /// The next chunk a reader gives, or `None` at its end.
    async fn read(reader: &ReadableStreamDefaultReader) -> Option<Vec<u8>> {
        let result = JsFuture::from(reader.read()).await.ok()?;
        if Reflect::get(&result, &"done".into()).ok()?.as_bool()? {
            return None;
        }
        let value = Reflect::get(&result, &"value".into()).ok()?;
        Some(Uint8Array::new(&value).to_vec())
    }

    /// Hand a message to the session's writers, which exist.
    fn write(shared: &mut Shared, kind: Kind) {
        let Some(writers) = &shared.writers else {
            return;
        };
        match kind {
            Kind::Stream(bytes) => {
                drop(
                    writers
                        .stream
                        .write_with_chunk(&Uint8Array::from(&frame(&bytes)[..])),
                );
            }
            Kind::Datagram(bytes) => {
                // A datagram that would queue behind others is dropped: the network may drop it.
                if writers
                    .datagrams
                    .desired_size()
                    .ok()
                    .flatten()
                    .is_none_or(|room| room > 0.0)
                {
                    drop(
                        writers
                            .datagrams
                            .write_with_chunk(&Uint8Array::from(&bytes[..])),
                    );
                }
            }
            Kind::Close => {
                if let Some(session) = shared.transport.take() {
                    session.close();
                }
            }
        }
    }

    impl Transport for WebLink {
        fn send(&mut self, kind: Kind) {
            let mut shared = self.shared.borrow_mut();
            if shared.writers.is_some() {
                write(&mut shared, kind);
            } else if shared.waiting.len() < 64 {
                shared.waiting.push_back(kind);
            }
        }

        fn recv(&mut self) -> Option<Incoming> {
            self.shared.borrow_mut().inbox.pop_front()
        }

        fn rtt(&self) -> Option<f64> {
            // WebTransport has no `getStats` in the browsers tried: the client machine's own
            // measurement, of the exchange that took the ship, is shown instead.
            None
        }
    }

    impl Drop for WebLink {
        fn drop(&mut self) {
            if let Some(session) = self.shared.borrow_mut().transport.take() {
                session.close();
            }
        }
    }
}
