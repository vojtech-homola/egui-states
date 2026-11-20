//! The client handles of the state kinds.

pub(crate) mod atomics;
pub(crate) mod data;
pub(crate) mod data_take;
pub(crate) mod file_transfer;
pub(crate) mod image;
pub(crate) mod initial_value;
pub(crate) mod map;
pub(crate) mod values;
pub(crate) mod vec;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Whether the server changed a state since the UI last took the flag.
///
/// The clones of a state handle share it. The client sets it after the change
/// is visible to readers, and the UI takes it before reading, so a change is
/// never missed: at worst a change that lands between the two is reported
/// once more.
#[derive(Clone, Default)]
pub(crate) struct ChangedFlag(Arc<AtomicBool>);

impl ChangedFlag {
    /// Records a server change that readers can already see.
    pub(crate) fn mark(&self) {
        self.0.store(true, Ordering::Release);
    }

    /// Tells whether the state changed since the last call, and clears the
    /// flag.
    pub(crate) fn take(&self) -> bool {
        self.0.swap(false, Ordering::AcqRel)
    }
}
