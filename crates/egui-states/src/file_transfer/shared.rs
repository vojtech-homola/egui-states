//! The state of one transfer, shared by its handles and the engine.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock, Weak};
use std::time::Duration;

use bytes::Bytes;
use parking_lot::{Condvar, Mutex};

use crate::file_transfer::engine::{self, Repaint};
use crate::file_transfer::{TransferError, TransferProgress, TransferStatus};

/// How long a running transfer waits before the progress it reported asks
/// for the next repaint.
const PROGRESS_REPAINT: Duration = Duration::from_millis(100);

pub(crate) struct TransferShared {
    name: OnceLock<String>,
    /// Sizes of the files, in transfer order.
    sizes: OnceLock<Vec<u64>>,
    state: Mutex<SharedState>,
    changed: Condvar,
    repaint: Option<Repaint>,
    /// The engine that runs the transfer, for `cancel`.
    pub(super) engine: Weak<engine::Inner>,
    /// The endpoint of the transfer.
    pub(super) endpoint: u64,
    /// A key that is unique for the life of the engine.
    pub(super) key: u64,
}

struct SharedState {
    status: TransferStatus,
    done: u64,
    total: u64,
    data: Option<Vec<u8>>,
    path: Option<PathBuf>,
    meta: Option<Bytes>,
}

impl TransferShared {
    pub(super) fn new(
        engine: Weak<engine::Inner>,
        endpoint: u64,
        key: u64,
        repaint: Option<Repaint>,
    ) -> Arc<Self> {
        Arc::new(Self {
            name: OnceLock::new(),
            sizes: OnceLock::new(),
            state: Mutex::new(SharedState {
                status: TransferStatus::Pending,
                done: 0,
                total: 0,
                data: None,
                path: None,
                meta: None,
            }),
            changed: Condvar::new(),
            repaint,
            engine,
            endpoint,
            key,
        })
    }

    /// A transfer that failed before it started, with no engine behind it.
    pub(crate) fn failed(error: TransferError, repaint: Option<Repaint>) -> Arc<Self> {
        let shared = Self::new(Weak::new(), 0, u64::MAX, repaint);
        shared.state.lock().status = TransferStatus::Failed(error);
        shared
    }

    /// Records the files once they are known.
    pub(super) fn bind(&self, name: &str, sizes: Vec<u64>, meta: Bytes) {
        let _ = self.name.set(name.to_string());
        let total = sizes.iter().sum();
        let _ = self.sizes.set(sizes);
        let mut state = self.state.lock();
        state.total = total;
        state.meta = Some(meta);
    }

    pub(super) fn set_path(&self, path: PathBuf) {
        self.state.lock().path = Some(path);
    }

    pub(super) fn set_done_bytes(&self, done: u64) {
        self.state.lock().done = done;
    }

    pub(super) fn set_data(&self, data: Vec<u8>) {
        self.state.lock().data = Some(data);
    }

    /// Moves the transfer to `status` unless it has already finished;
    /// returns whether it changed.
    pub(super) fn set_status(&self, status: TransferStatus) -> bool {
        {
            let mut state = self.state.lock();
            if is_finished(&state.status) || state.status == status {
                return false;
            }
            if status == TransferStatus::Done {
                state.done = state.total;
            }
            state.status = status;
        }
        self.changed.notify_all();
        if let Some(repaint) = &self.repaint {
            repaint(None);
        }
        true
    }

    pub(crate) fn status(&self) -> TransferStatus {
        self.state.lock().status.clone()
    }

    pub(crate) fn is_finished(&self) -> bool {
        is_finished(&self.state.lock().status)
    }

    pub(crate) fn name(&self) -> &str {
        self.name.get().map_or("", String::as_str)
    }

    pub(crate) fn progress(&self) -> TransferProgress {
        let (done, total, running) = {
            let state = self.state.lock();
            (
                state.done,
                state.total,
                state.status == TransferStatus::Running,
            )
        };
        if running && let Some(repaint) = &self.repaint {
            repaint(Some(PROGRESS_REPAINT));
        }
        let sizes = self.sizes.get().map_or(&[][..], Vec::as_slice);
        let finished = self.status() == TransferStatus::Done;
        TransferProgress {
            done,
            total: Some(total),
            files_done: if finished {
                sizes.len() as u32
            } else {
                files_done(sizes, done)
            },
            files_total: sizes.len() as u32,
        }
    }

    pub(crate) fn take_data(&self) -> Option<Vec<u8>> {
        let mut state = self.state.lock();
        if state.status == TransferStatus::Done {
            state.data.take()
        } else {
            None
        }
    }

    #[cfg_attr(not(any(feature = "server", feature = "python")), allow(dead_code))]
    pub(crate) fn path(&self) -> Option<PathBuf> {
        self.state.lock().path.clone()
    }

    #[cfg_attr(not(any(feature = "server", feature = "python")), allow(dead_code))]
    pub(crate) fn meta(&self) -> Option<Bytes> {
        self.state.lock().meta.clone()
    }

    #[cfg_attr(not(any(feature = "server", feature = "python")), allow(dead_code))]
    pub(crate) fn key(&self) -> u64 {
        self.key
    }

    pub(crate) fn error(&self) -> Option<TransferError> {
        match &self.state.lock().status {
            TransferStatus::Failed(error) => Some(error.clone()),
            _ => None,
        }
    }

