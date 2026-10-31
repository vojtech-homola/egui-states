use parking_lot::RwLock;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bytes::Bytes;

use crate::event::Event;
use crate::serialization::{ServerHeader, check_value_size};
use crate::server_core::sender::MessageSender;
use crate::server_core::server::{Acknowledge, SyncTrait};
use crate::server_core::signals::SignalsManager;

// Value --------------------------------------------------
pub(crate) struct ValueCore {
    pub(crate) name: String,
    id: u64,
    type_id: u32,
    value: RwLock<(Bytes, usize)>,
    sender: MessageSender,
    connected: Arc<AtomicBool>,
    signals: SignalsManager,
}

impl ValueCore {
    pub(crate) fn new(
        name: String,
        id: u64,
        type_id: u32,
        value: Bytes,
        sender: MessageSender,
        connected: Arc<AtomicBool>,
        signals: SignalsManager,
    ) -> Arc<Self> {
        Arc::new(Self {
            name,
            id,
            type_id,
            value: RwLock::new((value, 0)),
            sender,
            connected,
            signals,
        })
    }

    #[inline]
    fn store(&self, slot: &mut Bytes, value: Bytes, set_signals: bool) {
        if set_signals {
            let previous = std::mem::replace(slot, value.clone());
            self.signals.set_with_previous(self.id, value, previous);
        } else {
            *slot = value;
        }
    }

    pub(crate) fn update_value(
        &self,
        type_id: u32,
        signal: bool,
        value: Bytes,
    ) -> Result<(), String> {
        if type_id != self.type_id {
            return Err(format!("Type id mismatch for Value: {}", self.name));
        }

        let mut w = self.value.write();
        if w.1 == 0 {
            self.store(&mut w.0, value, signal);
        }

        Ok(())
    }

    #[inline]
    pub(crate) fn get(&self) -> Bytes {
        self.value.read().0.clone()
    }

    pub(crate) fn set(&self, value: Bytes, set_signals: bool, update: bool) -> Result<(), String> {
        // checked even when disconnected, otherwise the value would be sent by sync()
        check_value_size(&self.name, value.len())?;

        if self.connected.load(Ordering::Acquire) {
            let message = ServerHeader::serialize_value(self.id, self.type_id, update, &value)?;
            let mut w = self.value.write();

            w.1 += 1;
            self.sender.send(message);
            // Stored last, but still before the lock is released, so the ordering the
            // client and the signal consumers observe is unchanged: message first.
            self.store(&mut w.0, value, set_signals);
        } else {
            let mut w = self.value.write();
            self.store(&mut w.0, value, set_signals);
        }
        Ok(())
    }
}

impl Acknowledge for ValueCore {
    fn acknowledge(&self) {
        let mut w = self.value.write();
        if w.1 > 0 {
            w.1 -= 1;
        }
    }

    fn reset(&self) {
        // Nothing in flight survives a disconnect.
        self.value.write().1 = 0;
    }
}

impl SyncTrait for ValueCore {
    fn sync(&self) -> Result<(), ()> {
        let mut w = self.value.write();
        w.1 = 1;
        let data =
            ServerHeader::serialize_value(self.id, self.type_id, false, &w.0).map_err(|_| ())?;
        drop(w);

        self.sender.send(data);
        Ok(())
    }
}

// ValueTake --------------------------------------------------
pub(crate) struct ValueTakeCore {
    // #[cfg_attr(not(feature = "python"), allow(dead_code))]
    pub(crate) name: String,
    id: u64,
    type_id: u32,
    event: Event,
    lock: RwLock<()>,
    sender: MessageSender,
    connected: Arc<AtomicBool>,
}

impl ValueTakeCore {
    pub(crate) fn new(
        name: String,
        id: u64,
        type_id: u32,
        sender: MessageSender,
        connected: Arc<AtomicBool>,
    ) -> Arc<Self> {
        Arc::new(Self {
            name,
            id,
            type_id,
            event: Event::new(),
            lock: RwLock::new(()),
            sender,
            connected,
        })
    }

    pub(crate) fn set(&self, value: Bytes, blocking: bool, update: bool) -> Result<(), String> {
        check_value_size(&self.name, value.len())?;

        if self.connected.load(Ordering::Acquire) {
            let message = ServerHeader::serialize_value_take(
                self.id,
                self.type_id,
                blocking,
                update,
                &value,
            )?;

            let _guard = self.lock.write();

            match blocking {
                true => self.event.wait_clear(),
                false => self.event.wait(),
            }
            if !self.connected.load(Ordering::Acquire) {
                return Ok(());
            }

            self.sender.send(message);
        }
        Ok(())
    }
}

