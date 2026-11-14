//! The server handles of the file transfer endpoints.

use std::marker::PhantomData;
use std::time::Duration;

use bytes::Bytes;

use crate::file_transfer::engine::Engine;
use crate::file_transfer::source::Source;
use crate::file_transfer::{TransferError, TransferHandle, TransferMeta, meta_type_id};
use crate::protocol::file_transfer::{FileEntry, OfferBody};
use crate::server::Result;
use crate::server::state_server::StateServer;

/// An endpoint that sends files to the client.
///
/// The client side of the same state tree field receives them. `M` is the
/// type of the metadata sent with each offer, `()` for none. The endpoint
/// carries at most one unfinished transfer at a time.
pub struct Download<M = ()> {
    pub(crate) id: u64,
    pub(crate) engine: Engine,
    _meta: PhantomData<fn() -> M>,
}

/// An endpoint that receives files from the client.
///
/// `M` is the type of the metadata sent with each offer, `()` for none. The
/// endpoint carries at most one unfinished transfer at a time.
pub struct Upload<M = ()> {
    pub(crate) id: u64,
    pub(crate) engine: Engine,
    _meta: PhantomData<fn() -> M>,
}

impl<M: TransferMeta> Download<M> {
    /// Registers a download endpoint under `name`.
    pub fn new(server: &StateServer, name: impl Into<String>) -> Result<Self> {
        let (id, engine) = server.add_transfer(name.into(), meta_type_id::<M>(), true)?;
        Ok(Self {
            id,
            engine,
            _meta: PhantomData,
        })
    }
}

impl<M: TransferMeta> Download<M> {
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
        let shared = if wait {
            self.engine
                .send_wait(self.id, body, meta, source, timeout)?
        } else {
            self.engine.send(self.id, body, meta, source)?
        };
        Ok(TransferHandle::new(shared))
    }

    /// Returns the most recent transfer of this endpoint, finished or not.
    pub fn last(&self) -> Option<TransferHandle> {
        self.engine.last(self.id).map(TransferHandle::new)
    }
}

impl<M: TransferMeta> Upload<M> {
    /// Registers an upload endpoint under `name`.
    pub fn new(server: &StateServer, name: impl Into<String>) -> Result<Self> {
        let (id, engine) = server.add_transfer(name.into(), meta_type_id::<M>(), false)?;
        Ok(Self {
            id,
            engine,
            _meta: PhantomData,
        })
    }
}

impl<M> Clone for Download<M> {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            engine: self.engine.clone(),
            _meta: PhantomData,
        }
    }
}

impl<M> Clone for Upload<M> {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            engine: self.engine.clone(),
            _meta: PhantomData,
        }
    }
}

#[cfg(all(test, feature = "client"))]
mod tests;