    /// Cancels the transfer; nothing happens once it has finished.
    pub(crate) fn cancel(self: &Arc<Self>, reason: Option<String>) {
        if self.is_finished() {
            return;
        }
        match self.engine.upgrade() {
            Some(engine) => engine::Engine::from_inner(engine).cancel(self, reason),
            None => {
                self.set_status(TransferStatus::Failed(TransferError::Cancelled {
                    by_peer: false,
                    reason,
                }));
            }
        }
    }

    /// Blocks until the transfer has finished or `timeout` has passed;
    /// returns whether it finished.
    #[cfg(any(feature = "server", feature = "python"))]
    pub(crate) fn wait(&self, timeout: Option<Duration>) -> bool {
        let deadline = timeout.map(|timeout| std::time::Instant::now() + timeout);
        let mut state = self.state.lock();
        while !is_finished(&state.status) {
            match deadline {
                Some(deadline) => {
                    if self.changed.wait_until(&mut state, deadline).timed_out() {
                        return is_finished(&state.status);
                    }
                }
                None => self.changed.wait(&mut state),
            }
        }
        true
    }
}

impl std::fmt::Debug for TransferShared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TransferShared")
            .field("name", &self.name())
            .field("status", &self.status())
            .finish()
    }
}

fn is_finished(status: &TransferStatus) -> bool {
    matches!(status, TransferStatus::Done | TransferStatus::Failed(_))
}

/// Files completely covered by the first `done` bytes. An empty file counts
/// once the bytes before it are done and the transfer has gone past it.
fn files_done(sizes: &[u64], done: u64) -> u32 {
    let mut end = 0;
    let mut count = 0;
    for &size in sizes {
        end += size;
        if end < done || (end == done && size > 0) {
            count += 1;
        } else {
            break;
        }
    }
    count
}

/// One upload or download.
///
/// A handle is cheap to clone, and every clone shows the same transfer.
/// Dropping it does not cancel the transfer.
#[derive(Clone)]
pub struct TransferHandle {
    pub(crate) shared: Arc<TransferShared>,
}

impl TransferHandle {
    pub(crate) fn new(shared: Arc<TransferShared>) -> Self {
        Self { shared }
    }

    /// Returns where the transfer stands.
    pub fn status(&self) -> TransferStatus {
        self.shared.status()
    }

    /// Returns how far the transfer has come.
    ///
    /// While the transfer runs, each call asks the egui client to repaint
    /// again in 100 ms, so a progress bar keeps moving without other repaint
    /// requests.
    pub fn progress(&self) -> TransferProgress {
        self.shared.progress()
    }

    /// Returns the file name, or the name of the set; empty while a file is
    /// still being picked.
    pub fn name(&self) -> &str {
        self.shared.name()
    }

    /// Returns whether the transfer is done or has failed.
    pub fn is_finished(&self) -> bool {
        self.shared.is_finished()
    }

    /// Cancels the transfer. Calling it again, or after the transfer has
    /// finished, has no effect.
    pub fn cancel(&self) {
        self.shared.cancel(None);
    }

    /// Takes the data of a download accepted into memory, once, after it is
    /// done; `None` for any other transfer.
    pub fn take_data(&self) -> Option<Vec<u8>> {
        self.shared.take_data()
    }
}

#[cfg(any(feature = "server", feature = "python"))]
impl TransferHandle {
    /// Blocks until the transfer has finished, up to `timeout`.
    ///
    /// # Errors
    ///
    /// The transfer's error when it failed; [`TransferError::Timeout`] when
    /// the time ran out, which leaves the transfer running.
    pub fn wait(&self, timeout: Option<Duration>) -> Result<(), TransferError> {
        if !self.shared.wait(timeout) {
            return Err(TransferError::Timeout);
        }
        match self.shared.error() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    /// Returns why the transfer failed, `None` while it has not.
    pub fn error(&self) -> Option<TransferError> {
        self.shared.error()
    }

    /// Returns where a received transfer is saved: the file, or the folder of
    /// several files. `None` for data received into memory and for sent
    /// transfers.
    pub fn path(&self) -> Option<PathBuf> {
        self.shared.path()
    }

    /// Cancels the transfer, telling the other side `reason`.
    pub fn cancel_with(&self, reason: impl Into<String>) {
        self.shared.cancel(Some(reason.into()));
    }
}

impl std::fmt::Debug for TransferHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TransferHandle")
            .field("name", &self.name())
            .field("status", &self.status())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_done_counts_complete_files() {
        let sizes = [10, 0, 5];
        assert_eq!(files_done(&sizes, 0), 0);
        assert_eq!(files_done(&sizes, 9), 0);
        assert_eq!(files_done(&sizes, 10), 1);
        assert_eq!(files_done(&sizes, 11), 2);
        assert_eq!(files_done(&sizes, 15), 3);
        assert_eq!(files_done(&[0], 0), 0);
    }

    #[test]
    fn a_finished_status_does_not_change() {
        let shared = TransferShared::new(Weak::new(), 1, 1, None);
        assert!(shared.set_status(TransferStatus::Running));
        assert!(shared.set_status(TransferStatus::Done));
        assert!(!shared.set_status(TransferStatus::Failed(TransferError::Disconnected)));
        assert_eq!(shared.status(), TransferStatus::Done);
    }
}
