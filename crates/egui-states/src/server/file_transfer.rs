//! The server handles of the file transfer endpoints.

use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;

use crate::file_transfer::engine::Engine;
use crate::file_transfer::engine::OfferToken;
use crate::file_transfer::fs::{FileSink, Target};
use crate::file_transfer::sink::Sink;
use crate::file_transfer::source::Source;
use crate::file_transfer::{TransferError, TransferHandle, TransferMeta, meta_type_id};
use crate::protocol::file_transfer::AcceptMode;
use crate::protocol::file_transfer::OfferBody;
use crate::server::Result;
use crate::server::callbacks::CallbackHandle;
use crate::server::state_server::{StateServer, WeakServer};
use crate::server_core::file_transfer as core;
use crate::server_core::file_transfer::{
    EVENT_FINISHED, EVENT_OFFER, TransferEvents, decode_event,
};

type Listener = Arc<dyn Fn(TransferHandle) + Send + Sync>;

/// The `connect` callbacks of one endpoint, run by one dispatcher.
#[derive(Default)]
struct Listeners {
    next: u64,
    list: Vec<(u64, Listener)>,
    dispatcher: Option<CallbackHandle>,
}

/// What every endpoint handle holds.
struct EndpointRef {
    id: u64,
    engine: Engine,
    server: WeakServer,
    events: Arc<TransferEvents>,
    listeners: Arc<parking_lot::Mutex<Listeners>>,
}

impl EndpointRef {
    fn new(server: &StateServer, name: String, meta_type: u32, download: bool) -> Result<Self> {
        let (id, engine, events) = server.add_transfer(name, meta_type, download)?;
        Ok(Self {
            id,
            engine,
            server: server.downgrade(),
            events,
            listeners: Arc::default(),
        })
    }

    /// Adds a `connect` callback. One dispatcher per endpoint takes each
    /// finished transfer and runs every callback, so the event releases the
    /// transfer once all of them ran.
    fn connect(&self, callback: impl Fn(TransferHandle) + Send + Sync + 'static) -> CallbackHandle {
        let mut listeners = self.listeners.lock();
        let listener_id = listeners.next;
        listeners.next += 1;
        listeners.list.push((listener_id, Arc::new(callback)));
        if listeners.dispatcher.is_none() {
            let events = self.events.clone();
            let weak = Arc::downgrade(&self.listeners);
            let dispatcher = self.server.add_raw_callback(self.id, move |payload, _| {
                let Some((EVENT_FINISHED, key)) = decode_event(&payload) else {
                    return Ok(());
                };
                let Some(handle) = events.take(key) else {
                    return Ok(());
                };
                let callbacks: Vec<Listener> = weak
                    .upgrade()
                    .map(|listeners| {
                        listeners
                            .lock()
                            .list
                            .iter()
                            .map(|(_, c)| c.clone())
                            .collect()
                    })
                    .unwrap_or_default();
                let mut panicked = false;
                for callback in callbacks {
                    let handle = TransferHandle::new(handle.clone());
                    let run = std::panic::AssertUnwindSafe(|| callback(handle));
                    panicked |= std::panic::catch_unwind(run).is_err();
                }
                if panicked {
                    return Err(crate::server::ServerError::new(
                        "a transfer callback panicked",
                    ));
                }
                Ok(())
            });
            listeners.dispatcher = Some(dispatcher);
        }
        drop(listeners);
        let listeners = self.listeners.clone();
        CallbackHandle::detached(self.id).on_disconnect(move || {
            let mut guard = listeners.lock();
            guard.list.retain(|(id, _)| *id != listener_id);
            let dispatcher = if guard.list.is_empty() {
                guard.dispatcher.take()
            } else {
                None
            };
            drop(guard);
            // Unregisters the dispatcher, so finished transfers are no longer
            // kept for it.
            drop(dispatcher);
        })
    }
}

impl Clone for EndpointRef {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            engine: self.engine.clone(),
            server: self.server.clone(),
            events: self.events.clone(),
            listeners: self.listeners.clone(),
        }
    }
}

