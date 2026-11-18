#[cfg(test)]
use tokio::sync::mpsc::error::TryRecvError;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use crate::protocol::FastVec;

pub(crate) type SenderData = FastVec<32>;
/// A queued message and whether it must go in a frame of its own; `None`
/// closes the connection.
pub(crate) type QueuedMessage = Option<(SenderData, bool)>;
/// The receiving end of one lane.
pub(crate) type LaneReceiver = UnboundedReceiver<QueuedMessage>;

/// The receiving ends of a connection's two lanes.
///
/// The writer sends the normal lane first. The low-priority lane carries the
/// bulk states created with low priority; it is sent only while the normal
/// lane is empty, within a window of unconfirmed bytes.
pub(crate) struct MessageReceiver {
    pub(crate) normal: LaneReceiver,
    pub(crate) low: LaneReceiver,
}

impl MessageReceiver {
    /// Takes the next message of the normal lane without waiting.
    #[cfg(test)]
    pub(crate) fn try_recv(&mut self) -> Result<QueuedMessage, TryRecvError> {
        self.normal.try_recv()
    }

    /// Drops every message both lanes hold.
    pub(crate) fn clear(&mut self) {
        while self.normal.try_recv().is_ok() {}
        while self.low.try_recv().is_ok() {}
    }
}

#[derive(Clone)]
pub(crate) struct MessageSender {
    internal_sender: UnboundedSender<QueuedMessage>,
    /// The low-priority lane, for [`MessageSender::lane`].
    low: UnboundedSender<QueuedMessage>,
}
impl MessageSender {
    pub(crate) fn new() -> (Self, MessageReceiver) {
        let (sender, normal) = unbounded_channel();
        let (low_sender, low) = unbounded_channel();
        (
            Self {
                internal_sender: sender,
                low: low_sender,
            },
            MessageReceiver { normal, low },
        )
    }

    /// A sender into the low-priority lane with `low_priority`, otherwise
    /// into this sender's lane.
    pub(crate) fn lane(&self, low_priority: bool) -> Self {
        match low_priority {
            true => Self {
                internal_sender: self.low.clone(),
                low: self.low.clone(),
            },
            false => self.clone(),
        }
    }

    #[inline]
    pub(crate) fn send(&self, msg: SenderData) {
        let _ = self.internal_sender.send(Some((msg, false)));
    }

    #[inline]
    pub(crate) fn send_set(&self, msg: SenderData, single: bool) {
        let _ = self.internal_sender.send(Some((msg, single)));
    }

    #[inline]
    pub(crate) fn close(&self) {
        let _ = self.internal_sender.send(None);
    }
}
