//! The server's side of browser downloads.
//!
//! A browser cannot write to disk, so a browser client accepts a download
//! as one or more HTTP downloads that its download manager saves. For each
//! file, or for the zip of all files, the server sends a one-time ticket over
//! the WebSocket; the browser fetches `DOWNLOAD_PATH` plus the ticket from the
//! server's port, and the HTTP response streams the data.

use std::time::{Duration, Instant};

use super::*;
use crate::protocol::file_transfer::FileEntry;

/// A ticket the browser does not fetch within this time fails the transfer,
/// for example when the browser blocked the download.
const TICKET_LIFETIME: Duration = Duration::from_secs(30);

/// At most this often the client hears how far a download is.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

pub(super) struct HttpState {
    /// One zip of all files instead of one download per file.
    bundled: bool,
    /// The file the next ticket fetches.
    next_file: usize,
    /// The ticket the browser has not fetched yet, and when it was issued.
    ticket: Option<([u8; 16], Instant)>,
    /// Whether a response is streaming.
    serving: bool,
    last_progress: Instant,
}

/// One HTTP response to stream.
pub(crate) struct HttpJob {
    pub(crate) endpoint: u64,
    pub(crate) serial: u32,
    /// The file of the response, or every file for a zip.
    pub(crate) files: Vec<FileEntry>,
    pub(crate) bundled: bool,
    /// The name the browser saves the response under.
    pub(crate) name: String,
    /// Where the data comes from; it returns to the transfer at the end.
    pub(crate) source: Source,
    /// Bytes of earlier responses of the same transfer.
    pub(crate) offset: u64,
}

impl Engine {
    /// Whether this peer sends on any endpoint, so browser downloads may
    /// arrive on its port.
    pub(crate) fn has_downloads(&self) -> bool {
        self.0
            .state
            .lock()
            .endpoints
            .values()
            .any(|ep| ep.role == Role::Send)
    }

    pub(super) fn start_http(
        &self,
        state: &mut State,
        endpoint: u64,
        serial: u32,
        bundled: bool,
        fx: &mut Effects,
    ) {
        let Some(t) = Self::current_mut(state, endpoint, serial) else {
            return;
        };
        t.phase = Phase::Http;
        t.http = Some(HttpState {
            bundled: bundled && t.body.files.len() > 1,
            next_file: 0,
            ticket: None,
            serving: false,
            last_progress: Instant::now(),
        });
        self.issue_ticket(state, endpoint, serial, fx);
    }

    fn issue_ticket(&self, state: &mut State, endpoint: u64, serial: u32, fx: &mut Effects) {
        let mut ticket = [0u8; 16];
        if let Err(error) = getrandom::fill(&mut ticket) {
            let error = TransferError::Io(format!("no random ticket: {error}"));
            self.fail_locked(state, endpoint, serial, error, fx);
            return;
        }
        let Some(http) = Self::current_mut(state, endpoint, serial).and_then(|t| t.http.as_mut())
        else {
            return;
        };
        http.ticket = Some((ticket, Instant::now()));
        let file = if http.bundled {
            u32::MAX
        } else {
            http.next_file as u32
        };
        self.emit(
            state,
            endpoint,
            &TransferFrame::Ticket {
                serial,
                file,
                ticket,
            },
            &[],
        );
    }

    /// Hands out the response for `ticket`, once and only before it expired.
    pub(crate) fn claim_ticket(&self, ticket: &[u8; 16]) -> Option<HttpJob> {
        let mut state = self.0.state.lock();
        let (endpoint, serial) = state.endpoints.iter().find_map(|(id, ep)| {
            let t = ep.current.as_ref()?;
            let (issued, at) = t.http.as_ref()?.ticket?;
            (t.phase == Phase::Http && issued == *ticket && at.elapsed() < TICKET_LIFETIME)
                .then_some((*id, t.serial))
        })?;
        let t = Self::current_mut(&mut state, endpoint, serial)?;
        let source = t.source.take()?;
        let http = t.http.as_mut()?;
        http.ticket = None;
        http.serving = true;
        let (files, name) = if http.bundled {
            (t.body.files.clone(), zip_name(&t.body.name))
        } else {
            let file = t.body.files.get(http.next_file)?.clone();
            let name = names::flatten(&file.name);
            (vec![file], name)
        };
        t.shared.set_status(TransferStatus::Running);
        Some(HttpJob {
            endpoint,
            serial,
            files,
            bundled: http.bundled,
            name,
            source,
            offset: t.sent,
        })
    }