/// An endpoint that sends files to the client.
///
/// The client side of the same state tree field receives them. `M` is the
/// type of the metadata sent with each offer, `()` for none. The endpoint
/// carries at most one unfinished transfer at a time.
pub struct Download<M = ()> {
    endpoint: EndpointRef,
    _meta: PhantomData<fn() -> M>,
}

/// An endpoint that receives files from the client.
///
/// `M` is the type of the metadata sent with each offer, `()` for none. The
/// endpoint carries at most one unfinished transfer at a time.
pub struct Upload<M = ()> {
    endpoint: EndpointRef,
    _meta: PhantomData<fn() -> M>,
}

impl<M: TransferMeta> Download<M> {
    /// Registers a download endpoint under `name`.
    pub fn new(server: &StateServer, name: impl Into<String>) -> Result<Self> {
        Ok(Self {
            endpoint: EndpointRef::new(server, name.into(), meta_type_id::<M>(), true)?,
            _meta: PhantomData,
        })
    }

    /// Offers the file at `path` to the client, under its own file name or
    /// `name`.
    ///
    /// The size and modification time are read now; the file itself is read
    /// once the client accepts, and a file that changed size by then fails
    /// the transfer. `wait` and `timeout` work as in [`Self::send_bytes`].
    ///
    /// # Errors
    ///
    /// As [`Self::send_bytes`], and [`TransferError::Io`] for a file that
    /// cannot be read.
    pub fn send_file(
        &self,
        path: impl AsRef<Path>,
        name: Option<&str>,
        meta: &M,
        wait: bool,
        timeout: Option<Duration>,
    ) -> std::result::Result<TransferHandle, TransferError> {
        let (body, source) = core::file_offer(path.as_ref(), name)?;
        self.send(body, meta, source, wait, timeout)
    }

    /// Offers several files as one transfer named `name`.
    ///
    /// Each file is offered under its own file name or, with `root`, under
    /// its path relative to `root`, so the client recreates the folders.
    /// `wait` and `timeout` work as in [`Self::send_bytes`].
    ///
    /// # Errors
    ///
    /// As [`Self::send_file`], and [`TransferError::InvalidInput`] for no
    /// files, a file outside `root` or two files with the same name.
    pub fn send_files(
        &self,
        paths: &[PathBuf],
        name: &str,
        root: Option<&Path>,
        meta: &M,
        wait: bool,
        timeout: Option<Duration>,
    ) -> std::result::Result<TransferHandle, TransferError> {
        let (body, source) = core::files_offer(paths, name, root)?;
        self.send(body, meta, source, wait, timeout)
    }

    /// Offers `data` to the client as one file named `name`.
    ///
    /// Returns once the offer is queued; the data follows after the client
    /// accepts. Without `wait`, a send while the endpoint has an unfinished
    /// transfer fails with [`TransferError::Busy`]. With `wait`, the call
    /// blocks until the endpoint is free, up to `timeout`, and threads that
    /// wait are served in the order they called. Do not wait inside a
    /// callback: it holds a callback worker for as long as it waits.
    ///
    /// # Errors
    ///
    /// [`TransferError::NotConnected`] without a client, also when the client
    /// disconnects during the wait; [`TransferError::Busy`];
    /// [`TransferError::Timeout`]; [`TransferError::InvalidInput`] for an
    /// invalid name or a `timeout` without `wait`; [`TransferError::Meta`]
    /// for metadata that cannot be encoded or is too large.
    pub fn send_bytes(
        &self,
        data: Vec<u8>,
        name: &str,
        meta: &M,
        wait: bool,
        timeout: Option<Duration>,
    ) -> std::result::Result<TransferHandle, TransferError> {
        let (body, source) = core::bytes_offer(data, name);
        self.send(body, meta, source, wait, timeout)
    }

    pub(crate) fn send(
        &self,
        body: OfferBody,
        meta: &M,
        source: Source,
        wait: bool,
        timeout: Option<Duration>,
    ) -> std::result::Result<TransferHandle, TransferError> {
        let meta = postcard::to_stdvec(meta)
            .map(Bytes::from)
            .map_err(|error| TransferError::Meta(error.to_string()))?;
        let engine = &self.endpoint.engine;
        core::send(engine, self.endpoint.id, body, meta, source, wait, timeout)
            .map(TransferHandle::new)
    }

