//! The client handles of the file transfer endpoints.

use std::marker::PhantomData;
use std::time::{Duration, SystemTime};

use bytes::Bytes;

use crate::file_transfer::engine::{Engine, OfferToken};
use crate::file_transfer::shared::TransferShared;
use crate::file_transfer::sink::Sink;
use crate::file_transfer::source::Source;
use crate::file_transfer::{TransferError, TransferHandle, TransferMeta};
use crate::protocol::file_transfer::{AcceptMode, FileEntry, OfferBody};

/// An endpoint that receives files from the server.
///
/// The server sends a file, a set of files or a buffer as an offer. Take the
/// offer with [`Self::take_offer`] in `update` and decide where the data goes;
/// nothing is sent until the offer is accepted. `M` is the type of the
/// metadata that comes with each offer, `()` for none. The endpoint carries at
/// most one unfinished transfer at a time.
pub struct Download<M = ()> {
    pub(crate) endpoint: u64,
    pub(crate) engine: Engine,
    _meta: PhantomData<fn() -> M>,
}

/// An endpoint that sends files to the server.
///
/// Every send returns a handle at once and runs in the background. The
/// server decides where the data goes. `M` is the type of the metadata that
/// comes with each offer, `()` for none. The endpoint carries at most one
/// unfinished transfer at a time.
pub struct Upload<M = ()> {
    pub(crate) endpoint: u64,
    pub(crate) engine: Engine,
    _meta: PhantomData<fn() -> M>,
}

/// One file of an offer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OfferedFile {
    name: String,
    size: u64,
    modified: Option<SystemTime>,
}

impl OfferedFile {
    /// Returns the name the sender gave the file: a relative path with `/`
    /// between folders, such as `run42/0001.tiff`. The library has checked
    /// that it stays inside a folder it is saved to.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the size in bytes.
    pub fn size(&self) -> u64 {
        self.size
    }

    /// Returns the time the file was last modified on the sender, if known.
    pub fn modified(&self) -> Option<SystemTime> {
        self.modified
    }
}

/// A transfer the server offers, not answered yet.
///
/// Accept it to start the transfer, or reject it. The offer may be kept
/// across frames, for example while a dialog is open. Dropping it rejects the
/// offer.
pub struct TransferOffer<M> {
    pub(crate) token: OfferToken,
    name: String,
    files: Vec<OfferedFile>,
    total: u64,
    meta: M,
}

impl<M: TransferMeta> Download<M> {
    pub(crate) fn new(endpoint: u64, engine: Engine) -> Self {
        Self {
            endpoint,
            engine,
            _meta: PhantomData,
        }
    }

    /// Takes the offer the server sent, once; `None` while there is none.
    ///
    /// Call it in `update`: a new offer requests a repaint.
    pub fn take_offer(&self) -> Option<TransferOffer<M>> {
        let raw = self.engine.take_offer(self.endpoint)?;
        let meta = match postcard::from_bytes::<M>(&raw.meta) {
            Ok(meta) => meta,
            Err(error) => {
                raw.token.fail(TransferError::Meta(error.to_string()));
                return None;
            }
        };
        Some(TransferOffer::new(raw.token, &raw.body, meta))
    }

    /// Returns the most recent transfer accepted on this endpoint, finished
    /// or not; it stays available after the connection ends.
    pub fn last(&self) -> Option<TransferHandle> {
        self.engine.last(self.endpoint).map(TransferHandle::new)
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
                modified: file.modified.and_then(time_from_millis),
            })
            .collect();
        let total = files.iter().map(|file| file.size).sum();
        Self {
            token,
            name: body.name.clone(),
            files,
            total,
            meta,
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

    /// Returns the size of all files together.
    pub fn total_size(&self) -> Option<u64> {
        Some(self.total)
    }

    /// Returns the metadata the server sent with the offer.
    pub fn meta(&self) -> &M {
        &self.meta
    }

    /// Returns whether the server withdrew the offer or the connection ended.
    /// Accepting a cancelled offer returns a handle that has already failed.
    pub fn is_cancelled(&self) -> bool {
        self.token.is_cancelled()
    }

    /// Accepts a single-file offer into memory, up to `limit` bytes; take the
    /// data with [`TransferHandle::take_data`] once it is done.
    ///
    /// An offer of several files, or larger than `limit`, is rejected, and
    /// the handle fails.
    pub fn accept_memory(self, limit: u64) -> TransferHandle {
        TransferHandle::new(
            self.token
                .accept(Sink::memory(limit), AcceptMode::Lane, None),
        )
    }

    /// Rejects the offer; the server's transfer fails with `reason`.
    pub fn reject(self, reason: impl Into<String>) {
        self.token.reject(&reason.into(), None);
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

    /// Offers `data` to the server as one file named `name`.
    ///
    /// Returns at once. A send while the endpoint is busy or the client is
    /// not connected returns a handle that has already failed, and leaves
    /// [`Self::last`] unchanged.
    pub fn send_bytes(&self, data: Vec<u8>, name: impl Into<String>, meta: M) -> TransferHandle {
        let name = name.into();
        let body = OfferBody {
            name: name.clone(),
            files: vec![FileEntry {
                name,
                size: data.len() as u64,
                modified: None,
            }],
        };
        self.send(body, meta, Source::bytes(data))
    }

    pub(crate) fn send(&self, body: OfferBody, meta: M, source: Source) -> TransferHandle {
        let sent =
            encode_meta(&meta).and_then(|meta| self.engine.send(self.endpoint, body, meta, source));
        TransferHandle::new(sent.unwrap_or_else(|error| TransferShared::failed(error, None)))
    }

    /// Returns the most recent transfer of this endpoint, finished or not; it
    /// stays available after the connection ends.
    pub fn last(&self) -> Option<TransferHandle> {
        self.engine.last(self.endpoint).map(TransferHandle::new)
    }

    /// Returns whether the endpoint has an unfinished transfer, or a send
    /// whose file is still being picked.
    pub fn is_busy(&self) -> bool {
        self.engine.is_busy(self.endpoint)
    }
}

fn encode_meta<M: TransferMeta>(meta: &M) -> Result<Bytes, TransferError> {
    postcard::to_stdvec(meta)
        .map(Bytes::from)
        .map_err(|error| TransferError::Meta(error.to_string()))
}

fn time_from_millis(millis: i64) -> Option<SystemTime> {
    let offset = Duration::from_millis(millis.unsigned_abs());
    if millis >= 0 {
        SystemTime::UNIX_EPOCH.checked_add(offset)
    } else {
        SystemTime::UNIX_EPOCH.checked_sub(offset)
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
