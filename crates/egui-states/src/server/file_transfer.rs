//! The server handles of the file transfer endpoints.

use std::marker::PhantomData;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;

use crate::file_transfer::engine::Engine;
use crate::file_transfer::fs::{FilesSource, modified_millis};
use crate::file_transfer::source::Source;
use crate::file_transfer::{TransferError, TransferHandle, TransferMeta, meta_type_id};
use crate::protocol::file_transfer::{FileEntry, OfferBody};
use crate::server::Result;
use crate::server::callbacks::CallbackHandle;
use crate::server::state_server::{StateServer, WeakServer};
use crate::server_core::file_transfer::{EVENT_FINISHED, TransferEvents, decode_event};

/// What every endpoint handle holds.
struct EndpointRef {
    id: u64,
    engine: Engine,
    server: WeakServer,
    events: Arc<TransferEvents>,
}

impl EndpointRef {
    fn new(server: &StateServer, name: String, meta_type: u32, download: bool) -> Result<Self> {
        let (id, engine, events) = server.add_transfer(name, meta_type, download)?;
        Ok(Self {
            id,
            engine,
            server: server.downgrade(),
            events,
        })
    }

    fn connect(&self, callback: impl Fn(TransferHandle) + Send + Sync + 'static) -> CallbackHandle {
        let events = self.events.clone();
        self.server.add_raw_callback(self.id, move |payload, _| {
            if let Some((EVENT_FINISHED, key)) = decode_event(&payload)
                && let Some(handle) = events.finished(key)
            {
                callback(TransferHandle::new(handle));
            }
            Ok(())
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
        let path = path.as_ref();
        let name = match name {
            Some(name) => name.to_string(),
            None => file_name(path)?,
        };
        let (entry, size) = stat(path, name)?;
        let body = OfferBody {
            name: entry.name.clone(),
            files: vec![entry],
        };
        let source = Source::Files(FilesSource::new(vec![(path.to_path_buf(), size)]));
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
        let mut files = Vec::with_capacity(paths.len());
        let mut sources = Vec::with_capacity(paths.len());
        for path in paths {
            let file_name = match root {
                Some(root) => relative_name(path, root)?,
                None => file_name(path)?,
            };
            let (entry, size) = stat(path, file_name)?;
            files.push(entry);
            sources.push((path.clone(), size));
        }
        let body = OfferBody {
            name: name.to_string(),
            files,
        };
        self.send(
            body,
            meta,
            Source::Files(FilesSource::new(sources)),
            wait,
            timeout,
        )
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
        let body = OfferBody {
            name: name.to_string(),
            files: vec![FileEntry {
                name: name.to_string(),
                size: data.len() as u64,
                modified: None,
            }],
        };
        self.send(body, meta, Source::bytes(data), wait, timeout)
    }

    pub(crate) fn send(
        &self,
        body: OfferBody,
        meta: &M,
        source: Source,
        wait: bool,
        timeout: Option<Duration>,
    ) -> std::result::Result<TransferHandle, TransferError> {
        if timeout.is_some() && !wait {
            return Err(TransferError::InvalidInput(
                "a timeout needs wait".to_string(),
            ));
        }
        let meta = postcard::to_stdvec(meta)
            .map(Bytes::from)
            .map_err(|error| TransferError::Meta(error.to_string()))?;
        let engine = &self.endpoint.engine;
        let shared = if wait {
            engine.send_wait(self.endpoint.id, body, meta, source, timeout)?
        } else {
            engine.send(self.endpoint.id, body, meta, source)?
        };
        Ok(TransferHandle::new(shared))
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

    #[cfg(any(all(test, feature = "client"), feature = "bench"))]
    pub(crate) fn discard_offers(&self) {
        self.endpoint.engine.discard_offers(self.endpoint.id);
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

/// The size and offer entry of the file at `path`.
fn stat(path: &Path, name: String) -> std::result::Result<(FileEntry, u64), TransferError> {
    let metadata = std::fs::metadata(path)
        .map_err(|error| TransferError::Io(format!("{}: {error}", path.display())))?;
    if !metadata.is_file() {
        return Err(TransferError::Io(format!(
            "{} is not a file",
            path.display()
        )));
    }
    let size = metadata.len();
    let entry = FileEntry {
        name,
        size,
        modified: modified_millis(&metadata),
    };
    Ok((entry, size))
}

fn file_name(path: &Path) -> std::result::Result<String, TransferError> {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
        .ok_or_else(|| {
            TransferError::InvalidInput(format!("{} has no UTF-8 file name", path.display()))
        })
}

/// The name of `path` relative to `root`, with `/` between folders.
fn relative_name(path: &Path, root: &Path) -> std::result::Result<String, TransferError> {
    let relative = path.strip_prefix(root).map_err(|_| {
        TransferError::InvalidInput(format!(
            "{} is not inside {}",
            path.display(),
            root.display()
        ))
    })?;
    let mut parts = Vec::new();
    for component in relative.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_str().ok_or_else(|| {
                TransferError::InvalidInput(format!("{} is not UTF-8", path.display()))
            })?),
            _ => {
                return Err(TransferError::InvalidInput(format!(
                    "{} is not inside {}",
                    path.display(),
                    root.display()
                )));
            }
        }
    }
    if parts.is_empty() {
        return Err(TransferError::InvalidInput(format!(
            "{} is the root itself",
            path.display()
        )));
    }
    Ok(parts.join("/"))
}

#[cfg(all(test, feature = "client"))]
mod tests;