    /// Returns the most recent transfer of this endpoint, finished or not.
    pub fn last(&self) -> Option<TransferHandle> {
        self.endpoint
            .engine
            .last(self.endpoint.id)
            .map(TransferHandle::new)
    }

    /// Calls `callback` on a callback worker once for every transfer of this
    /// endpoint when it is done or has failed.
    ///
    /// Retain the returned handle; dropping it unregisters the callback.
    pub fn connect(
        &self,
        callback: impl Fn(TransferHandle) + Send + Sync + 'static,
    ) -> CallbackHandle {
        self.endpoint.connect(callback)
    }
}

impl<M: TransferMeta> Upload<M> {
    /// Registers an upload endpoint under `name`.
    pub fn new(server: &StateServer, name: impl Into<String>) -> Result<Self> {
        Ok(Self {
            endpoint: EndpointRef::new(server, name.into(), meta_type_id::<M>(), false)?,
            _meta: PhantomData,
        })
    }

    /// Returns the most recent transfer accepted on this endpoint, finished
    /// or not.
    pub fn last(&self) -> Option<TransferHandle> {
        self.endpoint
            .engine
            .last(self.endpoint.id)
            .map(TransferHandle::new)
    }

    /// Calls `callback` on a callback worker once for every accepted transfer
    /// of this endpoint when it is done or has failed.
    ///
    /// Retain the returned handle; dropping it unregisters the callback.
    pub fn connect(
        &self,
        callback: impl Fn(TransferHandle) + Send + Sync + 'static,
    ) -> CallbackHandle {
        self.endpoint.connect(callback)
    }

    /// Calls `receiver` on a callback worker with every offer the client
    /// sends.
    ///
    /// Answer the offer by accepting or rejecting it; dropping it rejects it.
    /// It may be moved elsewhere and answered later. While no receiver is
    /// connected, offers are rejected with "no receiver". Retain the returned
    /// handle; dropping it unregisters the receiver.
    pub fn on_offer(
        &self,
        receiver: impl Fn(TransferOffer<M>) + Send + Sync + 'static,
    ) -> CallbackHandle {
        let engine = self.endpoint.engine.clone();
        let id = self.endpoint.id;
        let handle = self
            .endpoint
            .server
            .add_raw_callback(id, move |payload, _| {
                if let Some((EVENT_OFFER, _)) = decode_event(&payload)
                    && let Some(raw) = engine.take_offer(id)
                {
                    match postcard::from_bytes::<M>(&raw.meta) {
                        Ok(meta) => receiver(TransferOffer::new(raw.token, &raw.body, meta)),
                        Err(error) => {
                            raw.token.fail(TransferError::Meta(error.to_string()));
                        }
                    }
                }
                Ok(())
            });
        self.endpoint.engine.set_receiver(id, true);
        let engine = self.endpoint.engine.clone();
        handle.on_disconnect(move || engine.set_receiver(id, false))
    }

    #[cfg(any(all(test, feature = "client"), feature = "bench"))]
    pub(crate) fn discard_offers(&self) {
        self.endpoint.engine.discard_offers(self.endpoint.id);
    }
}

/// A transfer the client offers, not answered yet.
///
/// Dropping it rejects the offer.
pub struct TransferOffer<M> {
    token: OfferToken,
    name: String,
    files: Vec<OfferedFile>,
    total: u64,
    meta: M,
    body: OfferBody,
}

/// One file of an offer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OfferedFile {
    name: String,
    size: u64,
    modified: Option<std::time::SystemTime>,
}

impl OfferedFile {
    /// Returns the name the client gave the file: a relative path with `/`
    /// between folders. The library has checked that it stays inside a
    /// folder it is saved to.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the size in bytes; the client cannot send more.
    pub fn size(&self) -> u64 {
        self.size
    }

