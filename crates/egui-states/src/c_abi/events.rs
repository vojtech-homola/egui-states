//! Owned client change events and the claims they hold.

use bytes::Bytes;

use crate::server_core::signals::SignalsManager;

use super::error::{Error, Result};
use super::reader::Reader;
use super::tree::TypeRef;

/// A claimed change of one subscribed state.
///
/// While the event is alive, no other event of the same state is delivered;
/// dropping it releases the claim so the next change can be claimed. An event
/// stays valid after its server is dropped.
pub struct ChangeEvent {
    signals: SignalsManager,
    id: u64,
    value: Bytes,
    previous: Option<Bytes>,
    value_type: TypeRef,
}

impl ChangeEvent {
    pub(crate) fn new(
        signals: SignalsManager,
        id: u64,
        value: Bytes,
        previous: Option<Bytes>,
        value_type: TypeRef,
    ) -> Self {
        Self {
            signals,
            id,
            value,
            previous,
            value_type,
        }
    }

    /// Protocol id of the changed state.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Loads the new value into `reader`.
    pub fn load_value(&self, reader: &mut Reader) {
        reader.load(self.value.clone(), &self.value_type);
    }

    /// Returns whether the event carries the value it replaced.
    ///
    /// Only `Value` states subscribed with the previous value carry one. In
    /// single mode, coalesced changes report the value last delivered: for
    /// `a -> b -> c` delivered as one event, the previous value is `a`.
    pub fn has_previous(&self) -> bool {
        self.previous.is_some()
    }

    /// Loads the replaced value into `reader`.
    pub fn load_previous(&self, reader: &mut Reader) -> Result<()> {
        match &self.previous {
            Some(previous) => {
                reader.load(previous.clone(), &self.value_type);
                Ok(())
            }
            None => Err(Error::not_found("the event carries no previous value")),
        }
    }
}

impl std::fmt::Debug for ChangeEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChangeEvent")
            .field("id", &self.id)
            .field("has_previous", &self.has_previous())
            .finish_non_exhaustive()
    }
}

impl Drop for ChangeEvent {
    fn drop(&mut self) {
        self.signals.release(self.id);
    }
}