impl Acknowledge for ValueTakeCore {
    fn acknowledge(&self) {
        self.event.set();
    }
}

impl SyncTrait for ValueTakeCore {
    fn sync(&self) -> Result<(), ()> {
        self.event.set();
        Ok(())
    }
}

// ValueStatic --------------------------------------------
pub(crate) struct ValueStaticCore {
    // #[cfg_attr(not(feature = "python"), allow(dead_code))]
    pub(crate) name: String,
    id: u64,
    type_id: u32,
    value: RwLock<Bytes>,
    sender: MessageSender,
    connected: Arc<AtomicBool>,
}

impl ValueStaticCore {
    pub(crate) fn new(
        name: String,
        id: u64,
        type_id: u32,
        value: Bytes,
        sender: MessageSender,
        connected: Arc<AtomicBool>,
    ) -> Arc<Self> {
        Arc::new(Self {
            name,
            id,
            type_id,
            value: RwLock::new(value),
            sender,
            connected,
        })
    }

    pub(crate) fn set(&self, value: Bytes, update: bool) -> Result<(), String> {
        // checked even when disconnected, otherwise the value would be sent by sync()
        check_value_size(&self.name, value.len())?;

        if self.connected.load(Ordering::Acquire) {
            let message = ServerHeader::serialize_static(self.id, self.type_id, update, &value)?;
            let mut w = self.value.write();

            *w = value;
            self.sender.send(message);
        } else {
            let mut w = self.value.write();
            *w = value;
        }
        Ok(())
    }

    #[inline]
    pub(crate) fn get(&self) -> Bytes {
        self.value.read().clone()
    }
}

impl SyncTrait for ValueStaticCore {
    fn sync(&self) -> Result<(), ()> {
        let w = self.value.read();
        let data =
            ServerHeader::serialize_static(self.id, self.type_id, false, &w).map_err(|_| ())?;
        drop(w);

        self.sender.send(data);
        Ok(())
    }
}

// Signals --------------------------------------------
pub(crate) struct SignalCore {
    pub(crate) name: String,
    id: u64,
    type_id: u32,
    signals: SignalsManager,
}

impl SignalCore {
    pub(crate) fn new(name: String, id: u64, type_id: u32, signals: SignalsManager) -> Arc<Self> {
        Arc::new(Self {
            name,
            id,
            type_id,
            signals,
        })
    }

    pub(crate) fn set(&self, value: Bytes) {
        self.signals.set(self.id, value);
    }

    pub(crate) fn update_signal(&self, type_id: u32, value: Bytes) -> Result<(), String> {
        if type_id != self.type_id {
            return Err(format!("Type id mismatch for Signal: {}", self.name));
        }

        self.signals.set(self.id, value);
        Ok(())
    }
}

#[cfg(all(test, feature = "server"))]
mod tests {
    use super::*;
    use crate::serialization::VALUE_MAX_SIZE;
    use crate::server_core::sender::MessageReceiver;

    const ID: u64 = 40;
    const TYPE_ID: u32 = 7;

    fn b(value: &str) -> Bytes {
        Bytes::copy_from_slice(value.as_bytes())
    }

    fn value(connected: bool) -> (Arc<ValueCore>, SignalsManager, MessageReceiver) {
        let (sender, receiver) = MessageSender::new();
        let signals = SignalsManager::new();
        signals.set_register(ID, true, true);
        let value = ValueCore::new(
            "root.value".into(),
            ID,
            TYPE_ID,
            b("initial"),
            sender,
            Arc::new(AtomicBool::new(connected)),
            signals.clone(),
        );
        (value, signals, receiver)
    }

    /// Returns the header and payload of the next queued server message.
    fn next_message(receiver: &mut MessageReceiver) -> (ServerHeader, Vec<u8>) {
        let (message, _) = receiver
            .try_recv()
            .expect("expected a queued message")
            .expect("expected a message, not a close request");
        let bytes = message.into_bytes();
        let (header, payload) = postcard::take_from_bytes::<ServerHeader>(&bytes).unwrap();
        (header, payload.to_vec())
    }

    #[test]
    fn client_update_is_ignored_while_a_server_send_is_unacknowledged() {
        let (value, signals, mut receiver) = value(true);
        value.set(b("server"), false, false).unwrap();
        assert!(matches!(
            next_message(&mut receiver),
            (ServerHeader::Value(ID, TYPE_ID, false, 6), payload) if payload == b"server"
        ));

        // The client has not yet seen "server", so its concurrent write must lose:
        // the client applies "server" when it arrives, and both peers converge.
        value.update_value(TYPE_ID, true, b("client")).unwrap();
        assert_eq!(value.get(), b("server"));
        assert!(
            signals.try_changed_value(None).is_none(),
            "an ignored client write must not signal callbacks"
        );

        value.acknowledge();
        value.update_value(TYPE_ID, true, b("client")).unwrap();
        assert_eq!(value.get(), b("client"));
        assert_eq!(
            signals.try_changed_value(None),
            Some((ID, b("client"), Some(b("server"))))
        );
    }