    /// Returns the time the file was last modified on the client, if known.
    pub fn modified(&self) -> Option<std::time::SystemTime> {
        self.modified
    }
}

impl<M> TransferOffer<M> {
    fn new(token: OfferToken, body: &OfferBody, meta: M) -> Self {
        let files: Vec<OfferedFile> = body
            .files
            .iter()
            .map(|file| OfferedFile {
                name: file.name.clone(),
                size: file.size,
                modified: file
                    .modified
                    .and_then(crate::file_transfer::fs::time_from_millis),
            })
            .collect();
        let total = files.iter().map(|file| file.size).sum();
        Self {
            token,
            name: body.name.clone(),
            files,
            total,
            meta,
            body: body.clone(),
        }
    }

    /// Returns the file name, or the name of the set for several files.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the files of the offer; there is at least one.
    pub fn files(&self) -> &[OfferedFile] {
        &self.files
    }

    /// Returns the size of all files together; the client cannot send more.
    pub fn total_size(&self) -> Option<u64> {
        Some(self.total)
    }

    /// Returns the metadata the client sent with the offer.
    pub fn meta(&self) -> &M {
        &self.meta
    }

    /// Accepts a single-file offer into the file at `path`.
    ///
    /// The data goes to a temporary file next to `path`, which replaces any
    /// existing file only once the transfer is complete; missing folders are
    /// created and the client's modification time is kept.
    ///
    /// # Errors
    ///
    /// [`TransferError::Unsupported`] for an offer of several files; the
    /// offer is then rejected.
    pub fn accept_file(
        self,
        path: impl Into<PathBuf>,
    ) -> std::result::Result<TransferHandle, TransferError> {
        self.accept_into(Target::File(path.into()))
    }

    /// Accepts the offer into the folder `dir`, every file under its own
    /// name; names like `run42/0001.tiff` create subfolders. If the transfer
    /// fails, files already complete stay.
    pub fn accept_dir(
        self,
        dir: impl Into<PathBuf>,
    ) -> std::result::Result<TransferHandle, TransferError> {
        self.accept_into(Target::Dir(dir.into()))
    }

    /// Accepts a single-file offer into memory, up to `limit` bytes; take the
    /// data with [`TransferHandle::take_data`] once it is done.
    ///
    /// # Errors
    ///
    /// [`TransferError::Unsupported`] for an offer of several files and
    /// [`TransferError::TooLarge`] for one larger than `limit`; the offer is
    /// then rejected.
    pub fn accept_memory(self, limit: u64) -> std::result::Result<TransferHandle, TransferError> {
        self.accept_sink(Sink::memory(limit), None)
    }

    /// Rejects the offer; the client's transfer fails with `reason`.
    pub fn reject(self, reason: impl Into<String>) {
        self.token.reject(&reason.into(), None);
    }

    fn accept_into(self, target: Target) -> std::result::Result<TransferHandle, TransferError> {
        let single = matches!(target, Target::File(_));
        let path = FileSink::display_path(&target);
        let sink = Sink::Files {
            sink: Box::new(FileSink::new(target, &self.body.files)),
            single,
        };
        self.accept_sink(sink, Some(path))
    }

    fn accept_sink(
        self,
        sink: Sink,
        path: Option<PathBuf>,
    ) -> std::result::Result<TransferHandle, TransferError> {
        let shared = self.token.accept(sink, AcceptMode::Lane, path);
        match shared.status() {
            crate::file_transfer::TransferStatus::Failed(
                error @ (TransferError::Unsupported(_) | TransferError::TooLarge),
            ) => Err(error),
            _ => Ok(TransferHandle::new(shared)),
        }
    }
}

impl<M> Clone for Download<M> {
    fn clone(&self) -> Self {
        Self {
            endpoint: self.endpoint.clone(),
            _meta: PhantomData,
        }
    }
}

impl<M> Clone for Upload<M> {
    fn clone(&self) -> Self {
        Self {
            endpoint: self.endpoint.clone(),
            _meta: PhantomData,
        }
    }
}

#[cfg(all(test, feature = "client"))]
mod tests;
