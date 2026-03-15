use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::marker::PhantomData;
use std::sync::Arc;
use tokio_tungstenite::tungstenite::http::header;

use crate::client::atomics::{Atomic, AtomicLock, AtomicLockStatic, AtomicStatic};
use crate::client::sender::{ChannelMessage, MessageSender};
use crate::serialization::{deserialize, to_message};

pub(crate) enum UpdateDataMessage {
    Header
}

pub(crate) trait UpdateData: Sync + Send {
    fn update_header(&self, data_size: usize, data: &[u8]) -> Result<(), String>;
    fn update_data(&self, data: &[u8]) -> Result<(), String>;
}

pub struct Data<T> {
    name: String,
    id: u64,
    inner: Arc<RwLock<(T, Vec<u8>)>>,
    buffer: Arc<RwLock<Option<(T, Vec<u8>)>>>,
    sender: MessageSender,
}

impl<T> Data<T>
where
    T: Serialize + Clone,
{
    pub(crate) fn new(name: String, id: u64, header: T, sender: MessageSender) -> Self {
        Self {
            name,
            id,
            inner: Arc::new(RwLock::new((header, Vec::new()))),
            buffer: Arc::new(RwLock::new(None)),
            sender,
        }
    }

    pub fn get(&self) -> (T, Vec<u8>) {
        let inner = self.inner.read();
        (inner.0.clone(), inner.1.clone())
    }

    pub fn read<R>(&self, f: impl Fn((&T, &Vec<u8>)) -> R) -> R {
        let inner = self.inner.read();
        f((&inner.0, &inner.1))
    }

    pub fn set(&self, header: T, data: Vec<u8>) {
        let header_data = to_message(&header);
        let data_copy = data.clone();

        let mut inner = self.inner.write();
        self.sender
            .send(ChannelMessage::Data(self.id, false, header_data, data_copy));

        *inner = (header, data);
    }

    pub fn set_signal(&self, header: T, data: Vec<u8>) {
        let header_data = to_message(&header);
        let data_copy = data.clone();

        let mut inner = self.inner.write();
        self.sender
            .send(ChannelMessage::Data(self.id, true, header_data, data_copy));

        *inner = (header, data);
    }

    pub fn write<R>(&self, f: impl Fn((&mut T, &mut Vec<u8>)) -> R) -> R {
        let mut inner = self.inner.write();

        let (header, data) = &mut *inner;
        let result = f((header, data));

        let header_data = to_message(&inner.0);
        let data_copy = inner.1.clone();
        self.sender
            .send(ChannelMessage::Data(self.id, false, header_data, data_copy));

        result
    }

    pub fn write_signal<R>(&self, f: impl Fn((&mut T, &mut Vec<u8>)) -> R) -> R {
        let mut inner = self.inner.write();

        let (header, data) = &mut *inner;
        let result = f((header, data));

        let header_data = to_message(&inner.0);
        let data_copy = inner.1.clone();
        self.sender
            .send(ChannelMessage::Data(self.id, true, header_data, data_copy));

        result
    }
}
