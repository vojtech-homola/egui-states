//! The server's file transfer endpoints.
//!
//! The endpoints are registered like states, so their ids are unique and the
//! front-ends find them by kind, but they are never synchronized or
//! acknowledged: the transfer engine carries them.

/// A registered `Download` or `Upload` endpoint.
pub(crate) struct TransferEndpoint {
    #[cfg_attr(not(feature = "python"), allow(dead_code))]
    pub(crate) name: String,
}

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;

use crate::file_transfer::TransferError;
use crate::file_transfer::engine::{Engine, EngineEvent};
use crate::file_transfer::fs::{FilesSource, file_name, stat};
use crate::file_transfer::shared::TransferShared;
use crate::file_transfer::source::Source;
use crate::protocol::file_transfer::{FileEntry, OfferBody};
use crate::server_core::signals::SignalsManager;

/// Kind of a transfer event in the endpoint's signal queue; the event's
/// payload is this one byte.
pub(crate) const EVENT_OFFER: u8 = 0;
pub(crate) const EVENT_FINISHED: u8 = 1;

/// Delivers an engine event through the endpoint's signal queue, so the
/// callback workers of every front-end run it.
///
/// A finished transfer travels with its event: the queue releases it once the
/// event's callbacks have run, or with the event when the endpoint has no
/// callback, so no front-end has to release it.
pub(crate) fn record_event(event: EngineEvent, signals: &SignalsManager) {
    match event {
        EngineEvent::Offer { endpoint } => {
            signals.set(endpoint, Bytes::from_static(&[EVENT_OFFER]));
        }
        EngineEvent::Finished { endpoint, handle } => {
            signals.set_attached(endpoint, Bytes::from_static(&[EVENT_FINISHED]), handle);
        }
    }
}

/// The finished transfer of the event the callbacks of `endpoint` are
/// running for; only a callback of the endpoint may ask.
pub(crate) fn finished_transfer(
    signals: &SignalsManager,
    endpoint: u64,
) -> Option<Arc<TransferShared>> {
    signals.attachment(endpoint)?.downcast().ok()
}

/// Sends an offer; the front-ends share the rules for `wait` and `timeout`.
pub(crate) fn send(
    engine: &Engine,
    endpoint: u64,
    body: OfferBody,
    meta: Bytes,
    source: Source,
    wait: bool,
    timeout: Option<Duration>,
) -> Result<Arc<TransferShared>, TransferError> {
    if timeout.is_some() && !wait {
        return Err(TransferError::InvalidInput(
            "a timeout needs wait".to_string(),
        ));
    }
    if wait {
        engine.send_wait(endpoint, body, meta, source, timeout)
    } else {
        engine.send(endpoint, body, meta, source)
    }
}

/// The offer of the file at `path`, under its own name or `name`.
pub(crate) fn file_offer(
    path: &Path,
    name: Option<&str>,
) -> Result<(OfferBody, Source), TransferError> {
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
    Ok((body, source))
}

/// The offer of several files as one transfer named `name`.
pub(crate) fn files_offer(
    paths: &[PathBuf],
    name: &str,
    root: Option<&Path>,
) -> Result<(OfferBody, Source), TransferError> {
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
    Ok((body, Source::Files(FilesSource::new(sources))))
}

/// The offer of `data` as one file named `name`.
pub(crate) fn bytes_offer(data: Vec<u8>, name: &str) -> (OfferBody, Source) {
    let body = OfferBody {
        name: name.to_string(),
        files: vec![FileEntry {
            name: name.to_string(),
            size: data.len() as u64,
            modified: None,
        }],
    };
    (body, Source::bytes(data))
}

/// The name of `path` relative to `root`, with `/` between folders.
fn relative_name(path: &Path, root: &Path) -> Result<String, TransferError> {
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
