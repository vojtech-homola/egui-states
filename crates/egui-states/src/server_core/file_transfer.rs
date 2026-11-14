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

use std::collections::VecDeque;
use std::sync::Arc;

use bytes::Bytes;
use parking_lot::Mutex;

use crate::file_transfer::engine::EngineEvent;
use crate::file_transfer::shared::TransferShared;
use crate::server_core::signals::SignalsManager;

/// Kind of a transfer event in the endpoint's signal queue.
pub(crate) const EVENT_OFFER: u8 = 0;
pub(crate) const EVENT_FINISHED: u8 = 1;

/// Finished transfers kept until their callbacks have run. The newest win
/// when more pile up than this, which only a callback that never returns
/// would cause.
const KEPT: usize = 1024;

/// Delivers the engine's events through the endpoints' signal queues, so the
/// callback workers of every front-end run them.
#[derive(Default)]
pub(crate) struct TransferEvents {
    finished: Mutex<VecDeque<(u64, Arc<TransferShared>)>>,
}

impl TransferEvents {
    pub(crate) fn record(&self, event: EngineEvent, signals: &SignalsManager) {
        let (endpoint, kind, key) = match event {
            EngineEvent::Offer { endpoint } => (endpoint, EVENT_OFFER, 0),
            EngineEvent::Finished { endpoint, handle } => {
                if !signals.is_registered(endpoint) {
                    return;
                }
                let key = handle.key();
                let mut finished = self.finished.lock();
                if finished.len() == KEPT {
                    finished.pop_front();
                }
                finished.push_back((key, handle));
                (endpoint, EVENT_FINISHED, key)
            }
        };
        if let Ok(payload) = postcard::to_stdvec(&(kind, key)) {
            signals.set(endpoint, Bytes::from(payload));
        }
    }

    /// The finished transfer an event names.
    pub(crate) fn finished(&self, key: u64) -> Option<Arc<TransferShared>> {
        self.finished
            .lock()
            .iter()
            .find(|(entry, _)| *entry == key)
            .map(|(_, handle)| handle.clone())
    }
}

/// Decodes the payload of a transfer event.
pub(crate) fn decode_event(payload: &[u8]) -> Option<(u8, u64)> {
    postcard::from_bytes(payload).ok()
}