    /// Records that a response has sent `done` bytes of the transfer in
    /// total; `false` when the transfer was cancelled meanwhile.
    pub(crate) fn http_progress(&self, endpoint: u64, serial: u32, done: u64) -> bool {
        let mut state = self.0.state.lock();
        let Some(t) =
            Self::current_mut(&mut state, endpoint, serial).filter(|t| t.phase == Phase::Http)
        else {
            return false;
        };
        t.sent = done;
        t.shared.set_done_bytes(done);
        let Some(http) = t.http.as_mut() else {
            return false;
        };
        if http.last_progress.elapsed() >= PROGRESS_INTERVAL {
            http.last_progress = Instant::now();
            let files_done = t.shared.progress().files_done;
            self.emit(
                &state,
                endpoint,
                &TransferFrame::HttpProgress {
                    serial,
                    done,
                    files_done,
                },
                &[],
            );
        }
        true
    }

    /// Ends a response that sent everything: the next file gets its ticket,
    /// or the transfer is done.
    pub(crate) fn http_finished(&self, job: HttpJob) {
        let mut fx = Effects::default();
        {
            let mut state = self.0.state.lock();
            let (endpoint, serial) = (job.endpoint, job.serial);
            let Some(t) =
                Self::current_mut(&mut state, endpoint, serial).filter(|t| t.phase == Phase::Http)
            else {
                return;
            };
            let files = t.body.files.len();
            let Some(http) = t.http.as_mut() else {
                return;
            };
            http.serving = false;
            let last = http.bundled || http.next_file + 1 >= files;
            if last {
                let done = t.total;
                let files_done = files as u32;
                self.emit(
                    &state,
                    endpoint,
                    &TransferFrame::HttpProgress {
                        serial,
                        done,
                        files_done,
                    },
                    &[],
                );
                self.emit(
                    &state,
                    endpoint,
                    &TransferFrame::Finished {
                        serial,
                        error: None,
                    },
                    &[],
                );
                Self::finish_locked(&mut state, endpoint, TransferStatus::Done, &mut fx);
            } else {
                http.next_file += 1;
                t.source = Some(job.source);
                self.issue_ticket(&mut state, endpoint, serial, &mut fx);
            }
        }
        self.apply(fx);
    }

    /// Fails the transfer of a response that could not be sent.
    pub(crate) fn http_failed(&self, endpoint: u64, serial: u32, error: TransferError) {
        let mut fx = Effects::default();
        {
            let mut state = self.0.state.lock();
            if Self::current_mut(&mut state, endpoint, serial).is_some() {
                self.fail_locked(&mut state, endpoint, serial, error, &mut fx);
            }
        }
        self.apply(fx);
    }

    /// Ends a transfer whose download the browser stopped, for example in its
    /// download list; both sides see it as cancelled by the other.
    pub(crate) fn http_cancelled(&self, endpoint: u64, serial: u32, reason: &str) {
        let mut fx = Effects::default();
        {
            let mut state = self.0.state.lock();
            if Self::current_mut(&mut state, endpoint, serial).is_some() {
                self.emit(
                    &state,
                    endpoint,
                    &TransferFrame::Cancel { serial },
                    reason.as_bytes(),
                );
                let status = TransferStatus::Failed(TransferError::Cancelled {
                    by_peer: true,
                    reason: Some(reason.to_string()),
                });
                Self::finish_locked(&mut state, endpoint, status, &mut fx);
            }
        }
        self.apply(fx);
    }

    /// Fails the transfers whose ticket the browser did not fetch in time.
    pub(crate) fn expire_tickets(&self) {
        let mut fx = Effects::default();
        {
            let mut state = self.0.state.lock();
            let expired: Vec<(u64, u32)> = state
                .endpoints
                .iter()
                .filter_map(|(id, ep)| {
                    let t = ep.current.as_ref()?;
                    let (_, at) = t.http.as_ref()?.ticket?;
                    (at.elapsed() >= TICKET_LIFETIME).then_some((*id, t.serial))
                })
                .collect();
            for (endpoint, serial) in expired {
                let error =
                    TransferError::Io("the browser did not start the download in time".to_string());
                self.fail_locked(&mut state, endpoint, serial, error, &mut fx);
            }
        }
        self.apply(fx);
    }
}
