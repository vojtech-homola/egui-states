//! The server handles of the file transfer endpoints.

use std::marker::PhantomData;

use crate::file_transfer::engine::Engine;
use crate::file_transfer::{TransferMeta, meta_type_id};
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
