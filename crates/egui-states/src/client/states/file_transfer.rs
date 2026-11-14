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

/// A file on the user's machine to upload: a path on native clients, a
/// browser `File` on the web.
#[derive(Clone, Debug)]
pub struct LocalFile(LocalFileInner);

#[derive(Clone, Debug)]
enum LocalFileInner {
    #[cfg(not(target_arch = "wasm32"))]
    Path(std::path::PathBuf),
}

#[cfg(not(target_arch = "wasm32"))]
impl From<std::path::PathBuf> for LocalFile {
    fn from(path: std::path::PathBuf) -> Self {
        Self(LocalFileInner::Path(path))
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl From<&std::path::Path> for LocalFile {
    fn from(path: &std::path::Path) -> Self {
        Self(LocalFileInner::Path(path.to_path_buf()))
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl From<&str> for LocalFile {
    fn from(path: &str) -> Self {
        Self(LocalFileInner::Path(path.into()))
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl From<String> for LocalFile {
    fn from(path: String) -> Self {
        Self(LocalFileInner::Path(path.into()))
    }
}

/// Builds the offer of `files`, named `name` for several files. It reads the
/// files' sizes, so it runs off the UI thread.
fn local_offer(
    files: Vec<LocalFile>,
    name: Option<String>,
) -> Result<(OfferBody, Source), TransferError> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        use crate::file_transfer::fs::{FilesSource, file_name, stat};
        let single = files.len() == 1;
        let mut entries = Vec::with_capacity(files.len());
        let mut sources = Vec::with_capacity(files.len());
        for file in files {
            let LocalFileInner::Path(path) = file.0;
            let (entry, size) = stat(&path, file_name(&path)?)?;
            entries.push(entry);
            sources.push((path, size));
        }
        let name = match (name, single) {
            (Some(name), _) => name,
            (None, true) => entries[0].name.clone(),
            (None, false) => {
                return Err(TransferError::InvalidInput(
                    "a set of files needs a name".to_string(),
                ));
            }
        };
        let body = OfferBody {
            name,
            files: entries,
        };
        Ok((body, Source::Files(FilesSource::new(sources))))
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (files, name);
        Err(TransferError::Unsupported(
            "file uploads from the browser are not supported yet".to_string(),
        ))
    }
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

    /// Accepts a single-file offer into the file at `path`.
    ///
    /// The data goes to a temporary file next to `path`, which replaces any
    /// existing file only once the transfer is complete; a failed transfer
    /// removes it. Missing folders are created, and the modification time
    /// the server sent is kept. An offer of several files is rejected, and
    /// the handle fails. No disk access happens in this call.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn accept_file(self, path: impl Into<std::path::PathBuf>) -> TransferHandle {
        let path = path.into();
        self.accept_files(crate::file_transfer::fs::Target::File(path))
    }

    /// Accepts the offer into the folder `dir`, every file under its own
    /// name; names like `run42/0001.tiff` create subfolders.
    ///
    /// Each file is written as with [`Self::accept_file`]. If the transfer
    /// fails, files already complete stay. No disk access happens in this
    /// call.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn accept_dir(self, dir: impl Into<std::path::PathBuf>) -> TransferHandle {
        let dir = dir.into();
        self.accept_files(crate::file_transfer::fs::Target::Dir(dir))
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn accept_files(self, target: crate::file_transfer::fs::Target) -> TransferHandle {
        use crate::file_transfer::fs::{FileSink, Target};
        let single = matches!(target, Target::File(_));
        let path = FileSink::display_path(&target);
        let body = OfferBody {
            name: self.name.clone(),
            files: self
                .files
                .iter()
                .map(|file| FileEntry {
                    name: file.name.clone(),
                    size: file.size,
                    modified: file.modified.and_then(millis_from_time),
                })
                .collect(),
        };
        let sink = FileSink::new(target, &body.files);
        let sink = Sink::Files {
            sink: Box::new(sink),
            single,
        };
        TransferHandle::new(self.token.accept(sink, AcceptMode::Lane, Some(path)))
    }

    /// Accepts the offer into a single-file path that `pick` returns, for
    /// example from a file dialog.
    ///
    /// Returns a pending handle at once and runs `pick` on its own thread, so
    /// a blocking dialog does not stop the UI. `None` rejects the offer as
    /// cancelled by the user. An offer of several files fails at once, and
    /// `pick` is not run.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn accept_file_with(
        self,
        pick: impl FnOnce(&TransferOffer<M>) -> Option<std::path::PathBuf> + Send + 'static,
    ) -> TransferHandle
    where
        M: Send + 'static,
    {
        if self.files.len() != 1 {
            let error = TransferError::Unsupported(format!(
                "the offer has {} files; accept it into a folder",
                self.files.len()
            ));
            return TransferHandle::new(self.token.fail(error));
        }
        self.pick_then(pick, |offer, path| offer.accept_file(path))
    }

    /// Accepts the offer into a folder that `pick` returns, for example from
    /// a folder dialog; see [`Self::accept_dir`] and
    /// [`Self::accept_file_with`].
    #[cfg(not(target_arch = "wasm32"))]
    pub fn accept_dir_with(
        self,
        pick: impl FnOnce(&TransferOffer<M>) -> Option<std::path::PathBuf> + Send + 'static,
    ) -> TransferHandle
    where
        M: Send + 'static,
    {
        self.pick_then(pick, |offer, dir| offer.accept_dir(dir))
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn pick_then(
        self,
        pick: impl FnOnce(&TransferOffer<M>) -> Option<std::path::PathBuf> + Send + 'static,
        accept: fn(TransferOffer<M>, std::path::PathBuf) -> TransferHandle,
    ) -> TransferHandle
    where
        M: Send + 'static,
    {
        self.token.expose();
        let handle = TransferHandle::new(self.token.shared().clone());
        // A thread of its own: a dialog may stay open for minutes, and the
        // runtime's threads carry the connection.
        let spawned = std::thread::Builder::new()
            .name("egui_states_pick".to_string())
            .spawn(move || match pick(&self) {
                Some(path) => {
                    accept(self, path);
                }
                None => self.token.reject(
                    "cancelled by user",
                    Some(TransferError::Cancelled {
                        by_peer: false,
                        reason: None,
                    }),
                ),
            });
        if let Err(error) = spawned {
            // The offer was moved into the closure that failed to start;
            // dropping it rejected the offer.
            handle
                .shared
                .cancel(Some(format!("cannot start the picker: {error}")));
        }
        handle
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

    /// Offers `file` to the server under its own file name.
    ///
    /// Returns at once: the file's size is read on a thread of its own and
    /// the data once the server accepts, so a missing file shows as a failed
    /// handle. A send while the endpoint is busy or the client is not
    /// connected returns a handle that has already failed.
    pub fn send_file(&self, file: impl Into<LocalFile>, meta: M) -> TransferHandle {
        self.send_local(vec![file.into()], None, meta)
    }

    /// Offers several files as one transfer named `name`, each under its own
    /// file name; see [`Self::send_file`].
    pub fn send_files(
        &self,
        files: Vec<LocalFile>,
        name: impl Into<String>,
        meta: M,
    ) -> TransferHandle {
        self.send_local(files, Some(name.into()), meta)
    }

    /// Offers the file that `pick` returns, for example from a file dialog.
    ///
    /// Returns a pending handle at once and runs `pick` on a thread of its
    /// own, so a blocking dialog does not stop the UI; the endpoint counts as
    /// busy meanwhile. `None` ends the handle as cancelled and sends nothing.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn send_file_with(
        &self,
        pick: impl FnOnce() -> Option<LocalFile> + Send + 'static,
        meta: M,
    ) -> TransferHandle {
        self.send_picked(move || pick().map(|file| vec![file]), None, meta)
    }

    /// Offers the files that `pick` returns as one transfer named `name`;
    /// see [`Self::send_file_with`].
    #[cfg(not(target_arch = "wasm32"))]
    pub fn send_files_with(
        &self,
        pick: impl FnOnce() -> Option<Vec<LocalFile>> + Send + 'static,
        name: impl Into<String>,
        meta: M,
    ) -> TransferHandle {
        self.send_picked(pick, Some(name.into()), meta)
    }

    /// Offers the file that the future `pick` resolves to, for example from
    /// an async file dialog; it works in the browser too.
    ///
    /// On native clients the future runs on a thread of its own. Otherwise it
    /// behaves like [`Self::send_file_with`].
    #[cfg(not(target_arch = "wasm32"))]
    pub fn send_file_with_async<F>(&self, pick: F, meta: M) -> TransferHandle
    where
        F: std::future::Future<Output = Option<LocalFile>> + Send + 'static,
    {
        self.send_picked(move || block_on(pick).map(|file| vec![file]), None, meta)
    }

    /// Offers the files that the future `pick` resolves to as one transfer
    /// named `name`; see [`Self::send_file_with_async`].
    #[cfg(not(target_arch = "wasm32"))]
    pub fn send_files_with_async<F>(
        &self,
        pick: F,
        name: impl Into<String>,
        meta: M,
    ) -> TransferHandle
    where
        F: std::future::Future<Output = Option<Vec<LocalFile>>> + Send + 'static,
    {
        self.send_picked(move || block_on(pick), Some(name.into()), meta)
    }

    fn send_local(&self, files: Vec<LocalFile>, name: Option<String>, meta: M) -> TransferHandle {
        self.send_picked(move || Some(files), name, meta)
    }

    /// Reserves the endpoint, then picks and reads the files off the
    /// calling thread.
    fn send_picked(
        &self,
        pick: impl FnOnce() -> Option<Vec<LocalFile>> + Send + 'static,
        name: Option<String>,
        meta: M,
    ) -> TransferHandle {
        let meta = match encode_meta(&meta) {
            Ok(meta) => meta,
            Err(error) => return TransferHandle::new(TransferShared::failed(error, None)),
        };
        let shared = match self.engine.reserve(self.endpoint) {
            Ok(shared) => shared,
            Err(error) => return TransferHandle::new(TransferShared::failed(error, None)),
        };
        let (engine, reserved) = (self.engine.clone(), shared.clone());
        let work = move || match pick() {
            None => engine.release(
                &reserved,
                TransferError::Cancelled {
                    by_peer: false,
                    reason: None,
                },
            ),
            Some(files) => match local_offer(files, name) {
                Ok((body, source)) => engine.send_reserved(&reserved, body, meta, source),
                Err(error) => engine.release(&reserved, error),
            },
        };
        spawn_work(work, &self.engine, &shared);
        TransferHandle::new(shared)
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

/// Runs `work` on a thread of its own: a dialog may stay open for minutes,
/// and the client runtime's threads carry the connection.
#[cfg(not(target_arch = "wasm32"))]
fn spawn_work(
    work: impl FnOnce() + Send + 'static,
    engine: &Engine,
    shared: &std::sync::Arc<TransferShared>,
) {
    let spawned = std::thread::Builder::new()
        .name("egui_states_pick".to_string())
        .spawn(work);
    if let Err(error) = spawned {
        engine.release(
            shared,
            TransferError::Io(format!("cannot start a thread: {error}")),
        );
    }
}

#[cfg(target_arch = "wasm32")]
fn spawn_work(
    work: impl FnOnce() + 'static,
    _engine: &Engine,
    _shared: &std::sync::Arc<TransferShared>,
) {
    work();
}

/// Runs a picker future to completion on the current thread.
#[cfg(not(target_arch = "wasm32"))]
fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime for the picker")
        .block_on(future)
}

fn encode_meta<M: TransferMeta>(meta: &M) -> Result<Bytes, TransferError> {
    postcard::to_stdvec(meta)
        .map(Bytes::from)
        .map_err(|error| TransferError::Meta(error.to_string()))
}

#[cfg(not(target_arch = "wasm32"))]
fn millis_from_time(time: SystemTime) -> Option<i64> {
    match time.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(after) => i64::try_from(after.as_millis()).ok(),
        Err(before) => i64::try_from(before.duration().as_millis())
            .ok()
            .map(|ms| -ms),
    }
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
