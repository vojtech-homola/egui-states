//! The client handles of the file transfer endpoints.

use std::marker::PhantomData;

use crate::file_transfer::TransferMeta;
use crate::file_transfer::engine::Engine;

/// An endpoint that receives files from the server.
///
/// The server sends a file, a set of files or a buffer as an offer; the
/// client takes the offer and decides where the data goes. `M` is the type of
/// the metadata that comes with each offer, `()` for none. The endpoint
/// carries at most one unfinished transfer at a time.
pub struct Download<M = ()> {
    pub(crate) endpoint: u64,
    pub(crate) engine: Engine,
    _meta: PhantomData<fn() -> M>,
}

/// An endpoint that sends files to the server.
///
/// The client offers a file, a set of files or a buffer; the server decides
/// where the data goes. `M` is the type of the metadata that comes with each
/// offer, `()` for none. The endpoint carries at most one unfinished transfer
/// at a time.
pub struct Upload<M = ()> {
    pub(crate) endpoint: u64,
    pub(crate) engine: Engine,
    _meta: PhantomData<fn() -> M>,
}

impl<M: TransferMeta> Download<M> {
    pub(crate) fn new(endpoint: u64, engine: Engine) -> Self {
        Self {
            endpoint,
            engine,
            _meta: PhantomData,
        }
    }
}

impl<M: TransferMeta> Upload<M> {
    pub(crate) fn new(endpoint: u64, engine: Engine) -> Self {
        Self {
            endpoint,
            engine,
            _meta: PhantomData,
        }
    }
}

impl<M> Clone for Download<M> {
    fn clone(&self) -> Self {
        Self {
            endpoint: self.endpoint,
            engine: self.engine.clone(),
            _meta: PhantomData,
        }
    }
}

impl<M> Clone for Upload<M> {
    fn clone(&self) -> Self {
        Self {
            endpoint: self.endpoint,
            engine: self.engine.clone(),
            _meta: PhantomData,
        }
    }
}