    #[test]
    fn every_unacknowledged_send_needs_its_own_acknowledgement() {
        let (value, _signals, mut receiver) = value(true);
        value.set(b("first"), false, false).unwrap();
        value.set(b("second"), false, false).unwrap();
        next_message(&mut receiver);
        next_message(&mut receiver);

        value.acknowledge();
        value.update_value(TYPE_ID, false, b("client")).unwrap();
        assert_eq!(value.get(), b("second"), "one send is still in flight");

        value.acknowledge();
        value.update_value(TYPE_ID, false, b("client")).unwrap();
        assert_eq!(value.get(), b("client"));

        // Surplus acknowledgements must not make the counter wrap.
        value.acknowledge();
        value.update_value(TYPE_ID, false, b("again")).unwrap();
        assert_eq!(value.get(), b("again"));
    }

    #[test]
    fn reset_discards_every_unacknowledged_send() {
        let (value, _signals, mut receiver) = value(true);
        value.set(b("first"), false, false).unwrap();
        value.set(b("second"), false, false).unwrap();
        next_message(&mut receiver);
        next_message(&mut receiver);

        value.reset();
        value.update_value(TYPE_ID, false, b("client")).unwrap();
        assert_eq!(value.get(), b("client"));
    }

    #[test]
    fn connection_sync_is_pending_until_acknowledged() {
        let (value, _signals, mut receiver) = value(true);
        value.sync().unwrap();
        assert!(matches!(
            next_message(&mut receiver),
            (ServerHeader::Value(ID, TYPE_ID, false, 7), payload) if payload == b"initial"
        ));

        value.update_value(TYPE_ID, false, b("client")).unwrap();
        assert_eq!(value.get(), b("initial"));
        value.acknowledge();
        value.update_value(TYPE_ID, false, b("client")).unwrap();
        assert_eq!(value.get(), b("client"));
    }

    #[test]
    fn disconnected_server_writes_are_stored_without_sending_or_blocking_clients() {
        let (value, _signals, mut receiver) = value(false);
        value.set(b("offline"), false, false).unwrap();
        assert!(receiver.try_recv().is_err());
        value.update_value(TYPE_ID, false, b("client")).unwrap();
        assert_eq!(value.get(), b("client"));
    }

    #[test]
    fn mismatched_client_type_is_rejected_without_changing_the_value() {
        let (value, signals, _receiver) = value(true);
        let error = value
            .update_value(TYPE_ID + 1, true, b("client"))
            .unwrap_err();
        assert!(error.contains("root.value"), "{error}");
        assert_eq!(value.get(), b("initial"));
        assert!(signals.try_changed_value(None).is_none());
    }

    fn value_take(connected: bool) -> (Arc<ValueTakeCore>, MessageReceiver) {
        let (sender, receiver) = MessageSender::new();
        let value = ValueTakeCore::new(
            "root.take".into(),
            41,
            TYPE_ID,
            sender,
            Arc::new(AtomicBool::new(connected)),
        );
        (value, receiver)
    }

    #[test]
    fn oversized_value_take_is_rejected() {
        let (value, mut receiver) = value_take(true);
        value.sync().unwrap();
        let error = value
            .set(Bytes::from(vec![0; VALUE_MAX_SIZE + 1]), false, false)
            .unwrap_err();
        assert!(error.contains("root.take"), "{error}");
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn disconnected_blocking_value_take_returns_without_sending() {
        let (value, mut receiver) = value_take(false);
        // A regression would block forever, so bound it from another thread.
        let (done, finished) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result = value.set(b("payload"), true, false);
            let _ = done.send(result);
        });
        let result = finished
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("blocking ValueTake::set waited without a client");
        assert!(result.is_ok());
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn disconnected_static_is_sent_by_the_next_sync() {
        let (sender, mut receiver) = MessageSender::new();
        let connected = Arc::new(AtomicBool::new(false));
        let value = ValueStaticCore::new(
            "root.static".into(),
            42,
            TYPE_ID,
            b("initial"),
            sender,
            connected.clone(),
        );
        value.set(b("latest"), true).unwrap();
        assert!(receiver.try_recv().is_err());

        connected.store(true, Ordering::Release);
        value.sync().unwrap();
        assert!(matches!(
            next_message(&mut receiver),
            (ServerHeader::Static(42, TYPE_ID, false, 6), payload) if payload == b"latest"
        ));
    }
}
